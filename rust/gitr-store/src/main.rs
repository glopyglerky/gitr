//! One SurrealKV owner, stateless authenticated requests, durable action state.
//! Local trusted-user profile only; not multi-tenant authorization or HA.
use anyhow::{Context, Result, bail, ensure};
use axum::{Router, body::{Body, to_bytes}, extract::State, http::{Request as HttpRequest, StatusCode, header}, response::{IntoResponse, Response}, routing::post, Json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, io::{self, Write}, net::{IpAddr, Ipv4Addr, SocketAddr}, path::Path, sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};
use subtle::ConstantTimeEq;
use surrealdb::{Surreal, engine::any::{Any, connect}, types::{QueryError, RecordId, SurrealValue}};
use tokio::sync::Semaphore;

const MAX_BYTES: usize = 1024 * 1024;
const MAX_REQUEST: usize = 2 * MAX_BYTES + 32 * 1024;
const PROFILE: &str = "gitr-actions-v1/wasmi-2.0.0/fuel-10000000/memory-16MiB/io-8MiB/calls-4096";
const LEASE_MS: u64 = 60_000;
const STACK_BYTES: usize = 10 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Spec {
    profile: String,
    module_sha256: String,
    commit_oid: String,
    blob_oid: String,
    path: String,
    input_sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Status { Queued, Running, Passed, Failed, Cancelled }
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Job {
    id: String,
    spec: Spec,
    status: Status,
    attempt: u64,
    lease_until_ms: Option<u64>,
    artifact_sha256: Option<String>,
    exit_code: Option<i32>,
    failure: Option<String>,
}
#[derive(Debug, SurrealValue)]
struct Row { revision: i64, payload: String }
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Install { module_hex: String },
    Submit { job: String, spec: Spec, input_hex: String },
    Status { job: String },
    Cancel { job: String },
    Claim { job: String },
    Finish { job: String, attempt: u64, exit_code: Option<i32>, output_hex: String },
    Object { sha256: String },
    Artifact { job: String },
}
#[derive(Clone)]
struct Store {
    db: Surreal<Any>,
    repository: String,
    authorization: Arc<Vec<u8>>,
    permits: Arc<Semaphore>,
}
#[derive(Debug)]
enum Fault { Invalid, Missing, Stale, Corrupt, Busy }
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{self:?}") }
}
impl std::error::Error for Fault {}

fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 128 && key.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}
fn valid_oid(id: &str, n: usize) -> bool {
    id.len() == n && id.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn decode(value: &str) -> Result<Vec<u8>> {
    if value.len() > MAX_BYTES * 2 || !value.len().is_multiple_of(2) || !value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
        return Err(Fault::Invalid.into());
    }
    let digit = |c: u8| if c <= b'9' { c - b'0' } else { c - b'a' + 10 };
    Ok(value.as_bytes().chunks_exact(2).map(|p| digit(p[0]) * 16 + digit(p[1])).collect())
}
fn now_ms() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().try_into()?)
}
fn record(table: &'static str, id: &str) -> RecordId { RecordId::from((table, id.to_owned())) }
fn retryable(error: &anyhow::Error) -> bool {
    let Some(error) = error.downcast_ref::<surrealdb::Error>() else { return false; };
    let mut cause = Some(error);
    for _ in 0..32 {
        let Some(error) = cause else { break; };
        // gitr_cas is our own fixed application marker, not an upstream message.
        if matches!(error.query_details(), Some(QueryError::TransactionConflict)) || error.is_already_exists()
            || (error.is_thrown() && error.message() == "gitr_cas") { return true; }
        cause = error.cause();
    }
    false
}

impl Request {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Install { module_hex } => {
                let bytes = decode(module_hex)?;
                if !bytes.starts_with(b"\0asm") { return Err(Fault::Invalid.into()); }
            }
            Self::Submit { job, spec, input_hex } => {
                let bytes = decode(input_hex)?;
                if !valid_key(job) || spec.profile != PROFILE || !valid_oid(&spec.module_sha256, 64)
                    || !valid_oid(&spec.input_sha256, 64) || digest(&bytes) != spec.input_sha256
                    || !(spec.commit_oid.len() == 40 || spec.commit_oid.len() == 64)
                    || !valid_oid(&spec.commit_oid, spec.commit_oid.len())
                    || !valid_oid(&spec.blob_oid, spec.commit_oid.len())
                    || spec.path.is_empty() || spec.path.len() > 4096 || spec.path.contains(['\0', '\\'])
                    || spec.path.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
                    return Err(Fault::Invalid.into());
                }
            }
            Self::Object { sha256 } => if !valid_oid(sha256, 64) { return Err(Fault::Invalid.into()); },
            Self::Finish { job, attempt, output_hex, exit_code } => {
                if !valid_key(job) || *attempt == 0 || *attempt > i64::MAX as u64 { return Err(Fault::Invalid.into()); }
                decode(output_hex)?;
                // Failed guests cannot persist partial output.
                if *exit_code != Some(0) && !output_hex.is_empty() { return Err(Fault::Invalid.into()); }
            }
            Self::Status { job } | Self::Cancel { job } | Self::Claim { job } | Self::Artifact { job } => {
                if !valid_key(job) { return Err(Fault::Invalid.into()); }
            }
        }
        Ok(())
    }
}

