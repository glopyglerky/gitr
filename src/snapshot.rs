// SPDX-License-Identifier: MIT OR Apache-2.0
//! Immutable, local reads of a committed Git tree without a checkout.
//!
//! A snapshot resolves a ref once and retains the resulting commit and tree IDs.
//! Paths are repository-relative bytes; symlinks are read as their stored target
//! bytes, and submodules are never entered. Listing and stat read tree metadata,
//! not the blobs referenced by that metadata. Objects are read only from the
//! existing Git object database, with replacement refs disabled and their hashes
//! checked before use.
//!
//! This is not a mount or a garbage-collection lease. If an object is unavailable
//! locally, including in a partial clone or after collection, the read fails.
//! The current object reader materializes a whole object. The size checks below
//! bound accepted content, not the reader's peak memory on corrupt compressed
//! input or delta reconstruction.

use std::{cmp::Ordering, collections::HashSet, path::Path};

use sley::{
    FullName, GitError, GitObjectType, LoadedObject, ObjectFormat, ObjectId, OpenOptions,
    ReferenceTarget, Repository, ResourceLimitKind, Result, TagObject, TreeObject,
};

/// Default maximum uncompressed blob size accepted by a snapshot read.
pub const DEFAULT_MAX_BLOB_BYTES: u64 = 8 * 1024 * 1024;
/// Hard maximum uncompressed blob size accepted by this API.
pub const MAX_BLOB_BYTES: u64 = 64 * 1024 * 1024;
/// Maximum uncompressed size of each commit, tag, or tree read.
pub const MAX_METADATA_BYTES: u64 = 8 * 1024 * 1024;
/// Maximum byte length of a repository-relative path or revision name.
pub const MAX_PATH_BYTES: usize = 64 * 1024;
/// Maximum number of components in a repository-relative path.
pub const MAX_PATH_DEPTH: usize = 256;
/// Maximum number of entries in one directory, not a recursive total.
pub const MAX_TREE_ENTRIES: usize = 100_000;

const TREE_MODE: u32 = 0o040000;
const MAX_REFERENCE_DEPTH: usize = 32;
const MAX_TAG_DEPTH: usize = 32;

/// One entry in a committed tree. `name` is a raw basename, empty for the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotEntry {
    pub name: Vec<u8>,
    pub mode: u32,
    pub oid: ObjectId,
}

/// A verified blob body retained by the object database's shared allocation.
#[derive(Debug, Clone)]
pub struct SnapshotBlob {
    entry: SnapshotEntry,
    object: LoadedObject,
}

impl SnapshotBlob {
    /// The exact committed bytes, including symlink targets and binary content.
    pub fn bytes(&self) -> &[u8] {
        &self.object.encoded().body
    }

    pub fn entry(&self) -> &SnapshotEntry {
        &self.entry
    }
}

/// A read-only view of one immutable commit's tree in an existing repository.
#[derive(Debug, Clone)]
pub struct Snapshot {
    repository: Repository,
    commit: ObjectId,
    tree: ObjectId,
}

impl Snapshot {
    /// Resolve a commit, verify it and its root tree, and keep its immutable IDs.
    ///
    /// `revision` accepts HEAD, a full ref name, an unambiguous short ref name,
    /// a full object ID, or a unique hexadecimal prefix of at least four digits.
    /// Annotated tags are verified and peeled to a commit. Revision expressions
    /// such as `HEAD~1`, `tag^{commit}`, reflog selectors, and `rev:path` are not
    /// accepted. Symbolic refs and tag chains are limited to 32 hops each.
    /// `GIT_DIR` and `GIT_WORK_TREE` do not redirect repository discovery.
    /// Storage overrides (`GIT_COMMON_DIR`, `GIT_OBJECT_DIRECTORY`,
    /// `GIT_ALTERNATE_OBJECT_DIRECTORIES`, and `GIT_REFERENCE_BACKEND`) must be
    /// unset; the underlying readers do not yet provide an isolated environment
    /// policy for those variables.
    /// Replacement refs cannot change the meaning of an object ID.
    pub fn open(path: &Path, revision: &str) -> Result<Self> {
        let repository = open_repository(path)?;
        let oid = resolve_revision(&repository, revision)?;
        let (commit, tree) = resolve_commit(&repository, oid)?;
        let snapshot = Self {
            repository,
            commit,
            tree,
        };
        // Establish that the selected commit has a valid, available root. Child
        // trees and file payloads remain lazy.
        snapshot.read_tree(tree)?;
        Ok(snapshot)
    }

