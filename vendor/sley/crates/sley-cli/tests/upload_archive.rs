//! Real upload-archive peers: Git supplies fixtures and a client; the service
//! under test is Sley, with an empty PATH for direct protocol invocations.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use sley_protocol::{
    PktLineFrame, UploadArchiveRequest, demux_upload_archive_response, parse_pkt_line_stream,
    read_upload_archive_response, write_upload_archive_request,
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new(format: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "sley-upload-archive-{}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create fixture");
        let fixture = Self(root);
        fixture.git(&[
            "init",
            "-q",
            "--initial-branch=main",
            &format!("--object-format={format}"),
        ]);
        fs::create_dir(fixture.0.join("dir")).expect("create directory");
        fs::write(fixture.0.join("file.txt"), b"first\n").expect("write file");
        fs::write(fixture.0.join("dir/nested.txt"), b"nested\n").expect("write nested file");
        fs::write(fixture.0.join("large.bin"), vec![42; 200_000]).expect("write large blob");
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-qm", "first"]);
        fixture.git(&["tag", "release"]);
        fs::write(fixture.0.join("file.txt"), b"second\n").expect("modify file");
        fixture.git(&["commit", "-qam", "second"]);
        fs::create_dir(fixture.0.join("empty-path")).expect("create empty PATH");
        fixture
    }

    fn command(&self, program: &str) -> Command {
        let mut command = sley_testkit::hermetic_git_command_with_identity(program);
        command
            .current_dir(&self.0)
            .env("TZ", "UTC")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES");
        command
    }

    fn git(&self, args: &[&str]) -> Vec<u8> {
        let output = self
            .command(sley_testkit::oracle_git())
            .args(args)
            .output()
            .expect("run Git oracle");
        assert_success(&output);
        output.stdout
    }

    fn service(&self, repository: &Path, args: &[&str]) -> Output {
        let mut request = Vec::new();
        write_upload_archive_request(
            &mut request,
            &UploadArchiveRequest {
                arguments: args.iter().map(|arg| (*arg).to_string()).collect(),
            },
        )
        .expect("encode request");
        self.service_bytes(repository, &request)
    }

    fn service_bytes(&self, repository: &Path, input: &[u8]) -> Output {
        let mut child = self
            .command(sley_testkit::sley_bin!())
            .arg("upload-archive")
            .arg(repository)
            .env("PATH", self.0.join("empty-path"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start native upload-archive");
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(input)
            .expect("write request");
        child
            .wait_with_output()
            .expect("finish native upload-archive")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status {:?}, stderr {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn archive_bytes(output: &Output) -> Vec<u8> {
    assert_success(output);
    assert!(output.stdout.starts_with(b"0008ACK\n0000"));
    let mut input = output.stdout.as_slice();
    let response = read_upload_archive_response(&mut input).expect("read response from peer");
    assert!(input.is_empty(), "unexpected bytes after archive flush");
    demux_upload_archive_response(&response)
        .expect("successful sideband response")
        .data
}

fn assert_fatal(output: &Output) {
    assert!(!output.status.success(), "rejected request succeeded");
    let frames = parse_pkt_line_stream(&output.stdout).expect("well-formed fatal response");
    assert_eq!(frames.first(), Some(&PktLineFrame::Data(b"ACK\n".to_vec())));
    assert_eq!(frames.get(1), Some(&PktLineFrame::Flush));
    assert!(
        frames.iter().any(|frame| {
            matches!(frame, PktLineFrame::Data(payload) if payload.first() == Some(&3))
        }),
        "peer did not receive a sideband fatal: {frames:?}"
    );
    assert!(
        !frames.iter().any(|frame| {
            matches!(frame, PktLineFrame::Data(payload) if payload.first() == Some(&1))
        }),
        "rejected request leaked archive data"
    );
}

#[test]
fn native_upload_archive_streams_tar_and_zip_with_no_git_on_path() {
    for format in ["sha1", "sha256"] {
        let fixture = Fixture::new(format);
        for args in [
            vec!["--format=tar", "--prefix=release/", "HEAD"],
            vec!["--format=zip", "--prefix=release/", "HEAD", "dir"],
            vec!["--format=tar.gz", "HEAD", "dir/nested.txt"],
            vec!["--format=tar", "--mtime=@1234567890", "HEAD:dir"],
        ] {
            let mut oracle_args = vec!["archive"];
            oracle_args.extend_from_slice(&args);
            let expected = fixture.git(&oracle_args);
            let actual = fixture.service(&fixture.0, &args);
            let actual = archive_bytes(&actual);
            if args[0] == "--format=tar.gz" {
                let decode = |bytes: &[u8]| {
                    let mut tar = Vec::new();
                    flate2::read::GzDecoder::new(bytes)
                        .read_to_end(&mut tar)
                        .expect("valid gzip archive and checksum");
                    tar
                };
                // Compare the complete tar stream, including names, modes,
                // timestamps and commit identity. Deflate bitstreams can differ
                // between the native Rust codec and the oracle's zlib.
                assert_eq!(decode(&actual), decode(&expected), "{format}: {args:?}");
            } else {
                assert_eq!(actual, expected, "{format}: {args:?}");
            }
        }
        let listed = fixture.service(&fixture.0, &["--list"]);
        assert_eq!(archive_bytes(&listed), fixture.git(&["archive", "--list"]));
    }
}

#[test]
fn native_upload_archive_accepts_only_refs_and_ref_subtrees_by_default() {
    let fixture = Fixture::new("sha1");
    let tree = String::from_utf8(fixture.git(&["rev-parse", "HEAD^{tree}"])).expect("hex tree oid");
    fixture.git(&["update-ref", "refs/tags/tree", tree.trim()]);
    let oid = String::from_utf8(fixture.git(&["rev-parse", "HEAD"]))
        .expect("hex oid")
        .trim()
        .to_string();
    for name in [
        "HEAD",
        "main",
        "release",
        "refs/heads/main",
        "HEAD:dir",
        "tree",
        "tree:dir",
    ] {
        let output = fixture.service(&fixture.0, &["--mtime=@1234567890", name]);
        assert_success(&output);
        archive_bytes(&output);
    }
    for name in [
        oid.as_str(),
        &oid[..12],
        "HEAD^",
        "HEAD~1",
        "HEAD@{0}",
        "HEAD^{tree}",
    ] {
        assert_fatal(&fixture.service(&fixture.0, &[name]));
    }
    fixture.git(&["config", "uploadArchive.allowUnreachable", "true"]);
    for name in [oid.as_str(), "HEAD^", "HEAD^{tree}"] {
        let args = ["--mtime=@1234567890", name];
        let output = fixture.service(&fixture.0, &args);
        let expected = fixture.git(&["archive", args[0], args[1]]);
        assert_eq!(archive_bytes(&output), expected, "{name}");
    }
}

#[test]
fn native_upload_archive_does_not_execute_client_options_or_read_extra_files() {
    let fixture = Fixture::new("sha1");
    fs::write(fixture.0.join("private.txt"), b"not an archived object\n")
        .expect("write private file");
    for args in [
        vec!["--output=injected.tar", "HEAD"],
        vec!["-o", "injected.tar", "HEAD"],
        vec!["--remote=/", "HEAD"],
        vec!["--exec=touch injected", "HEAD"],
        vec!["--add-file=private.txt", "HEAD"],
        vec!["--add-virtual-file=extra:content", "HEAD"],
    ] {
        assert_fatal(&fixture.service(&fixture.0, &args));
    }
    assert!(!fixture.0.join("injected.tar").exists());
    assert!(!fixture.0.join("injected").exists());
    assert_fatal(&fixture.service_bytes(&fixture.0, b"000cbad arg\n0000"));
    assert_fatal(&fixture.service_bytes(&fixture.0, b"0000"));
    let too_many = vec!["--verbose"; 65];
    assert_fatal(&fixture.service(&fixture.0, &too_many));
}

#[test]
fn native_upload_archive_does_not_discover_a_parent_repository() {
    let fixture = Fixture::new("sha1");
    assert_fatal(&fixture.service(&fixture.0.join("dir"), &["HEAD"]));
    assert_fatal(&fixture.service(&fixture.0.join("missing"), &["HEAD"]));
}

#[test]
fn native_upload_archive_honors_bare_repositories_and_remote_format_policy() {
    let fixture = Fixture::new("sha1");
    fixture.git(&["clone", "-q", "--bare", ".", "bare.git"]);
    let bare = fixture.0.join("bare.git");
    assert_eq!(
        archive_bytes(&fixture.service(&bare, &["HEAD"])),
        fixture.git(&["archive", "HEAD"])
    );
    fixture.git(&["--git-dir=bare.git", "config", "tar.tgz.remote", "false"]);
    assert_fatal(&fixture.service(&bare, &["--format=tgz", "HEAD"]));
    assert_eq!(
        archive_bytes(&fixture.service(&bare, &["--list"])),
        fixture.git(&["archive", "--remote=./bare.git", "--list"])
    );
}

#[test]
#[cfg(unix)]
fn stock_git_archive_client_accepts_the_native_service() {
    let fixture = Fixture::new("sha1");
    let executable = sley_testkit::sley_bin!().replace('\'', "'\\''");
    let exec = format!("--exec='{executable}' upload-archive");
    let remote = format!("--remote={}", fixture.0.display());
    for args in [
        vec!["--format=tar", "--prefix=client/", "HEAD"],
        vec!["--format=zip", "HEAD", "dir"],
        vec!["--list"],
    ] {
        let mut remote_args = vec!["archive", remote.as_str(), exec.as_str()];
        remote_args.extend_from_slice(&args);
        let mut local_args = vec!["archive"];
        local_args.extend_from_slice(&args);
        assert_eq!(
            fixture.git(&remote_args),
            fixture.git(&local_args),
            "{args:?}"
        );
    }
}

#[test]
#[cfg(all(unix, feature = "git-compat-i18n"))]
fn stock_git_archive_client_uses_the_installed_native_helper() {
    let fixture = Fixture::new("sha1");
    let helpers = fixture
        .command(sley_testkit::sley_bin!())
        .arg("--exec-path")
        .output()
        .expect("locate native compatibility helpers");
    assert_success(&helpers);
    let helpers = String::from_utf8(helpers.stdout).expect("helper path");
    assert!(
        Path::new(helpers.trim())
            .join("git-upload-archive")
            .is_file()
    );
    let output = fixture
        .command(sley_testkit::oracle_git())
        .env("GIT_EXEC_PATH", helpers.trim())
        .env("SLEY_BIN", sley_testkit::sley_bin!())
        .args([
            "archive",
            &format!("--remote={}", fixture.0.display()),
            "--format=tar",
            "HEAD",
        ])
        .output()
        .expect("request archive through installed native helper");
    assert_success(&output);
    assert_eq!(
        output.stdout,
        fixture.git(&["archive", "--format=tar", "HEAD"])
    );
}
