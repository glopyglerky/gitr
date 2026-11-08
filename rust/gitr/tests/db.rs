use gitr::{db, git};
use redb::{
    ConcurrencyMode, Database, ReadableDatabase, ReadableTable, ReadableTableMetadata,
    TableDefinition,
};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn reference(repo: &Path, args: &[&str], input: Option<&[u8]>) -> String {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "user.name=Gitr Database Tests",
            "-c",
            "user.email=gitr-tests@example.invalid",
        ])
        .args(args);
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command
        .spawn()
        .expect("reference Git is required for compatibility tests");
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn repository() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    reference(
        directory.path(),
        &["init", "--initial-branch=main", "-q"],
        None,
    );
    directory
}

fn commit(repo: &Path, message: &str) -> String {
    reference(repo, &["add", "-A"], None);
    reference(repo, &["commit", "-qm", message], None);
    reference(repo, &["rev-parse", "HEAD"], None)
}

fn graph(repo: &gix::Repository, revision: &str, path: Option<&[u8]>) -> Vec<serde_json::Value> {
    let mut output = Vec::new();
    db::write_graph(repo, revision, path, &mut output).unwrap();
    output
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}

#[test]
fn named_snapshots_never_fall_back_to_the_last_indexed_commit() {
    let directory = repository();
    fs::write(
        directory.path().join("first.rs"),
        "tangerine tangerine retrieval",
    )
    .unwrap();
    let first = commit(directory.path(), "first");
    fs::remove_file(directory.path().join("first.rs")).unwrap();
    fs::write(
        directory.path().join("second.rs"),
        "cobalt cobalt retrieval",
    )
    .unwrap();
    let second = commit(directory.path(), "second");
    let repo = git::discover_at(directory.path()).unwrap();
    let report = db::index(&repo, &first, db::DEFAULT_MAX_BLOB_BYTES).unwrap();
    assert_eq!(report.commit_oid, first);
    assert_eq!(report.indexed_files, 1);
    assert!(!report.learned_semantic_embeddings);
    let failure = db::search(&repo, "tangerine", "HEAD", 10).unwrap_err();
    assert!(failure.to_string().contains("is not indexed"));
    assert_eq!(
        db::search(&repo, "tangerine", &first, 10).unwrap().hits[0]
            .path
            .utf8
            .as_deref(),
        Some("first.rs")
    );
    db::index(&repo, &second, db::DEFAULT_MAX_BLOB_BYTES).unwrap();
    let latest = db::search(&repo, "cobalt", "HEAD", 10).unwrap();
    assert_eq!(latest.commit_oid, second);
    assert_eq!(latest.hits[0].path.utf8.as_deref(), Some("second.rs"));
    assert!(
        latest
            .hits
            .iter()
            .all(|hit| hit.path.utf8.as_deref() != Some("first.rs"))
    );
    assert_eq!(
        db::search(&repo, "tangerine", &first, 10).unwrap().hits[0]
            .path
            .utf8
            .as_deref(),
        Some("first.rs")
    );
    assert_eq!(
        db::index(&repo, &first, db::DEFAULT_MAX_BLOB_BYTES)
            .unwrap()
            .generation,
        report.generation
    );
}

