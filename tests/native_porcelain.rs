//! Qualification of the default imported Rust Git CLI, with no Git executable
//! on its PATH. Reference Git is invoked separately as a behavioral oracle.
use std::{
    ffi::OsStr,
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

fn command(program: impl AsRef<OsStr>, cwd: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(program);
    cmd.current_dir(cwd)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", cwd.join("absent-global-config"))
        .env("GIT_AUTHOR_NAME", "Native Parity")
        .env("GIT_AUTHOR_EMAIL", "parity@example.invalid")
        .env("GIT_COMMITTER_NAME", "Native Parity")
        .env("GIT_COMMITTER_EMAIL", "parity@example.invalid")
        .env("GIT_AUTHOR_DATE", "1700000000 +0000")
        .env("GIT_COMMITTER_DATE", "1700000000 +0000")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_CONFIG_COUNT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn execute(mut command: Command, input: &[u8]) -> Output {
    let mut child = command.spawn().expect("start qualification command");
    let mut stdin = child.stdin.take().unwrap();
    let input = input.to_vec();
    let writer = thread::spawn(move || stdin.write_all(&input));
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out = thread::spawn(move || { let mut bytes = Vec::new(); stdout.read_to_end(&mut bytes).unwrap(); bytes });
    let err = thread::spawn(move || { let mut bytes = Vec::new(); stderr.read_to_end(&mut bytes).unwrap(); bytes });
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait for qualification command") { break status; }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("qualification command exceeded 60 seconds: {command:?}");
        }
        thread::sleep(Duration::from_millis(10));
    };
    let _ = writer.join().expect("stdin writer");
    Output { status, stdout: out.join().unwrap(), stderr: err.join().unwrap() }
}

fn output(cwd: &Path, args: &[&str], input: &[u8]) -> Output {
    let mut cmd = command(env!("CARGO_BIN_EXE_gitr"), cwd, args);
    // Internal Rust self-dispatch must work without a C Git, shell, or helper
    // available through PATH. The external reference command is separate.
    cmd.env("PATH", cwd.join("no-external-programs"));
    execute(cmd, input)
}

fn success(output: Output, args: &[&str]) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{args:?}: status={}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn native(cwd: &Path, args: &[&str]) -> Vec<u8> {
    success(output(cwd, args, &[]), args)
}

fn reference(cwd: &Path, args: &[&str]) -> Vec<u8> {
    success(execute(command("git", cwd, args), &[]), args)
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap().trim_end().to_owned()
}

fn fixture() -> TempDir {
    let temp = tempfile::tempdir().unwrap();
    native(temp.path(), &["init", "-q", "-b", "main"]);
    native(temp.path(), &["config", "user.name", "Native Parity"]);
    native(temp.path(), &["config", "user.email", "parity@example.invalid"]);
    native(temp.path(), &["config", "core.autocrlf", "false"]);
    temp
}

fn commit(root: &Path, name: &str, data: &[u8], message: &str) -> String {
    fs::write(root.join(name), data).unwrap();
    native(root, &["add", "--", name]);
    native(root, &["commit", "-q", "-m", message]);
    text(native(root, &["rev-parse", "HEAD"]))
}

fn compare(root: &Path, args: &[&str]) {
    assert_eq!(native(root, args), reference(root, args), "{args:?}");
}

#[test]
fn add_commit_log_status_diff_and_tag_are_git_compatible() {
    let temp = fixture();
    let root = temp.path();
    let first = commit(root, "file.txt", b"first\n", "first commit");
    assert_eq!(first, text(reference(root, &["rev-parse", "HEAD"])));
    let expected_tree = text(reference(root, &["write-tree"]));
    let expected_commit = success(
        execute(command("git", root, &["commit-tree", &expected_tree]), b"first commit\n"),
        &["commit-tree"],
    );
    assert_eq!(first, text(expected_commit), "exact Git commit identity");
    compare(root, &["log", "--format=%H %P %s", "-n", "1"]);
    compare(root, &["status", "--porcelain=v1", "--untracked-files=all"]);
    fs::write(root.join("file.txt"), b"changed\n").unwrap();
    compare(root, &["diff", "--no-ext-diff", "--no-textconv", "--no-color"]);
    native(root, &["add", "file.txt"]);
    compare(root, &["diff", "--cached", "--no-ext-diff", "--no-textconv", "--no-color"]);
    native(root, &["commit", "-q", "-m", "second"]);
    native(root, &["tag", "v1"]);
    compare(root, &["show-ref", "--tags"]);
    reference(root, &["fsck", "--strict"]);
}

