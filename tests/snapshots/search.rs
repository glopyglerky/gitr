// SPDX-License-Identifier: MIT OR Apache-2.0
//! Search acceptance through the snapshot API and real CLI, with no Git on PATH.

use super::*;
use rit::snapshot::SearchLimits;

fn search(f: &Fixture, options: &[&str]) -> Output {
    f.command(env!("CARGO_BIN_EXE_rit"))
        .args(["--rit-snapshot", "search"])
        .args(options)
        .output()
        .unwrap()
}

#[test]
fn exact_search_locations_survive_packing_and_reopen_in_both_object_formats() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        fs::create_dir(f.repo.join("tree")).unwrap();
        fs::write(f.repo.join("a.txt"), b"needle\nx needle needle\n").unwrap();
        fs::write(f.repo.join("binary"), b"\0needle\0").unwrap();
        fs::write(f.repo.join("tree/deep.txt"), b"prefix\nneedle").unwrap();
        let commit = f.commit("literal search fixture");
        let expected = vec![
            ("a.txt", 0, 1, 1),
            ("a.txt", 9, 2, 3),
            ("a.txt", 16, 2, 10),
            ("binary", 1, 1, 2),
            ("tree/deep.txt", 7, 2, 1),
        ];
        let mut previous = None;
        for packed in [false, true] {
            if packed {
                f.git(&["repack", "-adf"]);
                f.git(&["prune-packed"]);
            }
            let snapshot = Snapshot::open(&f.repo, &commit).unwrap();
            let report = snapshot.search(b"", b"needle", SearchLimits::default()).unwrap();
            assert_eq!(report.commit_id, snapshot.commit_id());
            assert_eq!(report.tree_id, snapshot.tree_id());
            assert_eq!(report.files, 3);
            assert_eq!(report.entries, 5);
            assert_eq!(report.blob_bytes, 44);
            assert!(report.tree_bytes > 0);
            assert_eq!(report.matches.len(), expected.len());
            for (found, (path, offset, line, column)) in report.matches.iter().zip(&expected) {
                assert_eq!(found.path, path.as_bytes());
                assert_eq!(found.byte_offset, *offset);
                assert_eq!(found.line, *line);
                assert_eq!(found.byte_column, *column);
                assert_eq!(
                    found.blob_id.to_hex(),
                    f.git_text(&["rev-parse", &format!("{commit}:{path}")])
                );
            }
            let json: Value = serde_json::from_slice(&success(search(
                &f, &["--query", "needle", "--revision", &commit],
            ))).unwrap();
            assert_eq!(json["snapshot"], f.identity(&commit));
            assert_eq!(json["complete"], true);
            assert_eq!(json["scope"], "regular-file-bytes");
            assert_eq!(json["matches"].as_array().unwrap().len(), 5);
            assert_eq!(json["matches"][1]["byte_offset"], 9);
            assert_eq!(json["matches"][1]["byte_length"], 6);
            assert_eq!(json["matches"][1]["byte_column"], 3);
            if let Some(previous) = previous {
                assert_eq!(json, previous);
            }
            previous = Some(json);
        }
        f.git(&["fsck", "--full"]);
    }
}

#[test]
fn opened_and_saved_snapshots_ignore_moving_heads_and_dirty_worktrees() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        fs::write(f.repo.join("source"), b"needle").unwrap();
        let first = f.commit("saved source");
        let snapshot = Snapshot::open(&f.repo, "HEAD").unwrap();
        success(f.command(env!("CARGO_BIN_EXE_rit")).args(["--rit-db", "init"]).output().unwrap());
        success(f.command(env!("CARGO_BIN_EXE_rit")).args([
            "--rit-db", "put", "--name", "source", "--revision", &first,
            "--expect-generation", "0", "--request-id", "source-1",
        ]).output().unwrap());
        fs::write(f.repo.join("source"), b"new committed content").unwrap();
        let second = f.commit("updated source");
        fs::write(f.repo.join("source"), b"dirty needle needle").unwrap();
        assert_ne!(first, second);
        assert_eq!(snapshot.search(b"", b"needle", SearchLimits::default()).unwrap().matches.len(), 1);
        let saved: Value = serde_json::from_slice(&success(search(
            &f, &["--saved", "source", "--query", "needle"],
        ))).unwrap();
        assert_eq!(saved["snapshot"], f.identity(&first));
        assert_eq!(saved["matches"].as_array().unwrap().len(), 1);
        let current: Value = serde_json::from_slice(&success(search(&f, &["--query", "needle"]))).unwrap();
        assert_eq!(current["snapshot"], f.identity(&second));
        assert!(current["matches"].as_array().unwrap().is_empty());
        assert_eq!(fs::read(f.repo.join("source")).unwrap(), b"dirty needle needle");
        rejected(search(&f, &["--saved", "absent", "--query", "needle"]));
        rejected(search(&f, &["--saved", "source", "--revision", "HEAD", "--query", "needle"]));
    }
}