#[test]
fn coverage_explains_binary_lfs_symlink_and_size_exclusions() {
    let directory = repository();
    fs::write(directory.path().join("source.rs"), "copper source").unwrap();
    fs::write(directory.path().join("empty.txt"), "").unwrap();
    fs::write(directory.path().join("asset.bin"), b"copper\0binary").unwrap();
    fs::write(directory.path().join("large.txt"), vec![b'x'; 1024]).unwrap();
    fs::write(
        directory.path().join("pointer.bin"),
        format!(
            "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 500\n",
            "1".repeat(64)
        ),
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("source.rs", directory.path().join("link")).unwrap();
    let revision = commit(directory.path(), "coverage");
    let repo = git::discover_at(directory.path()).unwrap();
    let report = db::index(&repo, &revision, 512).unwrap();
    assert_eq!(report.indexed_files, 1);
    let relations = graph(&repo, &revision, None);
    let state = |path: &str| {
        relations
            .iter()
            .find(|relation| relation["entry"]["path"]["utf8"] == path)
            .unwrap()["entry"]["lexical_state"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(state("asset.bin"), "binary");
    assert_eq!(state("empty.txt"), "no-ascii-tokens");
    assert_eq!(state("large.txt"), "too-large");
    assert_eq!(state("pointer.bin"), "lfs-pointer-not-asset-content");
    #[cfg(unix)]
    assert_eq!(state("link"), "symlink-not-followed");
    assert_eq!(relations[0]["graph_kind"], "git-tree-membership");
    assert_eq!(relations[0]["call_graph"], false);
    assert_eq!(
        db::search(&repo, "copper", &revision, 1)
            .unwrap()
            .hits
            .len(),
        1
    );
    // A larger explicit bound reindexes the same commit as a complete new generation.
    let expanded = db::index(&repo, &revision, 2048).unwrap();
    assert_eq!(expanded.indexed_files, 2);
    assert!(expanded.generation > report.generation);
    assert_eq!(
        graph(&repo, &revision, Some(b"large.txt"))[1]["entry"]["lexical_state"],
        "indexed"
    );
}

#[test]
fn a_failure_after_partial_traversal_keeps_the_old_snapshot_and_publishes_nothing() {
    let directory = repository();
    fs::write(directory.path().join("good.rs"), "rollback invariant").unwrap();
    let first = commit(directory.path(), "first");
    let good_blob = reference(directory.path(), &["rev-parse", "HEAD:good.rs"], None);
    let repo = git::discover_at(directory.path()).unwrap();
    db::index(&repo, &first, db::DEFAULT_MAX_BLOB_BYTES).unwrap();
    let mut tree = b"100644 a.rs\0".to_vec();
    tree.extend(
        gix::ObjectId::from_hex(good_blob.as_bytes())
            .unwrap()
            .as_bytes(),
    );
    tree.extend(b"100644 z-missing.rs\0");
    tree.extend([0x91; 20]);
    let tree_oid = reference(
        directory.path(),
        &["hash-object", "-w", "-t", "tree", "--stdin"],
        Some(&tree),
    );
    let broken = reference(
        directory.path(),
        &["commit-tree", &tree_oid, "-p", &first],
        Some(b"deliberately missing blob\n"),
    );
    let error = db::index(&repo, &broken, db::DEFAULT_MAX_BLOB_BYTES).unwrap_err();
    assert!(format!("{error:#}").contains("missing blob"));
    assert!(
        db::search(&repo, "rollback", &broken, 10)
            .unwrap_err()
            .to_string()
            .contains("is not indexed")
    );
    assert_eq!(
        db::search(&repo, "rollback", &first, 10)
            .unwrap()
            .hits
            .len(),
        1
    );
    let database = open_database(&db::catalog_path(&repo));
    let read = database.begin_read().unwrap();
    let members = read
        .open_table(TableDefinition::<&[u8], &[u8]>::new("members.v1"))
        .unwrap();
    assert_eq!(
        members.len().unwrap(),
        2,
        "only the original root tree and file were committed"
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_git_paths_are_distinct_and_round_trip_in_graph_queries() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let directory = repository();
    for name in [b"code-\xff.rs".as_slice(), b"code-\xfe.rs".as_slice()] {
        fs::write(
            directory.path().join(OsString::from_vec(name.to_vec())),
            "path identity",
        )
        .unwrap();
    }
    let revision = commit(directory.path(), "raw paths");
    let repo = git::discover_at(directory.path()).unwrap();
    db::index(&repo, &revision, db::DEFAULT_MAX_BLOB_BYTES).unwrap();
    let hits = db::search(&repo, "path identity", &revision, 10)
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|hit| hit.path.utf8.is_none()));
    assert_ne!(hits[0].path.hex, hits[1].path.hex);
    let selected = graph(&repo, &revision, Some(b"code-\xff.rs"));
    assert_eq!(selected.len(), 2);
    assert_eq!(selected[1]["entry"]["path"]["hex"], "636f64652dff2e7273");
}

#[test]
fn sha256_repositories_keep_their_native_object_identity() {
    let directory = tempfile::tempdir().unwrap();
    reference(
        directory.path(),
        &[
            "init",
            "--object-format=sha256",
            "--initial-branch=main",
            "-q",
        ],
        None,
    );
    fs::write(directory.path().join("hash.rs"), "native identity").unwrap();
    let revision = commit(directory.path(), "sha256");
    let repo = git::discover_at(directory.path()).unwrap();
    let report = db::index(&repo, "HEAD", db::DEFAULT_MAX_BLOB_BYTES).unwrap();
    assert_eq!(report.commit_oid, revision);
    assert_eq!(report.object_format, "sha256");
    let hit = &db::search(&repo, "native", "HEAD", 1).unwrap().hits[0];
    assert_eq!(
        hit.blob_oid,
        reference(directory.path(), &["rev-parse", "HEAD:hash.rs"], None)
    );
    assert_eq!(hit.blob_oid.len(), 64);
}

fn open_database(path: &Path) -> Database {
    Database::builder()
        .set_concurrency_mode(ConcurrencyMode::MultiWriter)
        .set_cache_size(1024 * 1024)
        .open(path)
        .unwrap()
}

fn worker(repo: &Path, revision: &str, ready: &Path, start: &Path, mode: &str) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "database_process_worker",
            "--nocapture",
        ])
        .env("GITR_DATABASE_TEST_REPO", repo)
        .env("GITR_DATABASE_TEST_REVISION", revision)
        .env("GITR_DATABASE_TEST_READY", ready)
        .env("GITR_DATABASE_TEST_START", start)
        .env("GITR_DATABASE_TEST_MODE", mode)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn await_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn successful_worker(child: Child) {
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "worker failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn separate_process_writers_preserve_both_complete_snapshots() {
    let directory = repository();
    let control = tempfile::tempdir().unwrap();
    for n in 0..24 {
        fs::write(
            directory.path().join(format!("file-{n}.rs")),
            "quartz firstgeneration",
        )
        .unwrap();
    }
    let first = commit(directory.path(), "first");
    fs::write(
        directory.path().join("second.rs"),
        "sapphire secondgeneration",
    )
    .unwrap();
    let second = commit(directory.path(), "second");
    let repo = git::discover_at(directory.path()).unwrap();
    // Initialize the catalog before testing contending writer processes.
    db::index(&repo, &first, 0).unwrap();
    let start = control.path().join("start");
    let ready_a = control.path().join("ready-a");
    let ready_b = control.path().join("ready-b");
    let a = worker(directory.path(), &first, &ready_a, &start, "index");
    let b = worker(directory.path(), &second, &ready_b, &start, "index");
    await_file(&ready_a);
    await_file(&ready_b);
    fs::write(&start, b"go").unwrap();
    successful_worker(a);
    successful_worker(b);
    assert_eq!(
        db::search(&repo, "quartz", &first, 100).unwrap().hits.len(),
        24
    );
    let second_report = db::search(&repo, "sapphire secondgeneration", &second, 100).unwrap();
    assert_eq!(second_report.indexed_files, 25);
    assert_eq!(
        second_report.hits[0].path.utf8.as_deref(),
        Some("second.rs")
    );
}

