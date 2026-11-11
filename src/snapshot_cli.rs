// SPDX-License-Identifier: MIT OR Apache-2.0
//! Explicit Rit extension for canonical, checkout-free source reads.

use std::{
    ffi::{OsStr, OsString},
    io::{self, Write},
    path::PathBuf,
};

use serde_json::{Value, json};
use sley::{GitError, Result};

use crate::catalog::SnapshotCatalog;
use crate::snapshot::{DEFAULT_MAX_BLOB_BYTES, MAX_BLOB_BYTES, SearchLimits, Snapshot, SnapshotEntry};

const HELP: &str = "\
usage: rit --rit-snapshot <inspect|list|stat|read|search> [options]

  --repo PATH         Repository to open, default: .
  --revision REV      Commit ID, HEAD, ref name, or annotated tag, default: HEAD
  --saved NAME        Saved catalog snapshot, instead of --revision
  --path PATH         Exact repository-relative path; required for read
  --path-hex HEX      Path bytes encoded as hexadecimal, instead of --path
  --max-bytes N       Read limit, default: 8388608; maximum: 67108864
  --query TEXT        Case-sensitive literal search query, required for search
  --max-total-bytes N Search blob budget, default: 33554432; maximum: 268435456
  --max-entries N     Search entries including root, default: 10000; max: 100000
  --max-matches N     Search occurrences, default: 1000; maximum: 10000
  --help              Show this help

inspect reports the resolved commit and root tree as JSON.
list reports one directory as JSON, without reading file payloads.
stat reports one entry as JSON. Empty paths select the root directory.
read writes the verified raw blob bytes, including symlink target bytes.
search returns exact JSON byte locations in regular files, including binary
content. Empty paths select the whole tree. Query length is 1..4096 bytes with
no newline. Symlinks and submodules are counted but not searched or followed.
Missing/corrupt objects or exceeded limits fail without partial results.
Search traversal accepts at most 64 MiB tree bytes and 8 MiB result path bytes.

Revision expressions such as HEAD~1 are not accepted. Resolve them to a full
commit ID first. Paths are not normalized and symlinks are not followed.
Reads do not apply checkout filters or fetch missing objects. Snapshot IDs do
not retain objects against pruning. Selected objects are materialized in memory.
--saved opens an existing catalog without creating it. Saved records also do
not retain objects; a deleted record or unavailable Git object is an error.

GIT_DIR and GIT_WORK_TREE do not redirect this reader. Unset GIT_COMMON_DIR,
GIT_OBJECT_DIRECTORY, GIT_ALTERNATE_OBJECT_DIRECTORIES and GIT_REFERENCE_BACKEND
before use; ambient storage overrides are rejected.
";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Inspect,
    List,
    Stat,
    Read,
    Search,
}

struct Options {
    action: Action,
    repository: PathBuf,
    revision: String,
    saved: Option<String>,
    path: Vec<u8>,
    max_bytes: u64,
    query: Option<Vec<u8>>,
    search_limits: SearchLimits,
}

fn usage(message: impl Into<String>) -> GitError {
    GitError::InvalidFormat(format!(
        "{}; see `rit --rit-snapshot --help`",
        message.into()
    ))
}

fn utf8<'a>(value: &'a OsStr, option: &str) -> Result<&'a str> {
    value
        .to_str()
        .ok_or_else(|| usage(format!("{option} requires UTF-8 text")))
}

fn path_bytes(value: &OsStr) -> Result<Vec<u8>> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(value.as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        Ok(utf8(value, "--path; use --path-hex for other bytes")?
            .as_bytes()
            .to_vec())
    }
}

fn from_hex(value: &OsStr) -> Result<Vec<u8>> {
    let value = utf8(value, "--path-hex")?.as_bytes();
    if value.len() % 2 != 0 || value.len() > crate::snapshot::MAX_PATH_BYTES * 2 {
        return Err(usage(
            "--path-hex requires an even number of hex digits within the path limit",
        ));
    }
    fn digit(byte: u8) -> Result<u8> {
        match byte {
            b'0'..=b'9' => Ok(byte - b'0'),
            b'a'..=b'f' => Ok(byte - b'a' + 10),
            b'A'..=b'F' => Ok(byte - b'A' + 10),
            _ => Err(usage("--path-hex contains a non-hexadecimal byte")),
        }
    }
    value
        .chunks_exact(2)
        .map(|pair| Ok((digit(pair[0])? << 4) | digit(pair[1])?))
        .collect()
}

