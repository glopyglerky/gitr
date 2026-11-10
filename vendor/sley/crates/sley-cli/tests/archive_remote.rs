// SPDX-License-Identifier: MIT OR Apache-2.0
//! Exercise the real archive client against Git and native upload-archive
//! services over pipes. SSH authentication and live sockets are outside these
//! fixtures; the SSH shim preserves the actual service command and protocol.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sley-archive-client-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn candidate() -> String {
    match std::env::var_os("RIT_TEST_BIN")
        .or_else(|| std::env::var_os("GITR_TEST_BIN"))
    {
        Some(path) => {
            assert!(
                Path::new(&path).is_absolute(),
                "RIT_TEST_BIN must be absolute"
            );
            path.into_string().expect("RIT_TEST_BIN must be UTF-8")
        }
        None => sley_testkit::sley_bin!().to_owned(),
    }
}

struct Fixture {
    root: TempDir,
    repo: PathBuf,
    tools: PathBuf,
    ssh: PathBuf,
}

impl Fixture {
    fn new(format: &str) -> Self {
        let root = TempDir::new();
        let repo = root.path().join("repo space's");
        let tools = root.path().join("tools");
        fs::create_dir(&repo).unwrap();
        fs::create_dir(&tools).unwrap();
        let ssh = tools.join("archive-ssh");
        script(
            &tools.join("git"),
            "printf fallback > \"$ARCHIVE_FALLBACK\"; exit 97\n",
        );
        script(
            &ssh,
            &format!(
                "test -z \"${{GIT_PROTOCOL+x}}\" || exit 86\n\
             printf '%s\\n' \"$@\" > \"$ARCHIVE_SSH_ARGUMENTS\"\n\
             for argument in \"$@\"; do service=$argument; done\n\
             GIT_EXEC_PATH={}; export GIT_EXEC_PATH\n\
             exec /bin/sh -c \"$service\"\n",
                sley_config::sq_quote(
                    &String::from_utf8(
                        Command::new(sley_testkit::oracle_git())
                            .arg("--exec-path")
                            .output()
                            .unwrap()
                            .stdout,
                    )
                    .unwrap()
                    .trim()
                ),
            ),
        );
        let fixture = Self {
            root,
            repo,
            tools,
            ssh,
        };
        fixture.git(&[
            "init",
            "-q",
            "--initial-branch=main",
            &format!("--object-format={format}"),
        ]);
        fs::create_dir(fixture.repo.join("nested")).unwrap();
        fs::write(fixture.repo.join("nested/file.txt"), b"inside\n").unwrap();
        fs::write(
            fixture.repo.join("large.bin"),
            (0..200_000).map(|i| (i % 251) as u8).collect::<Vec<_>>(),
        )
        .unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-qm", "archive fixture"]);
        fixture.git(&["tag", "release"]);
        fixture
    }

    fn command(&self, program: &str) -> Command {
        let mut command = sley_testkit::hermetic_git_command_with_identity(program);
        command
            .current_dir(self.root.path())
            .env("TZ", "UTC")
            .env("GIT_AUTHOR_DATE", "@1234567890 +0000")
            .env("GIT_COMMITTER_DATE", "@1234567890 +0000")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .env_remove("GIT_PROXY_COMMAND")
            .env_remove("GIT_SSH_COMMAND");
        command
    }

    fn git(&self, args: &[&str]) -> Vec<u8> {
        let output = self
            .command(sley_testkit::oracle_git())
            .current_dir(&self.repo)
            .args(args)
            .output()
            .unwrap();
        success(&output);
        output.stdout
    }

    fn client(&self) -> Command {
        let mut command = self.command(&candidate());
        command
            .env("PATH", &self.tools)
            .env("GIT_SSH", &self.ssh)
            .env("GIT_SSH_VARIANT", "simple")
            .env("GIT_PROTOCOL", "version=2")
            .env("ARCHIVE_FALLBACK", self.root.path().join("fallback"))
            .env(
                "ARCHIVE_SSH_ARGUMENTS",
                self.root.path().join("ssh-arguments"),
            );
        command
    }

