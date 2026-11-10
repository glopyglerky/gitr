// SPDX-License-Identifier: MIT OR Apache-2.0
//! Differential parent lists for path-limited history simplification.

use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::process::Output;
use std::time::{SystemTime, UNIX_EPOCH};

struct Repository(PathBuf);

impl Repository {
    fn new(format: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sley-rev-list-path-parents-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("empty-path")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        let repo = Self(root);
        repo.git(&[
            "init",
            "-q",
            "-b",
            "main",
            &format!("--object-format={format}"),
        ]);
        repo
    }

    fn output(&self, program: impl AsRef<OsStr>, args: &[&str], empty_path: bool) -> Output {
        let mut command = sley_testkit::hermetic_git_command_with_identity(program);
        command
            .current_dir(&self.0)
            .args(args)
            .env("HOME", self.0.join("home"))
            .env("XDG_CONFIG_HOME", self.0.join("home/.config"));
        if empty_path {
            command.env("PATH", self.0.join("empty-path"));
        }
        command.output().unwrap()
    }

    fn git(&self, args: &[&str]) -> Output {
        let output = self.output(sley_testkit::oracle_git(), args, false);
        assert!(
            output.status.success(),
            "oracle {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn commit(&self, path: &str, contents: &[u8], message: &str) -> String {
        let destination = self.0.join(path);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, contents).unwrap();
        self.git(&["add", "--", path]);
        self.git(&["commit", "-q", "-m", message]);
        String::from_utf8(self.git(&["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_owned()
    }

    fn assert_matches(&self, args: &[&str]) -> Vec<u8> {
        let expected = self.git(args);
        let binary = std::env::var_os("RIT_TEST_BIN")
            .or_else(|| std::env::var_os("GITR_TEST_BIN"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(sley_testkit::sley_bin!()));
        assert!(
            binary.is_absolute(),
            "RIT_TEST_BIN must be an absolute path"
        );
        let actual = self.output(binary, args, true);
        assert_eq!(
            actual.status.code(),
            expected.status.code(),
            "{args:?}: {}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert!(
            actual.stdout == expected.stdout,
            "stdout for {args:?}\nexpected: {}\nactual: {}",
            String::from_utf8_lossy(&expected.stdout),
            String::from_utf8_lossy(&actual.stdout)
        );
        assert_eq!(actual.stderr, expected.stderr, "stderr for {args:?}");
        actual.stdout
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn simplified_path_history_prunes_ancestors_before_directory_creation() {
    for format in ["sha1", "sha256"] {
        let repo = Repository::new(format);
        repo.commit("outside", b"one\n", "outside root");
        let before = repo.commit("outside", b"two\n", "before selected directory");
        let first = repo.commit("nested/file", b"one\n", "create selected directory");
        repo.commit("outside", b"three\n", "unselected change");
        let last = repo.commit("nested/file", b"two\n", "change selected directory");
        repo.commit("outside", b"four\n", "unselected tip");
        let args = [
            "rev-list",
            "--parents",
            "--simplify-merges",
            "HEAD",
            "--",
            "nested",
        ];
        let output = repo.assert_matches(&args);
        assert_eq!(output, format!("{last} {first}\n{first}\n").as_bytes());
        for options in [
            vec!["--reverse", "--topo-order", "--simplify-merges"],
            vec!["--first-parent", "--simplify-merges"],
            vec!["--full-history", "--simplify-merges"],
            vec!["--full-history"],
            vec![],
        ] {
            let mut args = vec!["rev-list", "--parents"];
            args.extend(options);
            args.extend(["HEAD", "--", "nested"]);
            repo.assert_matches(&args);
        }
        let range = format!("{before}..HEAD");
        let output = repo.assert_matches(&[
            "rev-list",
            "--parents",
            "--simplify-merges",
            &range,
            "--",
            "nested",
        ]);
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains(&format!("{first} {before}"))
        );
        repo.assert_matches(&["rev-list", "--parents", "--simplify-merges", "HEAD"]);

        // A shallow boundary that contains the selected path is its displayed
        // root, even though the stored commit object still records a parent.
        fs::write(repo.0.join(".git/shallow"), format!("{first}\n")).unwrap();
        let output = repo.assert_matches(&args);
        assert_eq!(output, format!("{last} {first}\n{first}\n").as_bytes());
    }
}

#[test]
fn simplified_path_history_retains_root_that_changes_the_selected_directory() {
    for format in ["sha1", "sha256"] {
        let repo = Repository::new(format);
        let first = repo.commit("nested/file", b"one\n", "selected root");
        repo.commit("outside", b"one\n", "unselected change");
        let last = repo.commit("nested/file", b"two\n", "selected change");
        repo.commit("outside", b"two\n", "unselected tip");
        let output = repo.assert_matches(&[
            "rev-list",
            "--parents",
            "--simplify-merges",
            "HEAD",
            "--",
            "nested",
        ]);
        assert_eq!(output, format!("{last} {first}\n{first}\n").as_bytes());
        repo.assert_matches(&[
            "rev-list",
            "--parents",
            "--reverse",
            "--simplify-merges",
            "HEAD",
            "--",
            "nested",
        ]);
    }
}

#[test]
fn simplified_merge_keeps_selected_parent_branches_and_explicit_boundary() {
    for format in ["sha1", "sha256"] {
        let repo = Repository::new(format);
        let root = repo.commit("outside", b"root\n", "unselected root");
        repo.git(&["branch", "side"]);
        let main = repo.commit("nested/main", b"main\n", "selected main change");
        repo.commit("main-outside", b"main\n", "unselected main change");
        repo.git(&["checkout", "-q", "side"]);
        let side = repo.commit("nested/side", b"side\n", "selected side change");
        let side_tip = repo.commit("side-outside", b"side\n", "unselected side change");
        repo.git(&["checkout", "-q", "main"]);
        repo.git(&[
            "merge",
            "--no-ff",
            "-q",
            "-m",
            "merge selected paths",
            "side",
        ]);
        let output = repo.assert_matches(&[
            "rev-list",
            "--parents",
            "--topo-order",
            "--simplify-merges",
            "HEAD",
            "--",
            "nested",
        ]);
        let text = String::from_utf8(output).unwrap();
        assert!(
            text.lines()
                .next()
                .unwrap()
                .ends_with(&format!(" {main} {side}"))
        );
        assert!(text.lines().any(|line| line == main));
        assert!(text.lines().any(|line| line == side));
        let output = repo.assert_matches(&[
            "rev-list",
            "--parents",
            "--first-parent",
            "--simplify-merges",
            "HEAD",
            "--",
            "nested",
        ]);
        assert!(
            String::from_utf8(output)
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .ends_with(&format!(" {main} {side_tip}"))
        );
        let range = format!("{root}..HEAD");
        repo.assert_matches(&[
            "rev-list",
            "--parents",
            "--topo-order",
            "--simplify-merges",
            &range,
            "--",
            "nested",
        ]);
    }
}
