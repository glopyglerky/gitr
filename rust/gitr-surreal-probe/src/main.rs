//! Isolated storage qualification; never point this at a production datastore.
use anyhow::{Context, Result, bail, ensure};
use std::{io::{self, Write}, path::Path, sync::Arc};
use surrealdb::{Surreal, engine::any::{Any, connect}, types::{QueryError, SurrealValue}};
use tokio::{sync::Barrier, task::JoinSet};

#[derive(Debug, SurrealValue)]
struct RefState { oid: String, generation: i64 }

async fn open(path: &Path) -> surrealdb::Result<Surreal<Any>> {
    let endpoint = format!("surrealkv://{}?sync=every", path.display());
    let db = connect(endpoint).await?;
    db.use_ns("gitr_qualification").use_db("contracts").await?;
    Ok(db)
}

fn conflict(error: &surrealdb::Error) -> bool {
    let mut next = Some(error);
    for _ in 0..32 {
        let Some(error) = next else { return false; };
        if matches!(error.query_details(), Some(QueryError::TransactionConflict)) { return true; }
        next = error.cause();
    }
    false
}

async fn seed(db: &Surreal<Any>) -> Result<()> {
    db.query(include_str!("../schema.surql")).await?.check()?;
    db.query("CREATE gitr_ref:master SET oid = $oid, generation = 0")
        .bind(("oid", "0".repeat(40))).await?.check()?;
    let gate = Arc::new(Barrier::new(8));
    let mut workers = JoinSet::new();
    for candidate in 1..=8 {
        let db = db.clone();
        let gate = gate.clone();
        workers.spawn(async move {
            gate.wait().await;
            let result = db.query("UPDATE gitr_ref:master SET oid = $new, generation = 1 WHERE generation = 0 AND oid = $old RETURN AFTER")
                .bind(("new", format!("{candidate:040x}")))
                .bind(("old", "0".repeat(40)))
                .await.and_then(|response| response.check());
            match result {
                Ok(mut response) => {
                    let rows: Vec<RefState> = response.take(0)?;
                    ensure!(rows.len() <= 1, "CAS returned duplicate records");
                    Ok::<bool, anyhow::Error>(!rows.is_empty())
                }
                Err(error) if conflict(&error) => Ok(false),
                Err(error) => Err(error.into()),
            }
        });
    }
    let mut winners = 0;
    while let Some(result) = workers.join_next().await { winners += usize::from(result??); }
    ensure!(winners == 1, "competing CAS writes did not have exactly one winner: {winners}");
    let stale: Vec<RefState> = db.query("UPDATE gitr_ref:master SET oid = $new WHERE generation = 0 RETURN AFTER")
        .bind(("new", "f".repeat(40))).await?.check()?.take(0)?;
    ensure!(stale.is_empty(), "stale writer changed authoritative state");

    db.query("BEGIN; CREATE artifact:pass SET payload = 'checked'; CREATE receipt:pass SET status = 'passed', artifact = artifact:pass; COMMIT;")
        .await?.check()?;
    let failure = db.query("BEGIN; CREATE artifact:rolled_back SET payload = 'partial'; CREATE receipt:rolled_back SET status = 'passed'; THROW 'intentional qualification abort'; COMMIT;")
        .await?.check();
    ensure!(failure.is_err(), "explicit aborted transaction was accepted");
    let rolled_back: Vec<String> = db.query("SELECT VALUE payload FROM artifact:rolled_back").await?.check()?.take(0)?;
    ensure!(rolled_back.is_empty(), "aborted artifact was published");
    let rolled_back: Vec<String> = db.query("SELECT VALUE status FROM receipt:rolled_back").await?.check()?.take(0)?;
    ensure!(rolled_back.is_empty(), "aborted receipt was published");

    db.query("CREATE code_unit:reader SET name = 'reader', body = 'atomic publication', embedding = [1.0, 0.0, 0.0]; CREATE code_unit:writer SET name = 'writer', body = 'file transfer', embedding = [0.0, 1.0, 0.0]; RELATE code_unit:reader->calls->code_unit:writer;")
        .await?.check()?;
    verify(db, false).await?;
    println!("seed: CAS=one-winner rollback=atomic graph=passed fulltext=passed vector=passed");
    Ok(())
}

async fn verify(db: &Surreal<Any>, acknowledged: bool) -> Result<()> {
    let refs: Vec<RefState> = db.query("SELECT * FROM gitr_ref:master").await?.check()?.take(0)?;
    ensure!(refs.len() == 1 && refs[0].generation == 1 && refs[0].oid != "0".repeat(40), "ref receipt was lost");
    let payloads: Vec<String> = db.query("SELECT VALUE artifact.payload FROM receipt:pass").await?.check()?.take(0)?;
    ensure!(payloads == ["checked"], "artifact and receipt did not persist together");
    let targets: Vec<String> = db.query("SELECT VALUE out.name FROM calls WHERE in = code_unit:reader").await?.check()?.take(0)?;
    ensure!(targets == ["writer"], "graph relationship mismatch: {targets:?}");
    let lexical: Vec<String> = db.query("SELECT VALUE name FROM code_unit WHERE body @0@ 'atomic'").await?.check()?.take(0)?;
    ensure!(lexical == ["reader"], "lexical retrieval mismatch: {lexical:?}");
    let nearest: Vec<String> = db.query("SELECT VALUE name FROM code_unit WHERE embedding <|1,16|> [1.0,0.0,0.0]").await?.check()?.take(0)?;
    ensure!(nearest == ["reader"], "vector retrieval mismatch: {nearest:?}");
    if acknowledged {
        let values: Vec<String> = db.query("SELECT VALUE value FROM marker:ack").await?.check()?.take(0)?;
        ensure!(values == ["durable-before-ack"], "acknowledged write lost after forced process termination");
    }
    Ok(())
}

fn main() -> Result<()> {
    // SurrealDB 4.0.0's embedded query planner uses deep worker stacks.
    // Use the documented multi-thread Tokio runtime with explicit 10 MiB stacks,
    // rather than relying on #[tokio::main]'s platform-default stack size.
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(4)
        .thread_stack_size(10 * 1024 * 1024)
        .build()?
        .block_on(run())
}

async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    ensure!(args.len() == 3, "usage: gitr-surreal-probe seed|verify|ack-and-wait|recover|locked DISPOSABLE_DIRECTORY");
    let path = Path::new(&args[2]);
    if args[1] == "locked" {
        match open(path).await {
            Ok(_) => bail!("second datastore owner was admitted"),
            Err(error) => {
                // A fixture assertion, never a production retry/error classifier.
                ensure!(error.to_string().to_ascii_lowercase().contains("lock"), "unexpected ownership failure: {error}");
                println!("ownership: rejected-second-owner");
                return Ok(());
            }
        }
    }
    let db = open(path).await.context("open isolated SurrealKV datastore")?;
    match args[1].as_str() {
        "seed" => seed(&db).await?,
        "verify" => { verify(&db, false).await?; println!("reopen: passed"); },
        "recover" => { verify(&db, true).await?; println!("killed-process-recovery: passed"); },
        "ack-and-wait" => {
            db.query("CREATE marker:ack SET value = 'durable-before-ack'").await?.check()?;
            println!("ACK");
            io::stdout().flush()?;
            // The qualification parent terminates us without graceful shutdown.
            let mut line = String::new();
            io::stdin().read_line(&mut line)?;
        }
        _ => bail!("unknown qualification phase"),
    }
    Ok(())
}
