// SPDX-License-Identifier: MIT OR Apache-2.0
//! Post-commit summaries must inspect oversized blobs through object metadata.
//! The deliberately incomplete fixture below exercises that contract without
//! allocating or writing its advertised 64 GiB body.

use flate2::Compression;
use flate2::write::ZlibEncoder;
use sley_core::ObjectFormat;
use sley_object::{EncodedObject, ObjectType};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Fixture {
    root: PathBuf,
    actual: PathBuf,
    expected: PathBuf,
    empty_path: PathBuf,
    format: ObjectFormat,
}

fn candidate() -> String {
    match std::env::var("GITR_TEST_BIN") {
        Ok(path) => {
            assert!(
                Path::new(&path).is_absolute(),
                "GITR_TEST_BIN must be absolute"
            );
            path
        }
        Err(_) => sley_testkit::sley_bin!().to_owned(),
    }
}

impl Fixture {
    fn new(format: ObjectFormat) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "gitr-commit-large-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let fixture = Self {
            actual: root.join("actual"),
            expected: root.join("expected"),
            empty_path: root.join("empty-path"),
            root,
            format,
        };
        fs::create_dir_all(&fixture.actual).unwrap();
        fs::create_dir_all(&fixture.expected).unwrap();
        fs::create_dir(&fixture.empty_path).unwrap();
        fs::create_dir(fixture.root.join("home")).unwrap();
        assert!(
            Path::new(sley_testkit::oracle_git()).is_absolute(),
            "SLEY_TEST_GIT must select an absolute reference Git"
        );
        fixture.setup(&[
            "init",
            "-q",
            "--initial-branch=main",
            match format {
                ObjectFormat::Sha1 => "--object-format=sha1",
                ObjectFormat::Sha256 => "--object-format=sha256",
            },
        ]);
        fixture.setup(&["config", "core.abbrev", "12"]);
        fixture.setup(&["config", "maintenance.auto", "false"]);
        fixture.setup(&["config", "gc.auto", "0"]);
        fixture
    }

    fn command(&self, program: &str, repo: &Path) -> Command {
        let mut command = sley_testkit::hermetic_git_command_with_identity(program);
        command
            .current_dir(repo)
            .env("LC_ALL", "C")
            .env("TZ", "UTC")
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("home/xdg"))
            .env("GIT_AUTHOR_DATE", "@1234567890 +0000")
            .env("GIT_COMMITTER_DATE", "@1234567890 +0000")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES");
        command
    }

    fn setup(&self, args: &[&str]) {
        for repo in [&self.actual, &self.expected] {
            let output = self
                .command(sley_testkit::oracle_git(), repo)
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "setup {args:?}: {output:?}");
        }
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        for repo in [&self.actual, &self.expected] {
            fs::write(repo.join(path), bytes).unwrap();
        }
    }

    fn compare(&self, args: &[&str]) -> Output {
        let expected = self
            .command(sley_testkit::oracle_git(), &self.expected)
            .args(args)
            .output()
            .unwrap();
        let actual = self
            .command(&candidate(), &self.actual)
            .env("PATH", &self.empty_path)
            .args(args)
            .output()
            .unwrap();
        assert_eq!(
            actual.status.code(),
            expected.status.code(),
            "{args:?}: actual={actual:?}, expected={expected:?}"
        );
        assert_eq!(
            actual.stdout, expected.stdout,
            "stdout {args:?}: actual={actual:?}, expected={expected:?}"
        );
        assert_eq!(
            actual.stderr, expected.stderr,
            "stderr {args:?}: actual={actual:?}, expected={expected:?}"
        );
        actual
    }

    fn commit(&self, args: &[&str]) -> Output {
        let output = self.compare(args);
        assert!(output.status.success(), "commit {args:?}: {output:?}");
        self.compare(&["rev-parse", "HEAD"]);
        output
    }

    fn assert_corrupt_native(&self, output: &Output, oid: &str) {
        let program = candidate();
        let name = Path::new(&program).file_name().unwrap().to_str().unwrap();
        assert_eq!(
            output.status.code(),
            Some(1),
            "existing corruption status changed: {output:?}"
        );
        assert_eq!(
            output.stderr,
            format!("{name}: invalid object: corrupt loose object '{oid}'\n").as_bytes()
        );
    }

    fn incomplete_blob(&self, advertised_size: u64) -> String {
        // Independently generated fixture: an incomplete zlib stream whose Git
        // header advertises more bytes than the stream contains. Its hash is
        // only a deterministic object-store key; this is not a valid blob.
        let content = b"incomplete object fixture\n";
        let oid = EncodedObject::new(ObjectType::Blob, content.to_vec())
            .object_id(self.format)
            .unwrap()
            .to_hex();
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        write!(encoder, "blob {advertised_size}\0").unwrap();
        encoder.write_all(content).unwrap();
        let mut compressed = encoder.finish().unwrap();
        compressed.truncate(compressed.len() - 4);
        for repo in [&self.actual, &self.expected] {
            let fanout = repo.join(".git/objects").join(&oid[..2]);
            fs::create_dir_all(&fanout).unwrap();
            fs::write(fanout.join(&oid[2..]), &compressed).unwrap();
        }
        oid
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn oversized_blob_add_mode_change_and_delete_summaries_match_git() {
    for format in [ObjectFormat::Sha1, ObjectFormat::Sha256] {
        let fixture = Fixture::new(format);
        let oid = fixture.incomplete_blob(68_719_476_737);
        fixture.setup(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{oid},huge"),
        ]);
        fixture.commit(&["commit", "-m", "huge"]);
        let full_read = fixture
            .command(&candidate(), &fixture.actual)
            .env("PATH", &fixture.empty_path)
            .args(["cat-file", "blob", &oid])
            .output()
            .unwrap();
        assert_eq!(
            full_read.status.code(),
            Some(128),
            "full object read unexpectedly succeeded: {full_read:?}"
        );
        assert_eq!(full_read.stderr, format!(
            "error: corrupt loose object '{oid}'\nfatal: loose object {oid} (stored in .git/objects/{}/{}) is corrupt\n",
            &oid[..2], &oid[2..],
        ).as_bytes());
        assert!(
            full_read.stdout.is_empty(),
            "incomplete blob escaped full-read validation"
        );
        fixture.setup(&["update-index", "--cacheinfo", &format!("100755,{oid},huge")]);
        fixture.commit(&["commit", "-m", "mode only"]);
        fixture.setup(&["update-index", "--force-remove", "huge"]);
        fixture.write("file", b"ordinary text\n");
        fixture.setup(&["add", "file"]);
        fixture.commit(&["commit", "-m", "remove huge"]);
    }
}

