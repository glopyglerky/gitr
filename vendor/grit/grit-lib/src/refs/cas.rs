//! Ref publication with an expected value captured before computing an update.
//!
//! The legacy upstream write helpers do not provide compare-and-swap. These
//! entry points validate the caller's snapshot while holding the same lock that
//! protects publication. `None` means the ref must not exist, not "any value".

use super::{Error, ObjectId, Ref, Result};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Read the exact ref value without dereferencing symbolic refs. A missing ref
/// is distinct from a malformed ref or an I/O failure.
pub fn read_ref_snapshot(git_dir: &Path, refname: &str) -> Result<Option<Ref>> {
    super::ensure_refname_safe_for_storage(refname)?;
    if crate::reftable::is_reftable_repo(git_dir) && refname != "HEAD" {
        return crate::reftable::reftable_read_ref_snapshot(git_dir, refname);
    }
    let (store, name) = crate::worktree_ref::resolve_ref_storage(git_dir, refname);
    let name = crate::ref_namespace::storage_ref_name(&name);
    match super::read_ref_file(&store.join(&name)) {
        Ok(value) => Ok(Some(value)),
        Err(Error::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            Ok(super::lookup_packed_ref(&store, &name)?.map(Ref::Direct))
        }
        Err(error) => Err(error),
    }
}

fn mismatch(refname: &str) -> Error {
    Error::InvalidRef(format!(
        "cannot lock ref '{refname}': value changed since the operation started"
    ))
}

/// Compare the caller's snapshot and publish a new direct/symbolic value, or
/// delete the ref when `new_value` is `None`. The comparison and publication
/// share an exclusive ref lock (or reftable stack lock).
///
/// Capture `expected` before reading objects, running hooks, or computing the
/// update. Reading it immediately before this call does not protect that work.
pub fn compare_and_swap_ref(
    git_dir: &Path,
    refname: &str,
    expected: Option<&Ref>,
    new_value: Option<&Ref>,
) -> Result<()> {
    super::ensure_refname_safe_for_storage(refname)?;
    if let Some(Ref::Symbolic(target)) = new_value {
        super::ensure_refname_safe_for_storage(target)?;
    }
    if crate::reftable::is_reftable_repo(git_dir) && refname != "HEAD" {
        return crate::reftable::reftable_compare_and_swap_ref(
            git_dir, refname, expected, new_value,
        );
    }
    let (store, name) = crate::worktree_ref::resolve_ref_storage(git_dir, refname);
    let name = crate::ref_namespace::storage_ref_name(&name);
    let mut lock = RefLock::acquire(store.join(&name))?;
    if read_ref_snapshot(git_dir, refname)?.as_ref() != expected {
        return Err(mismatch(refname));
    }
    if let Some(value) = new_value {
        if super::packed_ref_namespace_conflict(&store, refname)? {
            return Err(Error::InvalidRef(format!(
                "cannot update ref '{refname}': reference namespace conflict"
            )));
        }
        lock.publish(value)
    } else {
        // Keep the ref lock while the existing deletion code removes both the
        // packed and loose representations, so a writer cannot be lost between
        // verification and unlink. It also preserves upstream reflog cleanup.
        super::delete_ref_at_storage(&store, &name)?;
        sync_directory(lock.target.parent())
    }
}

/// Publish a commit from the HEAD state captured before its contents were
/// computed. HEAD remains locked while the branch is checked and updated, so
/// a concurrent checkout cannot retarget the operation. An unborn branch must
/// still be absent. Detached HEAD uses the same exclusive file publication.
pub fn update_head_cas(
    git_dir: &Path,
    expected: &crate::state::HeadState,
    new_oid: &ObjectId,
) -> Result<()> {
    use crate::state::HeadState;
    let mut lock = RefLock::acquire(git_dir.join("HEAD"))?;
    let expected_head = match expected {
        HeadState::Branch { refname, .. } => Ref::Symbolic(refname.clone()),
        HeadState::Detached { oid } => Ref::Direct(*oid),
        HeadState::Invalid => return Err(mismatch("HEAD")),
    };
    if read_ref_snapshot(git_dir, "HEAD")?.as_ref() != Some(&expected_head) {
        return Err(mismatch("HEAD"));
    }
    let value = Ref::Direct(*new_oid);
    match expected {
        HeadState::Branch { refname, oid, .. } => {
            let previous = oid.map(Ref::Direct);
            compare_and_swap_ref(git_dir, refname, previous.as_ref(), Some(&value))
        }
        HeadState::Detached { .. } => lock.publish(&value),
        HeadState::Invalid => Err(mismatch("HEAD")),
    }
}

