// SPDX-License-Identifier: MIT OR Apache-2.0
//! Product checks for immutable source reads from ordinary Git repositories.
//! The candidate has no Git executable on PATH. Reference Git is an explicit
//! absolute executable used only for fixture construction and comparisons.

use std::{
    ffi::{OsStr, OsString},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use rit::snapshot::Snapshot;
use serde_json::{Value, json};

struct Fixture {
    temp: tempfile::TempDir,
    repo: PathBuf,
    empty_path: PathBuf,
    oracle: PathBuf,
    format: String,
}

impl Fixture {
    fn new(format: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let empty_path = temp.path().join("empty-path");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&empty_path).unwrap();
        let oracle = std::env::var_os("RIT_REFERENCE_GIT")
            .or_else(|| std::env::var_os("GITR_REFERENCE_GIT"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/usr/bin/git"));
        assert!(
            oracle.is_absolute() && oracle.is_file(),
            "set RIT_REFERENCE_GIT to an absolute reference Git executable"
        );
        let fixture = Self {
            temp,
            repo,
            empty_path,
            oracle,
            format: format.to_owned(),
        };
        fixture.git(&[
            "init",
            "--initial-branch=main",
            &format!("--object-format={format}"),
        ]);
        fixture
    }

    fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(program);
        command
            .current_dir(&self.repo)
            .env("PATH", &self.empty_path)
            .env("HOME", self.temp.path())
            .env("XDG_CONFIG_HOME", self.temp.path().join("xdg"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.temp.path().join("absent-config"))
            .env("GIT_CONFIG_SYSTEM", self.temp.path().join("absent-system"))
            .env("GIT_CONFIG_COUNT", "0")
            .env("GIT_AUTHOR_NAME", "Snapshot Test")
            .env("GIT_AUTHOR_EMAIL", "snapshot@example.invalid")
            .env("GIT_COMMITTER_NAME", "Snapshot Test")
            .env("GIT_COMMITTER_EMAIL", "snapshot@example.invalid")
            .env("GIT_AUTHOR_DATE", "2001-02-03T04:05:06 +0000")
            .env("GIT_COMMITTER_DATE", "2001-02-03T04:05:06 +0000")
            .env("GIT_TERMINAL_PROMPT", "0");
        for key in [
            "GIT_DIR",
            "GIT_COMMON_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG",
            "GIT_REPLACE_REF_BASE",
            "GIT_NO_REPLACE_OBJECTS",
            "GIT_REFERENCE_BACKEND",
        ] {
            command.env_remove(key);
        }
        command
    }

    fn git(&self, args: &[&str]) -> Vec<u8> {
        success(self.command(&self.oracle).args(args).output().unwrap())
    }

    fn git_text(&self, args: &[&str]) -> String {
        String::from_utf8(self.git(args)).unwrap().trim().to_owned()
    }

    fn commit(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-m", message]);
        self.git_text(&["rev-parse", "HEAD"])
    }

    fn snapshot_at(
        &self,
        repo: &Path,
        action: &str,
        revision: &str,
        path: Option<&OsStr>,
        max_bytes: Option<&str>,
    ) -> Output {
        let mut command = self.command(env!("CARGO_BIN_EXE_rit"));
        command
            .args(["--rit-snapshot", action, "--repo"])
            .arg(repo)
            .args(["--revision", revision]);
        if let Some(path) = path {
            command.arg("--path").arg(path);
        }
        if let Some(limit) = max_bytes {
            command.args(["--max-bytes", limit]);
        }
        command.output().unwrap()
    }

    fn snapshot(
        &self,
        action: &str,
        revision: &str,
        path: Option<&OsStr>,
        max_bytes: Option<&str>,
    ) -> Output {
        self.snapshot_at(&self.repo, action, revision, path, max_bytes)
    }

    fn json(&self, action: &str, revision: &str, path: Option<&OsStr>) -> Value {
        serde_json::from_slice(&success(self.snapshot(action, revision, path, None))).unwrap()
    }

    fn identity(&self, commit: &str) -> Value {
        json!({
            "object_format": self.format,
            "commit_oid": commit,
            "root_tree_oid": self.git_text(&["rev-parse", &format!("{commit}^{{tree}}")]),
        })
    }

    fn entries(&self, commit: &str, path: &[u8]) -> Vec<Value> {
        let mut treeish = commit.as_bytes().to_vec();
        if !path.is_empty() {
            treeish.push(b':');
            treeish.extend_from_slice(path);
        }
        let bytes = success(
            self.command(&self.oracle)
                .args(["ls-tree", "-z"])
                .arg(os_path(&treeish))
                .output()
                .unwrap(),
        );
        bytes
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
            .map(|record| {
                let tab = record.iter().position(|byte| *byte == b'\t').unwrap();
                let header = std::str::from_utf8(&record[..tab]).unwrap();
                let mut fields = header.split(' ');
                let mode = u32::from_str_radix(fields.next().unwrap(), 8).unwrap();
                let _git_kind = fields.next().unwrap();
                let oid = fields.next().unwrap();
                let kind = match mode {
                    0o040000 => "tree",
                    0o100644 | 0o100755 => "blob",
                    0o120000 => "symlink",
                    0o160000 => "gitlink",
                    other => panic!("unexpected reference mode {other:o}"),
                };
                json!({
                    "name": raw_path(&record[tab + 1..]),
                    "mode": mode,
                    "object_oid": oid,
                    "kind": kind,
                })
            })
            .collect()
    }

    fn object_path(&self, oid: &str) -> PathBuf {
        self.repo
            .join(".git/objects")
            .join(&oid[..2])
            .join(&oid[2..])
    }

    fn write_object(&self, kind: &str, bytes: &[u8]) -> String {
        let mut child = self
            .command(&self.oracle)
            .args(["hash-object", "--literally", "-w", "-t", kind, "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        String::from_utf8(success(child.wait_with_output().unwrap()))
            .unwrap()
            .trim()
            .to_owned()
    }
}

fn success(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "status={}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output.stdout
}

fn rejected(output: Output) {
    assert!(!output.status.success(), "unexpected success: {output:?}");
    assert!(
        output.stdout.is_empty(),
        "failure leaked payload: {output:?}"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("rit snapshot:"),
        "missing snapshot diagnostic: {output:?}"
    );
}

fn assert_payload(actual: &[u8], expected: &[u8]) {
    assert!(
        actual == expected,
        "payload mismatch: actual length {}, expected length {}, first differing byte {:?}",
        actual.len(),
        expected.len(),
        actual.iter().zip(expected).position(|(a, b)| a != b)
    );
}

fn raw_path(bytes: &[u8]) -> Value {
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    json!({"hex": hex, "utf8": std::str::from_utf8(bytes).ok()})
}

#[cfg(unix)]
fn os_path(bytes: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes.to_vec())
}

#[cfg(not(unix))]
fn os_path(bytes: &[u8]) -> OsString {
    std::str::from_utf8(bytes).unwrap().into()
}

fn source_bytes() -> Vec<u8> {
    let mut state = 0x6a09_e667_f3bc_c909u64;
    (0..128 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

#[test]
fn snapshot_identity_entries_and_payloads_match_loose_and_delta_packed_git() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        fs::create_dir(f.repo.join("dir")).unwrap();
        fs::write(f.repo.join("dir/nested.txt"), b"nested\n").unwrap();
        fs::write(f.repo.join("-dash"), b"dash\n").unwrap();
        let old_bytes = source_bytes();
        fs::write(f.repo.join("source.bin"), &old_bytes).unwrap();
        let old_commit = f.commit("first snapshot");
        let mut new_bytes = old_bytes.clone();
        new_bytes[12345] ^= 0xff;
        fs::write(f.repo.join("source.bin"), &new_bytes).unwrap();
        let commit = f.commit("second snapshot");
        let old_blob = f.git_text(&["rev-parse", &format!("{old_commit}:source.bin")]);
        let new_blob = f.git_text(&["rev-parse", "HEAD:source.bin"]);
        f.git(&["tag", "-a", "snapshot-tag", "-m", "annotated", &commit]);
        for packed in [false, true] {
            if packed {
                f.git(&["repack", "-adf", "--window=50", "--depth=50"]);
                let index = fs::read_dir(f.repo.join(".git/objects/pack"))
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| path.extension() == Some(OsStr::new("idx")))
                    .unwrap();
                let verification = success(
                    f.command(&f.oracle)
                        .args(["verify-pack", "-v"])
                        .arg(index)
                        .output()
                        .unwrap(),
                );
                assert!(
                    String::from_utf8(verification)
                        .unwrap()
                        .lines()
                        .any(|line| {
                            let fields: Vec<_> = line.split_whitespace().collect();
                            fields.len() == 7 && (fields[0] == old_blob || fields[0] == new_blob)
                        }),
                    "a requested source blob must exercise a delta representation"
                );
            }
            for revision in [commit.as_str(), "main", "HEAD", "snapshot-tag"] {
                assert_eq!(f.json("inspect", revision, None), f.identity(&commit));
            }
            for path in [b"".as_slice(), b"dir"] {
                let listing = f.json("list", &commit, Some(&os_path(path)));
                assert_eq!(listing["snapshot"], f.identity(&commit));
                assert_eq!(listing["path"], raw_path(path));
                assert_eq!(listing["entries"], json!(f.entries(&commit, path)));
            }
            let nested = f.json("stat", &commit, Some(OsStr::new("dir/nested.txt")));
            assert_eq!(nested["snapshot"], f.identity(&commit));
            assert_eq!(nested["path"], raw_path(b"dir/nested.txt"));
            assert_eq!(nested["entry"], f.entries(&commit, b"dir")[0]);
            assert_payload(
                &success(f.snapshot("read", &commit, Some(OsStr::new("source.bin")), None)),
                &new_bytes,
            );
            assert_payload(
                &success(f.snapshot("read", &old_commit, Some(OsStr::new("source.bin")), None)),
                &old_bytes,
            );
            assert_eq!(
                success(f.snapshot("read", &commit, Some(OsStr::new("-dash")), None)),
                b"dash\n"
            );
        }
        f.git(&["fsck", "--full"]);
    }
}

#[test]
fn opened_snapshot_stays_on_its_commit_after_ref_and_worktree_changes() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        fs::write(f.repo.join("value"), b"original\n").unwrap();
        let old_commit = f.commit("original");
        let snapshot = Snapshot::open(&f.repo, "HEAD").unwrap();
        fs::write(f.repo.join("value"), b"new commit\n").unwrap();
        let new_commit = f.commit("new");
        fs::write(f.repo.join("value"), b"uncommitted worktree\n").unwrap();
        assert_ne!(old_commit, new_commit);
        assert_eq!(snapshot.commit_id().to_hex(), old_commit);
        assert_eq!(
            snapshot.read(b"value", 1024).unwrap().bytes(),
            b"original\n"
        );
        assert_eq!(
            success(f.snapshot("read", "HEAD", Some(OsStr::new("value")), None)),
            b"new commit\n"
        );
        assert_eq!(
            fs::read(f.repo.join("value")).unwrap(),
            b"uncommitted worktree\n"
        );
    }
}

