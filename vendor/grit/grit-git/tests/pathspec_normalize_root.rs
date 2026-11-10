//! Regression for repo-root pathspecs with leading `./` (t7501.76).
//!
//! `git add ./-` must stage `-`, not a literal `./-` path that would produce an invalid `.`
//! tree entry on write-tree / checkout.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use grit_test_support::configure_git_env;

const GRIT_BIN: &str = env!("CARGO_BIN_EXE_grit-git");

struct Output {
    status: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Output {
    fn ok(&self) -> bool {
        self.status == Some(0)
    }
}

fn unique_tmp(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut p = std::env::temp_dir();
    p.push(format!("grit-pathspec-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

fn grit(args: &[&str], dir: &Path) -> Output {
    let out = configure_git_env(&mut Command::new(GRIT_BIN))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn grit-git {args:?}: {e}"));
    Output {
        status: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

#[test]
fn add_dot_slash_dash_writes_tree_without_dot_entry() {
    let dir = unique_tmp("add-dot-dash");
    assert!(grit(&["init"], &dir).ok(), "init failed");

    std::fs::write(dir.join("-"), "body\n").expect("create file named '-'");

    let add = grit(&["add", "./-"], &dir);
    assert!(add.ok(), "add ./- failed: {}", add.stderr);

    let wt = grit(&["write-tree"], &dir);
    assert!(wt.ok(), "write-tree failed: {}", wt.stderr);
    let tree_oid = wt.stdout.trim();

    let ls = grit(&["ls-tree", tree_oid], &dir);
    assert!(ls.ok(), "ls-tree failed: {}", ls.stderr);
    assert!(
        ls.stdout.lines().any(|line| line.ends_with("\t-")),
        "expected tree entry named '-', got:\n{}",
        ls.stdout
    );
    assert!(
        !ls.stdout.lines().any(|line| line.ends_with("\t.")),
        "tree must not contain a '.' entry:\n{}",
        ls.stdout
    );

    let commit = grit(&["commit", "-m", "add dash"], &dir);
    assert!(commit.ok(), "commit failed: {}", commit.stderr);
    let head = grit(&["rev-parse", "HEAD"], &dir);
    assert!(head.ok());
    let commit_oid = head.stdout.trim();

    let checkout = grit(&["checkout", commit_oid], &dir);
    assert!(
        checkout.ok(),
        "checkout of commit with '-' file failed: {}",
        checkout.stderr
    );
    assert!(dir.join("-").is_file(), "checked-out file '-' missing");
}

#[test]
fn add_dot_slash_colon_bang_filename_does_not_exclude_other_files() {
    let dir = unique_tmp("add-colon-bang");
    assert!(grit(&["init"], &dir).ok(), "init failed");

    std::fs::write(dir.join(":!foo"), "x\n").expect("create :!foo");
    std::fs::write(dir.join("other"), "y\n").expect("create other");

    let add = grit(&["add", "./:!foo"], &dir);
    assert!(add.ok(), "add ./:!foo failed: {}", add.stderr);

    let cached = grit(&["diff", "--cached", "--name-only"], &dir);
    assert!(cached.ok(), "diff --cached failed: {}", cached.stderr);
    let names: Vec<&str> = cached.stdout.lines().collect();
    assert_eq!(
        names,
        vec![":!foo"],
        "expected only literal :!foo staged, got: {:?}",
        names
    );
}

#[test]
fn command_environment_excludes_host_configuration_but_retains_local_config() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dir = temp.path();
    assert!(grit(&["init"], dir).ok(), "init failed");
    let local = grit(&["config", "fixture.local", "kept"], dir);
    assert!(local.ok(), "local config setup failed: {}", local.stderr);
    let host_config = dir.join("host-config");
    std::fs::write(&host_config,
        "[filter \"lfs\"]\nprocess = unavailable-host-helper\n[fixture]\nleaked = host\n")
        .expect("host config fixture");

    for args in [vec!["rev-parse", "--git-dir"], vec!["config", "--get-regexp", "^fixture\\."]] {
        let mut command = Command::new(GRIT_BIN);
        command.args(&args).current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", &host_config)
            .env("GIT_CONFIG_SYSTEM", &host_config)
            .env("GIT_CONFIG", &host_config)
            .env("GIT_CONFIG_PARAMETERS", "'fixture.injected=parameters'")
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "fixture.injected")
            .env("GIT_CONFIG_VALUE_0", "environment");
        let output = configure_git_env(&mut command).output().expect("isolated command");
        assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
        if args[0] == "config" {
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "fixture.local kept");
        }
    }

    // An explicit test configuration applied afterward still reaches the
    // production read guard. Isolation must not disable that guard.
    let output = configure_git_env(&mut Command::new(GRIT_BIN))
        .args(["rev-parse", "--git-dir"]).current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", &host_config).output().expect("explicit config command");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("filter.lfs.process"));
}