    pub fn commit_id(&self) -> ObjectId {
        self.commit
    }

    pub fn tree_id(&self) -> ObjectId {
        self.tree
    }

    pub fn object_format(&self) -> ObjectFormat {
        self.repository.object_format()
    }

    /// List one directory in Git tree order without reading its file payloads.
    /// An empty path selects the root. This method never recurses.
    pub fn list(&self, path: &[u8]) -> Result<Vec<SnapshotEntry>> {
        let entry = self.stat(path)?;
        require_directory(&entry)?;
        self.read_tree(entry.oid)
    }

    /// Read entry metadata without loading the entry's blob or submodule commit.
    /// An empty path returns the root tree entry, whose name is empty.
    pub fn stat(&self, path: &[u8]) -> Result<SnapshotEntry> {
        let components = path_components(path)?;
        let mut entry = SnapshotEntry {
            name: Vec::new(),
            mode: TREE_MODE,
            oid: self.tree,
        };
        for component in components {
            require_directory(&entry)?;
            entry = self
                .read_tree(entry.oid)?
                .into_iter()
                .find(|candidate| candidate.name == component)
                .ok_or_else(|| GitError::not_found("path in snapshot tree"))?;
        }
        Ok(entry)
    }

    /// Read one regular file, executable, or symlink payload and verify its ID.
    ///
    /// Symlink payloads are returned unchanged; they are never followed. Trees
    /// and gitlinks are not readable as files. `max_bytes` may be zero for empty
    /// files but must not exceed [`MAX_BLOB_BYTES`]. No content conversion,
    /// checkout filters, network fetch, or working-tree reads are performed.
    pub fn read(&self, path: &[u8], max_bytes: u64) -> Result<SnapshotBlob> {
        if max_bytes > MAX_BLOB_BYTES {
            return Err(GitError::InvalidFormat(format!(
                "snapshot read limit must not exceed {MAX_BLOB_BYTES} bytes"
            )));
        }
        let entry = self.stat(path)?;
        if !matches!(entry.mode, 0o100644 | 0o100755 | 0o120000) {
            return Err(GitError::InvalidPath(
                "snapshot read requires a file or symlink; directories and gitlinks are not files"
                    .into(),
            ));
        }
        let object = read_verified(
            &self.repository,
            entry.oid,
            &[GitObjectType::Blob],
            max_bytes,
        )?;
        Ok(SnapshotBlob { entry, object })
    }