#[test]
fn snapshot_identity_and_reads_ignore_commit_and_blob_replacement_refs() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        fs::write(f.repo.join("value"), b"canonical\n").unwrap();
        let original = f.commit("canonical");
        let identity = f.identity(&original);
        let original_blob = f.git_text(&["rev-parse", "HEAD:value"]);
        fs::write(f.repo.join("value"), b"replacement\n").unwrap();
        let replacement = f.commit("replacement");
        let replacement_blob = f.git_text(&["rev-parse", "HEAD:value"]);
        f.git(&["replace", &original, &replacement]);
        f.git(&["replace", &original_blob, &replacement_blob]);
        assert_eq!(
            f.git(&["show", &format!("{original}:value")]),
            b"replacement\n"
        );
        assert_eq!(f.json("inspect", &original, None), identity);
        assert_eq!(
            success(f.snapshot("read", &original, Some(OsStr::new("value")), None)),
            b"canonical\n"
        );
    }
}

#[cfg(unix)]
#[test]
fn raw_names_modes_symlink_payloads_and_foreign_gitlinks_are_preserved() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        let names: &[&[u8]] = &[b"raw-\xff-name", b"tab\tname", b"line\nname", b" space "];
        for name in names {
            fs::write(f.repo.join(os_path(name)), *name).unwrap();
        }
        fs::write(f.repo.join("executable"), b"executable bytes\n").unwrap();
        fs::set_permissions(f.repo.join("executable"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(f.temp.path().join("outside"), b"must not read this\n").unwrap();
        symlink("../outside", f.repo.join("link")).unwrap();
        f.git(&["add", "-A"]);
        let foreign = "d".repeat(if format == "sha1" { 40 } else { 64 });
        f.git(&[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000",
            &foreign,
            "submodule",
        ]);
        f.git(&["commit", "-m", "byte paths and modes"]);
        let commit = f.git_text(&["rev-parse", "HEAD"]);
        let listing = f.json("list", &commit, None);
        assert_eq!(listing["entries"], json!(f.entries(&commit, b"")));
        for name in names {
            assert_eq!(
                success(f.snapshot("read", &commit, Some(&os_path(name)), None)),
                *name
            );
            let hex = raw_path(name)["hex"].as_str().unwrap().to_ascii_uppercase();
            assert_eq!(
                success(
                    f.command(env!("CARGO_BIN_EXE_rit"))
                        .args(["--rit-snapshot", "read", "--path-hex", &hex])
                        .output()
                        .unwrap()
                ),
                *name
            );
            let stat = f.json("stat", &commit, Some(&os_path(name)));
            assert_eq!(stat["path"], raw_path(name));
            assert_eq!(stat["entry"]["name"], raw_path(name));
        }
        assert_eq!(
            success(f.snapshot("read", &commit, Some(OsStr::new("link")), None)),
            b"../outside"
        );
        let executable = f.json("stat", &commit, Some(OsStr::new("executable")));
        assert_eq!(executable["entry"]["mode"], 0o100755);
        let link = f.json("stat", &commit, Some(OsStr::new("link")));
        assert_eq!(link["entry"]["kind"], "symlink");
        let gitlink = f.json("stat", &commit, Some(OsStr::new("submodule")));
        assert_eq!(gitlink["entry"]["kind"], "gitlink");
        assert_eq!(gitlink["entry"]["object_oid"], foreign);
        rejected(f.snapshot("read", &commit, Some(OsStr::new("submodule")), None));
    }
}

