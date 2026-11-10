//! Product byte-path preflight for the supported worktree writers.
//!
//! This reads the worktree and indexes before dispatch. It is not a filesystem
//! transaction: concurrent path replacement can invalidate an earlier check.
use anyhow::{bail, Context, Result};
use grit_lib::index::{Index, MODE_SYMLINK};
use grit_lib::objects::{parse_tree, ObjectId, ObjectKind};
use grit_lib::odb::Odb;
use grit_lib::repo::Repository;
use std::fs;
use std::io::Read;
use std::path::Path;

const MAX_ENTRIES: usize = 100_000;
const MAX_DEPTH: usize = 128;
const MAX_PATH_BYTES: usize = 16 * 1024 * 1024;
const MAX_INDEX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Default)]
struct Budget { entries: usize, bytes: usize }

impl Budget {
    fn path(&mut self, path: &str, depth: usize) -> Result<()> {
        self.entries = self.entries.checked_add(1).context("path count overflow")?;
        self.bytes = self.bytes.checked_add(path.len()).context("path byte count overflow")?;
        if self.entries > MAX_ENTRIES || depth > MAX_DEPTH || self.bytes > MAX_PATH_BYTES {
            bail!("path safety inspection exceeds its 100000-entry, 128-depth or 16 MiB path limit");
        }
        Ok(())
    }
}

pub(crate) fn utf8_bytes<'a>(bytes: &'a [u8], context: &str) -> Result<&'a str> {
    std::str::from_utf8(bytes).with_context(|| format!("{context}: non-UTF-8 paths or symlink targets are not supported"))
}

fn utf8_path<'a>(path: &'a Path, context: &str) -> Result<&'a str> {
    path.to_str().with_context(|| format!("{context}: non-UTF-8 paths or symlink targets are not supported"))
}

pub(crate) fn check(command: &str, args: &[String]) -> Result<()> {
    if !crate::write_safety::is_product() || !matches!(command, "add" | "rm" | "mv" | "restore" | "commit" | "checkout" | "switch") {
        return Ok(());
    }
    if args.len() == 1 && matches!(args[0].as_str(), "-h" | "--help" | "--help-all") {
        return Ok(());
    }
    for name in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY"] {
        if let Some(value) = std::env::var_os(name) {
            utf8_path(Path::new(&value), name)?;
        }
    }
    utf8_path(&std::env::current_dir()?, "current directory")?;
    let repo = Repository::discover(None).context("not a git repository")?;
    utf8_path(&repo.git_dir, "repository directory")?;
    let effective_index = repo.index_path_for_env()?;
    check_index_file(&repo, &effective_index, &mut Budget::default(), 0)?;
    if effective_index != repo.index_path() {
        check_index_file(&repo, &repo.index_path(), &mut Budget::default(), 0)?;
    }
    if let Some(worktree) = &repo.work_tree {
        check_worktree(worktree)?;
    }
    Ok(())
}

fn check_index_file(repo: &Repository, path: &Path, budget: &mut Budget, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH { bail!("split-index nesting exceeds path inspection limit"); }
    utf8_path(path, "index filename")?;
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_INDEX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_INDEX_BYTES { bail!("index exceeds the 64 MiB path inspection limit"); }
    if bytes.is_empty() { return Ok(()); }
    let index = Index::parse_with_algo(&bytes, repo.odb.hash_algo())?;
    check_index(&index, &repo.odb, budget)?;
    if let Some(base) = index.split_index_base_oid() {
        check_index_file(repo, &repo.git_dir.join(format!("sharedindex.{base}")), budget, depth + 1)?;
    }
    Ok(())
}

fn check_index(index: &Index, odb: &Odb, budget: &mut Budget) -> Result<()> {
    for entry in &index.entries {
        let path = utf8_bytes(&entry.path, "index entry")?;
        budget.path(path, path.split('/').count())?;
        if entry.mode == MODE_SYMLINK { check_link_object(odb, &entry.oid)?; }
        if entry.mode == 0o040000 {
            check_tree(odb, entry.oid, path.trim_end_matches('/'), budget)?;
        }
    }
    if let Some(records) = &index.resolve_undo {
        for (path, record) in records {
            let path = utf8_bytes(path, "resolve-undo index entry")?;
            budget.path(path, path.split('/').count())?;
            for (mode, oid) in record.modes.iter().zip(&record.oids) {
                if *mode == MODE_SYMLINK { check_link_object(odb, oid)?; }
            }
        }
    }
    Ok(())
}

fn check_link_object(odb: &Odb, oid: &ObjectId) -> Result<()> {
    let object = odb.read(oid)?;
    if object.kind != ObjectKind::Blob { bail!("symlink target is not a blob"); }
    utf8_bytes(&object.data, "indexed or tree symlink target")?;
    Ok(())
}

fn check_worktree(worktree: &Path) -> Result<()> {
    utf8_path(worktree, "worktree directory")?;
    let mut budget = Budget::default();
    let mut directories = vec![(worktree.to_path_buf(), 0usize)];
    while let Some((directory, depth)) = directories.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let name = entry.file_name();
            if name == ".git" { continue; }
            name.to_str().context("worktree entry has a non-UTF-8 filename")?;
            let path = entry.path();
            let relative = path.strip_prefix(worktree).context("worktree path escaped its root")?;
            budget.path(utf8_path(relative, "worktree entry")?, depth + 1)?;
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                utf8_path(&fs::read_link(&path)?, "worktree symlink target")?;
            } else if metadata.is_dir() {
                directories.push((path, depth + 1));
            }
        }
    }
    Ok(())
}