impl Store {
    async fn load(&self, id: &str) -> Result<Option<(Row, Job)>> {
        let mut rows: Vec<Row> = self.db.query("SELECT revision, payload FROM $job")
            .bind(("job", record("gitr_job", id))).await?.check()?.take(0)?;
        if rows.len() > 1 { return Err(Fault::Corrupt.into()); }
        let Some(row) = rows.pop() else { return Ok(None); };
        if row.payload.len() > 16 * 1024 || row.revision < 0 { return Err(Fault::Corrupt.into()); }
        let job: Job = serde_json::from_str(&row.payload)?;
        if job.id != id || job.spec.profile != PROFILE { return Err(Fault::Corrupt.into()); }
        Ok(Some((row, job)))
    }
    async fn object(&self, id: &str) -> Result<String> {
        let mut values: Vec<String> = self.db.query("SELECT VALUE hex FROM $object")
            .bind(("object", record("gitr_object", id))).await?.check()?.take(0)?;
        if values.len() > 1 { return Err(Fault::Corrupt.into()); }
        let value = values.pop().ok_or(Fault::Missing)?;
        if digest(&decode(&value)?) != id { return Err(Fault::Corrupt.into()); }
        Ok(value)
    }
    async fn install(&self, hex: &str) -> Result<String> {
        let id = digest(&decode(hex)?);
        self.db.query("BEGIN; LET $old = (SELECT VALUE hex FROM $object); IF array::len($old) > 0 { IF $old[0] != $hex { THROW 'gitr_integrity'; }; } ELSE { CREATE $object SET hex = $hex; }; COMMIT;")
            .bind(("object", record("gitr_object", &id))).bind(("hex", hex.to_owned())).await?.check()?;
        Ok(id)
    }
    async fn publish(&self, job: &Job, previous: Option<i64>, object: Option<(&str, &str)>) -> Result<()> {
        let next = match previous { Some(n) => n.checked_add(1).context("revision overflow")?, None => 0 };
        let (object_id, hex) = object.map(|(id, hex)| (Some(record("gitr_object", id)), hex.to_owned())).unwrap_or((None, String::new()));
        // The successful CAS and artifact are one committed transaction. No host
        // execution or unbounded IO takes place while this transaction is open.
        self.db.query("BEGIN;
            IF $object != NONE {
                LET $old = (SELECT VALUE hex FROM $object);
                IF array::len($old) > 0 { IF $old[0] != $hex { THROW 'gitr_integrity'; }; }
                ELSE { CREATE $object SET hex = $hex; };
            };
            IF $expected = NONE { CREATE $job SET revision = 0, payload = $payload; }
            ELSE {
                LET $updated = (UPDATE $job SET revision = $next, payload = $payload WHERE revision = $expected RETURN AFTER);
                IF array::len($updated) != 1 { THROW 'gitr_cas'; };
            };
            COMMIT;")
            .bind(("object", object_id)).bind(("hex", hex))
            .bind(("job", record("gitr_job", &job.id)))
            .bind(("expected", previous)).bind(("next", next))
            .bind(("payload", serde_json::to_string(job)?)).await?.check()?;
        Ok(())
    }
    async fn dispatch(&self, request: &Request) -> Result<Value> {
        for attempt in 0..8 {
            match self.once(request).await {
                Err(error) if retryable(&error) && attempt < 7 => {
                    tokio::time::sleep(Duration::from_millis(1 << attempt)).await;
                }
                other => return other,
            }
        }
        Err(Fault::Busy.into())
    }
    async fn once(&self, request: &Request) -> Result<Value> {
        match request {
            Request::Install { module_hex } => Ok(json!({"module_sha256": self.install(module_hex).await?, "profile": PROFILE})),
            Request::Object { sha256 } => Ok(json!({"hex": self.object(sha256).await?})),
            Request::Submit { job: id, spec, input_hex } => {
                if let Some((_, existing)) = self.load(id).await? {
                    if existing.spec != *spec { return Err(Fault::Stale.into()); }
                    return Ok(serde_json::to_value(existing)?);
                }
                let module = self.object(&spec.module_sha256).await?;
                if !decode(&module)?.starts_with(b"\0asm") { return Err(Fault::Corrupt.into()); }
                let job = Job { id: id.clone(), spec: spec.clone(), status: Status::Queued,
                    attempt: 0, lease_until_ms: None, artifact_sha256: None, exit_code: None, failure: None };
                self.publish(&job, None, Some((&spec.input_sha256, input_hex))).await?;
                Ok(serde_json::to_value(job)?)
            }
            Request::Status { job } => Ok(serde_json::to_value(self.load(job).await?.ok_or(Fault::Missing)?.1)?),
            Request::Artifact { job } => {
                let job = self.load(job).await?.ok_or(Fault::Missing)?.1;
                if job.status != Status::Passed { return Err(Fault::Stale.into()); }
                let oid = job.artifact_sha256.ok_or(Fault::Corrupt)?;
                Ok(json!({"sha256": oid, "hex": self.object(&oid).await?}))
            }
            Request::Cancel { job: id } => {
                let (row, mut job) = self.load(id).await?.ok_or(Fault::Missing)?;
                if matches!(job.status, Status::Queued | Status::Running) {
                    job.status = Status::Cancelled;
                    job.lease_until_ms = None;
                    self.publish(&job, Some(row.revision), None).await?;
                }
                Ok(serde_json::to_value(job)?)
            }
            Request::Claim { job: id } => {
                let (row, mut job) = self.load(id).await?.ok_or(Fault::Missing)?;
                let now = now_ms()?;
                if !(job.status == Status::Queued || (job.status == Status::Running && job.lease_until_ms.is_some_and(|end| now >= end))) {
                    return Ok(json!({"claim": null}));
                }
                job.attempt = job.attempt.checked_add(1).filter(|n| *n <= i64::MAX as u64).context("attempt overflow")?;
                job.status = Status::Running;
                job.lease_until_ms = Some(now.checked_add(LEASE_MS).context("lease overflow")?);
                self.publish(&job, Some(row.revision), None).await?;
                Ok(json!({"claim": job}))
            }
            Request::Finish { job: id, attempt, exit_code, output_hex } => {
                let (row, mut job) = self.load(id).await?.ok_or(Fault::Missing)?;
                let output_oid = if *exit_code == Some(0) { Some(digest(&decode(output_hex)?)) } else { None };
                // A lost response may be retried, but a different result for the
                // same terminal attempt is rejected rather than overwritten.
                if matches!(job.status, Status::Passed | Status::Failed) && job.attempt == *attempt {
                    if job.exit_code == *exit_code && job.artifact_sha256 == output_oid {
                        return Ok(serde_json::to_value(job)?);
                    }
                    return Err(Fault::Stale.into());
                }
                if job.status != Status::Running || job.attempt != *attempt || !job.lease_until_ms.is_some_and(|end| now_ms().is_ok_and(|now| now < end)) {
                    return Err(Fault::Stale.into());
                }
                job.status = if *exit_code == Some(0) { Status::Passed } else { Status::Failed };
                job.exit_code = *exit_code;
                job.artifact_sha256 = output_oid.clone();
                job.lease_until_ms = None;
                job.failure = match exit_code { Some(0) => None, Some(_) => Some("guest returned nonzero status; output discarded".into()), None => Some("guest validation, trap or resource limit failure; output discarded".into()) };
                self.publish(&job, Some(row.revision), output_oid.as_deref().map(|id| (id, output_hex.as_str()))).await?;
                Ok(serde_json::to_value(job)?)
            }
        }
    }
}

fn error_response(status: StatusCode, code: &'static str) -> Response {
    (status, Json(json!({"error": code}))).into_response()
}
async fn request(State(store): State<Store>, req: HttpRequest<Body>) -> Response {
    let headers = req.headers();
    let auth = headers.get(header::AUTHORIZATION).map(|h| h.as_bytes()).unwrap_or_default();
    if auth.len() != store.authorization.len() || !bool::from(auth.ct_eq(store.authorization.as_slice())) {
        return error_response(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    if headers.contains_key(header::ORIGIN) || headers.get("x-gitr-repository").and_then(|h| h.to_str().ok()) != Some(store.repository.as_str()) {
        return error_response(StatusCode::FORBIDDEN, "repository_scope");
    }
    if headers.get(header::CONTENT_TYPE).and_then(|h| h.to_str().ok()) != Some("application/json") {
        return error_response(StatusCode::UNSUPPORTED_MEDIA_TYPE, "content_type");
    }
    let Ok(_permit) = store.permits.try_acquire() else { return error_response(StatusCode::SERVICE_UNAVAILABLE, "busy"); };
    let bytes = match tokio::time::timeout(Duration::from_secs(10), to_bytes(req.into_body(), MAX_REQUEST)).await {
        Ok(Ok(bytes)) => bytes,
        _ => return error_response(StatusCode::PAYLOAD_TOO_LARGE, "body_limit_or_timeout"),
    };
    let request: Request = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    if request.validate().is_err() { return error_response(StatusCode::BAD_REQUEST, "invalid_request"); }
    match store.dispatch(&request).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => match error.downcast_ref::<Fault>() {
            Some(Fault::Invalid) => error_response(StatusCode::BAD_REQUEST, "invalid_request"),
            Some(Fault::Missing) => error_response(StatusCode::NOT_FOUND, "not_found"),
            Some(Fault::Stale) => error_response(StatusCode::CONFLICT, "conflict"),
            _ => error_response(StatusCode::SERVICE_UNAVAILABLE, "storage_failure"),
        },
    }
}

async fn serve(directory: String, address: SocketAddr, repository: String, token: String) -> Result<()> {
    let root = Path::new(&directory);
    if !root.exists() { fs::create_dir_all(root)?; }
    ensure!(fs::symlink_metadata(root)?.file_type().is_dir(), "storage root must be a real directory");
    let root = fs::canonicalize(root)?;
    let marker = root.join("gitr-store-v1.json");
    let expected = serde_json::to_vec(&json!({"schema": 1, "repository": repository}))?;
    if marker.exists() {
        ensure!(fs::symlink_metadata(&marker)?.file_type().is_file() && fs::read(&marker)? == expected, "wrong storage identity or schema");
    } else {
        ensure!(fs::read_dir(&root)?.next().is_none(), "refusing to adopt a nonempty unmarked directory");
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&marker)?;
        f.write_all(&expected)?;
        f.sync_all()?;
        #[cfg(unix)]
        fs::File::open(&root)?.sync_all()?;
    }
    let path = root.join("database");
    if let Ok(meta) = fs::symlink_metadata(&path) { ensure!(meta.file_type().is_dir(), "database must not be a symlink"); }
    let text = path.to_str().context("datastore path must be Unicode")?;
    ensure!(!text.contains(['?', '#']), "unsupported datastore path");
    let db = connect(format!("surrealkv://{text}?sync=every")).await?;
    db.use_ns("gitr_store").use_db("actions").await?;
    db.query("DEFINE TABLE IF NOT EXISTS gitr_job SCHEMAFULL; DEFINE FIELD IF NOT EXISTS revision ON gitr_job TYPE int; DEFINE FIELD IF NOT EXISTS payload ON gitr_job TYPE string; DEFINE TABLE IF NOT EXISTS gitr_object SCHEMAFULL; DEFINE FIELD IF NOT EXISTS hex ON gitr_object TYPE string;").await?.check()?;
    let store = Store { db, repository: repository.clone(), authorization: Arc::new(format!("Bearer {token}").into_bytes()), permits: Arc::new(Semaphore::new(8)) };
    let listener = tokio::net::TcpListener::bind(address).await?;
    let address = listener.local_addr()?;
    println!("{}", json!({"endpoint": format!("http://{address}/v1/action"), "repository": repository, "schema": 1}));
    io::stdout().flush()?;
    axum::serve(listener, Router::new().route("/v1/action", post(request)).with_state(store)).await?;
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    ensure!(args.len() == 4, "usage: gitr-store NEW_OR_GITR_STORE_DIRECTORY 127.0.0.1:PORT REPOSITORY_ID");
    let address: SocketAddr = args[2].parse()?;
    ensure!(address.ip() == IpAddr::V4(Ipv4Addr::LOCALHOST), "this profile binds only literal IPv4 loopback");
    ensure!(valid_key(&args[3]), "invalid repository identity");
    let token = std::env::var("GITR_STORE_TOKEN").context("GITR_STORE_TOKEN must contain a generated 64-character lowercase hexadecimal secret")?;
    ensure!(valid_oid(&token, 64), "invalid storage service token");
    let directory = args[1].clone();
    let repository = args[3].clone();
    std::thread::Builder::new().name("gitr-store-root".into()).stack_size(STACK_BYTES).spawn(move || {
        tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(4)
            .thread_stack_size(STACK_BYTES).build()?.block_on(serve(directory, address, repository, token))
    })?.join().map_err(|_| anyhow::anyhow!("storage service thread panicked"))?
}