/// Holds the files-backend authority needed for a branch checkout. The caller
/// captures HEAD and the destination before resolving trees or changing the
/// index/worktree; acquisition verifies those snapshots under exclusive locks.
///
/// This serializes cooperating ref writers, not arbitrary filesystem writes.
/// New branch and HEAD publication still use separate durable file renames;
/// this is not an atomic transaction over refs, index and worktree contents.
pub struct CheckoutRefGuard {
    git_dir: PathBuf,
    expected_head: Ref,
    head: RefLock,
    branches: Vec<(String, Option<Ref>, RefLock)>,
}

impl CheckoutRefGuard {
    pub fn acquire(
        git_dir: &Path,
        expected_head: &crate::state::HeadState,
        destination: Option<(&str, Option<&Ref>)>,
    ) -> Result<Self> {
        use crate::state::HeadState;
        if crate::reftable::is_reftable_repo(git_dir) {
            return Err(Error::InvalidRef(
                "checkout with reftable refs is unavailable until its transaction is qualified".into(),
            ));
        }
        let mut expected_branches = std::collections::BTreeMap::new();
        let raw_head = match expected_head {
            HeadState::Branch { refname, oid, .. } => {
                super::ensure_refname_safe_for_storage(refname)?;
                expected_branches.insert(refname.clone(), oid.map(Ref::Direct));
                Ref::Symbolic(refname.clone())
            }
            HeadState::Detached { oid } => Ref::Direct(*oid),
            HeadState::Invalid => return Err(mismatch("HEAD")),
        };
        if let Some((name, expected)) = destination {
            super::ensure_refname_safe_for_storage(name)?;
            if !name.starts_with("refs/heads/") || matches!(expected, Some(Ref::Symbolic(_))) {
                return Err(Error::InvalidRef(
                    "checkout requires a direct local branch reference".into(),
                ));
            }
            if let Some(previous) = expected_branches.get(name) {
                if previous.as_ref() != expected {
                    return Err(mismatch(name));
                }
            }
            expected_branches.insert(name.to_owned(), expected.cloned());
        }

        // Commit publication acquires HEAD before its branch. Use the same
        // order, then sort branch locks so overlapping checkouts are consistent.
        let head = RefLock::acquire(git_dir.join("HEAD"))?;
        if read_ref_snapshot(git_dir, "HEAD")?.as_ref() != Some(&raw_head) {
            return Err(mismatch("HEAD"));
        }
        let mut branches = Vec::new();
        for (name, expected) in expected_branches {
            let (store, stored_name) = crate::worktree_ref::resolve_ref_storage(git_dir, &name);
            let stored_name = crate::ref_namespace::storage_ref_name(&stored_name);
            let lock = RefLock::acquire(store.join(stored_name))?;
            if read_ref_snapshot(git_dir, &name)?.as_ref() != expected.as_ref() {
                return Err(mismatch(&name));
            }
            if super::packed_ref_namespace_conflict(&store, &name)? {
                return Err(Error::InvalidRef(format!(
                    "cannot update ref '{name}': reference namespace conflict"
                )));
            }
            branches.push((name, expected, lock));
        }
        Ok(Self { git_dir: git_dir.to_owned(), expected_head: raw_head, head, branches })
    }

