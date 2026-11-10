//! Checkout publication and preflight regressions without a reference Git process.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use grit_lib::index::{entry_from_stat, Index, MODE_REGULAR};
use grit_lib::objects::{ObjectId, ObjectKind};
use grit_lib::odb::Odb;
use grit_lib::refs::{compare_and_swap_ref, read_ref_snapshot, CheckoutRefGuard, Ref};
use grit_lib::state::resolve_head;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

struct Fixture {
    root: tempfile::TempDir,
    git_dir: PathBuf,
    main: ObjectId,
    other: ObjectId,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let git_dir = root.path().join(".git");
        fs::create_dir_all(git_dir.join("objects")).unwrap();
        fs::create_dir_all(git_dir.join("refs/heads")).unwrap();
        fs::write(git_dir.join("HEAD"), b"ref: refs/heads/main\n").unwrap();
        fs::write(git_dir.join("config"), b"[core]\nrepositoryformatversion = 0\nlogAllRefUpdates = true\n").unwrap();
        let odb = Odb::new(&git_dir.join("objects")).with_config_git_dir(git_dir.clone());
        let (main, blob) = make_commit(&odb, b"main\n", None);
        let (other, _) = make_commit(&odb, b"other\n", Some(main));
        fs::write(git_dir.join("refs/heads/main"), format!("{main}\n")).unwrap();
        fs::write(git_dir.join("refs/heads/other"), format!("{other}\n")).unwrap();
        fs::write(root.path().join("file"), b"main\n").unwrap();
        let mut index = Index::new();
        index.entries.push(entry_from_stat(&root.path().join("file"), b"file", blob, MODE_REGULAR).unwrap());
        index.write(&git_dir.join("index")).unwrap();
        Self { root, git_dir, main, other }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_grit-git"));
        command.current_dir(self.root.path()).args(args).env_clear()
            .env("PATH", self.root.path().join("no-programs"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.root.path().join("absent-global"))
            .env("GIT_CONFIG_SYSTEM", self.root.path().join("absent-system"))
            .env("GIT_AUTHOR_NAME", "Checkout Test")
            .env("GIT_AUTHOR_EMAIL", "checkout@example.invalid")
            .env("GIT_COMMITTER_NAME", "Checkout Test")
            .env("GIT_COMMITTER_EMAIL", "checkout@example.invalid");
        command
    }

    fn snapshot(&self) -> Vec<Vec<u8>> {
        [self.git_dir.join("HEAD"), self.git_dir.join("index"),
         self.git_dir.join("refs/heads/main"), self.git_dir.join("refs/heads/other"),
         self.git_dir.join("config"), self.root.path().join("file")]
            .iter().map(|path| fs::read(path).unwrap()).collect()
    }
}

fn make_commit(odb: &Odb, content: &[u8], parent: Option<ObjectId>) -> (ObjectId, ObjectId) {
    let blob = odb.write(ObjectKind::Blob, content).unwrap();
    let mut tree = b"100644 file\0".to_vec();
    tree.extend_from_slice(blob.as_bytes());
    let tree = odb.write(ObjectKind::Tree, &tree).unwrap();
    let parent = parent.map(|oid| format!("parent {oid}\n")).unwrap_or_default();
    let commit = format!("tree {tree}\n{parent}author Test <test@example.invalid> 1700000000 +0000\ncommitter Test <test@example.invalid> 1700000000 +0000\n\nfixture\n");
    (odb.write(ObjectKind::Commit, commit.as_bytes()).unwrap(), blob)
}