fn set_once<T>(slot: &mut Option<T>, value: T, option: &str) -> Result<()> {
    if slot.is_some() {
        return Err(usage(format!("{option} was specified more than once")));
    }
    *slot = Some(value);
    Ok(())
}

fn parse(args: &[OsString]) -> Result<Option<Options>> {
    let Some(first) = args.first() else {
        return Err(usage("a snapshot action is required"));
    };
    let action = match utf8(first, "snapshot action")? {
        "inspect" => Action::Inspect,
        "list" => Action::List,
        "stat" => Action::Stat,
        "read" => Action::Read,
        "search" => Action::Search,
        "--help" | "-h" if args.len() == 1 => return Ok(None),
        _ => return Err(usage("expected inspect, list, stat, read, or search")),
    };
    let mut repository = None;
    let mut revision = None;
    let mut saved = None;
    let mut path = None;
    let mut max_bytes = None;
    let mut query = None;
    let mut max_total_bytes = None;
    let mut max_entries = None;
    let mut max_matches = None;
    let mut remaining = args[1..].iter();
    while let Some(option) = remaining.next() {
        let option = utf8(option, "option name")?;
        if option == "--help" || option == "-h" {
            return Ok(None);
        }
        if !matches!(
            option,
            "--repo" | "--revision" | "--saved" | "--path" | "--path-hex" | "--max-bytes"
                | "--query" | "--max-total-bytes" | "--max-entries" | "--max-matches"
        ) {
            return Err(usage(format!("unknown snapshot option {option:?}")));
        }
        let value = remaining
            .next()
            .ok_or_else(|| usage(format!("{option} requires a value")))?;
        match option {
            "--repo" => {
                if value.is_empty() {
                    return Err(usage("--repo must not be empty"));
                }
                set_once(&mut repository, PathBuf::from(value), option)?;
            }
            "--revision" => {
                set_once(&mut revision, utf8(value, option)?.to_owned(), option)?;
            }
            "--saved" => set_once(&mut saved, utf8(value, option)?.to_owned(), option)?,
            "--path" => set_once(&mut path, path_bytes(value)?, "--path/--path-hex")?,
            "--path-hex" => set_once(&mut path, from_hex(value)?, "--path/--path-hex")?,
            "--query" => set_once(&mut query, utf8(value, option)?.as_bytes().to_vec(), option)?,
            "--max-bytes" | "--max-total-bytes" | "--max-entries" | "--max-matches" => {
                let text = utf8(value, option)?;
                if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(usage(format!("{option} must be an unsigned decimal integer")));
                }
                let number = text
                    .parse::<u64>()
                    .map_err(|_| usage(format!("{option} is too large")))?;
                let maximum = match option {
                    "--max-bytes" => MAX_BLOB_BYTES,
                    "--max-total-bytes" => SearchLimits::MAX_TOTAL_BYTES,
                    "--max-entries" => SearchLimits::MAX_ENTRIES as u64,
                    _ => SearchLimits::MAX_MATCHES as u64,
                };
                if number > maximum
                    || (number == 0 && matches!(option, "--max-entries" | "--max-matches"))
                {
                    return Err(usage(format!("{option} is outside its supported range")));
                }
                match option {
                    "--max-bytes" => set_once(&mut max_bytes, number, option)?,
                    "--max-total-bytes" => set_once(&mut max_total_bytes, number, option)?,
                    "--max-entries" => set_once(&mut max_entries, number as usize, option)?,
                    _ => set_once(&mut max_matches, number as usize, option)?,
                }
            }
            _ => unreachable!(),
        }
    }
    if revision.is_some() && saved.is_some() {
        return Err(usage("--revision and --saved are mutually exclusive"));
    }
    if action == Action::Inspect && path.is_some() {
        return Err(usage("inspect does not accept --path or --path-hex"));
    }
    if !matches!(action, Action::Read | Action::Search) && max_bytes.is_some() {
        return Err(usage("--max-bytes applies only to read or search"));
    }
    if action != Action::Search
        && (query.is_some() || max_total_bytes.is_some() || max_entries.is_some() || max_matches.is_some())
    {
        return Err(usage("search options apply only to search"));
    }
    if action == Action::Search && query.is_none() {
        return Err(usage("search requires --query"));
    }
    if query.as_ref().is_some_and(|bytes| {
        bytes.is_empty() || bytes.len() > SearchLimits::MAX_QUERY_BYTES || bytes.contains(&b'\n')
    }) {
        return Err(usage("--query must contain 1..4096 bytes and no newline"));
    }
    if action == Action::Read && path.as_ref().is_none_or(Vec::is_empty) {
        return Err(usage("read requires a nonempty --path or --path-hex"));
    }
    let defaults = SearchLimits::default();
    Ok(Some(Options {
        action,
        repository: repository.unwrap_or_else(|| PathBuf::from(".")),
        revision: revision.unwrap_or_else(|| "HEAD".to_owned()),
        saved,
        path: path.unwrap_or_default(),
        max_bytes: max_bytes.unwrap_or(DEFAULT_MAX_BLOB_BYTES),
        query,
        search_limits: SearchLimits {
            max_blob_bytes: max_bytes.unwrap_or(DEFAULT_MAX_BLOB_BYTES),
            max_total_bytes: max_total_bytes.unwrap_or(defaults.max_total_bytes),
            max_entries: max_entries.unwrap_or(defaults.max_entries),
            max_matches: max_matches.unwrap_or(defaults.max_matches),
        },
    }))
}

