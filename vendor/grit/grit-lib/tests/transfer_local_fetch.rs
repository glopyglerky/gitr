//! Integration tests for `grit_lib::transfer::fetch_local` — the in-process
//! local / `file://` fetch path (no subprocess, no wire protocol).
//!
//! A remote repo is built with the system `git`; an empty local repo is
//! initialized; `fetch_local` copies refs + the minimal object set into the
//! local odb. We assert the tracking refs, the presence of the transferred
//! objects, the per-ref `UpdateMode`, and the resolved default branch — then
//! repeat after advancing the remote to assert a `FastForward`.

use std::path::Path;

use grit_lib::objects::{ObjectId, ObjectKind};
use grit_lib::odb::Odb;
use grit_lib::refs::resolve_ref;
use grit_lib::transfer::{fetch_local, FetchOptions, TagMode, UpdateMode};
use grit_test_support::git;

fn rev_parse(dir: &Path, rev: &str) -> ObjectId {
    ObjectId::from_hex(git(dir, &["rev-parse", rev]).trim()).expect("valid oid")
}

fn open_odb(git_dir: &Path) -> Odb {
    Odb::new(&git_dir.join("objects")).with_config_git_dir(git_dir.to_path_buf())
}

/// Build a remote with two commits on `main` plus an annotated tag.
fn build_remote(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main", "."]);
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    git(dir, &["add", "a.txt"]);
    git(dir, &["commit", "-q", "-m", "c1"]);
    std::fs::write(dir.join("b.txt"), "two\n").unwrap();
    git(dir, &["add", "b.txt"]);
    git(dir, &["commit", "-q", "-m", "c2"]);
    git(dir, &["tag", "-a", "v1", "-m", "release one"]);
}

#[test]
fn fetch_local_copies_refs_and_objects_then_fast_forwards() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let remote = tmp.path().join("remote");
    let local = tmp.path().join("local");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&local).unwrap();

    build_remote(&remote);
    // Bare-style git_dir for both is just the `.git` directory.
    let remote_git = remote.join(".git");

    // Empty local repo.
    git(&local, &["init", "-q", "-b", "main", "."]);
    let local_git = local.join(".git");

    let remote_main = rev_parse(&remote, "refs/heads/main");
    let remote_c1 = rev_parse(&remote, "HEAD~1");
    let remote_v1 = rev_parse(&remote, "refs/tags/v1");

    // --- First fetch: everything is New. ---
    let opts = FetchOptions {
        refspecs: vec!["+refs/heads/*:refs/remotes/origin/*".to_owned()],
        tags: TagMode::All,
        ..Default::default()
    };
    let outcome = fetch_local(&local_git, &remote_git, &opts).expect("fetch_local");

    // default_branch from remote HEAD symref.
    assert_eq!(outcome.default_branch.as_deref(), Some("main"));

    // Tracking ref written and equals the remote tip.
    let tracked = resolve_ref(&local_git, "refs/remotes/origin/main").expect("tracking ref");
    assert_eq!(tracked, remote_main);

    // Objects present locally: read them back from the local odb.
    let local_odb = open_odb(&local_git);
    let c2 = local_odb.read(&remote_main).expect("commit c2 present");
    assert_eq!(c2.kind, ObjectKind::Commit);
    assert!(local_odb.exists(&remote_c1), "parent commit c1 present");
    // The b.txt blob introduced by c2 must be present (proves tree+blob copied).
    let b_blob = rev_parse(&remote, "HEAD:b.txt");
    let blob = local_odb.read(&b_blob).expect("b.txt blob present");
    assert_eq!(blob.kind, ObjectKind::Blob);
    assert_eq!(blob.data, b"two\n");

    // The annotated tag object and its tracking ref are present.
    assert!(local_odb.exists(&remote_v1), "tag object present");
    assert_eq!(
        resolve_ref(&local_git, "refs/tags/v1").expect("tag ref"),
        remote_v1
    );

    // Every head/tag update was New.
    let head_update = outcome
        .updates
        .iter()
        .find(|u| u.remote_ref == "refs/heads/main")
        .expect("main update present");
    assert_eq!(head_update.mode, UpdateMode::New);
    assert_eq!(head_update.new_oid, Some(remote_main));
    assert!(head_update.old_oid.is_none());
    for u in &outcome.updates {
        if u.mode == UpdateMode::DeletedMissing {
            continue;
        }
        assert_eq!(u.mode, UpdateMode::New, "ref {} not New", u.remote_ref);
    }

    // --- Advance the remote, fetch again: FastForward. ---
    std::fs::write(remote.join("c.txt"), "three\n").unwrap();
    git(&remote, &["add", "c.txt"]);
    git(&remote, &["commit", "-q", "-m", "c3"]);
    let remote_main2 = rev_parse(&remote, "refs/heads/main");
    let c_blob = rev_parse(&remote, "HEAD:c.txt");
    assert_ne!(remote_main2, remote_main);

    let outcome2 = fetch_local(&local_git, &remote_git, &opts).expect("second fetch");

    let head_update2 = outcome2
        .updates
        .iter()
        .find(|u| u.remote_ref == "refs/heads/main")
        .expect("main update present (2)");
    assert_eq!(head_update2.mode, UpdateMode::FastForward);
    assert_eq!(head_update2.old_oid, Some(remote_main));
    assert_eq!(head_update2.new_oid, Some(remote_main2));

    // Tracking ref advanced and the new objects arrived.
    assert_eq!(
        resolve_ref(&local_git, "refs/remotes/origin/main").unwrap(),
        remote_main2
    );
    let local_odb = open_odb(&local_git);
    assert!(local_odb.exists(&remote_main2), "new commit present");
    let c_blob_obj = local_odb.read(&c_blob).expect("c.txt blob present");
    assert_eq!(c_blob_obj.data, b"three\n");

    // The tag did not move, so it is UpToDate on the second fetch.
    let tag_update2 = outcome2
        .updates
        .iter()
        .find(|u| u.remote_ref == "refs/tags/v1");
    if let Some(u) = tag_update2 {
        assert_eq!(u.mode, UpdateMode::UpToDate);
    }
}
