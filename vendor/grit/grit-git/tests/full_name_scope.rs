//! Regression tests: `--full-name` does not affect scope in `ls-files` / `ls-tree`.
//!
//! Both commands shared the same bug class: `--full-name` (a display-only flag
//! that controls whether paths are printed repo-relative or cwd-relative) was
//! treated as a scope-widening flag so running from a subdirectory listed the
//! entire repository instead of only files under the cwd.
//!
//! | # | Issue | Fix  | Scope                                            |
//! |---|-------|------|--------------------------------------------------|
//! | 1 | #835  | —    | kevmoo reports `ls-files --full-name` scope leak |
//! | 2 | #856  | #857 | ls-files: drop `!args.full_name`                 |
//! | 3 | #856  | #858 | ls-tree: split scope + display flags             |
//!
//! Each test cross-checks grit output against system `git`.

mod common;

use crate::common::*;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

/// Build a repo with files at root level and inside `sub/`:
///
/// ```text
/// <root>/
///   root         (file created in first commit)
///   sub/a        (created in second commit)
///   sub/b
/// ```
fn build_test_repo(tag: &str) -> PathBuf {
    let dir = unique_tmp("full-name", tag);

    git_cmd(&["init", "-q", "-b", "main", "."])
        .in_dir(&dir)
        .suc();
    write_file(&dir, "root", "root content\n");
    git_cmd(&["add", "root"]).in_dir(&dir).suc();
    git_cmd(&["commit", "-q", "-m", "root"]).in_dir(&dir).suc();

    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).unwrap_or_else(|e| panic!("fixture: mkdir sub: {e}"));

    write_file(&sub, "a", "a content\n");
    write_file(&sub, "b", "b content\n");
    git_cmd(&["add", "sub/a", "sub/b"]).in_dir(&dir).suc();
    git_cmd(&["commit", "-q", "-m", "sub"]).in_dir(&dir).suc();

    dir
}

fn sub(dir: &Path) -> PathBuf {
    dir.join("sub")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn ls_files_full_name_from_subdir_no_pathspec() {
    let dir = build_test_repo("lsf-np");
    grit_cmd(&["ls-files", "--full-name"])
        .in_dir(&sub(&dir))
        .check();
}

#[test]
fn ls_files_full_name_from_subdir_with_pathspec() {
    let dir = build_test_repo("lsf-ps");
    grit_cmd(&["ls-files", "--full-name", "--", "sub"])
        .in_dir(&sub(&dir))
        .check();
}

#[test]
fn ls_tree_full_name_from_subdir_no_pathspec() {
    let dir = build_test_repo("lst-np");
    grit_cmd(&["ls-tree", "--full-name", "HEAD"])
        .in_dir(&sub(&dir))
        .check();
}

#[test]
fn ls_tree_full_name_from_subdir_with_pathspec() {
    let dir = build_test_repo("lst-ps");
    grit_cmd(&["ls-tree", "--full-name", "HEAD", "--", "sub"])
        .in_dir(&sub(&dir))
        .check();
}

#[test]
fn ls_tree_full_name_is_not_full_tree() {
    // --full-tree (scope) and --full-name (display) must NOT produce
    // the same output from a subdirectory.
    let dir = build_test_repo("lst-nt");
    let s = sub(&dir);

    let name = grit_cmd(&["ls-tree", "--full-name", "HEAD"])
        .in_dir(&s)
        .suc();
    let tree = grit_cmd(&["ls-tree", "--full-tree", "HEAD"])
        .in_dir(&s)
        .suc();

    assert_ne!(
        name.stdout, tree.stdout,
        "--full-name and --full-tree produced identical output\n\
         --full-name:\n{}\n--full-tree:\n{}",
        name.stdout, tree.stdout,
    );
}