    fn read_tree(&self, oid: ObjectId) -> Result<Vec<SnapshotEntry>> {
        let object = read_verified(
            &self.repository,
            oid,
            &[GitObjectType::Tree],
            MAX_METADATA_BYTES,
        )?;
        let tree = TreeObject::parse(self.object_format(), &object.encoded().body)?;
        if tree.entries.len() > MAX_TREE_ENTRIES {
            return Err(GitError::InvalidObject(format!(
                "snapshot tree {oid} exceeds the limit of {MAX_TREE_ENTRIES} entries"
            )));
        }
        let mut names = HashSet::with_capacity(tree.entries.len());
        for (index, entry) in tree.entries.iter().enumerate() {
            let name = entry.name.as_bytes();
            if name.is_empty()
                || name.len() > MAX_PATH_BYTES
                || name.contains(&0)
                || name.contains(&b'/')
                || name == b"."
                || name == b".."
            {
                return Err(GitError::InvalidObject(format!(
                    "snapshot tree {oid} contains an invalid entry name"
                )));
            }
            if !matches!(
                entry.mode,
                TREE_MODE | 0o100644 | 0o100755 | 0o120000 | 0o160000
            ) {
                return Err(GitError::InvalidObject(format!(
                    "snapshot tree {oid} contains unsupported mode {:o}",
                    entry.mode
                )));
            }
            if !names.insert(name) {
                return Err(GitError::InvalidObject(format!(
                    "snapshot tree {oid} contains duplicate entry names"
                )));
            }
            if let Some(previous) = index.checked_sub(1).map(|index| &tree.entries[index])
                && tree_name_cmp(previous.name.as_bytes(), previous.mode, name, entry.mode)
                    != Ordering::Less
            {
                return Err(GitError::InvalidObject(format!(
                    "snapshot tree {oid} is not in canonical Git tree order"
                )));
            }
        }
        // Parsing a mode as an integer also accepts alternate spellings. Only
        // canonical Git encodings are accepted by this source view.
        if tree.write() != object.encoded().body {
            return Err(GitError::InvalidObject(format!(
                "snapshot tree {oid} uses a noncanonical entry encoding"
            )));
        }
        Ok(tree
            .entries
            .into_iter()
            .map(|entry| SnapshotEntry {
                name: entry.name.into_bytes(),
                mode: entry.mode,
                oid: entry.oid,
            })
            .collect())
    }
}

// Catalog operations use the same explicit repository and storage policy, even
// before the first commit exists. Keep environment checks in one place.
pub(crate) fn open_repository(path: &Path) -> Result<Repository> {
    for variable in [
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_REFERENCE_BACKEND",
    ] {
        if std::env::var_os(variable).is_some() {
            return Err(GitError::InvalidFormat(format!(
                "snapshot reads require {variable} to be unset"
            )));
        }
    }
    Repository::open_with(
        path,
        OpenOptions::new()
            .respect_environment(false)
            .replace_objects(false),
    )
}

fn path_components(path: &[u8]) -> Result<Vec<&[u8]>> {
    if path.len() > MAX_PATH_BYTES {
        return Err(GitError::InvalidPath(format!(
            "snapshot path exceeds {MAX_PATH_BYTES} bytes"
        )));
    }
    if path.is_empty() {
        return Ok(Vec::new());
    }
    if path.contains(&0) {
        return Err(GitError::InvalidPath(
            "snapshot paths cannot contain NUL bytes".into(),
        ));
    }
    let components: Vec<_> = path.split(|byte| *byte == b'/').collect();
    if components.len() > MAX_PATH_DEPTH {
        return Err(GitError::InvalidPath(format!(
            "snapshot path exceeds {MAX_PATH_DEPTH} components"
        )));
    }
    if components
        .iter()
        .any(|part| part.is_empty() || *part == b"." || *part == b"..")
    {
        return Err(GitError::InvalidPath(
            "snapshot paths must be relative, with no empty, '.' or '..' components".into(),
        ));
    }
    Ok(components)
}

fn require_directory(entry: &SnapshotEntry) -> Result<()> {
    if entry.mode == TREE_MODE {
        Ok(())
    } else {
        Err(GitError::InvalidPath(
            "snapshot traversal requires a directory; files, symlinks and gitlinks cannot be entered"
                .into(),
        ))
    }
}

// Git orders a directory as though its name ends with '/', while files have a
// NUL terminator. The bytes before that terminator are compared verbatim.
fn tree_name_cmp(left: &[u8], left_mode: u32, right: &[u8], right_mode: u32) -> Ordering {
    let common = left.len().min(right.len());
    match left[..common].cmp(&right[..common]) {
        Ordering::Equal => {
            let next = |name: &[u8], mode| {
                name.get(common)
                    .copied()
                    .unwrap_or(if mode == TREE_MODE { b'/' } else { 0 })
            };
            next(left, left_mode).cmp(&next(right, right_mode))
        }
        ordering => ordering,
    }
}