#[test]
fn unrequested_blobs_are_lazy_and_missing_or_wrong_hash_reads_fail_without_payload() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        fs::write(f.repo.join("good"), b"available\n").unwrap();
        fs::write(f.repo.join("bad"), b"requested original\n").unwrap();
        fs::write(f.repo.join("different"), b"different valid object\n").unwrap();
        let commit = f.commit("three blobs");
        let bad = f.git_text(&["rev-parse", "HEAD:bad"]);
        let different = f.git_text(&["rev-parse", "HEAD:different"]);
        fs::remove_file(f.object_path(&bad)).unwrap();
        assert_eq!(
            success(f.snapshot("read", &commit, Some(OsStr::new("good")), None)),
            b"available\n"
        );
        assert_eq!(
            f.json("list", &commit, None)["entries"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        rejected(f.snapshot("read", &commit, Some(OsStr::new("bad")), None));
        fs::copy(f.object_path(&different), f.object_path(&bad)).unwrap();
        rejected(f.snapshot("read", &commit, Some(OsStr::new("bad")), None));
        fs::remove_file(f.object_path(&bad)).unwrap();
        fs::write(f.object_path(&bad), b"not a zlib object").unwrap();
        rejected(f.snapshot("read", &commit, Some(OsStr::new("bad")), None));
    }
}

