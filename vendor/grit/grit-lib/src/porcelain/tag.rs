//! `git tag` listing/filtering core.
//!
//! The full tag command in the `grit` binary parses argv, decides between
//! create/list/delete/verify modes, resolves the tagger identity, launches the
//! tag-message editor, writes the tag object and ref, prints the tag listing,
//! and maps exit codes. Those responsibilities — argv parsing, terminal output,
//! editor/hook subprocess dispatch, identity/env resolution, object/ref writes,
//! and exit-code mapping — stay in the CLI.
//!
//! What lives here is the self-contained, presentation-free part of tag listing
//! and message cleanup: the predicates and orderings that filter and sort the
//! `refs/tags/*` set, and the comment-stripping applied to `-m`/`-F` messages.
//! Each computes a result from object/ref data (or a plain string) alone, with
//! no presentation, argv, or process state.
//!
//! # What this module owns
//!
//! - The `--contains` / `--no-contains` / `--points-at` filter predicates
//!   ([`tag_contains`], [`tag_points_at`]) and the [`peel_to_commit`] helper they
//!   share.
//! - The `--sort` comparators: numeric `version:refname` ordering
//!   ([`compare_version`] / [`version_segments`]) and the `creatordate` /
//!   `taggerdate` epoch extraction ([`creator_date`] / [`parse_epoch_from_ident`]).
//! - The `-l` glob matcher ([`glob_matches`]) and the `-n<N>` annotation
//!   extraction ([`get_tag_annotation`]).
//! - The `-m`/`-F` message cleanup ([`strip_comments`]).

use std::collections::{HashSet, VecDeque};

use crate::objects::{parse_commit, parse_tag, ObjectId, ObjectKind};
use crate::repo::Repository;

/// Get annotation text for a tag (up to `n` lines).
///
/// Returns `None` if the tag has no annotation (lightweight) or `n == 0`.
#[must_use]
pub fn get_tag_annotation(repo: &Repository, oid: &ObjectId, n: u32) -> Option<String> {
    if n == 0 {
        return None;
    }
    let obj = repo.odb.read(oid).ok()?;
    let tag = parse_tag(&obj.data).ok()?;
    if tag.message.trim().is_empty() {
        return None;
    }
    let lines: Vec<&str> = tag
        .message
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(n as usize)
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(lines.join(" "))
}

/// Check if a tag contains (has reachable ancestry from) a commit.
///
/// Peels the tag ref to a commit, then walks ancestors.
#[must_use]
pub fn tag_contains(repo: &Repository, tag_oid: &ObjectId, target: &ObjectId) -> bool {
    // Peel to commit
    let commit_oid = match peel_to_commit(repo, tag_oid) {
        Some(oid) => oid,
        None => return false,
    };

    if &commit_oid == target {
        return true;
    }

    // BFS/DFS walk
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(commit_oid);

    while let Some(oid) = queue.pop_front() {
        if !visited.insert(oid) {
            continue;
        }
        if &oid == target {
            return true;
        }
        if let Ok(obj) = repo.odb.read(&oid) {
            if obj.kind == ObjectKind::Commit {
                if let Ok(commit) = parse_commit(&obj.data) {
                    for parent in commit.parents {
                        if !visited.contains(&parent) {
                            queue.push_back(parent);
                        }
                    }
                }
            }
        }
    }

    false
}

/// Check if a tag points at (or peels to) a given object.
#[must_use]
pub fn tag_points_at(repo: &Repository, tag_oid: &ObjectId, target: &ObjectId) -> bool {
    if tag_oid == target {
        return true;
    }
    // Peel through tag objects
    let mut current = *tag_oid;
    for _ in 0..10 {
        let obj = match repo.odb.read(&current) {
            Ok(o) => o,
            Err(_) => return false,
        };
        match obj.kind {
            ObjectKind::Tag => {
                let tag = match parse_tag(&obj.data) {
                    Ok(t) => t,
                    Err(_) => return false,
                };
                if &tag.object == target {
                    return true;
                }
                current = tag.object;
            }
            _ => return false,
        }
    }
    false
}

/// Peel an object to a commit OID (following tags).
#[must_use]
pub fn peel_to_commit(repo: &Repository, oid: &ObjectId) -> Option<ObjectId> {
    let mut current = *oid;
    for _ in 0..10 {
        let obj = repo.odb.read(&current).ok()?;
        match obj.kind {
            ObjectKind::Commit => return Some(current),
            ObjectKind::Tag => {
                let tag = parse_tag(&obj.data).ok()?;
                current = tag.object;
            }
            _ => return None,
        }
    }
    None
}