fn read_verified(
    repository: &Repository,
    oid: ObjectId,
    allowed_types: &[GitObjectType],
    max_bytes: u64,
) -> Result<LoadedObject> {
    let header = repository
        .read_object_header(&oid)?
        .ok_or_else(|| GitError::not_found(format!("snapshot object {oid}")))?;
    if !allowed_types.contains(&header.0) {
        return Err(GitError::InvalidObject(format!(
            "snapshot object {oid} has unexpected type {}",
            header.0.as_str()
        )));
    }
    if header.1 > max_bytes {
        return Err(GitError::ResourceLimit {
            kind: ResourceLimitKind::DecodedObject,
            limit: max_bytes,
            attempted: header.1,
        });
    }
    let object = repository.load_object(&oid)?;
    if object.header() != header {
        return Err(GitError::InvalidObject(format!(
            "snapshot object {oid} changed type or size while being read"
        )));
    }
    let actual = object.encoded().object_id(repository.object_format())?;
    if actual != oid {
        return Err(GitError::InvalidObject(format!(
            "snapshot object {oid} fails hash verification (got {actual})"
        )));
    }
    Ok(object)
}

fn resolve_revision(repository: &Repository, revision: &str) -> Result<ObjectId> {
    if revision.len() > MAX_PATH_BYTES {
        return Err(GitError::InvalidFormat(format!(
            "snapshot revision exceeds {MAX_PATH_BYTES} bytes"
        )));
    }
    // Ref-name validation excludes revision operators, ranges, reflog selectors,
    // control bytes and path traversal before any reference files are read.
    FullName::new(revision).map_err(|_| {
        GitError::InvalidFormat(
            "snapshot revision must be a single ref name or hexadecimal object ID; revision expressions are unsupported"
                .into(),
        )
    })?;
    let format = repository.object_format();
    let is_hex = revision.bytes().all(|byte| byte.is_ascii_hexdigit());
    if is_hex && revision.len() == format.hex_len() {
        return ObjectId::from_hex(format, revision);
    }
    let candidates = if revision == "HEAD" || revision.starts_with("refs/") {
        vec![revision.to_owned()]
    } else {
        vec![
            format!("refs/{revision}"),
            format!("refs/tags/{revision}"),
            format!("refs/heads/{revision}"),
            format!("refs/remotes/{revision}"),
            format!("refs/remotes/{revision}/HEAD"),
            revision.to_owned(),
        ]
    };
    let mut resolved = None;
    for candidate in candidates {
        if let Some(oid) = resolve_reference(repository, &candidate)? {
            if resolved.is_some() {
                return Err(GitError::InvalidFormat(
                    "snapshot revision is an ambiguous short ref name; use a full ref name".into(),
                ));
            }
            resolved = Some(oid);
        }
    }
    if let Some(oid) = resolved {
        return Ok(oid);
    }
    if is_hex && (4..format.hex_len()).contains(&revision.len()) {
        // Prefix enumeration inspects object names/indexes, not object bodies.
        // Never let a general revision resolver walk unverified commit data.
        let matches = repository
            .object_database()
            .object_ids_with_prefix(&revision.to_ascii_lowercase())?;
        return match matches.as_slice() {
            [oid] => Ok(*oid),
            [] => Err(GitError::not_found("snapshot revision")),
            _ => Err(GitError::InvalidObjectId(
                "snapshot object ID prefix is ambiguous".into(),
            )),
        };
    }
    Err(GitError::not_found("snapshot revision"))
}

fn resolve_reference(repository: &Repository, name: &str) -> Result<Option<ObjectId>> {
    let mut current = name.to_owned();
    let mut seen = HashSet::new();
    for _ in 0..=MAX_REFERENCE_DEPTH {
        if !seen.insert(current.clone()) {
            return Err(GitError::InvalidFormat(
                "snapshot reference contains a symbolic cycle".into(),
            ));
        }
        match repository.find_reference(&current)? {
            Some(reference) => match reference.target {
                ReferenceTarget::Direct(oid) => return Ok(Some(oid)),
                ReferenceTarget::Symbolic(target) => current = target,
            },
            None => return Ok(None),
        }
    }
    Err(GitError::InvalidFormat(format!(
        "snapshot reference exceeds {MAX_REFERENCE_DEPTH} symbolic hops"
    )))
}