#[test]
fn commit_tree_and_tag_metadata_must_be_verified_and_unambiguous_before_output() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        fs::write(f.repo.join("value"), b"first metadata target\n").unwrap();
        let commit = f.commit("first");
        let tree = f.git_text(&["rev-parse", "HEAD^{tree}"]);
        let blob = f.git_text(&["rev-parse", "HEAD:value"]);
        fs::write(f.repo.join("value"), b"second metadata target\n").unwrap();
        let other_commit = f.commit("second");
        let other_tree = f.git_text(&["rev-parse", "HEAD^{tree}"]);

        // Both substituted payloads are valid zlib objects. Their valid contents
        // cannot be trusted under a different content-addressed filename.
        for (target, substitute) in [(&commit, &other_commit), (&tree, &blob)] {
            let original = fs::read(f.object_path(target)).unwrap();
            fs::remove_file(f.object_path(target)).unwrap();
            fs::copy(f.object_path(substitute), f.object_path(target)).unwrap();
            rejected(f.snapshot("inspect", &commit, None, None));
            rejected(f.snapshot("read", &commit, Some(OsStr::new("value")), None));
            fs::remove_file(f.object_path(target)).unwrap();
            fs::write(f.object_path(target), original).unwrap();
        }

        let identities = "author Snapshot Test <snapshot@example.invalid> 981173106 +0000\n\
                          committer Snapshot Test <snapshot@example.invalid> 981173106 +0000\n";
        for headers in [
            format!("tree {tree}\n{identities}tree {other_tree}\n"),
            format!("{identities}tree {tree}\n"),
            format!("tree {blob}\n{identities}"),
        ] {
            let malformed = f.write_object("commit", format!("{headers}\nmalformed\n").as_bytes());
            rejected(f.snapshot("inspect", &malformed, None, None));
            rejected(f.snapshot("read", &malformed, Some(OsStr::new("value")), None));
        }

        let tagger = "tagger Snapshot Test <snapshot@example.invalid> 981173106 +0000\n";
        for headers in [
            format!("object {commit}\ntype commit\ntag malformed\n{tagger}object {other_commit}\n"),
            format!("object {commit}\ntype commit\ntag malformed\n{tagger}type tree\n"),
            format!("type commit\nobject {commit}\ntag malformed\n{tagger}"),
        ] {
            let malformed = f.write_object("tag", format!("{headers}\nmalformed\n").as_bytes());
            rejected(f.snapshot("inspect", &malformed, None, None));
        }

        f.git(&["tag", "-a", "canonical-tag", "-m", "canonical", &commit]);
        let tag = f.git_text(&["rev-parse", "refs/tags/canonical-tag"]);
        let original = fs::read(f.object_path(&tag)).unwrap();
        fs::remove_file(f.object_path(&tag)).unwrap();
        fs::copy(f.object_path(&other_commit), f.object_path(&tag)).unwrap();
        rejected(f.snapshot("inspect", "canonical-tag", None, None));
        fs::remove_file(f.object_path(&tag)).unwrap();
        fs::write(f.object_path(&tag), original).unwrap();
        assert_eq!(
            f.json("inspect", "canonical-tag", None),
            f.identity(&commit)
        );
    }
}