#[test]
fn literal_search_preserves_overlaps_and_byte_columns_without_regex_semantics() {
    let f = Fixture::new("sha1");
    fs::write(f.repo.join("source"), b"ababa\naaaa\n.*\n\xc3\xa9needle\n").unwrap();
    f.commit("literal locations");
    let snapshot = Snapshot::open(&f.repo, "HEAD").unwrap();
    for (query, offsets) in [
        (b"aba".as_slice(), vec![0, 2]),
        (b"aa".as_slice(), vec![6, 7, 8]),
        (b".*".as_slice(), vec![11]),
        (b"needle".as_slice(), vec![16]),
        (b"NEEDLE".as_slice(), vec![]),
    ] {
        let report = snapshot.search(b"source", query, SearchLimits::default()).unwrap();
        assert_eq!(report.matches.iter().map(|item| item.byte_offset).collect::<Vec<_>>(), offsets);
    }
    let report = snapshot.search(b"source", b"needle", SearchLimits::default()).unwrap();
    assert_eq!((report.matches[0].line, report.matches[0].byte_column), (4, 3));
}

#[test]
fn subtree_search_does_not_read_outside_the_selected_path() {
    let f = Fixture::new("sha256");
    fs::create_dir(f.repo.join("selected")).unwrap();
    fs::write(f.repo.join("selected/source"), b"needle").unwrap();
    fs::write(f.repo.join("unavailable"), b"missing outside subtree").unwrap();
    f.commit("subtree isolation");
    let missing = f.git_text(&["rev-parse", "HEAD:unavailable"]);
    fs::remove_file(f.object_path(&missing)).unwrap();
    let snapshot = Snapshot::open(&f.repo, "HEAD").unwrap();
    let report = snapshot.search(b"selected", b"needle", SearchLimits::default()).unwrap();
    assert_eq!(report.matches[0].path, b"selected/source");
    assert_eq!(report.entries, 2);
    assert!(snapshot.search(b"", b"needle", SearchLimits::default()).is_err());
    rejected(search(&f, &["--query", "needle"]));
}

#[test]
fn search_limits_are_exact_and_never_return_successful_partial_results() {
    let f = Fixture::new("sha1");
    fs::write(f.repo.join("a"), b"aaaaa").unwrap();
    fs::write(f.repo.join("b"), b"aaaaa").unwrap();
    f.commit("search budgets");
    let exact = ["--query", "aa", "--max-bytes", "5", "--max-total-bytes", "10", "--max-entries", "3", "--max-matches", "8"];
    let result: Value = serde_json::from_slice(&success(search(&f, &exact))).unwrap();
    assert_eq!(result["matches"].as_array().unwrap().len(), 8);
    for (option, value) in [("--max-bytes", "4"), ("--max-total-bytes", "9"), ("--max-entries", "2"), ("--max-matches", "7")] {
        rejected(search(&f, &["--query", "aa", option, value]));
    }
    let snapshot = Snapshot::open(&f.repo, "HEAD").unwrap();
    for limits in [
        SearchLimits { max_entries: 0, ..SearchLimits::default() },
        SearchLimits { max_matches: 0, ..SearchLimits::default() },
        SearchLimits { max_total_bytes: SearchLimits::MAX_TOTAL_BYTES + 1, ..SearchLimits::default() },
        SearchLimits { max_blob_bytes: rit::snapshot::MAX_BLOB_BYTES + 1, ..SearchLimits::default() },
    ] {
        assert!(snapshot.search(b"", b"aa", limits).is_err());
    }
    for options in [
        vec![], vec!["--query", ""], vec!["--query", "a\nb"],
        vec!["--query", "aa", "--query", "aa"],
        vec!["--query", "aa", "--max-entries", "0"],
        vec!["--query", "aa", "--max-matches", "10001"],
        vec!["--query", "aa", "--max-total-bytes", "268435457"],
        vec!["--query", "aa", "--max-total-bytes", "+10"],
        vec!["--query", "aa", "--max-total-bytes", "10", "--max-total-bytes", "10"],
        vec!["--query", "aa", "--path", "../a"],
    ] {
        rejected(search(&f, &options));
    }
    rejected(search(&f, &["--query", &"a".repeat(SearchLimits::MAX_QUERY_BYTES + 1)]));
    for action in ["inspect", "list", "stat", "read"] {
        rejected(f.command(env!("CARGO_BIN_EXE_rit")).args([
            "--rit-snapshot", action, "--query", "aa",
        ]).output().unwrap());
    }
}