#[test]
fn configured_big_file_threshold_boundaries_match_git() {
    for format in [ObjectFormat::Sha1, ObjectFormat::Sha256] {
        for threshold in ["0", "7", "8", "9", "1k"] {
            let fixture = Fixture::new(format);
            fixture.setup(&["config", "core.bigFileThreshold", "0"]);
            let override_config = format!("core.bigFileThreshold={threshold}");
            fixture.write("file", b"one\ntwo\n");
            fixture.setup(&["add", "file"]);
            fixture.commit(&["-c", &override_config, "commit", "-m", "eight bytes"]);
            fixture.write("file", b"one\nthree");
            fixture.setup(&["add", "file"]);
            fixture.commit(&["-c", &override_config, "commit", "-m", "nine bytes"]);
            fixture.write("file", b"one\ntwo\n");
            fixture.setup(&["add", "file"]);
            fixture.commit(&["-c", &override_config, "commit", "-m", "eight again"]);
            fixture.setup(&["update-index", "--force-remove", "file"]);
            fixture.commit(&["-c", &override_config, "commit", "-m", "removed"]);
        }
    }
}

#[test]
fn invalid_big_file_threshold_is_rejected_before_committing() {
    for format in [ObjectFormat::Sha1, ObjectFormat::Sha256] {
        for threshold in ["invalid", "-1", "9999999999999999999999999999"] {
            let fixture = Fixture::new(format);
            fixture.write("file", b"one\ntwo\n");
            fixture.setup(&["add", "file"]);
            for quiet in [false, true] {
                let config = format!("core.bigFileThreshold={threshold}");
                let mut args = vec!["-c", &config, "commit", "-m", "invalid config"];
                if quiet {
                    args.push("-q");
                }
                let output = fixture.compare(&args);
                assert_eq!(output.status.code(), Some(128));
                for repo in [&fixture.actual, &fixture.expected] {
                    assert!(
                        !repo.join(".git/refs/heads/main").exists(),
                        "invalid config published HEAD"
                    );
                }
            }
        }
    }
}

