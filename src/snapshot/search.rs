// SPDX-License-Identifier: MIT OR Apache-2.0
//! Bounded literal search over verified immutable source objects.

use sley::{GitError, GitObjectType, ObjectId, Result};

use super::{
    DEFAULT_MAX_BLOB_BYTES, MAX_BLOB_BYTES, MAX_PATH_BYTES, MAX_PATH_DEPTH, Snapshot,
    SnapshotEntry, path_components, read_verified,
};

/// Limits bound accepted input and result sizes, not the object reader's peak
/// decompression/delta memory or wall time. Exceeding any limit returns an error.
#[derive(Debug, Clone, Copy)]
pub struct SearchLimits {
    pub max_blob_bytes: u64,
    pub max_total_bytes: u64,
    pub max_entries: usize,
    pub max_matches: usize,
}

impl SearchLimits {
    pub const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
    pub const MAX_ENTRIES: usize = 100_000;
    pub const MAX_MATCHES: usize = 10_000;
    pub const MAX_QUERY_BYTES: usize = 4096;
    pub const MAX_TREE_BYTES: u64 = 64 * 1024 * 1024;
    pub const MAX_RESULT_PATH_BYTES: usize = 8 * 1024 * 1024;

    fn validate(self, query: &[u8]) -> Result<()> {
        if query.is_empty()
            || query.len() > Self::MAX_QUERY_BYTES
            || query.contains(&b'\n')
        {
            return Err(GitError::InvalidFormat(format!(
                "snapshot search query must contain 1..={} bytes and no newline",
                Self::MAX_QUERY_BYTES
            )));
        }
        if self.max_blob_bytes > MAX_BLOB_BYTES
            || self.max_total_bytes > Self::MAX_TOTAL_BYTES
            || !(1..=Self::MAX_ENTRIES).contains(&self.max_entries)
            || !(1..=Self::MAX_MATCHES).contains(&self.max_matches)
        {
            return Err(GitError::InvalidFormat(
                "snapshot search limits exceed supported bounds".into(),
            ));
        }
        Ok(())
    }
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            max_blob_bytes: DEFAULT_MAX_BLOB_BYTES,
            max_total_bytes: 32 * 1024 * 1024,
            max_entries: 10_000,
            max_matches: 1000,
        }
    }
}

/// An exact byte occurrence. Line and byte column are one-based, offset is
/// zero-based. Paths remain raw Git bytes; a match never contains a lossy excerpt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    pub path: Vec<u8>,
    pub blob_id: ObjectId,
    pub byte_offset: u64,
    pub line: u64,
    pub byte_column: u64,
}

/// A complete result for the requested subtree and regular-file search scope.
/// This is a read result, not a durable receipt, index, grant or retention lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchReport {
    pub commit_id: ObjectId,
    pub tree_id: ObjectId,
    pub matches: Vec<SearchMatch>,
    pub entries: usize,
    pub files: usize,
    pub blob_bytes: u64,
    pub tree_bytes: u64,
    pub skipped_symlinks: usize,
    pub skipped_gitlinks: usize,
}

impl Snapshot {
    /// Search regular and executable files in one committed subtree or file.
    /// The empty path selects the root. Query bytes are literal, case-sensitive,
    /// single-line and may overlap. Binary file contents are searched as bytes.
    ///
    /// Results follow depth-first Git tree order, then increasing byte offset.
    /// Symlinks and gitlinks inside a directory are counted but not read or
    /// followed. Selecting either directly is an error. All visited objects
    /// are hash-verified, and any missing/corrupt object or exceeded budget fails
    /// the entire operation. There is no successful partial or truncated result.
    ///
    /// Metadata needed to resolve the selected path has the existing snapshot
    /// limits. `tree_bytes` measures traversal after that path has been resolved.
    /// No worktree, network, hook, filter, external command or write is involved.
    pub fn search(&self, path: &[u8], query: &[u8], limits: SearchLimits) -> Result<SearchReport> {
        limits.validate(query)?;
        let depth = path_components(path)?.len();
        let entry = self.stat(path)?;
        if !matches!(entry.mode, 0o040000 | 0o100644 | 0o100755) {
            return Err(GitError::InvalidPath(
                "snapshot search requires a directory or regular file".into(),
            ));
        }
        let mut search = Search {
            snapshot: self,
            query,
            prefix: prefix_lengths(query),
            limits,
            path_bytes: 0,
            report: SearchReport {
                commit_id: self.commit_id(),
                tree_id: self.tree_id(),
                matches: Vec::new(),
                entries: 0,
                files: 0,
                blob_bytes: 0,
                tree_bytes: 0,
                skipped_symlinks: 0,
                skipped_gitlinks: 0,
            },
        };
        search.visit(path, entry, depth)?;
        Ok(search.report)
    }
}

struct Search<'a> {
    snapshot: &'a Snapshot,
    query: &'a [u8],
    prefix: Vec<usize>,
    limits: SearchLimits,
    path_bytes: usize,
    report: SearchReport,
}

