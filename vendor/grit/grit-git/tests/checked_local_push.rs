//! The checked local profile uses native children and expected-old publication.
#![cfg(unix)]
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};

const NATIVE: &str = env!("CARGO_BIN_EXE_grit-git");

fn command(program: &str, directory: &Path) -> Command {
    let mut command = Command::new(program);
    command.current_dir(directory)
        .env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test").env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test").env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_AUTHOR_DATE", "1700000000 +0000").env("GIT_COMMITTER_DATE", "1700000000 +0000")
        .env_remove("GIT_DIR").env_remove("GIT_WORK_TREE").env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT").env_remove("GIT_NAMESPACE").env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES");
    command
}
fn git(directory: &Path, arguments: &[&str]) -> Output {
    let out = command("git", directory).args(arguments).output().unwrap();
    assert!(out.status.success(), "git {arguments:?}: {}", String::from_utf8_lossy(&out.stderr)); out
}
fn native(directory: &Path, arguments: &[&str]) -> Output {
    command(NATIVE, directory).args(arguments).env("PATH", "").output().unwrap()
}
fn success(out: Output) { assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr)); }
fn oid(directory: &Path, name: &str) -> String {
    String::from_utf8(git(directory, &["rev-parse", name]).stdout).unwrap().trim().to_owned()
}
fn source(directory: &Path) {
    fs::create_dir_all(directory).unwrap();
    git(directory, &["init", "-q", "-b", "main"]);
    fs::write(directory.join("tracked"), b"first\n").unwrap();
    git(directory, &["add", "tracked"]); git(directory, &["commit", "-qm", "first"]);
}
fn destination(directory: &Path) {
    fs::create_dir_all(directory).unwrap(); git(directory, &["init", "--bare", "-q", "-b", "main"]);
    git(directory, &["config", "gitr.redbObjects", "true"]);
}

#[test]
fn native_local_push_creates_advances_and_deletes_with_empty_path() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("source"); let dst = temp.path().join("remote");
    source(&src); destination(&dst);
    git(&src, &["remote", "add", "origin", dst.to_str().unwrap()]);
    success(native(&src, &["push", "origin", "main"]));
    assert_eq!(oid(&src, "HEAD"), oid(&dst, "refs/heads/main"));
    assert_eq!(oid(&src, "HEAD"), oid(&src, "refs/remotes/origin/main"));
    assert!(dst.join("gitr/objects.redb").exists());
    fs::write(src.join("tracked"), b"second\n").unwrap();
    git(&src, &["commit", "-qam", "second"]);
    success(native(&src, &["push", "origin", "main"]));
    assert_eq!(oid(&src, "HEAD"), oid(&dst, "refs/heads/main"));
    let refused = native(&src, &["push", "--delete", "origin", "main"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("deletion of the current branch prohibited"));
    assert_eq!(oid(&src, "HEAD"), oid(&dst, "refs/heads/main"));
    git(&src, &["branch", "topic"]);
    success(native(&src, &["push", "origin", "topic"]));
    success(native(&src, &["push", "--delete", "origin", "topic"]));
    assert!(!dst.join("refs/heads/topic").exists());
    assert!(!src.join(".git/refs/remotes/origin/topic").exists());
    git(&dst, &["fsck", "--full"]);
}

fn receive(directory: &Path, lines: &[String], before_send: impl FnOnce()) -> Vec<u8> {
    receive_with_pack(directory, lines, &[], before_send)
}

fn receive_with_pack(directory: &Path, lines: &[String], pack: &[u8], before_send: impl FnOnce()) -> Vec<u8> {
    let mut child = command(NATIVE, directory).args(["receive-pack", directory.to_str().unwrap()])
        .env("PATH", "").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    loop {
        let mut header = [0u8; 4]; stdout.read_exact(&mut header).unwrap();
        let len = usize::from_str_radix(std::str::from_utf8(&header).unwrap(), 16).unwrap();
        if len == 0 { break; }
        let mut body = vec![0; len - 4]; stdout.read_exact(&mut body).unwrap();
    }
    before_send();
    let mut stdin = child.stdin.take().unwrap();
    for line in lines { write!(stdin, "{:04x}{line}", line.len() + 4).unwrap(); }
    stdin.write_all(b"0000").unwrap(); stdin.write_all(pack).unwrap(); drop(stdin);
    let mut body = Vec::new(); stdout.read_to_end(&mut body).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr)); body
}

