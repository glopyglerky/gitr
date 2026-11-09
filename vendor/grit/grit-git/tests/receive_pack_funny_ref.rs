//! Regression for receive-pack rejecting traversal ref names (issue #894).

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use tempfile::tempdir;

const TRAVERSAL_REF: &str = "refs/heads/../../config";

fn install_hook_tracers(repo: &Path, log: &Path) {
    let hooks = repo.join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    fs::write(hooks.join("fixture-log-path"), log.to_string_lossy().as_bytes()).unwrap();
    let helper = hooks.join("pre-receive");
    grit_test_support::compile_rust_fixture(&helper, r#"
use std::{env, fs, io::Write};
fn main() -> std::io::Result<()> {
    let executable = env::current_exe()?;
    let name = executable.file_name().unwrap().to_string_lossy();
    let log_path = fs::read_to_string(executable.parent().unwrap().join("fixture-log-path"))?;
    let mut log = fs::OpenOptions::new().create(true).append(true).open(log_path)?;
    write!(log, "{name}")?;
    if name != "pre-receive" {
        if let Some(argument) = env::args().nth(1) { write!(log, " {argument}")?; }
    }
    writeln!(log)?;
    Ok(())
}
"#).unwrap();
    fs::copy(&helper, hooks.join("reference-transaction")).unwrap();
    fs::copy(&helper, hooks.join("update")).unwrap();
}

fn traversal_delete_pkt_line() -> String {
    let zero = "0".repeat(40);
    let line = format!("{zero} {zero} {TRAVERSAL_REF}\n");
    let len = format!("{:04x}", 4 + line.len());
    format!("{len}{line}0000")
}

fn run_receive_pack(repo: &Path, input: &str) -> std::process::Output {
    let grit = env!("CARGO_BIN_EXE_grit-git");
    let mut child = Command::new(grit)
        .arg("receive-pack")
        .arg(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn receive_pack_rejects_traversal_ref_delete() {
    let dir = tempdir().unwrap();
    let repo = dir.path().join("repo");
    let grit = env!("CARGO_BIN_EXE_grit-git");
    assert!(Command::new(grit)
        .args(["init", "--bare", repo.to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    assert!(repo.join("config").is_file());

    let out = run_receive_pack(&repo, &traversal_delete_pkt_line());
    assert!(out.status.success());

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("error: refusing to update funny ref 'refs/heads/../../config' remotely"),
        "stderr={stderr}"
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ng refs/heads/../../config funny refname"),
        "stdout={stdout}"
    );
    assert!(
        repo.join("config").is_file(),
        "bare repository config must not be deleted"
    );
}

#[test]
fn receive_pack_funny_ref_skips_reference_transaction_and_update_hooks() {
    let dir = tempdir().unwrap();
    let repo = dir.path().join("repo");
    let grit = env!("CARGO_BIN_EXE_grit-git");
    assert!(Command::new(grit)
        .args(["init", "--bare", repo.to_str().unwrap()])
        .status()
        .unwrap()
        .success());

    let hook_log = dir.path().join("hook.log");
    install_hook_tracers(&repo, &hook_log);

    let out = run_receive_pack(&repo, &traversal_delete_pkt_line());
    assert!(out.status.success());
    assert!(repo.join("config").is_file());

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("error: refusing to update funny ref 'refs/heads/../../config' remotely")
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ng refs/heads/../../config funny refname"));

    let log = fs::read_to_string(&hook_log).unwrap_or_default();
    assert_eq!(
        log.trim(),
        "pre-receive",
        "malformed ref must not reach reference-transaction or update hooks; log={log:?}"
    );
}