    fn remote(&self) -> String {
        format!("archive.invalid:{}", self.repo.display())
    }

    fn service(&self, native: bool) -> String {
        let program = if native {
            candidate()
        } else {
            sley_testkit::oracle_git().to_owned()
        };
        format!("{} upload-archive", sley_config::sq_quote(&program))
    }

    fn archive(&self, native_server: bool, args: &[&str]) -> Output {
        self.client()
            .args([
                "archive",
                "--remote",
                &self.remote(),
                "--exec",
                &self.service(native_server),
            ])
            .args(args)
            .output()
            .unwrap()
    }

    fn no_fallback(&self) {
        assert!(
            !self.root.path().join("fallback").exists(),
            "client invoked PATH Git"
        );
    }
}

fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn ssh_archive_matches_git_for_both_hash_formats_without_path_git() {
    for format in ["sha1", "sha256"] {
        let fixture = Fixture::new(format);
        for args in [
            vec!["--format=tar", "--prefix=release/", "HEAD"],
            vec!["--format=zip", "HEAD", "nested"],
            vec!["--format=tar.gz", "HEAD"],
            vec!["--format=tar", "--mtime=@1234567890", "HEAD:nested"],
            vec!["--list"],
        ] {
            let actual = fixture.archive(false, &args);
            success(&actual);
            let mut oracle_args = vec!["archive"];
            oracle_args.extend_from_slice(&args);
            assert_eq!(
                actual.stdout,
                fixture.git(&oracle_args),
                "{format}: {args:?}"
            );
        }
        let native = fixture.archive(true, &["--format=tar", "HEAD"]);
        success(&native);
        assert_eq!(
            native.stdout,
            fixture.git(&["archive", "--format=tar", "HEAD"])
        );
        fixture.no_fallback();
    }
}

#[test]
fn remote_archive_output_infers_format_and_respects_explicit_override() {
    let fixture = Fixture::new("sha1");
    for (name, options, format) in [
        ("inferred.zip", vec![], "zip"),
        ("explicit.zip", vec!["--format=tar"], "tar"),
        ("inferred.tar.gz", vec![], "tar.gz"),
    ] {
        let path = fixture.root.path().join(name);
        let mut args = vec!["--output", path.to_str().unwrap()];
        args.extend(options);
        args.push("HEAD");
        let actual = fixture.archive(false, &args);
        success(&actual);
        assert!(actual.stdout.is_empty());
        assert_eq!(
            fs::read(path).unwrap(),
            fixture.git(&["archive", &format!("--format={format}"), "HEAD"])
        );
    }
    fixture.no_fallback();
}

#[test]
fn remote_output_inference_uses_client_formats_and_server_permissions() {
    for format in ["sha1", "sha256"] {
        let fixture = Fixture::new(format);
        // Explicitly configured archive filters require a shell. Git remains
        // trapped; only this caller-selected compatibility dependency is added.
        std::os::unix::fs::symlink("/bin/sh", fixture.tools.join("sh")).unwrap();
        fixture.git(&["config", "tar.foo.command", "/bin/cat; printf MARKER"]);
        fixture.git(&["config", "tar.foo.remote", "true"]);
        let client = fixture.root.path().join("client");
        fs::create_dir(&client).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "tar.foo.command", "/bin/cat"],
        ] {
            success(
                &fixture
                    .command(sley_testkit::oracle_git())
                    .current_dir(&client)
                    .args(args)
                    .output()
                    .unwrap(),
            );
        }
        let expected_path = client.join("expected.foo");
        success(
            &fixture
                .command(sley_testkit::oracle_git())
                .current_dir(&client)
                .args([
                    "archive",
                    "--remote",
                    fixture.repo.to_str().unwrap(),
                    "--output",
                    expected_path.to_str().unwrap(),
                    "HEAD",
                ])
                .output()
                .unwrap(),
        );
        let expected = fs::read(expected_path).unwrap();
        assert!(expected.ends_with(b"MARKER"));
        // Exercise both the native local service path and an actual Git
        // upload-archive service through the configured SSH shim.
        for ssh in [false, true] {
            let output = client.join(if ssh { "ssh.foo" } else { "local.foo" });
            let mut command = fixture.client();
            command.current_dir(&client).arg("archive");
            if ssh {
                command.args([
                    "--remote",
                    &fixture.remote(),
                    "--exec",
                    &fixture.service(false),
                ]);
            } else {
                command.args(["--remote", fixture.repo.to_str().unwrap()]);
            }
            success(
                &command
                    .args(["--output", output.to_str().unwrap(), "HEAD"])
                    .output()
                    .unwrap(),
            );
            assert_eq!(fs::read(output).unwrap(), expected);
        }
        // Selecting the name must not bypass a server that disables it.
        fixture.git(&["config", "tar.foo.remote", "false"]);
        let denied = fixture
            .client()
            .current_dir(&client)
            .args([
                "archive",
                "--remote",
                &fixture.remote(),
                "--exec",
                &fixture.service(false),
                "--output",
                client.join("denied.foo").to_str().unwrap(),
                "HEAD",
            ])
            .output()
            .unwrap();
        assert!(!denied.status.success());
        fixture.no_fallback();
    }
}