fn validate_leading_headers(oid: ObjectId, body: &[u8], prefixes: &[&[u8]]) -> Result<()> {
    let mut headers = body
        .split(|byte| *byte == b'\n')
        .take_while(|line| !line.is_empty());
    for prefix in prefixes {
        if !headers.next().is_some_and(|line| line.starts_with(prefix)) {
            return Err(GitError::InvalidObject(format!(
                "snapshot object {oid} has noncanonical leading headers"
            )));
        }
    }
    if headers.any(|line| prefixes.iter().any(|prefix| line.starts_with(prefix))) {
        return Err(GitError::InvalidObject(format!(
            "snapshot object {oid} contains duplicate selector headers"
        )));
    }
    Ok(())
}

fn resolve_commit(repository: &Repository, initial: ObjectId) -> Result<(ObjectId, ObjectId)> {
    let mut oid = initial;
    let mut expected = None;
    let mut seen = HashSet::new();
    for depth in 0..=MAX_TAG_DEPTH {
        if !seen.insert(oid) {
            return Err(GitError::InvalidObject(
                "snapshot tag chain contains a cycle".into(),
            ));
        }
        let object = read_verified(
            repository,
            oid,
            expected
                .as_ref()
                .map(std::slice::from_ref)
                .unwrap_or(&[GitObjectType::Commit, GitObjectType::Tag]),
            MAX_METADATA_BYTES,
        )?;
        match object.header().0 {
            GitObjectType::Commit => {
                // The general parser accepts misplaced or repeated selectors
                // and keeps the last. Do not let malformed canonical bytes
                // select a different tree from Git's first-header semantics.
                validate_leading_headers(oid, &object.encoded().body, &[b"tree "])?;
                let commit = object.commit_ref(repository.object_format())?;
                return Ok((oid, commit.tree));
            }
            GitObjectType::Tag if depth < MAX_TAG_DEPTH => {
                validate_leading_headers(
                    oid,
                    &object.encoded().body,
                    &[b"object ", b"type ", b"tag "],
                )?;
                let tag = TagObject::parse_ref(repository.object_format(), &object.encoded().body)?;
                if !matches!(tag.object_type, GitObjectType::Commit | GitObjectType::Tag) {
                    return Err(GitError::InvalidObject(
                        "snapshot tag must resolve to a commit".into(),
                    ));
                }
                oid = tag.object;
                expected = Some(tag.object_type);
            }
            GitObjectType::Tag => {
                return Err(GitError::InvalidObject(format!(
                    "snapshot tag chain exceeds {MAX_TAG_DEPTH} tags"
                )));
            }
            _ => {
                return Err(GitError::InvalidObject(
                    "snapshot revision must resolve to a commit".into(),
                ));
            }
        }
    }
    unreachable!("the final loop iteration returns a commit or tag-depth error")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_preserve_bytes_and_reject_escape_syntax() {
        assert_eq!(
            path_components(b"src/\xffCase\\name").unwrap(),
            vec![b"src".as_slice(), b"\xffCase\\name".as_slice()]
        );
        assert!(path_components(b"").unwrap().is_empty());
        for path in [
            b"/file".as_slice(),
            b"file/",
            b"a//b",
            b"./file",
            b"a/../b",
            b"a\0b",
        ] {
            assert!(path_components(path).is_err(), "accepted {path:?}");
        }
        assert!(path_components(&vec![b'a'; MAX_PATH_BYTES + 1]).is_err());
        let deep = vec!["a"; MAX_PATH_DEPTH + 1].join("/");
        assert!(path_components(deep.as_bytes()).is_err());
    }

    #[test]
    fn directory_order_uses_git_slash_terminator() {
        assert_eq!(
            tree_name_cmp(b"a.c", 0o100644, b"a", TREE_MODE),
            Ordering::Less
        );
        assert_eq!(
            tree_name_cmp(b"a", TREE_MODE, b"a0", 0o100644),
            Ordering::Less
        );
        assert_eq!(
            tree_name_cmp(b"a", 0o100644, b"a.c", 0o100644),
            Ordering::Less
        );
    }
}