/// Extract the "creator date" for a tag object.
///
/// For annotated tags, this is the tagger date.  For lightweight tags
/// (which point directly at a commit), this is the committer date.
/// Returns 0 if the date cannot be determined.
#[must_use]
pub fn creator_date(repo: &Repository, oid: &ObjectId) -> i64 {
    let obj = match repo.odb.read(oid) {
        Ok(o) => o,
        Err(_) => return 0,
    };
    match obj.kind {
        ObjectKind::Tag => {
            // Parse tagger line for epoch
            if let Ok(tag) = parse_tag(&obj.data) {
                if let Some(ref tagger) = tag.tagger {
                    return parse_epoch_from_ident(tagger);
                }
            }
            0
        }
        ObjectKind::Commit => {
            if let Ok(commit) = parse_commit(&obj.data) {
                parse_epoch_from_ident(&commit.committer)
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// Extract the epoch timestamp from a Git identity string.
///
/// Format: `Name <email> <epoch> <offset>`
#[must_use]
pub fn parse_epoch_from_ident(ident: &str) -> i64 {
    // The epoch is the second-to-last token
    let parts: Vec<&str> = ident.rsplitn(3, ' ').collect();
    if parts.len() >= 2 {
        parts[1].parse().unwrap_or(0)
    } else {
        0
    }
}

/// Compare two tag names as version strings (for `version:refname`).
///
/// Splits each name on `.` and `-` boundaries, comparing numeric segments
/// numerically and non-numeric segments lexicographically.  This matches
/// the behaviour of `git tag --sort=version:refname` (strverscmp-like).
#[must_use]
pub fn compare_version(a: &str, b: &str) -> std::cmp::Ordering {
    let seg_a = version_segments(a);
    let seg_b = version_segments(b);
    for (sa, sb) in seg_a.iter().zip(seg_b.iter()) {
        let ord = match (sa.parse::<u64>(), sb.parse::<u64>()) {
            (Ok(na), Ok(nb)) => na.cmp(&nb),
            _ => sa.cmp(sb),
        };
        if ord != std::cmp::Ordering::Equal {
            return ord;
        }
    }
    seg_a.len().cmp(&seg_b.len())
}

/// Split a version string into segments at `.` and `-` boundaries.
#[must_use]
pub fn version_segments(s: &str) -> Vec<&str> {
    // Split on `.` and `-` keeping non-empty pieces
    s.split(['.', '-']).filter(|seg| !seg.is_empty()).collect()
}

/// Strip `#comment` lines from a message and normalize whitespace.
///
/// Also: strip trailing whitespace from each line, collapse multiple blank lines
/// to one, and drop leading/trailing blank lines. This is the `whitespace`
/// cleanup `git tag` applies to `-m`/`-F` messages (anything but `--cleanup=verbatim`).
#[must_use]
pub fn strip_comments(s: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in s.lines() {
        if line.starts_with('#') {
            continue;
        }
        lines.push(line.trim_end().to_string());
    }
    // Remove leading blank lines
    while lines.first().map(|l| l.is_empty()).unwrap_or(false) {
        lines.remove(0);
    }
    // Remove trailing blank lines
    while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    if lines.is_empty() {
        return String::new();
    }
    // Collapse multiple consecutive blank lines to at most one
    let mut result = Vec::new();
    let mut last_blank = false;
    for line in &lines {
        let is_blank = line.is_empty();
        if is_blank && last_blank {
            continue; // skip extra blank lines
        }
        result.push(line.clone());
        last_blank = is_blank;
    }
    result.join("\n") + "\n"
}

/// Simple glob pattern matching for tag names.
///
/// Supports `*` (matches any sequence) and `?` (matches any single character).
#[must_use]
pub fn glob_matches(pattern: &str, name: &str) -> bool {
    glob_match_bytes(pattern.as_bytes(), name.as_bytes())
}

/// Recursive glob matcher.
fn glob_match_bytes(pat: &[u8], text: &[u8]) -> bool {
    match (pat.first(), text.first()) {
        (None, None) => true,
        (Some(&b'*'), _) => {
            // Skip consecutive stars
            let pat_rest = pat
                .iter()
                .position(|&b| b != b'*')
                .map_or(&pat[pat.len()..], |i| &pat[i..]);
            if pat_rest.is_empty() {
                return true;
            }
            for i in 0..=text.len() {
                if glob_match_bytes(pat_rest, &text[i..]) {
                    return true;
                }
            }
            false
        }
        (Some(&b'?'), Some(_)) => glob_match_bytes(&pat[1..], &text[1..]),
        (Some(p), Some(t)) if p == t => glob_match_bytes(&pat[1..], &text[1..]),
        _ => false,
    }
}