#[test]
fn hash_mismatch_after_an_earlier_match_fails_without_any_stdout() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        fs::write(f.repo.join("a"), b"needle").unwrap();
        fs::write(f.repo.join("z"), b"other!").unwrap();
        f.commit("verification before publication");
        let good = f.git_text(&["rev-parse", "HEAD:a"]);
        let bad = f.git_text(&["rev-parse", "HEAD:z"]);
        fs::copy(f.object_path(&good), f.object_path(&bad)).unwrap();
        rejected(search(&f, &["--query", "needle"]));
    }
}

#[cfg(unix)]
#[test]
fn raw_paths_are_exact_and_symlinks_and_gitlinks_are_never_followed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        let name = b"raw-\xff-name";
        fs::write(f.repo.join(os_path(name)), b"needle").unwrap();
        fs::set_permissions(f.repo.join(os_path(name)), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(f.temp.path().join("outside"), b"needle").unwrap();
        symlink("../outside", f.repo.join("link")).unwrap();
        f.git(&["add", "-A"]);
        let foreign = "d".repeat(if format == "sha1" { 40 } else { 64 });
        f.git(&["update-index", "--add", "--cacheinfo", "160000", &foreign, "submodule"]);
        f.git(&["commit", "-m", "raw source paths"]);
        let result: Value = serde_json::from_slice(&success(search(&f, &["--query", "needle"]))).unwrap();
        assert_eq!(result["matches"].as_array().unwrap().len(), 1);
        assert_eq!(result["matches"][0]["path"], raw_path(name));
        assert_eq!(result["scanned"]["skipped_symlinks"], 1);
        assert_eq!(result["scanned"]["skipped_gitlinks"], 1);
        let path_hex = raw_path(name)["hex"].as_str().unwrap().to_owned();
        let selected: Value = serde_json::from_slice(&success(search(&f, &["--query", "needle", "--path-hex", &path_hex]))).unwrap();
        assert_eq!(selected["matches"], result["matches"]);
        rejected(search(&f, &["--query", "needle", "--path", "link"]));
        rejected(search(&f, &["--query", "needle", "--path", "submodule"]));
    }
}

#[test]
fn empty_files_and_empty_trees_return_complete_empty_results() {
    let f = Fixture::new("sha1");
    f.git(&["commit", "--allow-empty", "-m", "empty tree"]);
    let result: Value = serde_json::from_slice(&success(search(&f, &["--query", "needle", "--max-total-bytes", "0"]))).unwrap();
    assert_eq!(result["complete"], true);
    assert_eq!(result["scanned"]["entries"], 1);
    assert!(result["matches"].as_array().unwrap().is_empty());
    fs::write(f.repo.join("empty"), b"").unwrap();
    f.commit("empty file");
    let result: Value = serde_json::from_slice(&success(search(&f, &["--query", "needle", "--path", "empty", "--max-bytes", "0", "--max-total-bytes", "0"]))).unwrap();
    assert_eq!(result["scanned"]["files"], 1);
    assert!(result["matches"].as_array().unwrap().is_empty());
}
