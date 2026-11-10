// SPDX-License-Identifier: MIT OR Apache-2.0
//! Explicit Rit extension for canonical, checkout-free source reads.

use std::{
    ffi::{OsStr, OsString},
    io::{self, Write},
    path::PathBuf,
};

use serde_json::{Value, json};
use sley::{GitError, Result};

use crate::snapshot::{DEFAULT_MAX_BLOB_BYTES, MAX_BLOB_BYTES, Snapshot, SnapshotEntry};

const HELP: &str = "\
usage: rit --rit-snapshot <inspect|list|stat|read> [options]

  --repo PATH         Repository to open, default: .
  --revision REV      Commit ID, HEAD, ref name, or annotated tag, default: HEAD
  --path PATH         Exact repository-relative path; required for read
  --path-hex HEX      Path bytes encoded as hexadecimal, instead of --path
  --max-bytes N       Read limit, default: 8388608; maximum: 67108864
  --help              Show this help

inspect reports the resolved commit and root tree as JSON.
list reports one directory as JSON, without reading file payloads.
stat reports one entry as JSON. Empty paths select the root directory.
read writes the verified raw blob bytes, including symlink target bytes.

Revision expressions such as HEAD~1 are not accepted. Resolve them to a full
commit ID first. Paths are not normalized and symlinks are not followed.
Reads do not apply checkout filters or fetch missing objects. Snapshot IDs do
not retain objects against pruning. Selected objects are materialized in memory.

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
}

struct Options {
    action: Action,
    repository: PathBuf,
    revision: String,
    path: Vec<u8>,
    max_bytes: u64,
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
        "--help" | "-h" if args.len() == 1 => return Ok(None),
        _ => return Err(usage("expected inspect, list, stat, or read")),
    };
    let mut repository = None;
    let mut revision = None;
    let mut path = None;
    let mut max_bytes = None;
    let mut remaining = args[1..].iter();
    while let Some(option) = remaining.next() {
        let option = utf8(option, "option name")?;
        if option == "--help" || option == "-h" {
            return Ok(None);
        }
        if !matches!(
            option,
            "--repo" | "--revision" | "--path" | "--path-hex" | "--max-bytes"
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
            "--path" => set_once(&mut path, path_bytes(value)?, "--path/--path-hex")?,
            "--path-hex" => set_once(&mut path, from_hex(value)?, "--path/--path-hex")?,
            "--max-bytes" => {
                let text = utf8(value, option)?;
                if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(usage("--max-bytes must be an unsigned decimal integer"));
                }
                let number = text
                    .parse::<u64>()
                    .map_err(|_| usage("--max-bytes is too large"))?;
                if number > MAX_BLOB_BYTES {
                    return Err(usage(format!(
                        "--max-bytes must not exceed {MAX_BLOB_BYTES}"
                    )));
                }
                set_once(&mut max_bytes, number, option)?;
            }
            _ => unreachable!(),
        }
    }
    if action == Action::Inspect && path.is_some() {
        return Err(usage("inspect does not accept --path or --path-hex"));
    }
    if action != Action::Read && max_bytes.is_some() {
        return Err(usage("--max-bytes applies only to read"));
    }
    if action == Action::Read && path.as_ref().is_none_or(Vec::is_empty) {
        return Err(usage("read requires a nonempty --path or --path-hex"));
    }
    Ok(Some(Options {
        action,
        repository: repository.unwrap_or_else(|| PathBuf::from(".")),
        revision: revision.unwrap_or_else(|| "HEAD".to_owned()),
        path: path.unwrap_or_default(),
        max_bytes: max_bytes.unwrap_or(DEFAULT_MAX_BLOB_BYTES),
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
    let snapshot = Snapshot::open(&options.repository, &options.revision)?;
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
    }
}
