//! Native CLI trust boundaries: arguments, aliases and configured verifiers.
#![cfg(unix)]

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const NATIVE: &str = env!("CARGO_BIN_EXE_grit-git");
const HELPER: &str = r#"
fn main() -> std::io::Result<()> {
    let marker = std::env::var_os("GITR_NATIVE_TEST_MARKER").expect("marker path");
    std::fs::write(marker, b"executed")
}
"#;

fn isolated_command(program: impl AsRef<std::ffi::OsStr>, dir: &Path) -> Command {
    let mut command = Command::new(program);
    command.current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_AUTHOR_DATE", "1700000000 +0000")
        .env("GIT_COMMITTER_DATE", "1700000000 +0000")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_EXTERNAL_DIFF");
    command
}

fn git(dir: &Path, arguments: &[&str]) -> Output {
    let output = isolated_command("git", dir).args(arguments).output().unwrap();
    assert!(output.status.success(), "git {arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
    output
}

fn repository(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["commit", "--allow-empty", "-q", "-m", "fixture"]);
}

fn helper(dir: &Path) -> PathBuf {
    let path = dir.join("native-fixture-helper");
    grit_test_support::compile_rust_fixture(&path, HELPER).unwrap();
    path
}

fn native(dir: &Path, arguments: &[&str], marker: &Path) -> Output {
    isolated_command(NATIVE, dir)
        .args(arguments)
        .env("GITR_NATIVE_TEST_MARKER", marker)
        .output().unwrap()
}