    /// Publish HEAD, optionally creating one previously absent destination
    /// branch. All lockfiles are already owned; no lock is reacquired between
    /// validation and publication. Consume the guard before running hooks.
    pub fn publish(mut self, new_head: &Ref, new_branch: Option<(&str, &ObjectId)>) -> Result<()> {
        if let Ref::Symbolic(name) = new_head {
            if !self.branches.iter().any(|(locked, _, _)| locked == name) {
                return Err(Error::InvalidRef("checkout destination was not locked".into()));
            }
        }
        if read_ref_snapshot(&self.git_dir, "HEAD")?.as_ref() != Some(&self.expected_head) {
            return Err(mismatch("HEAD"));
        }
        for (name, expected, _) in &self.branches {
            if read_ref_snapshot(&self.git_dir, name)?.as_ref() != expected.as_ref() {
                return Err(mismatch(name));
            }
        }
        let branch_index = if let Some((name, _)) = new_branch {
            let index = self.branches.iter().position(|(locked, expected, _)| {
                locked == name && expected.is_none()
            }).ok_or_else(|| Error::InvalidRef("checkout new branch was not locked as absent".into()))?;
            if new_head != &Ref::Symbolic(name.to_owned()) {
                return Err(Error::InvalidRef("checkout new branch does not match HEAD".into()));
            }
            Some(index)
        } else {
            None
        };
        // Prepare both files first. Ordinary write/sync failures therefore
        // leave both original refs intact. Rename failures can still occur
        // between publications; files refs have no multi-file atomic rename.
        self.head.prepare(new_head)?;
        if let (Some(index), Some((_, oid))) = (branch_index, new_branch) {
            self.branches[index].2.prepare(&Ref::Direct(*oid))?;
            self.branches[index].2.commit()?;
        }
        self.head.commit()
    }
}

struct RefLock {
    path: PathBuf,
    target: PathBuf,
    file: Option<fs::File>,
    owned: bool,
}

impl RefLock {
    fn acquire(target: PathBuf) -> Result<Self> {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        super::remove_empty_ref_directory(&target);
        let path = super::lock_path_for_ref(&target);
        let file = fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        Ok(Self { path, target, file: Some(file), owned: true })
    }

    fn publish(&mut self, value: &Ref) -> Result<()> {
        self.prepare(value)?;
        self.commit()
    }

    fn prepare(&mut self, value: &Ref) -> Result<()> {
        let content = match value {
            Ref::Direct(oid) => format!("{oid}\n"),
            Ref::Symbolic(target) => format!("ref: {target}\n"),
        };
        if let Some(file) = self.file.as_mut() {
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
        } else {
            return Err(Error::InvalidRef("ref lock was already consumed".to_owned()));
        }
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        // Close before rename for Windows. Drop still cleans up after a failed
        // write/rename; it never removes another writer's lock after success.
        drop(self.file.take());
        fs::rename(&self.path, &self.target)?;
        self.owned = false;
        sync_directory(self.target.parent())
    }
}