#[test]
fn ordinary_incomplete_blob_still_fails_full_read() {
    for format in [ObjectFormat::Sha1, ObjectFormat::Sha256] {
        let fixture = Fixture::new(format);
        let oid = fixture.incomplete_blob(64);
        fixture.setup(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{oid},broken"),
        ]);
        let actual = fixture
            .command(&candidate(), &fixture.actual)
            .env("PATH", &fixture.empty_path)
            .args(["commit", "-m", "broken"])
            .output()
            .unwrap();
        let expected = fixture
            .command(sley_testkit::oracle_git(), &fixture.expected)
            .args(["commit", "-m", "broken"])
            .output()
            .unwrap();
        assert!(
            !actual.status.success(),
            "candidate accepted incomplete ordinary blob: {actual:?}"
        );
        assert!(
            !expected.status.success(),
            "oracle accepted incomplete ordinary blob: {expected:?}"
        );
        fixture.assert_corrupt_native(&actual, &oid);
        // A large replacement must not let the shortcut suppress corruption
        // in the ordinary old side, which Git checks first.
        fixture.write("broken", &[b'x'; 128]);
        fixture.setup(&["add", "broken"]);
        let args = [
            "-c",
            "core.bigFileThreshold=64",
            "commit",
            "-m",
            "large replacement",
        ];
        let actual = fixture
            .command(&candidate(), &fixture.actual)
            .env("PATH", &fixture.empty_path)
            .args(args)
            .output()
            .unwrap();
        let expected = fixture
            .command(sley_testkit::oracle_git(), &fixture.expected)
            .args(args)
            .output()
            .unwrap();
        fixture.assert_corrupt_native(&actual, &oid);
        assert!(
            !expected.status.success(),
            "oracle ignored corrupt old side: {expected:?}"
        );
    }
}

#[test]
fn forced_text_overrides_size_classification_without_running_textconv() {
    for format in [ObjectFormat::Sha1, ObjectFormat::Sha256] {
        for attributes in ["file diff\n", "file diff=forced\n"] {
            let fixture = Fixture::new(format);
            fixture.write(".git/info/attributes", attributes.as_bytes());
            fixture.setup(&["config", "diff.forced.binary", "false"]);
            fixture.setup(&[
                "config",
                "diff.forced.textconv",
                "nonexistent-textconv-must-not-run",
            ]);
            fixture.setup(&["config", "core.bigFileThreshold", "1"]);
            fixture.write("file", b"one\0\ntwo\n");
            fixture.setup(&["add", "file"]);
            fixture.commit(&["commit", "-m", "forced text"]);
            fixture.write("file", b"one\0\nthree\n");
            fixture.setup(&["add", "file"]);
            fixture.commit(&["commit", "-m", "changed text"]);
        }
    }
}

#[test]
fn gitlink_summaries_keep_synthetic_text_at_zero_threshold() {
    for format in [ObjectFormat::Sha1, ObjectFormat::Sha256] {
        let fixture = Fixture::new(format);
        fixture.write("file", b"base\n");
        fixture.setup(&["add", "file"]);
        fixture.setup(&["commit", "-qm", "base"]);
        let head = fixture.compare(&["rev-parse", "HEAD"]);
        let oid = String::from_utf8(head.stdout).unwrap();
        fixture.setup(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{},module", oid.trim()),
        ]);
        fixture.commit(&["-c", "core.bigFileThreshold=0", "commit", "-m", "gitlink"]);
        fixture.setup(&["update-index", "--force-remove", "module"]);
        fixture.commit(&[
            "-c",
            "core.bigFileThreshold=0",
            "commit",
            "-m",
            "remove gitlink",
        ]);
    }
}

#[test]
#[cfg(unix)]
fn symlink_summaries_keep_the_existing_content_path() {
    for format in [ObjectFormat::Sha1, ObjectFormat::Sha256] {
        let fixture = Fixture::new(format);
        for repo in [&fixture.actual, &fixture.expected] {
            std::os::unix::fs::symlink("first-target", repo.join("link")).unwrap();
        }
        fixture.setup(&["add", "link"]);
        fixture.commit(&["commit", "-m", "symlink"]);
        for repo in [&fixture.actual, &fixture.expected] {
            fs::remove_file(repo.join("link")).unwrap();
            std::os::unix::fs::symlink("second-target", repo.join("link")).unwrap();
        }
        fixture.setup(&["add", "link"]);
        fixture.commit(&["commit", "-m", "changed symlink"]);
    }
}

