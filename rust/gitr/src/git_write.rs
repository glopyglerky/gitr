//! Checkout-free Git object authoring through the already pinned Gitoxide.
//!
//! These commands write immutable objects only. They never move refs, modify an
//! index/worktree, run a hook/filter/signer, or invoke another executable. Publish
//! the resulting commit separately with Gitr's guarded `update-ref` command.

use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Subcommand};
use gix::{
    bstr::ByteSlice,
    hash::ObjectId,
    objs::{Kind, Tree, tree::Entry},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    path::PathBuf,
};

const MAX_INPUT: usize = 16 * 1024 * 1024;
const MAX_MESSAGE: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
const MAX_DEPTH: usize = 128;
const MAX_PARENTS: usize = 64;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Write a tree from validated ls-tree-style records on stdin; no refs change.
    Mktree(Mktree),
    /// Write a tree from the current index without modifying that index.
    WriteTree,
    /// Write an unsigned commit object without updating HEAD or any ref.
    CommitTree(CommitTree),
}

#[derive(Debug, Args)]
pub struct Mktree {
    /// Read NUL-terminated records and literal filename bytes, including newlines.
    #[arg(short = 'z')]
    pub nul: bool,
}

#[derive(Debug, Args)]
pub struct CommitTree {
    pub tree: OsString,
    /// Parent order is retained. Repeated parent IDs are ignored, as in Git.
    #[arg(short = 'p')]
    pub parents: Vec<OsString>,
    /// Repeat to add paragraphs. Cannot be combined with -F.
    #[arg(short = 'm', conflicts_with = "file")]
    pub messages: Vec<String>,
    /// Read an exact UTF-8 message from a file, or stdin with -F -.
    #[arg(short = 'F', conflicts_with = "messages")]
    pub file: Option<PathBuf>,
}

pub fn run(command: Command, out: &mut impl Write) -> Result<()> {
    let repo = super::discover()?;
    let id = match command {
        Command::Mktree(args) => {
            let bytes = bounded_read(io::stdin().lock(), MAX_INPUT)?;
            mktree(&repo, &bytes, args.nul)?
        }
        Command::WriteTree => write_tree(&repo)?,
        Command::CommitTree(args) => commit_tree(&repo, args)?,
    };
    writeln!(out, "{id}")?;
    Ok(())
}

fn bounded_read(input: impl Read, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    input.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "input exceeds the supported byte limit"
    );
    Ok(bytes)
}

fn component(name: &[u8]) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 4096
            && name != b"."
            && name != b".."
            && !name.eq_ignore_ascii_case(b".git")
            && !name.contains(&0)
            && !name.contains(&b'/'),
        "invalid or unsafe tree path component"
    );
    Ok(())
}

fn mode_kind(mode: u32) -> Result<Kind> {
    Ok(match mode {
        0o040000 => Kind::Tree,
        0o100644 | 0o100755 | 0o120000 => Kind::Blob,
        0o160000 => Kind::Commit,
        _ => bail!("unsupported tree entry mode"),
    })
}

fn validate_target(repo: &gix::Repository, mode: u32, id: ObjectId) -> Result<()> {
    let kind = mode_kind(mode)?;
    ensure!(
        id.kind() == repo.object_hash(),
        "object ID hash format mismatch"
    );
    ensure!(!id.is_null(), "null tree entry object ID");
    // Gitlinks refer to objects in a DIFFERENT repository. They may be absent
    // locally. If present, reject a demonstrably wrong object type.
    if mode == 0o160000 && !repo.has_object(id) {
        return Ok(());
    }
    ensure!(
        repo.find_header(id)?.kind() == kind,
        "tree entry mode does not match its stored object type"
    );
    Ok(())
}

fn entry(name: Vec<u8>, mode: u32, oid: ObjectId) -> Result<Entry> {
    Ok(Entry {
        filename: name.into(),
        mode: gix::objs::tree::EntryMode::try_from(mode)
            .map_err(|_| anyhow::anyhow!("invalid tree mode"))?,
        oid,
    })
}

fn store_tree(repo: &gix::Repository, mut entries: Vec<Entry>) -> Result<ObjectId> {
    // Git treats a directory name as if it ended in '/'. Plain lexical sorting
    // produces the wrong hash for names such as a.c and directory a.
    entries.sort_by(|a, b| {
        gix::objs::tree::name_order(
            a.filename.as_bytes(),
            a.mode.value() == 0o040000,
            b.filename.as_bytes(),
            b.mode.value() == 0o040000,
        )
    });
    Ok(repo.write_object(Tree { entries })?.detach())
}