#[test]
fn native_branch_checkout_and_switch_interoperate_with_git() {
    let temp = fixture();
    let root = temp.path();
    let base = commit(root, "base.txt", b"base\n", "base");
    native(root, &["branch", "topic"]);
    assert_eq!(text(reference(root, &["rev-parse", "topic"])), base);
    native(root, &["checkout", "topic"]);
    let topic = commit(root, "topic.txt", b"topic\n", "topic");
    assert_eq!(text(reference(root, &["rev-parse", "topic^1"])), base);
    assert_eq!(text(reference(root, &["rev-parse", "main"])), base);
    compare(root, &["ls-tree", "-r", &topic]);
    compare(root, &["status", "--porcelain"]);
    native(root, &["switch", "main"]);
    assert!(!root.join("topic.txt").exists());
    assert_eq!(text(reference(root, &["symbolic-ref", "HEAD"])), "refs/heads/main");
    native(root, &["switch", "-c", "new-topic"]);
    assert_eq!(text(reference(root, &["rev-parse", "new-topic"])), base);
    native(root, &["checkout", "--detach", &topic]);
    assert_eq!(fs::read(root.join("topic.txt")).unwrap(), b"topic\n");
    assert_eq!(fs::read(root.join(".git/HEAD")).unwrap(), format!("{topic}\n").as_bytes());
    reference(root, &["fsck", "--strict"]);
}

#[test]
fn native_inspection_preserves_git_merge_conflict_stages() {
    let temp = fixture();
    let root = temp.path();
    commit(root, "file", b"base\n", "base");
    reference(root, &["checkout", "-b", "topic"]);
    commit(root, "file", b"topic\n", "topic");
    reference(root, &["checkout", "main"]);
    let main = commit(root, "file", b"main\n", "main");
    let conflict = execute(command("git", root, &["merge", "--no-edit", "topic"]), &[]);
    assert!(!conflict.status.success(), "fixture requires an actual conflict");
    compare(root, &["ls-files", "--stage"]);
    compare(root, &["status", "--porcelain"]);
    let index = fs::read(root.join(".git/index")).unwrap();
    let result = output(root, &["merge", "--abort"], &[]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("disabled before mutation"));
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(text(reference(root, &["rev-parse", "HEAD"])), main);
    reference(root, &["merge", "--abort"]);
}

#[test]
fn unqualified_history_writers_and_aliases_fail_before_mutation() {
    let temp = fixture();
    let root = temp.path();
    let head = commit(root, "file", b"original\n", "base");
    let index = fs::read(root.join(".git/index")).unwrap();
    native(root, &["config", "alias.move-history", "reset --hard HEAD"]);
    for args in [
        vec!["rebase", "main"], vec!["cherry-pick", "HEAD"],
        vec!["revert", "--no-edit", "HEAD"], vec!["stash", "push"],
        vec!["reset", "--hard", "HEAD"], vec!["move-history"],
        vec!["--native-git", "reset", "--hard", "HEAD"],
    ] {
        let result = output(root, &args, &[]);
        assert!(!result.status.success(), "{args:?}");
        assert!(String::from_utf8_lossy(&result.stderr).contains("disabled before mutation"), "{args:?}: {}", String::from_utf8_lossy(&result.stderr));
        assert_eq!(text(reference(root, &["rev-parse", "HEAD"])), head);
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(fs::read(root.join("file")).unwrap(), b"original\n");
        assert!(!root.join(".git/refs/heads/unexpected").exists());
    }
}

#[test]
fn restore_move_and_remove_roundtrip_without_external_git() {
    let temp = fixture();
    let root = temp.path();
    commit(root, "file", b"original\n", "initial");
    fs::write(root.join("file"), b"dirty\n").unwrap();
    native(root, &["restore", "file"]);
    assert_eq!(fs::read(root.join("file")).unwrap(), b"original\n");
    native(root, &["mv", "file", "renamed"]);
    native(root, &["commit", "-q", "-m", "rename"]);
    native(root, &["rm", "renamed"]);
    native(root, &["commit", "-q", "-m", "remove"]);
    assert!(!root.join("renamed").exists());
    compare(root, &["status", "--porcelain"]);
    reference(root, &["fsck", "--strict"]);
}

#[test]
fn unqualified_transport_writers_do_not_create_or_move_remote_refs() {
    let parent = tempfile::tempdir().unwrap();
    let source = parent.path().join("source");
    fs::create_dir(&source).unwrap();
    native(&source, &["init", "-q", "-b", "main"]);
    let head = commit(&source, "file", b"first\n", "first");
    native(parent.path(), &["init", "-q", "--bare", "-b", "main", "remote.git"]);
    let remote = parent.path().join("remote.git");
    reference(&source, &["remote", "add", "origin", remote.to_str().unwrap()]);
    for args in [vec!["push", "origin", "main"], vec!["fetch", "origin"], vec!["pull", "origin", "main"]] {
        let result = output(&source, &args, &[]);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("disabled before mutation"));
        assert_eq!(text(reference(&source, &["rev-parse", "HEAD"])), head);
        assert!(reference(&remote, &["for-each-ref"]).is_empty());
    }
    let result = output(parent.path(), &["clone", remote.to_str().unwrap(), "clone"], &[]);
    assert!(!result.status.success());
    assert!(!parent.path().join("clone").exists());
}

