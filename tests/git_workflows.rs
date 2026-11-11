// SPDX-License-Identifier: MIT OR Apache-2.0
//! Exercise the installed frontend without an executable named Git on PATH.
//! The reference binary is invoked only by explicit absolute path assertions.
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Repo {
    temp: tempfile::TempDir,
    path: PathBuf,
    empty_path: PathBuf,
    oracle: PathBuf,
}

impl Repo {
    fn new(format: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("repo");
        let empty_path = temp.path().join("empty-path");
        fs::create_dir_all(&path).unwrap();
        fs::create_dir_all(&empty_path).unwrap();
        let oracle = std::env::var_os("RIT_REFERENCE_GIT")
            .or_else(|| std::env::var_os("GITR_REFERENCE_GIT"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/usr/bin/git"));
        assert!(
            oracle.is_absolute() && oracle.is_file(),
            "set RIT_REFERENCE_GIT to an absolute reference binary"
        );
        let repo = Self {
            temp,
            path,
            empty_path,
            oracle,
        };
        repo.git(&[
            "init",
            "--initial-branch=main",
            &format!("--object-format={format}"),
        ]);
        repo
    }

    fn command(&self, program: impl AsRef<OsStr>, directory: &Path) -> Command {
        let mut command = Command::new(program);
        command
            .current_dir(directory)
            .env("RIT_BACKEND", "native")
            .env_remove("RIT_TRACE_BACKEND")
            .env("PATH", &self.empty_path)
            .env("HOME", self.temp.path())
            .env("XDG_CONFIG_HOME", self.temp.path().join("xdg"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.temp.path().join("absent-config"))
            .env(
                "GIT_CONFIG_SYSTEM",
                self.temp.path().join("absent-system-config"),
            )
            .env("GIT_CONFIG_COUNT", "0")
            .env("GIT_AUTHOR_NAME", "Native Git Test")
            .env("GIT_AUTHOR_EMAIL", "native@example.invalid")
            .env("GIT_COMMITTER_NAME", "Native Git Test")
            .env("GIT_COMMITTER_EMAIL", "native@example.invalid")
            .env("GIT_AUTHOR_DATE", "2001-02-03T04:05:06 +0000")
            .env("GIT_COMMITTER_DATE", "2001-02-03T04:05:06 +0000")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env_remove("GIT_CONFIG")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES");
        command
    }

    fn native(&self, args: &[&str]) -> Output {
        self.command(env!("CARGO_BIN_EXE_rit"), &self.path)
            .args(args)
            .output()
            .unwrap()
    }

    fn git(&self, args: &[&str]) -> String {
        Self::success(self.native(args), args)
    }

    fn reference(&self, args: &[&str]) -> String {
        Self::success(
            self.command(&self.oracle, &self.path)
                .args(args)
                .output()
                .unwrap(),
            args,
        )
    }

    fn success(output: Output, args: &[&str]) -> String {
        assert!(
            output.status.success(),
            "{args:?}: {:?}\nstdout={}\nstderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn commit(&self, file: &str, content: &str, message: &str) -> String {
        fs::write(self.path.join(file), content).unwrap();
        self.git(&["add", "--", file]);
        self.git(&["commit", "-m", message]);
        self.git(&["rev-parse", "HEAD"]).trim().to_owned()
    }

    fn verify(&self) {
        self.reference(&["fsck", "--full"]);
        assert_eq!(
            self.git(&["status", "--porcelain=v1"]),
            self.reference(&["status", "--porcelain=v1"])
        );
        assert_eq!(
            self.git(&["ls-files", "--stage"]),
            self.reference(&["ls-files", "--stage"])
        );
        assert_eq!(
            self.git(&["rev-parse", "HEAD"]),
            self.reference(&["rev-parse", "HEAD"])
        );
    }
}

#[test]
fn native_history_merge_rebase_stash_and_worktree() {
    for format in ["sha1", "sha256"] {
        let r = Repo::new(format);
        let base = r.commit("base", "base\n", "base");
        r.git(&["switch", "-c", "topic"]);
        r.commit("topic", "topic\n", "topic");
        r.git(&["switch", "main"]);
        r.commit("main", "main\n", "main");
        r.git(&["merge", "--no-ff", "-m", "merge", "topic"]);
        r.verify();
        r.git(&["switch", "-c", "replay", &base]);
        r.commit("replayed", "replayed\n", "replay");
        r.git(&["rebase", "main"]);
        assert!(r.path.join("topic").is_file());
        assert!(r.path.join("main").is_file());
        assert!(r.path.join("replayed").is_file());
        r.verify();
        fs::write(r.path.join("base"), "changed\n").unwrap();
        r.git(&["stash", "push", "-m", "pending"]);
        assert_eq!(fs::read_to_string(r.path.join("base")).unwrap(), "base\n");
        r.git(&["stash", "pop"]);
        assert_eq!(
            fs::read_to_string(r.path.join("base")).unwrap(),
            "changed\n"
        );
        r.git(&["restore", "base"]);
        let worktree = r.temp.path().join("linked");
        r.git(&[
            "worktree",
            "add",
            "--detach",
            worktree.to_str().unwrap(),
            "HEAD",
        ]);
        assert_eq!(
            fs::read_to_string(worktree.join("replayed")).unwrap(),
            "replayed\n"
        );
        r.git(&["worktree", "remove", worktree.to_str().unwrap()]);
        r.verify();
    }
}

#[test]
fn native_clone_fetch_push_and_reference_compare_and_swap() {
    for format in ["sha1", "sha256"] {
        let r = Repo::new(format);
        let first = r.commit("file", "first\n", "first");
        let bare = r.temp.path().join("server.git");
        r.git(&[
            "init",
            "--bare",
            "--initial-branch=main",
            &format!("--object-format={format}"),
            bare.to_str().unwrap(),
        ]);
        r.git(&["remote", "add", "origin", bare.to_str().unwrap()]);
        r.git(&["push", "-u", "origin", "main"]);
        let cloned = r.temp.path().join("clone");
        r.git(&["clone", bare.to_str().unwrap(), cloned.to_str().unwrap()]);
        assert_eq!(fs::read_to_string(cloned.join("file")).unwrap(), "first\n");
        let second = r.commit("file", "second\n", "second");
        r.git(&["push", "origin", "main"]);
        Repo::success(
            r.command(env!("CARGO_BIN_EXE_rit"), &cloned)
                .args(["pull", "--ff-only"])
                .output()
                .unwrap(),
            &["pull", "--ff-only"],
        );
        assert_eq!(fs::read_to_string(cloned.join("file")).unwrap(), "second\n");
        r.git(&["update-ref", "refs/heads/cas", &first]);
        r.git(&["update-ref", "refs/heads/cas", &second, &first]);
        assert!(
            !r.native(&["update-ref", "refs/heads/cas", &first, &first])
                .status
                .success()
        );
        assert_eq!(r.reference(&["rev-parse", "refs/heads/cas"]).trim(), second);
        r.verify();
    }
}

#[test]
fn native_merge_conflict_abort_preserves_git_state() {
    let r = Repo::new("sha1");
    r.commit("conflict", "base\n", "base");
    r.git(&["switch", "-c", "topic"]);
    r.commit("conflict", "theirs\n", "theirs");
    r.git(&["switch", "main"]);
    let ours = r.commit("conflict", "ours\n", "ours");
    assert!(!r.native(&["merge", "topic"]).status.success());
    assert!(r.path.join(".git/MERGE_HEAD").is_file());
    assert!(r.reference(&["ls-files", "--unmerged"]).lines().count() >= 3);
    r.git(&["merge", "--abort"]);
    assert!(!r.path.join(".git/MERGE_HEAD").exists());
    assert_eq!(r.reference(&["rev-parse", "HEAD"]).trim(), ours);
    assert_eq!(
        fs::read_to_string(r.path.join("conflict")).unwrap(),
        "ours\n"
    );
    r.verify();
}

#[cfg(unix)]
#[test]
fn native_frontend_preserves_non_utf8_arguments() {
    use std::os::unix::ffi::OsStrExt;
    for format in ["sha1", "sha256"] {
        let r = Repo::new(format);
        // Private-use Unicode used to collide with the donor's raw-byte argv
        // encoding. Every spelling must remain a distinct index/tree entry.
        let names: &[&[u8]] = &[
            b"binary-\xff-name",
            "binary-\u{e0ff}-name".as_bytes(),
            b"nested/\xfe-byte",
            b"mixed-\xee\x83\xbf-\xff",
        ];
        fs::create_dir(r.path.join("nested")).unwrap();
        for name in names {
            let filename = OsStr::from_bytes(name);
            fs::write(r.path.join(filename), b"raw path\n").unwrap();
            Repo::success(
                r.command(env!("CARGO_BIN_EXE_rit"), &r.path)
                    .args([OsStr::new("add"), OsStr::new("--"), filename])
                    .output()
                    .unwrap(),
                &["add", "byte filename"],
            );
        }
        r.git(&["commit", "-m", "raw paths"]);
        r.verify();
        let index = r.native(&["ls-files", "-z"]);
        assert!(index.status.success());
        let mut indexed: Vec<_> = index
            .stdout
            .split(|byte| *byte == 0)
            .filter(|name| !name.is_empty())
            .collect();
        let mut expected = names.to_vec();
        indexed.sort();
        expected.sort();
        assert_eq!(indexed, expected);
        for name in names {
            fs::write(r.path.join(OsStr::from_bytes(name)), b"updated raw path\n").unwrap();
        }
        r.git(&["add", "-u"]);
        r.git(&["commit", "-m", "updated paths"]);
        r.verify();
        for name in names {
            fs::remove_file(r.path.join(OsStr::from_bytes(name))).unwrap();
        }
        r.git(&["add", "-A"]);
        r.git(&["commit", "-m", "removed paths"]);
        assert!(r.native(&["ls-files", "-z"]).stdout.is_empty());
        r.verify();
    }
}