fn succeeds(output: Output) {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn existing_branch_new_branch_and_detached_checkout_round_trip() {
    let f = Fixture::new();
    succeeds(f.command(&["switch", "other"]).output().unwrap());
    assert_eq!(fs::read(f.root.path().join("file")).unwrap(), b"other\n");
    assert_eq!(read_ref_snapshot(&f.git_dir, "HEAD").unwrap(), Some(Ref::Symbolic("refs/heads/other".into())));
    succeeds(f.command(&["checkout", "-b", "created", "main"]).output().unwrap());
    assert_eq!(fs::read(f.root.path().join("file")).unwrap(), b"main\n");
    assert_eq!(read_ref_snapshot(&f.git_dir, "refs/heads/created").unwrap(), Some(Ref::Direct(f.main)));
    succeeds(f.command(&["switch", "--detach", &f.other.to_hex()]).output().unwrap());
    assert_eq!(read_ref_snapshot(&f.git_dir, "HEAD").unwrap(), Some(Ref::Direct(f.other)));
    assert_eq!(fs::read(f.root.path().join("file")).unwrap(), b"other\n");
    succeeds(f.command(&["switch", "-c", "from-detached"]).output().unwrap());
    assert_eq!(read_ref_snapshot(&f.git_dir, "HEAD").unwrap(), Some(Ref::Symbolic("refs/heads/from-detached".into())));
    assert_eq!(read_ref_snapshot(&f.git_dir, "refs/heads/from-detached").unwrap(), Some(Ref::Direct(f.other)));
    assert!(!f.git_dir.join("HEAD.lock").exists());
    assert!(!f.git_dir.join("index.lock").exists());
}

#[test]
fn competing_head_branch_and_index_locks_fail_before_worktree_changes() {
    for lock in ["HEAD.lock", "refs/heads/main.lock", "refs/heads/other.lock", "index.lock"] {
        let f = Fixture::new();
        fs::write(f.git_dir.join(lock), b"competing owner").unwrap();
        let before = f.snapshot();
        let output = f.command(&["checkout", "other"]).output().unwrap();
        assert!(!output.status.success(), "checkout ignored {lock}");
        assert_eq!(f.snapshot(), before, "preflight failure changed repository with {lock}");
        assert_eq!(fs::read(f.git_dir.join(lock)).unwrap(), b"competing owner");
        if lock != "HEAD.lock" { assert!(!f.git_dir.join("HEAD.lock").exists()); }
        assert!(!f.git_dir.join("logs").exists());
    }
}

#[test]
fn force_checkout_still_respects_another_worktrees_branch() {
    let f = Fixture::new();
    let linked = tempfile::tempdir().unwrap();
    let admin = f.git_dir.join("worktrees/linked");
    fs::create_dir_all(&admin).unwrap();
    fs::write(admin.join("HEAD"), b"ref: refs/heads/other\n").unwrap();
    fs::write(admin.join("commondir"), b"../..\n").unwrap();
    fs::write(admin.join("gitdir"), format!("{}\n", linked.path().join(".git").display())).unwrap();
    fs::write(linked.path().join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
    fs::write(linked.path().join("file"), b"linked working copy\n").unwrap();
    let before = f.snapshot();
    let output = f.command(&["checkout", "-f", "other"]).output().unwrap();
    assert!(!output.status.success(), "-f stole a branch occupied by another worktree");
    assert!(String::from_utf8_lossy(&output.stderr).contains("already used by worktree"));
    assert_eq!(f.snapshot(), before);
    assert_eq!(fs::read(admin.join("HEAD")).unwrap(), b"ref: refs/heads/other\n");
    assert_eq!(fs::read(linked.path().join("file")).unwrap(), b"linked working copy\n");
    assert!(!f.git_dir.join("HEAD.lock").exists());
    assert!(!f.git_dir.join("refs/heads/other.lock").exists());
}

#[test]
fn unborn_new_branch_changes_only_head_and_preserves_staged_content() {
    let f = Fixture::new();
    fs::remove_file(f.git_dir.join("refs/heads/main")).unwrap();
    let index = fs::read(f.git_dir.join("index")).unwrap();
    let file = fs::read(f.root.path().join("file")).unwrap();
    succeeds(f.command(&["switch", "-c", "unborn"]).output().unwrap());
    assert_eq!(read_ref_snapshot(&f.git_dir, "HEAD").unwrap(), Some(Ref::Symbolic("refs/heads/unborn".into())));
    assert_eq!(read_ref_snapshot(&f.git_dir, "refs/heads/unborn").unwrap(), None);
    assert_eq!(fs::read(f.git_dir.join("index")).unwrap(), index);
    assert_eq!(fs::read(f.root.path().join("file")).unwrap(), file);
}

#[test]
fn unsupported_variants_and_dirty_conflicts_fail_without_side_effects() {
    let variants: &[&[&str]] = &[
        &["checkout", "-B", "other", "main"],
        &["switch", "--orphan", "created"],
        &["checkout", "-m", "other"],
        &["checkout", "--recurse-submodules", "other"],
        &["checkout", "--track", "origin/topic"],
        &["checkout", "--", "file"],
        &["checkout", "--pathspec-from-file=-"],
    ];
    for args in variants {
        let f = Fixture::new();
        let before = f.snapshot();
        let output = f.command(args).output().unwrap();
        assert!(!output.status.success(), "accepted unsupported {args:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("unavailable"), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(f.snapshot(), before, "unsupported {args:?} changed repository");
        assert!(!f.git_dir.join("refs/heads/created").exists());
    }
    let f = Fixture::new();
    fs::write(f.root.path().join("file"), b"uncommitted edit\n").unwrap();
    let before = f.snapshot();
    assert!(!f.command(&["checkout", "other"]).output().unwrap().status.success());
    assert_eq!(f.snapshot(), before);
}

#[test]
fn original_head_and_destination_expectations_are_checked_under_owned_locks() {
    let f = Fixture::new();
    let head = resolve_head(&f.git_dir).unwrap();
    let expected = Ref::Direct(f.other);
    compare_and_swap_ref(&f.git_dir, "refs/heads/other", Some(&expected), Some(&Ref::Direct(f.main))).unwrap();
    assert!(CheckoutRefGuard::acquire(&f.git_dir, &head, Some(("refs/heads/other", Some(&expected)))).is_err());
    assert!(!f.git_dir.join("HEAD.lock").exists());
    compare_and_swap_ref(&f.git_dir, "refs/heads/main", Some(&Ref::Direct(f.main)), Some(&Ref::Direct(f.other))).unwrap();
    assert!(CheckoutRefGuard::acquire(&f.git_dir, &head, None).is_err());
    assert!(!f.git_dir.join("HEAD.lock").exists());
}

#[test]
fn held_checkout_guard_rejects_a_real_competing_writer() {
    let f = Fixture::new();
    let head = resolve_head(&f.git_dir).unwrap();
    let expected = Ref::Direct(f.other);
    let guard = CheckoutRefGuard::acquire(&f.git_dir, &head, Some(("refs/heads/other", Some(&expected)))).unwrap();
    let git_dir = f.git_dir.clone();
    let main = f.main;
    let other = f.other;
    let result = std::thread::spawn(move || {
        compare_and_swap_ref(&git_dir, "refs/heads/other", Some(&Ref::Direct(other)), Some(&Ref::Direct(main)))
    }).join().unwrap();
    assert!(result.is_err());
    guard.publish(&Ref::Symbolic("refs/heads/other".into()), None).unwrap();
    assert_eq!(read_ref_snapshot(&f.git_dir, "refs/heads/other").unwrap(), Some(expected));
    compare_and_swap_ref(&f.git_dir, "refs/heads/other", Some(&Ref::Direct(other)), Some(&Ref::Direct(main))).unwrap();
}

#[test]
#[cfg(unix)]
fn hooks_cannot_race_prepublication_and_post_checkout_edits_are_preserved() {
    let f = Fixture::new();
    let hooks = f.git_dir.join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    let helper = hooks.join("post-index-change");
    grit_test_support::compile_rust_fixture(&helper, HOOK).unwrap();
    fs::copy(&helper, hooks.join("post-checkout")).unwrap();
    let output = f.command(&["checkout", "-b", "created", "other"])
        .env("GITR_CHECKOUT_TEST_DIR", &f.git_dir)
        .env("GITR_CHECKOUT_TEST_EXPECTED", f.other.to_hex())
        .env("GITR_CHECKOUT_TEST_REPLACEMENT", f.main.to_hex())
        .output().unwrap();
    succeeds(output);
    assert_eq!(fs::read(f.git_dir.join("index-writer-blocked")).unwrap(), b"yes");
    assert_eq!(fs::read(f.git_dir.join("checkout-writer-published")).unwrap(), b"yes");
    assert_eq!(read_ref_snapshot(&f.git_dir, "refs/heads/created").unwrap(), Some(Ref::Direct(f.main)));
    assert_eq!(read_ref_snapshot(&f.git_dir, "HEAD").unwrap(), Some(Ref::Symbolic("refs/heads/main".into())));
}

#[cfg(unix)]
const HOOK: &str = r#"
use std::{fs, io::Write, path::Path};
fn publish(path: &Path, value: &str) {
    let lock = path.with_extension("lock");
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&lock).unwrap();
    file.write_all(value.as_bytes()).unwrap();
    file.sync_all().unwrap();
    drop(file);
    fs::rename(lock, path).unwrap();
}
fn main() {
    let dir = std::path::PathBuf::from(std::env::var_os("GITR_CHECKOUT_TEST_DIR").unwrap());
    let executable = std::env::args().next().unwrap();
    let before = Path::new(&executable).file_name().unwrap() == "post-index-change";
    if before {
        assert_eq!(fs::read_to_string(dir.join("HEAD")).unwrap(), "ref: refs/heads/main\n");
        for name in ["HEAD.lock", "refs/heads/main.lock", "refs/heads/created.lock"] {
            let error = fs::OpenOptions::new().write(true).create_new(true).open(dir.join(name)).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        }
        fs::write(dir.join("index-writer-blocked"), b"yes").unwrap();
    } else {
        assert_eq!(fs::read_to_string(dir.join("HEAD")).unwrap(), "ref: refs/heads/created\n");
        let expected = std::env::var("GITR_CHECKOUT_TEST_EXPECTED").unwrap();
        assert_eq!(fs::read_to_string(dir.join("refs/heads/created")).unwrap().trim(), expected);
        let replacement = std::env::var("GITR_CHECKOUT_TEST_REPLACEMENT").unwrap();
        publish(&dir.join("refs/heads/created"), &format!("{replacement}\n"));
        publish(&dir.join("HEAD"), "ref: refs/heads/main\n");
        fs::write(dir.join("checkout-writer-published"), b"yes").unwrap();
    }
}
"#;