fn exceeded(name: &str, limit: impl std::fmt::Display) -> GitError {
    GitError::InvalidFormat(format!("snapshot search exceeds {name} limit of {limit}"))
}

impl Search<'_> {
    fn visit(&mut self, path: &[u8], entry: SnapshotEntry, depth: usize) -> Result<()> {
        if depth > MAX_PATH_DEPTH || path.len() > MAX_PATH_BYTES {
            return Err(GitError::InvalidPath(
                "snapshot search path exceeds snapshot path limits".into(),
            ));
        }
        if self.report.entries == self.limits.max_entries {
            return Err(exceeded("entries", self.limits.max_entries));
        }
        self.report.entries += 1;
        match entry.mode {
            0o040000 => {
                let bytes = self.object_size(entry.oid)?;
                if bytes > SearchLimits::MAX_TREE_BYTES - self.report.tree_bytes {
                    return Err(exceeded("tree bytes", SearchLimits::MAX_TREE_BYTES));
                }
                let (children, verified_bytes) = self.snapshot.read_tree_with_size(entry.oid)?;
                if verified_bytes > SearchLimits::MAX_TREE_BYTES - self.report.tree_bytes {
                    return Err(exceeded("tree bytes", SearchLimits::MAX_TREE_BYTES));
                }
                self.report.tree_bytes += verified_bytes;
                if children.len() > self.limits.max_entries - self.report.entries {
                    return Err(exceeded("entries", self.limits.max_entries));
                }
                for child in children {
                    let mut child_path = path.to_vec();
                    if !child_path.is_empty() {
                        child_path.push(b'/');
                    }
                    child_path.extend_from_slice(&child.name);
                    self.visit(&child_path, child, depth + 1)?;
                }
            }
            0o100644 | 0o100755 => {
                let bytes = self.object_size(entry.oid)?;
                if bytes > self.limits.max_total_bytes - self.report.blob_bytes {
                    return Err(exceeded("total blob bytes", self.limits.max_total_bytes));
                }
                let blob = read_verified(
                    &self.snapshot.repository,
                    entry.oid,
                    &[GitObjectType::Blob],
                    self.limits.max_blob_bytes,
                )?;
                let verified_bytes = blob.encoded().body.len() as u64;
                if verified_bytes > self.limits.max_total_bytes - self.report.blob_bytes {
                    return Err(exceeded("total blob bytes", self.limits.max_total_bytes));
                }
                self.report.files += 1;
                self.report.blob_bytes += verified_bytes;
                self.scan(path, entry.oid, &blob.encoded().body)?;
            }
            0o120000 => self.report.skipped_symlinks += 1,
            0o160000 => self.report.skipped_gitlinks += 1,
            _ => unreachable!("snapshot entries have validated Git modes"),
        }
        Ok(())
    }

    fn object_size(&self, oid: ObjectId) -> Result<u64> {
        self.snapshot
            .repository
            .read_object_header(&oid)?
            .map(|(_, bytes)| bytes)
            .ok_or_else(|| GitError::not_found(format!("snapshot object {oid}")))
    }

    fn scan(&mut self, path: &[u8], oid: ObjectId, bytes: &[u8]) -> Result<()> {
        let mut matched = 0;
        let mut line = 1;
        let mut line_start = 0;
        for (index, &byte) in bytes.iter().enumerate() {
            while matched > 0 && self.query[matched] != byte {
                matched = self.prefix[matched - 1];
            }
            if self.query[matched] == byte {
                matched += 1;
            }
            if matched == self.query.len() {
                if self.report.matches.len() == self.limits.max_matches {
                    return Err(exceeded("matches", self.limits.max_matches));
                }
                if path.len() > SearchLimits::MAX_RESULT_PATH_BYTES - self.path_bytes {
                    return Err(exceeded("result path bytes", SearchLimits::MAX_RESULT_PATH_BYTES));
                }
                self.path_bytes += path.len();
                let start = index + 1 - self.query.len();
                self.report.matches.push(SearchMatch {
                    path: path.to_vec(),
                    blob_id: oid,
                    byte_offset: start as u64,
                    line,
                    byte_column: (start - line_start + 1) as u64,
                });
                matched = self.prefix[matched - 1];
            }
            if byte == b'\n' {
                line += 1;
                line_start = index + 1;
            }
        }
        Ok(())
    }
}

// KMP keeps scan work linear in the input and query sizes, including repeated
// prefixes. Falling back after a match preserves overlapping occurrences.
fn prefix_lengths(query: &[u8]) -> Vec<usize> {
    let mut prefix = vec![0; query.len()];
    for index in 1..query.len() {
        let mut matched = prefix[index - 1];
        while matched > 0 && query[index] != query[matched] {
            matched = prefix[matched - 1];
        }
        if query[index] == query[matched] {
            matched += 1;
        }
        prefix[index] = matched;
    }
    prefix
}