#[test]
fn invalid_paths_revisions_and_read_limits_fail_before_output() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        fs::create_dir(f.repo.join("dir")).unwrap();
        fs::write(f.repo.join("dir/value"), b"12345").unwrap();
        fs::write(f.repo.join("empty"), b"").unwrap();
        let commit = f.commit("limits");
        for path in [
            "/dir/value",
            "../value",
            "dir/../empty",
            "./empty",
            "dir//value",
            "dir/./value",
            "dir/value/",
        ] {
            rejected(f.snapshot("read", &commit, Some(OsStr::new(path)), None));
        }
        for revision in [
            "HEAD^",
            "HEAD~0",
            "HEAD:dir",
            "HEAD..HEAD",
            "HEAD@{0}",
            "missing-ref",
        ] {
            rejected(f.snapshot("inspect", revision, None, None));
        }
        rejected(f.snapshot("read", &commit, None, None));
        rejected(f.snapshot("read", &commit, Some(OsStr::new("")), None));
        rejected(f.snapshot("read", &commit, Some(OsStr::new("dir")), None));
        rejected(f.snapshot("read", &commit, Some(OsStr::new("absent")), None));
        let snapshot = Snapshot::open(&f.repo, &commit).unwrap();
        assert!(snapshot.read(b"bad\0path", 1024).is_err());
        assert_eq!(
            success(f.snapshot("read", &commit, Some(OsStr::new("dir/value")), Some("5"))),
            b"12345"
        );
        rejected(f.snapshot("read", &commit, Some(OsStr::new("dir/value")), Some("4")));
        assert!(
            success(f.snapshot("read", &commit, Some(OsStr::new("empty")), Some("0"))).is_empty()
        );
        for limit in ["-1", "67108865", "not-a-number"] {
            rejected(f.snapshot("read", &commit, Some(OsStr::new("empty")), Some(limit)));
        }
        let root = f.json("stat", &commit, Some(OsStr::new("")));
        assert_eq!(root["path"], raw_path(b""));
        assert_eq!(root["entry"]["kind"], "tree");
        assert_eq!(
            root["entry"]["object_oid"],
            f.identity(&commit)["root_tree_oid"]
        );
    }
}

#[test]
fn bare_and_linked_worktree_reads_preserve_standard_repository_state() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        fs::write(f.repo.join("value"), b"shared object directory\n").unwrap();
        let commit = f.commit("common objects");
        let linked = f.temp.path().join("linked");
        success(
            f.command(&f.oracle)
                .args(["worktree", "add", "--detach"])
                .arg(&linked)
                .arg(&commit)
                .output()
                .unwrap(),
        );
        let bare = f.temp.path().join("bare.git");
        success(
            f.command(&f.oracle)
                .args(["clone", "--bare"])
                .arg(&f.repo)
                .arg(&bare)
                .output()
                .unwrap(),
        );
        let refs = f.git(&["for-each-ref"]);
        let index = fs::read(f.repo.join(".git/index")).unwrap();
        for repo in [&bare, &linked] {
            let identity: Value = serde_json::from_slice(&success(
                f.snapshot_at(repo, "inspect", "HEAD", None, None),
            ))
            .unwrap();
            assert_eq!(identity, f.identity(&commit));
            assert_eq!(
                success(f.snapshot_at(repo, "read", "HEAD", Some(OsStr::new("value")), None)),
                b"shared object directory\n"
            );
        }
        assert_eq!(f.git(&["for-each-ref"]), refs);
        assert_eq!(fs::read(f.repo.join(".git/index")).unwrap(), index);
        assert!(f.git(&["status", "--porcelain=v1"]).is_empty());
        f.git(&["fsck", "--full"]);
        assert!(!f.repo.join(".git/rit").exists());
    }
}