fn raw_name(bytes: &[u8]) -> Value {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 15) as usize] as char);
    }
    json!({ "hex": hex, "utf8": std::str::from_utf8(bytes).ok() })
}

fn identity(snapshot: &Snapshot) -> Value {
    json!({
        "object_format": snapshot.object_format().name(),
        "commit_oid": snapshot.commit_id().to_string(),
        "root_tree_oid": snapshot.tree_id().to_string(),
    })
}

fn entry(value: &SnapshotEntry) -> Value {
    let kind = match value.mode {
        0o040000 => "tree",
        0o100644 | 0o100755 => "blob",
        0o120000 => "symlink",
        0o160000 => "gitlink",
        _ => unreachable!("Snapshot validates entry modes before returning them"),
    };
    json!({
        "name": raw_name(&value.name),
        "mode": value.mode,
        "object_oid": value.oid.to_string(),
        "kind": kind,
    })
}

fn write_json(output: &mut impl Write, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *output, value)
        .map_err(|error| GitError::Command(format!("write snapshot JSON: {error}")))?;
    output.write_all(b"\n")?;
    Ok(())
}

pub(crate) fn run(args: &[OsString]) -> Result<()> {
    let mut output = io::stdout().lock();
    let Some(options) = parse(args)? else {
        output.write_all(HELP.as_bytes())?;
        return Ok(());
    };
    let snapshot = match options.saved {
        Some(name) => SnapshotCatalog::open(&options.repository)
            .and_then(|catalog| catalog.snapshot(&name))
            .map_err(|error| GitError::Command(error.to_string()))?,
        None => Snapshot::open(&options.repository, &options.revision)?,
    };
    match options.action {
        Action::Inspect => write_json(&mut output, &identity(&snapshot)),
        Action::List => {
            let entries = snapshot.list(&options.path)?;
            write_json(
                &mut output,
                &json!({
                    "snapshot": identity(&snapshot),
                    "path": raw_name(&options.path),
                    "entries": entries.iter().map(entry).collect::<Vec<_>>(),
                }),
            )
        }
        Action::Stat => {
            let selected = snapshot.stat(&options.path)?;
            write_json(
                &mut output,
                &json!({
                    "snapshot": identity(&snapshot),
                    "path": raw_name(&options.path),
                    "entry": entry(&selected),
                }),
            )
        }
        Action::Read => {
            // Complete verification precedes the first payload byte on stdout.
            let blob = snapshot.read(&options.path, options.max_bytes)?;
            output.write_all(blob.bytes())?;
            Ok(())
        }
        Action::Search => {
            let query = options.query.as_deref().expect("search parse requires a query");
            let report = snapshot.search(&options.path, query, options.search_limits)?;
            write_json(
                &mut output,
                &json!({
                    "snapshot": identity(&snapshot),
                    "path": raw_name(&options.path),
                    "query": raw_name(query),
                    "complete": true,
                    "scope": "regular-file-bytes",
                    "matches": report.matches.iter().map(|found| json!({
                        "path": raw_name(&found.path),
                        "blob_oid": found.blob_id.to_string(),
                        "byte_offset": found.byte_offset,
                        "byte_length": query.len(),
                        "line": found.line,
                        "byte_column": found.byte_column,
                    })).collect::<Vec<_>>(),
                    "scanned": {
                        "entries": report.entries,
                        "files": report.files,
                        "blob_bytes": report.blob_bytes,
                        "tree_bytes": report.tree_bytes,
                        "skipped_symlinks": report.skipped_symlinks,
                        "skipped_gitlinks": report.skipped_gitlinks,
                    },
                }),
            )
        }
    }
}