#[test]
fn stale_expected_old_update_ref_cannot_overwrite_a_new_commit() {
    let temp = fixture();
    let root = temp.path();
    let first = commit(root, "file", b"first\n", "first");
    let next = commit(root, "file", b"next\n", "next");
    let stale = output(root, &["update-ref", "refs/heads/main", &first, &first], &[]);
    assert!(!stale.status.success());
    assert_eq!(text(reference(root, &["rev-parse", "HEAD"])), next);
}

#[test]
fn amending_a_detached_commit_preserves_the_branch_and_detached_head() {
    let temp = fixture();
    let root = temp.path();
    let parent = commit(root, "file", b"parent\n", "parent");
    let original = commit(root, "file", b"original\n", "original");
    native(root, &["checkout", "--detach", &original]);
    fs::write(root.join("file"), b"amended\n").unwrap();
    native(root, &["add", "file"]);
    native(root, &["commit", "--amend", "-q", "-m", "amended"]);
    let amended = text(reference(root, &["rev-parse", "HEAD"]));
    assert_ne!(amended, original);
    assert_eq!(text(reference(root, &["rev-parse", "main"])), original);
    assert_eq!(text(reference(root, &["rev-parse", "HEAD^1"])), parent);
    assert_eq!(fs::read(root.join(".git/HEAD")).unwrap(), format!("{amended}\n").as_bytes());
    assert_eq!(reference(root, &["show", "HEAD:file"]), b"amended\n");
    reference(root, &["fsck", "--strict"]);
}

#[test]
fn native_authored_history_is_usable_by_existing_redb_catalog() {
    let temp = fixture();
    let root = temp.path();
    let head = commit(root, "source.rs", b"pub fn native_transport() {}\n", "native");
    let indexed: serde_json::Value = serde_json::from_slice(&native(root, &["db", "index", "HEAD"])).unwrap();
    assert_eq!(indexed["commit_oid"], head);
    let search: serde_json::Value = serde_json::from_slice(&native(root, &["db", "search", "native_transport", "--revision", "HEAD"])).unwrap();
    assert_eq!(search["commit_oid"], head);
    assert_eq!(search["hits"][0]["path"]["utf8"], "source.rs");
}

#[test]
fn unknown_commands_and_foreign_program_dependencies_fail_explicitly() {
    let temp = fixture();
    for args in [vec!["definitely-not-a-git-command"], vec!["filter-branch", "--all"]] {
        let result = output(temp.path(), &args, &[]);
        assert!(!result.status.success(), "unexpected success: {args:?}");
    }
}

#[test]
fn absent_objects_are_not_synthesized_for_an_upstream_test_typo() {
    let temp = fixture();
    let root = temp.path();
    let real_empty_tree = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
    let upstream_typo = "4b825dc642cb6eb9a060e54bf899d69f7c6948d4";
    compare(root, &["cat-file", "-t", real_empty_tree]);
    for mode in ["-e", "-t", "-p"] {
        let args = ["cat-file", mode, upstream_typo];
        let expected = execute(command("git", root, &args), &[]);
        let actual = output(root, &args, &[]);
        assert!(!expected.status.success());
        assert!(!actual.status.success(), "invented object for {args:?}");
        assert!(actual.stdout.is_empty());
    }
}

#[test]
fn raw_blob_streaming_preserves_sha1_sha256_and_redb_interoperability() {
    for format in ["sha1", "sha256"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        native(root, &["init", "-q", "--object-format", format]);
        native(root, &["config", "gitr.redbObjects", "true"]);
        let bytes = vec![0xa5; 3 * 1024 * 1024 + 13];
        fs::write(root.join("blob"), &bytes).unwrap();
        let expected = reference(root, &["hash-object", "--no-filters", "blob"]);
        assert_eq!(native(root, &["hash-object", "--no-filters", "blob"]), expected);
        assert_eq!(success(output(root, &["hash-object", "--stdin", "-w"], &bytes), &["hash-object"]), expected);
        assert!(root.join(".git/gitr/objects.redb").is_file());
        let oid = text(expected);
        assert_eq!(reference(root, &["cat-file", "blob", &oid]), bytes);
        assert_eq!(native(root, &["cat-file", "blob", &oid]), bytes);
    }
}