// Tests that deliberately hold a child's write transaction must reap the child
// even when an assertion panics, otherwise the process and lock outlive the test.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn killed_writer_rolls_back_and_releases_the_process_writer_lock() {
    let directory = repository();
    let control = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("first.rs"),
        "durable completed snapshot",
    )
    .unwrap();
    let first = commit(directory.path(), "first");
    let repo = git::discover_at(directory.path()).unwrap();
    db::index(&repo, &first, db::DEFAULT_MAX_BLOB_BYTES).unwrap();
    let ready = control.path().join("ready");
    let start = control.path().join("start");
    // Opening a new writable handle can itself wait for an active writer's
    // allocator lock. Open this handle first so the assertion below specifically
    // exercises nonblocking transaction acquisition on an existing handle.
    let database = open_database(&db::catalog_path(&repo));
    let mut child = KillOnDrop(worker(
        directory.path(),
        &first,
        &ready,
        &start,
        "hold-write",
    ));
    await_file(&ready);
    assert!(matches!(
        database.try_begin_write(),
        Err(redb::TransactionError::WouldBlock)
    ));
    assert_eq!(
        db::search(&repo, "durable", &first, 10).unwrap().hits.len(),
        1,
        "reader still sees the completed snapshot"
    );
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let write = database
        .try_begin_write()
        .expect("dead writer must release its process lock");
    let table = write
        .open_table(TableDefinition::<&str, u64>::new("interrupted-test.v1"))
        .unwrap();
    assert!(table.get("uncommitted").unwrap().is_none());
    drop(table);
    write.abort().unwrap();
    assert_eq!(
        db::search(&repo, "durable", &first, 10).unwrap().hits.len(),
        1
    );
}

#[test]
#[ignore = "invoked as a controlled child process by the multi-process tests"]
fn database_process_worker() {
    let repository = std::env::var_os("GITR_DATABASE_TEST_REPO").expect("worker repository");
    let revision = std::env::var("GITR_DATABASE_TEST_REVISION").unwrap();
    let ready = std::env::var_os("GITR_DATABASE_TEST_READY").unwrap();
    let start = std::env::var_os("GITR_DATABASE_TEST_START").unwrap();
    let repo = git::discover_at(Path::new(&repository)).unwrap();
    if std::env::var("GITR_DATABASE_TEST_MODE").unwrap() == "hold-write" {
        let database = open_database(&db::catalog_path(&repo));
        let transaction = database.begin_write().unwrap();
        let mut table = transaction
            .open_table(TableDefinition::<&str, u64>::new("interrupted-test.v1"))
            .unwrap();
        table.insert("uncommitted", 1).unwrap();
        fs::write(&ready, b"holding write transaction").unwrap();
        await_file(Path::new(&start)); // Parent kills this process while the write is uncommitted.
        drop(table);
        transaction.abort().unwrap();
    } else {
        fs::write(&ready, b"ready").unwrap();
        await_file(Path::new(&start));
        db::index(&repo, &revision, db::DEFAULT_MAX_BLOB_BYTES).unwrap();
    }
}