#[test]
fn summary_does_not_compile_patch_regexes_or_run_patch_commands() {
    for format in [ObjectFormat::Sha1, ObjectFormat::Sha256] {
        for binary in ["auto", "false", "true"] {
            let fixture = Fixture::new(format);
            fixture.write(".gitattributes", b"file diff=custom\n");
            fixture.write("file", b"hello\n");
            fixture.setup(&["config", "core.bigFileThreshold", "1"]);
            fixture.setup(&["config", "diff.custom.binary", binary]);
            fixture.setup(&["config", "diff.custom.xfuncname", "["]);
            fixture.setup(&["config", "diff.custom.wordRegex", "["]);
            fixture.setup(&[
                "config",
                "diff.custom.textconv",
                "nonexistent-textconv-must-not-run",
            ]);
            fixture.setup(&[
                "config",
                "diff.custom.command",
                "nonexistent-external-diff-must-not-run",
            ]);
            fixture.setup(&["add", ".gitattributes", "file"]);
            fixture.commit(&["commit", "-m", "summary ignores patch-only settings"]);
        }
    }
}

#[test]
fn metadata_probe_preserves_malformed_header_and_wrong_type_diagnostics() {
    let compressed = |frame: &[u8]| {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(frame).unwrap();
        encoder.finish().unwrap()
    };
    let cases = [
        (
            "invalid zlib",
            b"not a valid loose object".to_vec(),
            Some("incorrect header check"),
            None,
        ),
        (
            "unknown method",
            vec![0x79, 0x18],
            Some("unknown compression method"),
            None,
        ),
        (
            "invalid size",
            compressed(b"blob invalid\0payload\n"),
            None,
            Some("invalid object size"),
        ),
        (
            "overlong header",
            compressed(b"blob 11111111111111111111111111111111111\0body"),
            None,
            Some("overlong"),
        ),
        (
            "wrong type",
            compressed(b"tree 0\0"),
            None,
            Some("wrong type"),
        ),
    ];
    for format in [ObjectFormat::Sha1, ObjectFormat::Sha256] {
        for (label, raw, inflate, reason) in &cases {
            let fixture = Fixture::new(format);
            let body = b"initial ordinary blob\n";
            fixture.write("file", body);
            fixture.setup(&["add", "file"]);
            fixture.setup(&["write-tree"]);
            let oid = EncodedObject::new(ObjectType::Blob, body.to_vec())
                .object_id(format)
                .unwrap()
                .to_hex();
            for repo in [&fixture.actual, &fixture.expected] {
                let path = repo.join(".git/objects").join(&oid[..2]).join(&oid[2..]);
                fs::remove_file(&path).unwrap();
                fs::write(&path, raw).unwrap();
                fs::remove_file(repo.join("file")).unwrap();
            }
            let actual = fixture
                .command(&candidate(), &fixture.actual)
                .env("PATH", &fixture.empty_path)
                .args(["commit", "-m", label])
                .output()
                .unwrap();
            assert_eq!(actual.status.code(), Some(1), "{label}: {actual:?}");
            let program = candidate();
            let name = Path::new(&program).file_name().unwrap().to_str().unwrap();
            // Frozen native behavior: full reads retry loose storage, emitting
            // two inflate records. A speculative header probe must add none.
            let mut expected = inflate.map_or_else(String::new, |error| {
                format!("error: inflate: data stream error ({error})\n").repeat(2)
            });
            let reason = match reason {
                Some("overlong") => format!("header for {oid} too long, exceeds 32 bytes"),
                Some("wrong type") => format!("diff expected blob object {oid}"),
                Some(reason) => (*reason).to_owned(),
                None => format!("unable to unpack {oid} header"),
            };
            expected.push_str(&format!("{name}: invalid object: {reason}\n"));
            assert_eq!(actual.stderr, expected.as_bytes(), "{label}: {actual:?}");
        }
    }
}
