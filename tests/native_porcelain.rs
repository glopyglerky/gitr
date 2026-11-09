//! Qualification of the default imported Rust Git CLI, with no Git executable
//! on its PATH. Reference Git is invoked separately as a behavioral oracle.
use std::{
    ffi::OsStr,
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
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
        .env_remove("GITR_NATIVE_GIT")
        .env_remove("GIT_CONFIG_COUNT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn execute(mut command: Command, input: &[u8]) -> Output {
    let mut child = command.spawn().expect("start qualification command");
    if !input.is_empty() {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    drop(child.stdin.take());
    child.wait_with_output().expect("wait for qualification command")
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
fn branch_checkout_and_nonconflicting_merge_produce_valid_history() {
    let temp = fixture();
    let root = temp.path();
    commit(root, "base.txt", b"base\n", "base");
    native(root, &["switch", "-c", "topic"]);
    let topic = commit(root, "topic.txt", b"topic\n", "topic");
    native(root, &["checkout", "main"]);
    let main = commit(root, "main.txt", b"main\n", "main");
    native(root, &["merge", "--no-edit", "topic"]);
    assert_eq!(text(native(root, &["rev-parse", "HEAD^1"])), main);
    assert_eq!(text(native(root, &["rev-parse", "HEAD^2"])), topic);
    assert_eq!(fs::read(root.join("topic.txt")).unwrap(), b"topic\n");
    compare(root, &["ls-tree", "-r", "HEAD"]);
    compare(root, &["status", "--porcelain"]);
    reference(root, &["fsck", "--strict"]);
}

#[test]
fn merge_conflicts_preserve_index_stages_and_abort_restores_worktree() {
    let temp = fixture();
    let root = temp.path();
    commit(root, "file", b"base\n", "base");
    native(root, &["checkout", "-b", "topic"]);
    commit(root, "file", b"topic\n", "topic");
    native(root, &["checkout", "main"]);
    let main = commit(root, "file", b"main\n", "main");
    let conflict = output(root, &["merge", "--no-edit", "topic"], &[]);
    assert!(!conflict.status.success(), "conflicting merge must fail");
    compare(root, &["ls-files", "--stage"]);
    compare(root, &["status", "--porcelain"]);
    assert_eq!(text(reference(root, &["rev-parse", "HEAD"])), main);
    native(root, &["merge", "--abort"]);
    assert_eq!(fs::read(root.join("file")).unwrap(), b"main\n");
    assert!(native(root, &["status", "--porcelain"]).is_empty());
}

#[test]
fn rebase_cherry_pick_and_revert_run_without_external_git() {
    let temp = fixture();
    let root = temp.path();
    commit(root, "base", b"base\n", "base");
    native(root, &["switch", "-c", "topic"]);
    commit(root, "topic", b"topic\n", "topic");
    native(root, &["switch", "main"]);
    let main = commit(root, "main", b"main\n", "main");
    native(root, &["switch", "topic"]);
    native(root, &["rebase", "main"]);
    assert_eq!(text(native(root, &["rev-parse", "HEAD^"])), main);
    let rebased = text(native(root, &["rev-parse", "HEAD"]));
    native(root, &["switch", "main"]);
    native(root, &["cherry-pick", &rebased]);
    assert_eq!(fs::read(root.join("topic")).unwrap(), b"topic\n");
    native(root, &["revert", "--no-edit", "HEAD"]);
    assert!(!root.join("topic").exists());
    compare(root, &["log", "--format=%H %P %s", "--all"]);
    reference(root, &["fsck", "--strict"]);
}

#[test]
fn stash_restore_move_remove_and_reset_roundtrip() {
    let temp = fixture();
    let root = temp.path();
    let initial = commit(root, "file", b"original\n", "initial");
    fs::write(root.join("file"), b"dirty\n").unwrap();
    native(root, &["stash", "push", "-m", "pending"]);
    assert_eq!(fs::read(root.join("file")).unwrap(), b"original\n");
    native(root, &["stash", "pop"]);
    assert_eq!(fs::read(root.join("file")).unwrap(), b"dirty\n");
    native(root, &["restore", "file"]);
    assert_eq!(fs::read(root.join("file")).unwrap(), b"original\n");
    native(root, &["mv", "file", "renamed"]);
    native(root, &["commit", "-q", "-m", "rename"]);
    native(root, &["rm", "renamed"]);
    native(root, &["commit", "-q", "-m", "remove"]);
    native(root, &["reset", "--hard", &initial]);
    assert_eq!(fs::read(root.join("file")).unwrap(), b"original\n");
    assert!(!root.join("renamed").exists());
    compare(root, &["status", "--porcelain"]);
}

#[test]
fn native_clone_push_fetch_and_pull_interoperate_with_reference_git() {
    let parent = tempfile::tempdir().unwrap();
    let source = parent.path().join("source");
    fs::create_dir(&source).unwrap();
    native(&source, &["init", "-q", "-b", "main"]);
    let initial = commit(&source, "file", b"first\n", "first");
    native(parent.path(), &["init", "-q", "--bare", "-b", "main", "remote.git"]);
    let remote = parent.path().join("remote.git");
    native(&source, &["remote", "add", "origin", remote.to_str().unwrap()]);
    native(&source, &["push", "-u", "origin", "main"]);
    assert_eq!(text(reference(&remote, &["rev-parse", "refs/heads/main"])), initial);
    native(parent.path(), &["clone", remote.to_str().unwrap(), "clone"]);
    let clone = parent.path().join("clone");
    assert_eq!(fs::read(clone.join("file")).unwrap(), b"first\n");
    let next = commit(&source, "next", b"next\n", "next");
    native(&source, &["push", "origin", "main"]);
    native(&clone, &["fetch", "origin"]);
    assert_eq!(text(native(&clone, &["rev-parse", "origin/main"])), next);
    native(&clone, &["pull", "--ff-only", "origin", "main"]);
    assert_eq!(text(native(&clone, &["rev-parse", "HEAD"])), next);
    reference(&clone, &["fsck", "--strict"]);
    reference(&remote, &["fsck", "--strict"]);
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