fn mktree(repo: &gix::Repository, bytes: &[u8], nul: bool) -> Result<ObjectId> {
    let delimiter = if nul { 0 } else { b'\n' };
    let body = match bytes.strip_suffix(&[delimiter]) {
        Some(body) => body,
        None if bytes.is_empty() => bytes,
        None => bail!("mktree input must end with its record terminator"),
    };
    let mut entries = Vec::new();
    let mut names = BTreeSet::new();
    if !bytes.is_empty() {
        for line in body.split(|byte| *byte == delimiter) {
            ensure!(entries.len() < MAX_ENTRIES, "too many tree entries");
            let tab = line
                .iter()
                .position(|b| *b == b'\t')
                .context("tree record requires a tab before the filename")?;
            let header: Vec<_> = line[..tab].split(|b| *b == b' ').collect();
            ensure!(
                header.len() == 3 && header.iter().all(|s| !s.is_empty()),
                "expected mode, type and object ID in each tree record"
            );
            let mode_text = std::str::from_utf8(header[0])?;
            ensure!(
                mode_text.len() <= 6 && mode_text.bytes().all(|b| (b'0'..=b'7').contains(&b)),
                "invalid octal tree mode"
            );
            let mode = u32::from_str_radix(mode_text, 8)?;
            let expected_type: &[u8] = match mode_kind(mode)? {
                Kind::Blob => b"blob",
                Kind::Tree => b"tree",
                Kind::Commit => b"commit",
                Kind::Tag => unreachable!("mode_kind never produces tags"),
            };
            ensure!(
                header[1] == expected_type,
                "declared tree entry type disagrees with mode"
            );
            let name = &line[tab + 1..];
            ensure!(
                nul || !name.starts_with(b"\""),
                "quoted filenames are not supported; use mktree -z with literal path bytes"
            );
            component(name)?;
            ensure!(names.insert(name.to_vec()), "duplicate tree entry name");
            let id = ObjectId::from_hex(header[2])?;
            validate_target(repo, mode, id)?;
            entries.push(entry(name.to_vec(), mode, id)?);
        }
    }
    // All input is validated before the first object write.
    store_tree(repo, entries)
}

#[derive(Default)]
struct Directory {
    files: BTreeMap<Vec<u8>, (u32, ObjectId)>,
    children: BTreeMap<Vec<u8>, usize>,
}

fn write_tree(repo: &gix::Repository) -> Result<ObjectId> {
    let index = repo.index_or_empty()?;
    index.verify_integrity()?;
    index.verify_entries()?;
    ensure!(
        !index.is_sparse(),
        "sparse-index write-tree is not implemented"
    );
    ensure!(
        index.link().is_none(),
        "split-index write-tree is not implemented"
    );
    ensure!(
        index.entries().len() <= MAX_ENTRIES,
        "too many index entries"
    );
    let mut directories = vec![Directory::default()];
    let mut path_bytes = 0usize;
    for item in index.entries() {
        ensure!(
            item.stage_raw() == 0,
            "cannot write a tree from an unmerged index"
        );
        let path = item.path(&index).as_bytes();
        path_bytes = path_bytes
            .checked_add(path.len())
            .context("path byte overflow")?;
        ensure!(
            path_bytes <= MAX_INPUT,
            "index paths exceed the supported byte limit"
        );
        let components: Vec<_> = path.split(|b| *b == b'/').collect();
        ensure!(
            components.len() <= MAX_DEPTH,
            "index path nesting exceeds the limit"
        );
        for part in &components {
            component(part)?;
        }
        // Intent-to-add entries do not stage content.
        if item.flags.contains(gix::index::entry::Flags::INTENT_TO_ADD) {
            continue;
        }
        let mode = item.mode.bits();
        ensure!(
            mode != 0o040000,
            "directory entries require sparse-index support"
        );
        validate_target(repo, mode, item.id)?;
        let mut current = 0;
        for part in &components[..components.len() - 1] {
            ensure!(
                !directories[current].files.contains_key(*part),
                "index has a file/directory path conflict"
            );
            current = if let Some(child) = directories[current].children.get(*part) {
                *child
            } else {
                let child = directories.len();
                directories.push(Directory::default());
                directories[current].children.insert(part.to_vec(), child);
                child
            };
        }
        let name = *components.last().context("missing index filename")?;
        ensure!(
            !directories[current].children.contains_key(name),
            "index has a file/directory path conflict"
        );
        ensure!(
            directories[current]
                .files
                .insert(name.to_vec(), (mode, item.id))
                .is_none(),
            "duplicate index path"
        );
    }
    // Children are allocated after parents. Bottom-up processing avoids recursive
    // traversal or dependence on the index's cached TREE extension.
    let mut ids = vec![None; directories.len()];
    for position in (0..directories.len()).rev() {
        let dir = &directories[position];
        let mut entries = Vec::with_capacity(dir.files.len() + dir.children.len());
        for (name, (mode, id)) in &dir.files {
            entries.push(entry(name.clone(), *mode, *id)?);
        }
        for (name, child) in &dir.children {
            entries.push(entry(
                name.clone(),
                0o040000,
                ids[*child].context("missing child tree")?,
            )?);
        }
        ids[position] = Some(store_tree(repo, entries)?);
    }
    ids[0].context("missing root tree")
}

