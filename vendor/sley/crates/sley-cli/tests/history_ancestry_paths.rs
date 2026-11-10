// SPDX-License-Identifier: MIT OR Apache-2.0
//! Exercise ancestry selection through the actual product command boundary.

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
        assert!(std::path::Path::new(sley_testkit::oracle_git()).is_absolute());
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "rit-history-ancestry-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("empty-path")).unwrap();
        fs::write(root.join("empty-config"), b"").unwrap();
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

    fn output(&self, binary: impl AsRef<OsStr>, args: &[&str], candidate: bool) -> Output {
        let mut command = sley_testkit::hermetic_git_command_with_identity(binary);
        command
            .current_dir(&self.root)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", self.root.join("empty-config"))
            .env("GIT_AUTHOR_DATE", format!("{} +0000", self.clock.get()))
            .env("GIT_COMMITTER_DATE", format!("{} +0000", self.clock.get()));
        if candidate {
            command.env("PATH", self.root.join("empty-path"));
        }
        command.output().unwrap()
    }

    fn git(&self, args: &[&str]) -> Output {
        let result = self.output(sley_testkit::oracle_git(), args, false);
        assert!(
            result.status.success(),
            "oracle {args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        result
    }

    fn oid(&self, rev: &str) -> String {
        String::from_utf8(self.git(&["rev-parse", rev]).stdout)
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

    fn compare(&self, args: &[&str]) -> Vec<u8> {
        let expected = self.output(sley_testkit::oracle_git(), args, false);
        let actual = self.candidate(args);
        assert_eq!(
            actual.status.code(),
            expected.status.code(),
            "status {args:?}: {}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stderr, expected.stderr, "stderr {args:?}");
        assert!(
            actual.stdout == expected.stdout,
            "stdout {args:?}\nexpected: {}\nactual: {}",
            String::from_utf8_lossy(&expected.stdout),
            String::from_utf8_lossy(&actual.stdout)
        );
        actual.stdout
    }

    fn commit(&self, name: &str, parents: &[&str], watched: &str, outside: &str) {
        self.clock.set(self.clock.get() + 1);
        fs::write(self.root.join("watched"), watched).unwrap();
        fs::write(self.root.join("outside"), outside).unwrap();
        self.git(&["add", "watched", "outside"]);
        let tree = String::from_utf8(self.git(&["write-tree"]).stdout).unwrap();
        let mut args = vec!["commit-tree", tree.trim(), "-m", name];
        for parent in parents {
            args.extend(["-p", parent]);
        }
        let oid = String::from_utf8(self.git(&args).stdout).unwrap();
        self.git(&["update-ref", &format!("refs/heads/{name}"), oid.trim()]);
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn history(format: &str) -> Repository {
    let repo = Repository::new(format);
    repo.commit("A", &[], "a\n", "a\n");
    repo.commit("B", &["A"], "b\n", "a\n");
    repo.commit("C", &["A"], "a\n", "c\n");
    repo.commit("D", &["B"], "b\n", "d\n");
    repo.commit("E", &["C"], "e\n", "c\n");
    repo.commit("M", &["D", "E"], "b\n", "d\n");
    repo.commit("N", &["M"], "n\n", "d\n");
    repo.commit("X", &["E", "D"], "e\n", "c\n");
    repo.git(&["symbolic-ref", "HEAD", "refs/heads/N"]);
    repo
}

#[test]
fn explicit_anchors_union_relations_without_importing_unwalked_history() {
    for format in ["sha1", "sha256"] {
        let repo = history(format);
        let b = repo.oid("B");
        let raw = format!("--ancestry-path={b}");
        repo.git(&["tag", "-a", "anchor-tag", "-m", "anchor", "B"]);
        for options in [
            vec!["--ancestry-path=B"],
            vec!["--ancestry-path=E"],
            vec!["--ancestry-path=M"],
            vec!["--ancestry-path=X"],
            vec!["--ancestry-path=B", "--ancestry-path=E"],
            vec!["--ancestry-path=E", "--ancestry-path=B"],
            vec!["--ancestry-path=B", "--ancestry-path=B"],
            vec!["--ancestry-path=B", "--ancestry-path"],
            vec!["--ancestry-path", "--ancestry-path=B"],
            vec!["--ancestry-path=anchor-tag"],
            vec![raw.as_str()],
        ] {
            for prefix in [vec!["log", "--format=%H %s"], vec!["rev-list"]] {
                let mut args = prefix;
                args.extend_from_slice(&options);
                args.push("A..N");
                repo.compare(&args);
            }
        }
        for range in ["N", "D..N", "B..D", "B...E"] {
            for anchor in ["--ancestry-path=B", "--ancestry-path=M"] {
                repo.compare(&["log", "--format=%H %s", anchor, range]);
                repo.compare(&["rev-list", anchor, range]);
            }
        }
    }
}

#[test]
fn ancestry_filter_is_applied_to_listing_count_and_accelerated_modes() {
    for format in ["sha1", "sha256"] {
        let repo = history(format);
        for storage in 0..2 {
            if storage == 1 {
                repo.git(&["repack", "-adb"]);
                repo.git(&["commit-graph", "write", "--reachable"]);
            }
            for options in [
                vec![],
                vec!["--count"],
                vec!["--parents"],
                vec!["--topo-order"],
                vec!["--date-order"],
                vec!["--author-date-order"],
                vec!["--reverse"],
                vec!["--max-count=2"],
                vec!["--skip=1", "--max-count=2"],
                vec!["--use-bitmap-index", "--count"],
                vec!["--use-bitmap-index", "--objects", "--count"],
            ] {
                for selection in [
                    vec!["--ancestry-path", "X..M"],
                    vec!["--ancestry-path", "--all", "^X"],
                    vec!["--ancestry-path=B", "A..N"],
                ] {
                    let mut args = vec!["rev-list"];
                    args.extend_from_slice(&options);
                    args.extend_from_slice(&selection);
                    repo.compare(&args);
                }
            }
        }
        for options in [
            vec!["--format=%H"],
            vec!["--format=%H", "-2"],
            vec!["--oneline", "-2"],
            vec!["--format=%s", "-2"],
        ] {
            let mut args = vec!["log", "--ancestry-path=E"];
            args.extend_from_slice(&options);
            args.push("N");
            repo.compare(&args);
        }
    }
}

#[test]
fn filters_and_path_simplification_keep_ancestry_connectivity() {
    for format in ["sha1", "sha256"] {
        let repo = history(format);
        for options in [
            vec!["--grep=B"],
            vec!["--grep=N"],
            vec!["--no-merges"],
            vec!["--first-parent"],
            vec!["--full-history"],
            vec!["--simplify-merges"],
            vec!["--sparse"],
            vec!["--reverse"],
            vec!["--max-count=2"],
        ] {
            for anchor in ["--ancestry-path=B", "--ancestry-path=M"] {
                for path in [false, true] {
                    for prefix in [vec!["log", "--format=%H %s"], vec!["rev-list"]] {
                        let mut args = prefix;
                        args.push(anchor);
                        args.extend_from_slice(&options);
                        args.push("A..N");
                        if path {
                            args.extend(["--", "watched"]);
                        }
                        repo.compare(&args);
                    }
                }
            }
        }
    }
}

#[test]
fn invalid_anchors_and_missing_bottoms_match_git_rejections() {
    for format in ["sha1", "sha256"] {
        let repo = history(format);
        let tree = format!("--ancestry-path={}", repo.oid("B^{tree}"));
        let blob = format!("--ancestry-path={}", repo.oid("B:watched"));
        for command in ["log", "rev-list", "fast-export"] {
            for anchor in [
                "--ancestry-path=",
                "--ancestry-path=not-a-revision",
                tree.as_str(),
                blob.as_str(),
            ] {
                repo.compare(&[command, anchor, "A..N"]);
            }
            repo.compare(&[command, "--ancestry-path", "N"]);
        }
        repo.compare(&[
            "log",
            "--format=%s",
            "--ancestry-path",
            "--ancestry-path=B",
            "N",
        ]);
    }
}

#[test]
fn shallow_boundaries_and_export_use_the_same_ancestry_selection() {
    for format in ["sha1", "sha256"] {
        let repo = history(format);
        for args in [
            vec!["fast-export", "--ancestry-path=B", "A..N"],
            vec!["fast-export", "--ancestry-path=M", "A..N"],
            vec![
                "fast-export",
                "--ancestry-path=B",
                "--ancestry-path=E",
                "A..N",
            ],
            vec!["fast-export", "--ancestry-path", "X..M"],
        ] {
            repo.compare(&args);
        }
        fs::write(
            repo.root.join(".git/shallow"),
            format!("{}\n{}\n", repo.oid("B"), repo.oid("C")),
        )
        .unwrap();
        for anchor in [
            "--ancestry-path=A",
            "--ancestry-path=B",
            "--ancestry-path=M",
        ] {
            repo.compare(&["log", "--format=%H %s", anchor, "N"]);
            repo.compare(&["rev-list", anchor, "N"]);
            repo.compare(&["rev-list", "--count", anchor, "N"]);
        }
    }
}

#[test]
fn imported_export_marks_do_not_break_ancestry_connectivity() {
    for format in ["sha1", "sha256"] {
        let repo = Repository::new(format);
        repo.commit("A", &[], "a\n", "same\n");
        repo.commit("B", &["A"], "b\n", "same\n");
        repo.commit("C", &["B"], "c\n", "same\n");
        repo.commit("D", &["C"], "d\n", "same\n");
        repo.commit("E", &["D"], "e\n", "same\n");
        let marks = repo.root.join("marks");
        let import = format!("--import-marks={}", marks.display());
        for marked in [vec!["C"], vec!["A", "B", "C"]] {
            let contents = marked
                .iter()
                .enumerate()
                .map(|(i, rev)| format!(":{} {}\n", i + 10, repo.oid(rev)))
                .collect::<String>();
            fs::write(&marks, contents).unwrap();
            for selection in [
                vec!["--ancestry-path", "A..E"],
                vec!["--ancestry-path=B", "E"],
            ] {
                let mut args = vec![
                    "fast-export",
                    "--no-data",
                    "--show-original-ids",
                    import.as_str(),
                ];
                args.extend_from_slice(&selection);
                repo.compare(&args);
            }
        }
    }
}

#[test]
fn tag_anchors_preserve_raw_identity_and_no_walk_still_lists_tips() {
    for format in ["sha1", "sha256"] {
        let repo = history(format);
        repo.git(&["tag", "-a", "middle-tag", "-m", "middle", "D"]);
        let tag_oid = repo.oid("middle-tag");
        let raw_tag = format!("--ancestry-path={tag_oid}");
        for anchor in [
            "--ancestry-path=middle-tag",
            "--ancestry-path=middle-tag^{commit}",
            "--ancestry-path=D",
            raw_tag.as_str(),
        ] {
            repo.compare(&["log", "--format=%H %s", anchor, "A..N"]);
            repo.compare(&["rev-list", anchor, "A..N"]);
            repo.compare(&["fast-export", anchor, "A..N"]);
        }
        // With only positive tips, --no-walk emits those tips without ancestry
        // limiting. Invalid explicit anchors must still fail during parsing.
        for mode in ["--no-walk", "--no-walk=sorted", "--no-walk=unsorted"] {
            for anchor in ["--ancestry-path=X", "--ancestry-path=B", "--ancestry-path"] {
                repo.compare(&["log", "--format=%H %s", mode, anchor, "N"]);
                repo.compare(&["rev-list", mode, anchor, "N"]);
            }
            repo.compare(&["log", mode, "--ancestry-path=missing", "N"]);
            repo.compare(&["rev-list", mode, "--ancestry-path=missing", "N"]);
        }
    }
}

#[test]
fn ancestry_children_keep_unselected_edges_and_original_walk_order() {
    for format in ["sha1", "sha256"] {
        let repo = Repository::new(format);
        repo.commit("A", &[], "a\n", "a\n");
        repo.commit("B", &["A"], "b\n", "a\n");
        repo.commit("C", &["B"], "c\n", "a\n");
        repo.commit("D", &["B"], "b\n", "d\n");
        repo.commit("E", &["C", "D"], "c\n", "a\n");
        repo.commit("F", &["E"], "c\n", "f\n");
        for order in [
            vec![],
            vec!["--topo-order"],
            vec!["--date-order"],
            vec!["--reverse"],
        ] {
            for path in [None, Some("watched"), Some("outside")] {
                let mut args = vec!["rev-list", "--ancestry-path=C", "--children"];
                args.extend_from_slice(&order);
                args.push("A..F");
                if let Some(path) = path {
                    args.extend(["--", path]);
                }
                repo.compare(&args);
            }
        }
    }
}

#[test]
fn no_walk_ancestry_export_retains_the_selected_ref() {
    // fast-export's existing no-walk traversal is not fully Git-compatible.
    // This outcome-only check prevents ancestry from turning a positive export
    // into a zero-object reset. It does not declare exact stream parity.
    for format in ["sha1", "sha256"] {
        let repo = history(format);
        let args = [
            "fast-export",
            "--no-data",
            "--show-original-ids",
            "--no-walk",
            "--ancestry-path",
            "N",
        ];
        let tip = repo.oid("N");
        let original = format!("original-oid {tip}\n");
        let delete = format!("from {}\n", "0".repeat(tip.len()));
        for output in [repo.git(&args), repo.candidate(&args)] {
            assert!(output.status.success());
            let stream = String::from_utf8(output.stdout).unwrap();
            assert!(
                stream.contains(&original),
                "selected tip missing from export"
            );
            assert!(
                !stream.contains(&delete),
                "positive export became a ref reset"
            );
        }
    }
}

#[test]
fn no_walk_ancestry_preserves_existing_child_list_order() {
    // The generic no-walk child decorations have a separate known mismatch.
    // An ancestry option that does not limit this walk must not reorder them.
    for format in ["sha1", "sha256"] {
        let repo = history(format);
        for mode in ["--no-walk", "--no-walk=unsorted"] {
            let plain = ["rev-list", "--children", mode, "A", "B", "C"];
            let ancestry = [
                "rev-list",
                "--children",
                mode,
                "--ancestry-path",
                "A",
                "B",
                "C",
            ];
            let reference_plain = repo.git(&plain);
            let reference_ancestry = repo.git(&ancestry);
            assert_eq!(reference_plain.stdout, reference_ancestry.stdout);
            let actual_plain = repo.candidate(&plain);
            let actual_ancestry = repo.candidate(&ancestry);
            assert!(actual_plain.status.success());
            assert_eq!(actual_plain.status.code(), actual_ancestry.status.code());
            assert_eq!(actual_plain.stderr, actual_ancestry.stderr);
            assert_eq!(actual_plain.stdout, actual_ancestry.stdout);
        }
    }
}
