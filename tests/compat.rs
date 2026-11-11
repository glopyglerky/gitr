// SPDX-License-Identifier: MIT OR Apache-2.0
//! Exercise process forwarding against the exact built Git companion.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

fn configured_companion() -> PathBuf {
    let binary = std::env::var_os("RIT_GIT_COMPAT")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join(".rit-ci/git-compat/bin/git"));
    assert!(binary.is_absolute() && binary.is_file(),
        "build the pinned companion with ci/build_git_compat.rs and set RIT_GIT_COMPAT before running compatibility acceptance tests");
    binary
}

fn run(program: &Path, companion: &Path, cwd: &Path, args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(program)
        .current_dir(cwd)
        .env_clear()
        .env("PATH", "")
        .env("HOME", cwd)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", cwd.join("no-config"))
        .env("RIT_GIT_COMPAT", companion)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn().unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn compatibility_preserves_stdio_cwd_exit_status_and_ordinary_environment() {
    let companion = configured_companion();
    let temp = tempfile::tempdir().unwrap();
    let rit = Path::new(env!("CARGO_BIN_EXE_rit"));
    for (args, input) in [
        (vec!["--version"], b"".as_slice()),
        (vec!["hash-object", "--stdin"], b"abc\0\xff\n".as_slice()),
        (vec!["config", "--file", "missing-config", "--get", "test.absent"], b"".as_slice()),
        (vec!["rev-parse", "--show-toplevel"], b"".as_slice()),
    ] {
        let expected = run(&companion, &companion, temp.path(), &args, input);
        let actual = run(rit, &companion, temp.path(), &args, input);
        assert_eq!(actual.status.code(), expected.status.code(), "{args:?}");
        assert_eq!(actual.stdout, expected.stdout, "{args:?}");
        assert_eq!(actual.stderr, expected.stderr, "{args:?}");
    }
    assert!(run(rit, &companion, temp.path(), &["init", "--quiet", "repository"], b"").status.success());
    assert!(temp.path().join("repository/.git/HEAD").is_file());
    let expected = run(&companion, &companion, temp.path(), &["-C", "repository", "rev-parse", "--show-toplevel"], b"");
    let actual = run(rit, &companion, temp.path(), &["-C", "repository", "rev-parse", "--show-toplevel"], b"");
    assert_eq!(actual.stdout, expected.stdout);
    assert_eq!(actual.status.code(), expected.status.code());
}

#[test]
fn invalid_backend_and_missing_companion_fail_before_git_mutation() {
    let temp = tempfile::tempdir().unwrap();
    for backend in ["unrecognized", "compat"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rit"))
            .current_dir(temp.path())
            .env("RIT_BACKEND", backend)
            .env("RIT_GIT_COMPAT", temp.path().join("absent/bin/git"))
            .args(["init", "not-created"])
            .output().unwrap();
        assert_eq!(output.status.code(), Some(128));
        assert!(!temp.path().join("not-created").exists());
    }
    // A deliberate native invocation remains usable without the companion.
    let output = Command::new(env!("CARGO_BIN_EXE_rit"))
        .current_dir(temp.path())
        .env("RIT_BACKEND", "native")
        .env("RIT_GIT_COMPAT", temp.path().join("absent/bin/git"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", temp.path().join("absent-config"))
        .args(["init", "native-created"])
        .output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(temp.path().join("native-created/.git/HEAD").is_file());
}

#[test]
fn companion_requires_the_matching_source_receipt_and_rejects_rit() {
    let companion = configured_companion();
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let binary = bin.join(if cfg!(windows) { "git.exe" } else { "git" });
    fs::copy(&companion, &binary).unwrap();
    for receipt in [None, Some("revision=wrong\n")] {
        if let Some(receipt) = receipt {
            fs::write(temp.path().join("rit-git-source.txt"), receipt).unwrap();
        }
        let output = run(Path::new(env!("CARGO_BIN_EXE_rit")), &binary, temp.path(), &["init", "not-created"], b"");
        assert_eq!(output.status.code(), Some(128));
        assert!(!temp.path().join("not-created").exists());
    }
    fs::copy(env!("CARGO_BIN_EXE_rit"), &binary).unwrap();
    fs::write(temp.path().join("rit-git-source.txt"), include_str!("../compat/git-source.txt")).unwrap();
    let output = run(Path::new(env!("CARGO_BIN_EXE_rit")), &binary, temp.path(), &["init", "not-created"], b"");
    assert_eq!(output.status.code(), Some(128));
    assert!(String::from_utf8_lossy(&output.stderr).contains("recursive"));
    assert!(!temp.path().join("not-created").exists());
}

#[cfg(unix)]
#[test]
fn compatibility_preserves_non_utf8_filename_arguments() {
    use std::os::unix::ffi::OsStringExt;
    let companion = configured_companion();
    let temp = tempfile::tempdir().unwrap();
    let name = std::ffi::OsString::from_vec(b"file-\xff".to_vec());
    fs::write(temp.path().join(&name), b"raw filename\n").unwrap();
    let invoke = |program: &Path| Command::new(program)
        .current_dir(temp.path())
        .env("RIT_BACKEND", "compat")
        .env("RIT_GIT_COMPAT", &companion)
        .env_remove("RIT_TRACE_BACKEND")
        .args(["hash-object", "--"]).arg(&name).output().unwrap();
    let expected = invoke(&companion);
    let actual = invoke(Path::new(env!("CARGO_BIN_EXE_rit")));
    assert!(expected.status.success());
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout);
    assert_eq!(actual.stderr, expected.stderr);
}