pub(crate) fn check_restore_tree(repo: &Repository, tree: ObjectId) -> Result<()> {
    if crate::write_safety::is_product() {
        check_tree(&repo.odb, tree, "", &mut Budget::default())?;
    }
    Ok(())
}

fn check_tree(odb: &Odb, tree: ObjectId, prefix: &str, budget: &mut Budget) -> Result<()> {
    let mut trees = vec![(tree, prefix.to_owned())];
    while let Some((oid, prefix)) = trees.pop() {
        let object = odb.read(&oid)?;
        if object.kind != ObjectKind::Tree { bail!("restore source is not a tree"); }
        for entry in parse_tree(&object.data)? {
            let name = utf8_bytes(&entry.name, "source tree entry")?;
            let path = if prefix.is_empty() { name.to_owned() } else { format!("{prefix}/{name}") };
            budget.path(&path, path.split('/').count())?;
            if entry.mode == 0o040000 { trees.push((entry.oid, path)); }
            else if entry.mode == MODE_SYMLINK { check_link_object(odb, &entry.oid)?; }
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn worktree_guard_rejects_byte_names_and_targets_without_changing_files() {
        let temp = tempfile::tempdir().unwrap();
        let bad = temp.path().join(OsString::from_vec(b"bad-\xff".to_vec()));
        fs::write(&bad, b"preserve").unwrap();
        assert!(check_worktree(temp.path()).unwrap_err().to_string().contains("non-UTF-8"));
        assert_eq!(fs::read(&bad).unwrap(), b"preserve");
        fs::remove_file(&bad).unwrap();
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(OsString::from_vec(b"target-\xff".to_vec()), &link).unwrap();
        assert!(check_worktree(temp.path()).unwrap_err().to_string().contains("non-UTF-8"));
        assert_eq!(fs::read_link(link).unwrap(), PathBuf::from(OsString::from_vec(b"target-\xff".to_vec())));
    }

    #[test]
    fn worktree_guard_skips_git_internals_and_does_not_follow_links() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let invalid = OsString::from_vec(vec![0xff]);
        fs::write(outside.path().join(&invalid), b"outside").unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        fs::write(temp.path().join(".git").join(&invalid), b"internal").unwrap();
        std::os::unix::fs::symlink(outside.path(), temp.path().join("external")).unwrap();
        check_worktree(temp.path()).unwrap();
    }

    #[test]
    fn tree_preflight_rejects_byte_names_and_symlink_payloads() {
        let temp = tempfile::tempdir().unwrap();
        let odb = Odb::new(&temp.path().join("objects"));
        let blob = odb.write_loose_materialize(ObjectKind::Blob, b"value").unwrap();
        let mut raw = b"100644 bad-\xff\0".to_vec();
        raw.extend_from_slice(blob.as_bytes());
        let tree = odb.write_loose_materialize(ObjectKind::Tree, &raw).unwrap();
        assert!(check_tree(&odb, tree, "", &mut Budget::default()).unwrap_err().to_string().contains("non-UTF-8"));
        let target = odb.write_loose_materialize(ObjectKind::Blob, b"target-\xff").unwrap();
        let mut raw = b"120000 link\0".to_vec();
        raw.extend_from_slice(target.as_bytes());
        let tree = odb.write_loose_materialize(ObjectKind::Tree, &raw).unwrap();
        assert!(check_tree(&odb, tree, "", &mut Budget::default()).unwrap_err().to_string().contains("non-UTF-8"));
    }

    #[test]
    fn index_guard_rejects_byte_paths_and_symlink_targets() {
        let temp = tempfile::tempdir().unwrap();
        let odb = Odb::new(&temp.path().join("objects"));
        let oid = odb.write_loose_materialize(ObjectKind::Blob, b"target-\xff").unwrap();
        let mut index = Index::new();
        index.entries.push(grit_lib::index::IndexEntry {
            ctime_sec: 0, ctime_nsec: 0, mtime_sec: 0, mtime_nsec: 0,
            dev: 0, ino: 0, mode: 0o100644, uid: 0, gid: 0, size: 0,
            oid, flags: 0, flags_extended: None, path: b"bad-\xff".to_vec(),
            base_index_pos: 0,
        });
        assert!(check_index(&index, &odb, &mut Budget::default()).unwrap_err().to_string().contains("non-UTF-8"));
        index.entries[0].path = b"link".to_vec();
        index.entries[0].mode = MODE_SYMLINK;
        assert!(check_index(&index, &odb, &mut Budget::default()).unwrap_err().to_string().contains("non-UTF-8"));
    }

    #[test]
    fn path_budget_rejects_deep_or_excessive_walks() {
        assert!(Budget::default().path("file", MAX_DEPTH + 1).is_err());
        assert!(Budget { entries: MAX_ENTRIES, bytes: 0 }.path("file", 1).is_err());
        assert!(Budget { entries: 0, bytes: MAX_PATH_BYTES }.path("file", 1).is_err());
    }
}