#[test]
fn explicit_snapshot_prefix_does_not_replace_git_dispatch_or_aliases() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        fs::write(f.repo.join("value"), b"dispatch\n").unwrap();
        let commit = f.commit("dispatch");
        let defaults: Value = serde_json::from_slice(&success(
            f.command(env!("CARGO_BIN_EXE_rit"))
                .args(["--rit-snapshot", "inspect"])
                .output()
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(defaults, f.identity(&commit));
        f.git(&["config", "alias.snapshot", "rev-parse --verify HEAD"]);
        assert_eq!(
            success(
                f.command(env!("CARGO_BIN_EXE_rit"))
                    .arg("snapshot")
                    .output()
                    .unwrap()
            ),
            format!("{commit}\n").as_bytes()
        );
        let git_result = f
            .command(env!("CARGO_BIN_EXE_git"))
            .args(["--rit-snapshot", "inspect"])
            .output()
            .unwrap();
        assert!(!git_result.status.success());
        assert!(git_result.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&git_result.stderr).contains("rit snapshot:"));
    }
}

#[test]
fn snapshot_rejects_ambient_storage_overrides_and_ignores_discovery_overrides() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        fs::write(f.repo.join("value"), b"explicit repository\n").unwrap();
        let commit = f.commit("environment isolation");
        for variable in [
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_REFERENCE_BACKEND",
        ] {
            for value in ["", "absent-snapshot-storage"] {
                let output = f
                    .command(env!("CARGO_BIN_EXE_rit"))
                    .args(["--rit-snapshot", "read", "--repo"])
                    .arg(&f.repo)
                    .args(["--revision", &commit, "--path", "value"])
                    .env(variable, value)
                    .output()
                    .unwrap();
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains(variable),
                    "guard must identify {variable}: {output:?}"
                );
                rejected(output);
            }
        }
        assert_eq!(
            success(
                f.command(env!("CARGO_BIN_EXE_rit"))
                    .args(["--rit-snapshot", "read", "--repo"])
                    .arg(&f.repo)
                    .args(["--revision", &commit, "--path", "value"])
                    .env("GIT_DIR", f.temp.path().join("absent-git-dir"))
                    .env("GIT_WORK_TREE", f.temp.path().join("absent-worktree"))
                    .output()
                    .unwrap()
            ),
            b"explicit repository\n"
        );
    }
}

#[test]
fn snapshot_options_reject_conflicts_unknown_arguments_and_invalid_encoded_paths() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        fs::write(f.repo.join("value"), b"option validation\n").unwrap();
        f.commit("options");
        let invalid: &[&[&str]] = &[
            &[],
            &["unknown"],
            &["read", "--path", "value", "--path-hex", "76616c7565"],
            &["read", "--path", "value", "--path", "value"],
            &["read", "--path-hex", "0"],
            &["read", "--path-hex", "gg"],
            &["read", "--path-hex", "00"],
            &["read", "--path-hex", ""],
            &["read", "--path", "value", "--unknown", "1"],
            &["read", "--path"],
            &["read", "--path", "value", "unexpected"],
            &["read", "--path", "value", "--max-bytes", "+99"],
            &["inspect", "--path", "value"],
            &["inspect", "--path-hex", "76616c7565"],
            &["inspect", "--repo", ""],
            &["inspect", "--revision", "HEAD", "--revision", "HEAD"],
            &["list", "--max-bytes", "99"],
            &["stat", "--max-bytes", "99"],
        ];
        for args in invalid {
            rejected(
                f.command(env!("CARGO_BIN_EXE_rit"))
                    .arg("--rit-snapshot")
                    .args(*args)
                    .output()
                    .unwrap(),
            );
        }
        assert_eq!(
            success(
                f.command(env!("CARGO_BIN_EXE_rit"))
                    .args(["--rit-snapshot", "read", "--path-hex", "76616C7565"])
                    .output()
                    .unwrap()
            ),
            b"option validation\n"
        );
    }
}