impl Drop for RefLock {
    fn drop(&mut self) {
        drop(self.file.take());
        if self.owned {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn sync_directory(directory: Option<&Path>) -> Result<()> {
    #[cfg(unix)]
    if let Some(directory) = directory {
        fs::File::open(directory)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::sync::{Arc, Barrier};

    fn oid(byte: u8) -> ObjectId {
        ObjectId::from_bytes(&[byte; 20]).unwrap()
    }

    fn repository(reftable: bool) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("refs/heads")).unwrap();
        fs::write(dir.path().join("HEAD"), b"ref: refs/heads/main\n").unwrap();
        if reftable {
            fs::write(dir.path().join("config"), b"[extensions]\nrefStorage = reftable\n").unwrap();
            fs::create_dir(dir.path().join("reftable")).unwrap();
            fs::write(dir.path().join("reftable/tables.list"), b"").unwrap();
        }
        dir
    }

    #[test]
    fn snapshots_reject_stale_updates_and_creation_for_both_backends() {
        for reftable in [false, true] {
            let dir = repository(reftable);
            let first = Ref::Direct(oid(1));
            let next = Ref::Direct(oid(2));
            let stale = Ref::Direct(oid(3));
            compare_and_swap_ref(dir.path(), "refs/heads/main", None, Some(&first)).unwrap();
            assert!(compare_and_swap_ref(dir.path(), "refs/heads/main", None, Some(&next)).is_err());
            compare_and_swap_ref(dir.path(), "refs/heads/main", Some(&first), Some(&next)).unwrap();
            assert!(compare_and_swap_ref(dir.path(), "refs/heads/main", Some(&first), Some(&stale)).is_err());
            assert_eq!(read_ref_snapshot(dir.path(), "refs/heads/main").unwrap(), Some(next));
            assert!(!dir.path().join("refs/heads/main.lock").exists());
            assert!(!dir.path().join("reftable/tables.list.lock").exists());
        }
    }

    #[test]
    fn two_writers_from_one_snapshot_cannot_both_publish() {
        for reftable in [false, true] {
            let dir = repository(reftable);
            let first = Ref::Direct(oid(1));
            compare_and_swap_ref(dir.path(), "refs/heads/main", None, Some(&first)).unwrap();
            let barrier = Arc::new(Barrier::new(3));
            let mut writers = Vec::new();
            for byte in [2, 3] {
                let path = dir.path().to_path_buf();
                let barrier = Arc::clone(&barrier);
                let expected = first.clone();
                writers.push(std::thread::spawn(move || {
                    barrier.wait();
                    compare_and_swap_ref(&path, "refs/heads/main", Some(&expected), Some(&Ref::Direct(oid(byte)))).is_ok()
                }));
            }
            barrier.wait();
            let successes = writers.into_iter().map(|writer| writer.join().unwrap()).filter(|success| *success).count();
            assert_eq!(successes, 1);
        }
    }

    #[test]
    fn head_commit_checks_unborn_detached_and_retargeted_state() {
        for reftable in [false, true] {
            let dir = repository(reftable);
            let unborn = crate::state::resolve_head(dir.path()).unwrap();
            update_head_cas(dir.path(), &unborn, &oid(1)).unwrap();
            assert!(update_head_cas(dir.path(), &unborn, &oid(2)).is_err());
            let head = crate::state::resolve_head(dir.path()).unwrap();
            let symbolic = Ref::Symbolic("refs/heads/main".to_owned());
            compare_and_swap_ref(dir.path(), "HEAD", Some(&symbolic), Some(&Ref::Direct(oid(1)))).unwrap();
            assert!(update_head_cas(dir.path(), &head, &oid(2)).is_err());
            let detached = crate::state::resolve_head(dir.path()).unwrap();
            update_head_cas(dir.path(), &detached, &oid(2)).unwrap();
            assert!(update_head_cas(dir.path(), &detached, &oid(3)).is_err());
            assert_eq!(read_ref_snapshot(dir.path(), "HEAD").unwrap(), Some(Ref::Direct(oid(2))));
            assert!(!dir.path().join("HEAD.lock").exists());
        }
    }

    #[test]
    fn a_held_lock_and_stale_delete_leave_both_ref_representations_intact() {
        let dir = repository(false);
        let first = Ref::Direct(oid(1));
        let next = Ref::Direct(oid(2));
        fs::write(dir.path().join("packed-refs"), format!("{} refs/heads/main\n", oid(1))).unwrap();
        compare_and_swap_ref(dir.path(), "refs/heads/main", Some(&first), Some(&next)).unwrap();
        assert!(compare_and_swap_ref(dir.path(), "refs/heads/main", Some(&first), None).is_err());
        assert_eq!(read_ref_snapshot(dir.path(), "refs/heads/main").unwrap(), Some(next.clone()));
        let lock_path = dir.path().join("refs/heads/main.lock");
        fs::write(&lock_path, b"another writer").unwrap();
        assert!(compare_and_swap_ref(dir.path(), "refs/heads/main", Some(&next), None).is_err());
        assert_eq!(fs::read(&lock_path).unwrap(), b"another writer");
        fs::remove_file(&lock_path).unwrap();
        compare_and_swap_ref(dir.path(), "refs/heads/main", Some(&next), None).unwrap();
        assert_eq!(read_ref_snapshot(dir.path(), "refs/heads/main").unwrap(), None);
        assert!(!fs::read_to_string(dir.path().join("packed-refs")).unwrap().contains("refs/heads/main"));
    }
}