#[test]
fn configured_remote_rewrite_and_ssh_command_work_outside_and_inside_repositories() {
    let fixture = Fixture::new("sha1");
    fixture.git(&["remote", "add", "origin", &fixture.remote()]);
    let output = fixture
        .client()
        .current_dir(&fixture.repo)
        .env_remove("GIT_SSH")
        .args([
            "-c",
            &format!("core.sshCommand={} --configured", fixture.ssh.display()),
            "archive",
            "--remote=origin",
            "--exec",
            &fixture.service(false),
            "HEAD",
        ])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(output.stdout, fixture.git(&["archive", "HEAD"]));
    assert!(
        fs::read_to_string(fixture.root.path().join("ssh-arguments"))
            .unwrap()
            .starts_with("--configured\n")
    );
    let output = fixture
        .client()
        .args([
            "-c",
            &format!("url.{}.insteadOf=short:", fixture.remote()),
            "archive",
            "--remote=short:",
            "--exec",
            &fixture.service(false),
            "HEAD",
        ])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(output.stdout, fixture.git(&["archive", "HEAD"]));
    fixture.no_fallback();
}

#[test]
fn remote_archive_enforces_policy_and_server_ref_failures() {
    let fixture = Fixture::new("sha1");
    let output = fixture
        .client()
        .args([
            "-c",
            "protocol.ssh.allow=never",
            "archive",
            "--remote",
            &fixture.remote(),
            "HEAD",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!fixture.root.path().join("ssh-arguments").exists());
    let output = fixture.archive(false, &["HEAD~1"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no such ref"));
    let output = fixture.archive(false, &["--add-file=absent-secret", "HEAD"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used together"));
    fixture.no_fallback();
}

#[test]
fn local_explicit_archive_service_is_used_and_failures_propagate() {
    let fixture = Fixture::new("sha1");
    let output = fixture
        .client()
        .args([
            "archive",
            "--remote",
            fixture.repo.to_str().unwrap(),
            "--exec",
            &fixture.service(false),
            "HEAD",
        ])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(output.stdout, fixture.git(&["archive", "HEAD"]));
    let failure = fixture.tools.join("failing-archive");
    script(&failure, "exit 23\n");
    let output = fixture
        .client()
        .args([
            "archive",
            "--remote",
            fixture.repo.to_str().unwrap(),
            "--exec",
            failure.to_str().unwrap(),
            "HEAD",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "failed service must not generate a substitute archive"
    );
    let output = fixture
        .client()
        .args([
            "archive",
            "--remote",
            fixture.repo.to_str().unwrap(),
            "HEAD",
        ])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(output.stdout, fixture.git(&["archive", "HEAD"]));
    fixture.no_fallback();
}

#[test]
fn git_daemon_archive_interoperates_through_explicit_proxy() {
    let fixture = Fixture::new("sha256");
    let proxy = fixture.tools.join("archive-proxy");
    script(
        &proxy,
        &format!(
            "exec {} daemon --inetd --export-all --enable=upload-archive\n",
            sley_config::sq_quote(sley_testkit::oracle_git())
        ),
    );
    let remote = format!("git://archive.invalid{}", fixture.repo.display());
    let output = fixture
        .client()
        .env("GIT_PROXY_COMMAND", proxy)
        .env_remove("GIT_PROTOCOL")
        .args([
            "-c",
            "protocol.version=1",
            "archive",
            "--remote",
            &remote,
            "--format=zip",
            "HEAD",
        ])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(
        output.stdout,
        fixture.git(&["archive", "--format=zip", "HEAD"])
    );
    fixture.no_fallback();
}

#[test]
fn malformed_archive_peer_is_terminated_without_hanging() {
    let fixture = Fixture::new("sha1");
    let peer = fixture.tools.join("bad-peer");
    // Write a valid ACK followed by an invalid channel, then continue writing
    // enough data to fill a pipe. The client must stop and reap this process.
    script(
        &peer,
        "printf '0008ACK\\n00000006\\004x'; while :; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; done\n",
    );
    for proxy in [false, true] {
        let mut command = fixture.client();
        if proxy {
            command.env("GIT_PROXY_COMMAND", &peer).args([
                "archive",
                "--remote=git://archive.invalid/repo",
                "HEAD",
            ]);
        } else {
            command.args([
                "archive",
                "--remote",
                &fixture.remote(),
                "--exec",
                peer.to_str().unwrap(),
                "HEAD",
            ]);
        }
        let mut child = command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(!status.success());
                break;
            }
            if start.elapsed() > Duration::from_secs(10) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("client hung waiting for malformed peer, proxy={proxy}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fixture.no_fallback();
}

#[test]
fn failed_archive_service_terminates_descendants_holding_stderr() {
    let fixture = Fixture::new("sha1");
    let peer = fixture.tools.join("descendant-peer");
    let descendant_pid = fixture.root.path().join("descendant-pid");
    // The descendant writes no stdout. Killing only the shell leaves the
    // client's stderr-drain thread blocked until this inherited pipe closes.
    script(
        &peer,
        "/bin/sleep 30 >/dev/null &\nprintf '%s' \"$!\" > \"$ARCHIVE_DESCENDANT_PID\"\nprintf '0008ACK\\n00000006\\004x'\nwait\n",
    );
    for remote in [
        fixture.repo.to_string_lossy().into_owned(),
        fixture.remote(),
    ] {
        let mut child = fixture
            .client()
            .env("ARCHIVE_DESCENDANT_PID", &descendant_pid)
            .args([
                "archive",
                "--remote",
                &remote,
                "--exec",
                peer.to_str().unwrap(),
                "HEAD",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(!status.success());
                assert!(start.elapsed() < Duration::from_secs(3));
                break;
            }
            if start.elapsed() > Duration::from_secs(3) {
                let _ = child.kill();
                let _ = child.wait();
                if let Ok(pid) = fs::read_to_string(&descendant_pid) {
                    let _ = Command::new("/bin/kill").args(["-KILL", &pid]).status();
                }
                panic!("failed archive hung on descendant stderr: {remote}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fixture.no_fallback();
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn complete_archive_preserves_status_and_cleans_descendant_stderr() {
    let fixture = Fixture::new("sha1");
    let peer = fixture.tools.join("complete-descendant-peer");
    let descendant_pid = fixture.root.path().join("complete-descendant-pid");
    for exit in [0, 23] {
        script(
            &peer,
            &format!(
                "/bin/sleep 30 >/dev/null &\nprintf '%s' \"$!\" > \"$ARCHIVE_DESCENDANT_PID\"\n\
             /bin/dd bs=1 count=22 of=/dev/null 2>/dev/null\n\
             printf '0008ACK\\n0000000c\\001payload0000'\nexit {exit}\n",
            ),
        );
        for remote in [
            fixture.repo.to_string_lossy().into_owned(),
            fixture.remote(),
        ] {
            let mut child = fixture
                .client()
                .env("ARCHIVE_DESCENDANT_PID", &descendant_pid)
                .args([
                    "archive",
                    "--remote",
                    &remote,
                    "--exec",
                    peer.to_str().unwrap(),
                    "HEAD",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let start = Instant::now();
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert_eq!(
                        status.success(),
                        exit == 0,
                        "preserve service status {exit}"
                    );
                    assert!(start.elapsed() < Duration::from_secs(3));
                    let output = child.wait_with_output().unwrap();
                    assert_eq!(output.stdout, b"payload");
                    break;
                }
                if start.elapsed() > Duration::from_secs(3) {
                    let _ = child.kill();
                    let _ = child.wait();
                    if let Ok(pid) = fs::read_to_string(&descendant_pid) {
                        let _ = Command::new("/bin/kill").args(["-KILL", &pid]).status();
                    }
                    panic!("complete archive hung on descendant stderr: {remote}");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    fixture.no_fallback();
}

#[test]
fn archive_client_rejects_truncation_and_nonzero_exit_after_payload() {
    let fixture = Fixture::new("sha1");
    let peer = fixture.tools.join("partial-peer");
    let response = fixture.root.path().join("response");
    let mut bytes = b"0008ACK\n0000".to_vec();
    sley_protocol::write_sideband_payload(
        &mut bytes,
        sley_protocol::SideBandChannel::Data,
        b"partial archive",
    )
    .unwrap();
    fs::write(&response, &bytes).unwrap();
    script(
        &peer,
        &format!(
            "/bin/dd bs=1 count=22 of=/dev/null 2>/dev/null; /bin/cat {}; exit 0\n",
            sley_config::sq_quote(response.to_str().unwrap())
        ),
    );
    let output = fixture.archive(false, &["--exec", peer.to_str().unwrap(), "HEAD"]);
    assert!(!output.status.success(), "missing final flush must fail");
    bytes.extend_from_slice(b"0000");
    fs::write(&response, &bytes).unwrap();
    script(
        &peer,
        &format!(
            "/bin/dd bs=1 count=22 of=/dev/null 2>/dev/null; /bin/cat {}; exit 23\n",
            sley_config::sq_quote(response.to_str().unwrap())
        ),
    );
    let output = fixture.archive(false, &["--exec", peer.to_str().unwrap(), "HEAD"]);
    assert!(
        !output.status.success(),
        "service exit status must survive a complete stream"
    );
    assert_eq!(output.stdout, b"partial archive");
    // The proxy receives one daemon service request before the same 22-byte
    // HEAD argument request. Consume both before producing the reply so the
    // test does not race a peer exit against the client's request writes.
    let prelude_length = b"git-upload-archive /repo\0host=archive.invalid\0".len() + 4;
    script(
        &peer,
        &format!(
            "/bin/dd bs=1 count={} of=/dev/null 2>/dev/null; /bin/cat {}; exit 23\n",
            prelude_length + 22,
            sley_config::sq_quote(response.to_str().unwrap()),
        ),
    );
    let output = fixture
        .client()
        .env("GIT_PROXY_COMMAND", &peer)
        .args(["archive", "--remote=git://archive.invalid/repo", "HEAD"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "proxy exit status must survive a complete stream"
    );
    assert_eq!(output.stdout, b"partial archive");
    fixture.no_fallback();
}

#[test]
fn non_utf8_archive_argument_is_rejected_before_starting_transport() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new("sha1");
    let output = fixture
        .client()
        .args(["archive", "--remote", &fixture.remote(), "HEAD"])
        .arg(std::ffi::OsString::from_vec(b"raw-\xff".to_vec()))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!fixture.root.path().join("ssh-arguments").exists());
    fixture.no_fallback();
}

#[test]
fn local_archive_format_listing_preserves_git_option_handling() {
    let fixture = Fixture::new("sha1");
    let args = ["archive", "--exec=no-such-program", "--list"];
    let actual = fixture.client().args(args).output().unwrap();
    success(&actual);
    assert_eq!(actual.stdout, fixture.git(&args));
    fixture.no_fallback();
}