fn assert_refused(output: Output, marker: &Path, message: &str) {
    assert!(!output.status.success(), "expected rejection");
    assert!(String::from_utf8_lossy(&output.stderr).contains(message), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(!marker.exists(), "configuration-supplied helper executed");
}

#[test]
fn signature_inspection_never_runs_a_configured_verifier() {
    let temp = tempfile::tempdir().unwrap();
    repository(temp.path());
    let binary = helper(temp.path());
    let marker = temp.path().join("helper-ran");

    // Keep a real Git commit and add recognizable signature armor. Signature
    // validity must never be evaluated by the configured executable here.
    let original = String::from_utf8(git(temp.path(), &["cat-file", "commit", "HEAD"]).stdout).unwrap();
    let signed = original.replacen("\n\n", "\ngpgsig -----BEGIN PGP SIGNATURE-----\n AAAA\n -----END PGP SIGNATURE-----\n\n", 1);
    let mut child = isolated_command("git", temp.path())
        .args(["hash-object", "-w", "-t", "commit", "--stdin"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().unwrap();
    child.stdin.take().unwrap().write_all(signed.as_bytes()).unwrap();
    let hashed = child.wait_with_output().unwrap();
    assert!(hashed.status.success());
    let oid = String::from_utf8(hashed.stdout).unwrap();
    git(temp.path(), &["update-ref", "refs/heads/main", oid.trim()]);
    git(temp.path(), &["config", "log.showSignature", "true"]);

    for key in ["gpg.program", "gpg.openpgp.program", "gpg.x509.program", "gpg.ssh.program"] {
        git(temp.path(), &["config", key, binary.to_str().unwrap()]);
        for arguments in [vec!["log", "-1"], vec!["show", "HEAD"], vec!["verify-commit", "HEAD"]] {
            assert_refused(native(temp.path(), &arguments, &marker), &marker, key);
        }
        git(temp.path(), &["config", "--unset", key]);
    }
}

#[test]
fn inspection_alias_chains_cannot_execute_shell_or_dashed_helpers() {
    let temp = tempfile::tempdir().unwrap();
    repository(temp.path());
    let binary = helper(temp.path());
    let marker = temp.path().join("helper-ran");
    let shell_alias = format!("!{}", binary.display());
    git(temp.path(), &["config", "alias.whatchanged", &shell_alias]);
    assert_refused(native(temp.path(), &["whatchanged"], &marker), &marker, "inspection alias");

    git(temp.path(), &["config", "alias.whatchanged", "relay"]);
    git(temp.path(), &["config", "alias.relay", &shell_alias]);
    assert_refused(native(temp.path(), &["whatchanged"], &marker), &marker, "inspection alias");

    git(temp.path(), &["config", "--unset", "alias.relay"]);
    let helpers = temp.path().join("external-helpers");
    fs::create_dir(&helpers).unwrap();
    fs::copy(&binary, helpers.join("git-relay")).unwrap();
    let output = isolated_command(NATIVE, temp.path())
        .arg("whatchanged")
        .env("GIT_EXEC_PATH", &helpers)
        .env("GITR_NATIVE_TEST_MARKER", &marker)
        .output().unwrap();
    assert_refused(output, &marker, "external command");

    // An explicitly requested noninspection shell alias retains its interface.
    git(temp.path(), &["config", "alias.requested-helper", &shell_alias]);
    let output = native(temp.path(), &["requested-helper"], &marker);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(fs::read(&marker).unwrap(), b"executed");
}

#[test]
fn inspection_context_survives_alias_directory_and_config_transitions() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    repository(&first);
    repository(&second);
    let binary = helper(temp.path());
    let marker = temp.path().join("helper-ran");
    let alias = format!("-C {} -c alias.relay=!{} relay", second.display(), binary.display());
    git(&first, &["config", "alias.whatchanged", &alias]);
    assert_refused(native(&first, &["whatchanged"], &marker), &marker, "inspection alias");

    let shell_alias = format!("!{}", binary.display());
    git(&second, &["config", "alias.relay", &shell_alias]);
    let directory_alias = format!("-C {} relay", second.display());
    git(&first, &["config", "alias.whatchanged", &directory_alias]);
    assert_refused(native(&first, &["whatchanged"], &marker), &marker, "inspection alias");

    // Redirecting an inspection alias to a modifying builtin is also rejected.
    git(&first, &["config", "alias.whatchanged", "branch unexpected"]);
    assert_refused(native(&first, &["whatchanged"], &marker), &marker, "noninspection command");
    let refs = git(&first, &["for-each-ref", "--format=%(refname)"]);
    assert!(!String::from_utf8_lossy(&refs.stdout).contains("unexpected"));
}

#[test]
fn non_utf8_arguments_fail_before_traces_or_repository_writes() {
    let temp = tempfile::tempdir().unwrap();
    let invalid = OsString::from_vec(b"bad-\xff-name".to_vec());
    let trace = temp.path().join("trace.json");
    let output = isolated_command(NATIVE, temp.path())
        .arg("init").arg(&invalid)
        .env("GIT_TRACE2_EVENT", &trace)
        .output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not UTF-8"));
    assert!(!trace.exists(), "validation must run before trace creation");
    assert!(!temp.path().join(&invalid).exists());
    assert!(!temp.path().join("bad-\u{fffd}-name").exists());
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[test]
fn whitespace_named_monitor_is_not_treated_as_boolean_false() {
    let temp = tempfile::tempdir().unwrap();
    repository(temp.path());
    let binary = helper(temp.path());
    let marker = temp.path().join("helper-ran");
    fs::copy(&binary, temp.path().join(" false ")).unwrap();
    git(temp.path(), &["config", "core.fsmonitor", " false "]);
    assert_refused(native(temp.path(), &["status", "--porcelain"], &marker), &marker, "core.fsmonitor");
}

#[test]
fn self_update_fails_without_running_an_installer() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("installer-ran");
    let binary = helper(temp.path());
    for program in ["sh", "curl"] {
        fs::copy(&binary, temp.path().join(program)).unwrap();
    }
    let install = temp.path().join("install-target");
    let output = isolated_command(NATIVE, temp.path())
        .args(["update", "--dir"]).arg(&install)
        .env("PATH", temp.path())
        .env("GITR_NATIVE_TEST_MARKER", &marker)
        .output().unwrap();
    assert_refused(output, &marker, "self-update is not implemented");
    assert!(!install.exists());
}

#[test]
fn mergetool_requires_an_explicit_command_for_vimdiff() {
    let temp = tempfile::tempdir().unwrap();
    repository(temp.path());
    let file = temp.path().join("conflict");
    fs::write(&file, b"base\n").unwrap();
    git(temp.path(), &["add", "conflict"]);
    git(temp.path(), &["commit", "-qm", "base"]);
    git(temp.path(), &["checkout", "-qb", "other"]);
    fs::write(&file, b"remote\n").unwrap();
    git(temp.path(), &["commit", "-qam", "remote"]);
    git(temp.path(), &["checkout", "-q", "main"]);
    fs::write(&file, b"local\n").unwrap();
    git(temp.path(), &["commit", "-qam", "local"]);
    let merge = isolated_command("git", temp.path())
        .args(["merge", "--no-edit", "other"]).output().unwrap();
    assert!(!merge.status.success());
    let content_before = fs::read(&file).unwrap();
    let index_before = fs::read(temp.path().join(".git/index")).unwrap();
    let marker = temp.path().join("merge-tool-ran");
    let binary = temp.path().join("explicit merge tool");
    grit_test_support::compile_rust_fixture(&binary, r#"
fn main() -> std::io::Result<()> {
    for (name, expected) in [("BASE", b"base\n".as_slice()), ("LOCAL", b"local\n".as_slice()), ("REMOTE", b"remote\n".as_slice())] {
        let path = std::env::var_os(name).expect("merge path");
        assert_eq!(std::fs::read(path)?, expected);
    }
    let merged = std::env::var_os("MERGED").expect("merged path");
    std::fs::write(merged, b"resolved\n")?;
    std::fs::write(std::env::var_os("GITR_NATIVE_TEST_MARKER").unwrap(), b"executed")
}
"#).unwrap();
    git(temp.path(), &["config", "mergetool.vimdiff.path", binary.to_str().unwrap()]);
    assert_refused(
        native(temp.path(), &["mergetool", "--no-prompt"], &marker),
        &marker, "built-in vimdiff layout support is not implemented",
    );
    assert_eq!(fs::read(&file).unwrap(), content_before);
    assert_eq!(fs::read(temp.path().join(".git/index")).unwrap(), index_before);

    let command = shell_words::quote(binary.to_str().unwrap()).into_owned();
    git(temp.path(), &["config", "mergetool.vimdiff.cmd", &command]);
    let output = native(temp.path(), &["mergetool", "--no-prompt"], &marker);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(fs::read(&file).unwrap(), b"resolved\n");
    assert_eq!(fs::read(&marker).unwrap(), b"executed");
}

#[test]
#[cfg(feature = "test-tools")]
fn chmtime_uses_native_file_times_and_preserves_access_time() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("timestamp-target");
    fs::write(&file, b"content").unwrap();
    let atime = filetime::FileTime::from_unix_time(1_600_000_000, 123_000_000);
    filetime::set_file_times(&file, atime, atime).unwrap();
    let output = isolated_command(NATIVE, temp.path())
        .args(["test-tool", "chmtime", "=1700000000"]).arg(&file)
        .env("PATH", temp.path())
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let metadata = fs::metadata(&file).unwrap();
    assert_eq!(filetime::FileTime::from_last_modification_time(&metadata), filetime::FileTime::from_unix_time(1_700_000_000, 0));
    assert_eq!(filetime::FileTime::from_last_access_time(&metadata), atime);
}

#[test]
fn builtin_diff_context_is_computed_without_an_external_grep() {
    let temp = tempfile::tempdir().unwrap();
    repository(temp.path());
    fs::write(temp.path().join(".gitattributes"), b"*.rs diff=rust\n").unwrap();
    let file = temp.path().join("example.rs");
    fs::write(&file, b"fn greet() {\n    println!(\"first\");\n}\n").unwrap();
    git(temp.path(), &["add", ".gitattributes", "example.rs"]);
    git(temp.path(), &["commit", "-qm", "source"]);
    fs::write(&file, b"fn greet() {\n    println!(\"second\");\n}\n").unwrap();
    let output = isolated_command(NATIVE, temp.path())
        .args(["diff", "--no-ext-diff", "--no-textconv", "--unified=0", "--", "example.rs"])
        .env("PATH", temp.path())
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let diff = String::from_utf8(output.stdout).unwrap();
    assert!(diff.contains("@@ fn greet() {"), "{diff}");
}