#[test]
fn protocol_stale_update_and_delete_are_rejected_but_valid_ref_is_published() {
    let temp = tempfile::tempdir().unwrap(); let src = temp.path().join("source");
    let dst = temp.path().join("remote"); source(&src);
    let old = oid(&src, "HEAD"); git(&src, &["commit", "--allow-empty", "-qm", "next"]);
    let new = oid(&src, "HEAD");
    git(temp.path(), &["clone", "--bare", src.to_str().unwrap(), dst.to_str().unwrap()]);
    git(&dst, &["update-ref", "refs/heads/main", &old]);
    git(&dst, &["update-ref", "refs/heads/delete-me", &old]);
    git(&dst, &["update-ref", "refs/heads/hidden", &old]);
    git(&dst, &["config", "receive.hideRefs", "refs/heads/hidden"]);
    let zero = "0".repeat(40);
    let report = receive(&dst, &[
        format!("{old} {new} refs/heads/main\0report-status"),
        format!("{old} {zero} refs/heads/delete-me"),
        format!("{old} {zero} refs/heads/hidden"),
        format!("{zero} {new} refs/heads/accepted"),
    ], || {
        git(&dst, &["update-ref", "refs/heads/main", &new, &old]);
        git(&dst, &["update-ref", "refs/heads/delete-me", &new, &old]);
    });
    let report = String::from_utf8(report).unwrap();
    assert!(report.contains("ng refs/heads/main stale expected old value"), "{report}");
    assert!(report.contains("ng refs/heads/delete-me stale expected old value"), "{report}");
    assert!(report.contains("ng refs/heads/hidden hidden ref"), "{report}");
    assert!(report.contains("ok refs/heads/accepted"), "{report}");
    assert_eq!(oid(&dst, "refs/heads/delete-me"), new);
    assert_eq!(oid(&dst, "refs/heads/accepted"), new);
    assert_eq!(oid(&dst, "refs/heads/hidden"), old);
}

#[test]
fn corrupt_pack_stays_in_owned_quarantine_and_publishes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("source"); let dst = temp.path().join("remote");
    source(&src); destination(&dst);
    let mut builder = command("git", &src).args(["pack-objects", "--stdout", "--revs"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    builder.stdin.take().unwrap().write_all(b"HEAD\n").unwrap();
    let output = builder.wait_with_output().unwrap(); assert!(output.status.success());
    let mut pack = output.stdout; let last = pack.len() - 1; pack[last] ^= 1;
    let report = receive_with_pack(&dst, &[format!("{} {} refs/heads/main\0report-status", "0".repeat(40), oid(&src, "HEAD"))], &pack, || {});
    let report = String::from_utf8(report).unwrap();
    assert!(report.contains("unpack failed"), "{report}");
    assert!(report.contains("ng refs/heads/main unpacker error"), "{report}");
    assert!(!dst.join("refs/heads/main").exists());
    assert!(!dst.join("gitr/objects.redb").exists());
    assert!(fs::read_dir(dst.join("objects")).unwrap().all(|entry| matches!(entry.unwrap().file_name().to_str(), Some("info" | "pack"))));
}

#[test]
fn accepted_remote_push_does_not_overwrite_a_concurrent_tracking_change() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("source"); let dst = temp.path().join("remote");
    source(&src); destination(&dst);
    git(&src, &["remote", "add", "origin", dst.to_str().unwrap()]);
    success(native(&src, &["push", "origin", "main"]));
    git(&src, &["commit", "--allow-empty", "-qm", "second"]);
    let new = oid(&src, "HEAD");
    let hook = src.join(".git/hooks/pre-push");
    grit_test_support::compile_rust_fixture(&hook, r#"fn main() { std::fs::write(std::env::var_os("GITR_TRACKING_PATH").unwrap(), std::env::var("GITR_TRACKING_CONTENT").unwrap()).unwrap(); }"#).unwrap();
    let tracking = src.join(".git/refs/remotes/origin/main");
    let out = command(NATIVE, &src).args(["push", "origin", "main"]).env("PATH", "")
        .env("GITR_TRACKING_PATH", &tracking).env("GITR_TRACKING_CONTENT", format!("{new}\n")).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("remote accepted refs/heads/main, but local tracking ref"), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(oid(&dst, "refs/heads/main"), new);
    assert_eq!(fs::read_to_string(tracking).unwrap(), format!("{new}\n"));
}

#[test]
fn unsupported_atomic_and_receiving_hooks_fail_before_remote_publication() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("source"); let dst = temp.path().join("remote");
    source(&src); destination(&dst);
    let out = native(&src, &["push", "--atomic", dst.to_str().unwrap(), "main"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("atomic push"));
    let helper = temp.path().join("reject-hook");
    grit_test_support::compile_rust_fixture(&helper, r#"fn main() { std::fs::write(std::env::var_os("GITR_PUSH_HOOK_MARKER").unwrap(), b"called").unwrap(); std::process::exit(1); }"#).unwrap();
    git(&dst, &["config", "hook.reject.event", "pre-receive"]);
    git(&dst, &["config", "hook.reject.command", helper.to_str().unwrap()]);
    let marker = temp.path().join("hook-ran");
    let out = command(NATIVE, &src).args(["push", dst.to_str().unwrap(), "main"])
        .env("PATH", "").env("GITR_PUSH_HOOK_MARKER", &marker).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("refuses configured pre-receive hooks"));
    assert!(!marker.exists()); assert!(!dst.join("refs/heads/main").exists());
    assert!(!dst.join("gitr/objects.redb").exists());
    assert!(fs::read_dir(dst.join("objects")).unwrap().all(|entry| matches!(entry.unwrap().file_name().to_str(), Some("info" | "pack"))));
}
