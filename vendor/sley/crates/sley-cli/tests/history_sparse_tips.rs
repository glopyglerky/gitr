// SPDX-License-Identifier: MIT OR Apache-2.0
//! Preserve explicitly selected history tips and Git's sparse path traversal.

use std::cell::Cell;
use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::process::Output;
use std::time::{SystemTime, UNIX_EPOCH};

struct Repository {
    root: PathBuf,
    clock: Cell<u64>,
}

impl Repository {
    fn new(format: &str) -> Self {
        assert!(
            std::path::Path::new(sley_testkit::oracle_git()).is_absolute(),
            "SLEY_TEST_GIT must select an absolute reference Git"
        );
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "rit-history-sparse-tips-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("empty-path")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        let repo = Self {
            root,
            clock: Cell::new(1_000_000_000),
        };
        repo.git(&[
            "init",
            "-q",
            "-b",
            "main",
            &format!("--object-format={format}"),
        ]);
        repo
    }

    fn output(&self, program: impl AsRef<OsStr>, args: &[&str], candidate: bool) -> Output {
        let mut command = sley_testkit::hermetic_git_command_with_identity(program);
        command
            .current_dir(&self.root)
            .args(args)
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("home/.config"))
            .env("GIT_AUTHOR_DATE", format!("{} +0000", self.clock.get()))
            .env("GIT_COMMITTER_DATE", format!("{} +0000", self.clock.get()));
        if candidate {
            command.env("PATH", self.root.join("empty-path"));
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

    fn commit(&self, file: &str, value: &str, message: &str) -> String {
        self.clock.set(self.clock.get() + 1);
        fs::write(self.root.join(file), value).unwrap();
        self.git(&["add", "--", file]);
        self.git(&["commit", "-q", "-m", message]);
        String::from_utf8(self.git(&["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_owned()
    }

    fn candidate(&self, args: &[&str]) -> Output {
        let binary = std::env::var_os("RIT_TEST_BIN")
            .or_else(|| std::env::var_os("GITR_TEST_BIN"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(sley_testkit::sley_bin!()));
        assert!(binary.is_absolute(), "RIT_TEST_BIN must be absolute");
        self.output(binary, args, true)
    }

    fn assert_matches(&self, args: &[&str]) -> Vec<u8> {
        let expected = self.git(args);
        let actual = self.candidate(args);
        assert_eq!(
            actual.status.code(),
            expected.status.code(),
            "status for {args:?}: {}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stderr, expected.stderr, "stderr for {args:?}");
        assert!(
            actual.stdout == expected.stdout,
            "stdout for {args:?}\nexpected: {}\nactual: {}",
            String::from_utf8_lossy(&expected.stdout),
            String::from_utf8_lossy(&actual.stdout)
        );
        actual.stdout
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct History {
    repo: Repository,
    base: String,
    side: String,
}

fn history(format: &str) -> History {
    let repo = Repository::new(format);
    repo.commit("outside", "root\n", "unselected root");
    let base = repo.commit("watched", "base\n", "selected base");
    repo.git(&["branch", "side"]);
    repo.commit("watched", "main\n", "main change");
    repo.commit("outside", "main\n", "main unrelated");
    repo.git(&["checkout", "-q", "side"]);
    repo.commit("side-outside", "one\n", "side unrelated one");
    repo.commit("side-outside", "two\n", "side unrelated two");
    let side = repo.commit("watched", "side\n", "side change");
    repo.git(&["checkout", "-q", "main"]);
    repo.clock.set(repo.clock.get() + 1);
    repo.git(&[
        "merge",
        "-q",
        "--no-ff",
        "-s",
        "ours",
        "-m",
        "ignore side tree",
        "side",
    ]);
    repo.commit("watched", "tip\n", "main tip");
    History { repo, base, side }
}

fn compare_modes(repo: &Repository, options: &[&str], revisions: &[&str], path: bool) {
    for prefix in [
        vec!["log", "--graph", "--format=%H %P %s"],
        vec!["log", "--topo-order", "--format=%H %s"],
        vec!["rev-list", "--topo-order", "--parents"],
    ] {
        let mut args = prefix;
        args.extend_from_slice(options);
        args.extend_from_slice(revisions);
        if path {
            args.extend(["--", "watched"]);
        }
        repo.assert_matches(&args);
    }
}

#[test]
fn explicitly_selected_side_tip_survives_merge_diversion() {
    for format in ["sha1", "sha256"] {
        let History { repo, side, .. } = history(format);
        let only_main =
            repo.assert_matches(&["log", "--graph", "--format=%s", "main", "--", "watched"]);
        assert!(!String::from_utf8_lossy(&only_main).contains("side change"));
        for revisions in [
            vec!["main", "side"],
            vec!["side", "main"],
            vec!["--all"],
            vec!["main", side.as_str()],
        ] {
            compare_modes(&repo, &[], &revisions, true);
        }
        let both = repo.assert_matches(&[
            "log",
            "--graph",
            "--format=%s",
            "main",
            "side",
            "--",
            "watched",
        ]);
        assert!(String::from_utf8_lossy(&both).contains("side change"));
    }
}

#[test]
fn sparse_keeps_visited_path_history_and_dense_order_is_respected() {
    for format in ["sha1", "sha256"] {
        let History { repo, .. } = history(format);
        for options in [
            vec!["--sparse"],
            vec!["--dense"],
            vec!["--sparse", "--dense"],
            vec!["--dense", "--sparse"],
            vec!["--sparse", "--full-history"],
            vec!["--full-history", "--sparse"],
            vec!["--sparse", "--simplify-merges"],
            vec!["--simplify-merges", "--sparse"],
            vec!["--sparse", "--first-parent"],
            vec!["--sparse", "--full-history", "--first-parent"],
            vec!["--sparse", "--simplify-merges", "--first-parent"],
        ] {
            compare_modes(&repo, &options, &["--all"], true);
        }
        let sparse = repo.assert_matches(&[
            "log",
            "--graph",
            "--sparse",
            "--format=%s",
            "main",
            "--",
            "watched",
        ]);
        let text = String::from_utf8_lossy(&sparse);
        assert!(text.contains("unselected root") && text.contains("ignore side tree"));
        assert!(!text.contains("side change"));
        for prefix in [
            vec!["log", "--graph", "--format=%H %P %s"],
            vec!["log", "--topo-order", "--format=%H %s"],
            vec!["rev-list", "--topo-order", "--parents"],
        ] {
            let mut args = prefix;
            args.extend(["--sparse", "--all", "--", "absent-path"]);
            assert!(!repo.assert_matches(&args).is_empty());
        }
    }
}

#[test]
fn selected_tips_respect_ranges_filters_order_and_shallow_boundaries() {
    for format in ["sha1", "sha256"] {
        let History { repo, base, side } = history(format);
        let excluded_base = format!("^{base}");
        let excluded_side = format!("^{side}");
        for options in [
            vec![],
            vec!["--sparse"],
            vec!["--full-history"],
            vec!["--simplify-merges"],
            vec!["--first-parent"],
        ] {
            compare_modes(&repo, &options, &["main", "side", &excluded_base], true);
            compare_modes(&repo, &options, &["--all", &excluded_side], true);
        }
        for options in [
            vec!["--grep=main change"],
            vec!["--max-count=2"],
            vec!["--skip=1", "--max-count=2"],
        ] {
            compare_modes(&repo, &options, &["--all"], true);
        }
        for order in ["--date-order", "--author-date-order", "--topo-order"] {
            repo.assert_matches(&["log", order, "--format=%H %s", "--all", "--", "watched"]);
            repo.assert_matches(&[
                "log",
                order,
                "--reverse",
                "--format=%H %s",
                "--all",
                "--",
                "watched",
            ]);
            repo.assert_matches(&["rev-list", order, "--parents", "--all", "--", "watched"]);
            repo.assert_matches(&[
                "rev-list",
                order,
                "--reverse",
                "--parents",
                "--all",
                "--",
                "watched",
            ]);
        }
        compare_modes(&repo, &["--sparse"], &["--all"], false);
        fs::write(repo.root.join(".git/shallow"), format!("{base}\n")).unwrap();
        compare_modes(&repo, &[], &["--all"], true);
        compare_modes(&repo, &["--sparse"], &["--all"], true);
    }
}

#[test]
fn tied_and_backdated_clocks_keep_pre_simplification_order() {
    for format in ["sha1", "sha256"] {
        for offsets in [[0, 0, 0, 0, 0, 0], [0, 9, 4, 7, 8, 3]] {
            let repo = Repository::new(format);
            let commit = |index: usize, file: &str, contents: &str, message: &str| {
                repo.clock.set(1_000_000_000 + offsets[index] - 1);
                repo.commit(file, contents, message)
            };
            commit(0, "watched", "base\n", "base");
            repo.git(&["branch", "side"]);
            commit(1, "watched", "main\n", "main change");
            commit(2, "outside", "main\n", "main unchanged");
            repo.git(&["checkout", "-q", "side"]);
            commit(3, "watched", "side\n", "side change");
            repo.git(&["checkout", "-q", "main"]);
            repo.git(&[
                "merge",
                "-q",
                "--no-ff",
                "--no-commit",
                "-s",
                "ours",
                "side",
            ]);
            commit(4, "watched", "combined\n", "non-TREESAME merge");
            commit(5, "watched", "tip\n", "main tip");
            for options in [vec![], vec!["--full-history"], vec!["--simplify-merges"]] {
                for order in ["--topo-order", "--date-order", "--author-date-order"] {
                    let mut args = vec!["rev-list", "--parents"];
                    args.extend_from_slice(&options);
                    args.extend([order, "--all", "--", "watched"]);
                    repo.assert_matches(&args);
                }
            }
        }
    }
}

#[test]
fn sparse_skips_unused_linear_trees_but_requires_commit_objects() {
    for format in ["sha1", "sha256"] {
        let repo = Repository::new(format);
        repo.commit("watched", "base\n", "base");
        let middle = repo.commit("outside", "outside\n", "unrelated linear commit");
        repo.commit("watched", "tip\n", "tip");
        let tree = String::from_utf8(
            repo.git(&["rev-parse", &format!("{middle}^{{tree}}")])
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned();
        let loose = |oid: &str| {
            repo.root
                .join(".git/objects")
                .join(&oid[..2])
                .join(&oid[2..])
        };
        fs::remove_file(loose(&tree)).unwrap();
        for options in [
            vec!["--sparse"],
            vec!["--sparse", "--full-history"],
            vec!["--sparse", "--simplify-merges"],
        ] {
            for prefix in [vec!["log", "--format=%H %s"], vec!["rev-list", "--parents"]] {
                let mut args = prefix;
                args.extend_from_slice(&options);
                args.extend(["HEAD", "--", "watched"]);
                repo.assert_matches(&args);
            }
        }
        // Dense mode needs the missing tree for its comparison. Native
        // corruption diagnostics differ from Git, but neither may report
        // success. Sparse mode still needs every traversed commit object.
        let dense = ["log", "--format=%H %s", "HEAD", "--", "watched"];
        assert!(
            !repo
                .output(sley_testkit::oracle_git(), &dense, false)
                .status
                .success()
        );
        assert!(!repo.candidate(&dense).status.success());
        fs::remove_file(loose(&middle)).unwrap();
        let sparse = ["log", "--sparse", "--format=%H %s", "HEAD", "--", "watched"];
        assert!(
            !repo
                .output(sley_testkit::oracle_git(), &sparse, false)
                .status
                .success()
        );
        assert!(!repo.candidate(&sparse).status.success());
    }
}