fn message(args: &CommitTree) -> Result<Vec<u8>> {
    if !args.messages.is_empty() {
        let mut out = Vec::new();
        for part in &args.messages {
            // Plumbing -m separates nonempty accumulated content with one newline
            // and terminates nonempty paragraphs. No porcelain comment cleanup.
            ensure!(
                out.len().saturating_add(part.len()).saturating_add(2) <= MAX_MESSAGE,
                "commit message exceeds 1 MiB"
            );
            if !out.is_empty() {
                out.push(b'\n');
            }
            out.extend_from_slice(part.as_bytes());
            if !out.is_empty() && !out.ends_with(b"\n") {
                out.push(b'\n');
            }
        }
        Ok(out)
    } else {
        match args.file.as_deref() {
            Some(path) if path != std::path::Path::new("-") => {
                bounded_read(fs::File::open(path)?, MAX_MESSAGE)
            }
            _ => bounded_read(io::stdin().lock(), MAX_MESSAGE),
        }
    }
}

fn commit_tree(repo: &gix::Repository, args: CommitTree) -> Result<ObjectId> {
    ensure!(args.parents.len() <= MAX_PARENTS, "too many commit parents");
    let tree = repo
        .rev_parse_single(gix::path::os_str_into_bstr(&args.tree)?)?
        .detach();
    ensure!(
        repo.find_header(tree)?.kind() == Kind::Tree,
        "commit-tree requires a tree object"
    );
    let mut parents = Vec::new();
    let mut seen = BTreeSet::new();
    for parent in &args.parents {
        let id = repo
            .rev_parse_single(gix::path::os_str_into_bstr(parent)?)?
            .detach();
        ensure!(
            repo.find_header(id)?.kind() == Kind::Commit,
            "commit-tree -p requires a direct commit object; peel tags explicitly"
        );
        if seen.insert(id) {
            parents.push(id);
        }
    }
    let bytes = message(&args)?;
    ensure!(
        !bytes.contains(&0),
        "NUL bytes are not supported in commit messages"
    );
    let text =
        std::str::from_utf8(&bytes).context("commit-tree currently requires UTF-8 messages")?;
    let config = repo.config_snapshot();
    ensure!(
        !config.try_boolean("commit.gpgSign")?.unwrap_or(false),
        "commit signing is configured but not implemented; refusing an unsigned commit"
    );
    if let Some(encoding) = config.string("i18n.commitEncoding") {
        ensure!(
            encoding.as_bytes().eq_ignore_ascii_case(b"utf-8"),
            "non-UTF-8 commit encoding is not implemented"
        );
    }
    // No anonymous fallback identities or signing helper execution.
    let author = repo
        .author()
        .transpose()?
        .context("repository-local author identity is required")?;
    let committer = repo
        .committer()
        .transpose()?
        .context("repository-local committer identity is required")?;
    for identity in [author, committer] {
        for field in [identity.name, identity.email] {
            ensure!(
                !field.is_empty() && !field.as_bytes().iter().any(|b| b"\0\r\n<>".contains(b)),
                "commit identity contains empty or unsafe header fields"
            );
        }
    }
    Ok(repo
        .new_commit_as(committer, author, text, tree, parents)?
        .id)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(parts: &[&str]) -> CommitTree {
        CommitTree {
            tree: "HEAD".into(),
            parents: Vec::new(),
            messages: parts.iter().map(|s| (*s).to_owned()).collect(),
            file: None,
        }
    }
    #[test]
    fn message_paragraphs_match_plumbing_not_porcelain_cleanup() {
        assert_eq!(message(&args(&["one", "two"])).unwrap(), b"one\n\ntwo\n");
        assert_eq!(message(&args(&["one\n\n"])).unwrap(), b"one\n\n");
        assert_eq!(message(&args(&["a  \n\n#tag"])).unwrap(), b"a  \n\n#tag\n");
        assert_eq!(message(&args(&[""])).unwrap(), b"");
        assert_eq!(message(&args(&["", "second"])).unwrap(), b"second\n");
    }
    #[test]
    fn oversized_input_is_rejected_before_parsing() {
        assert!(bounded_read(&vec![0; MAX_MESSAGE + 1][..], MAX_MESSAGE).is_err());
        assert!(message(&args(&[&"x".repeat(MAX_MESSAGE)])).is_err());
    }
    #[test]
    fn unsafe_components_are_rejected_without_lossy_unicode_conversion() {
        for invalid in [b"".as_slice(), b".", b"..", b".Git", b"a/b", b"x\0y"] {
            assert!(component(invalid).is_err());
        }
        for valid in [b"space name".as_slice(), b"x\ny", b"raw\xff"] {
            component(valid).unwrap();
        }
    }
}
