// SPDX-License-Identifier: MIT OR Apache-2.0
//! Explicit Rit commands for the repository's saved snapshot catalog.

use std::{
    ffi::{OsStr, OsString},
    fmt,
    io::{self, Write},
    path::PathBuf,
};

use serde_json::{Value, json};

use crate::catalog::{CatalogError, SCHEMA_VERSION, SnapshotCatalog};

const HELP: &str = "\
usage: rit --rit-db <init|put|get|list|delete|pin> [options]

  --repo PATH              Repository to open, default: .
  --name NAME              Saved snapshot name; required for put/get/delete/pin
  --revision REV           Revision for put, default: HEAD
  --expect-generation N    Required for put/delete/pin; 0 requires an unused name for put
  --request-id ID          Required for put/delete; identifies one mutation
  --after NAME             Exclusive name cursor for list
  --limit N                List page size, default: 100; range: 1..1000
  --help                   Show this help

init creates a new repository catalog. Other commands never initialize it.
put saves the resolved commit and root tree. get and list include deletion
tombstones; use the returned generation when replacing or deleting a record.
Names and request IDs use 1..128 ASCII letters, digits, '.', '_' or '-',
starting with a letter or digit.

Repeat a request ID only for the same action, name, expected generation and
resolved snapshot. For retries after a branch moves, use the original full
commit ID. An exact retry returns its original result without applying it again.

Read a saved revision with `rit --rit-snapshot read --saved NAME --path PATH`.
Saved records alone do not retain Git objects against garbage collection.
`pin --name NAME --expect-generation N` creates a checked, create-only Git ref
for that exact generation, after verifying all reachable local objects. Pins
survive catalog replacement/deletion and have no automatic release command.
Do not run Git GC concurrently with pin publication. This catalog stores
metadata, not Git file payloads.

GIT_DIR and GIT_WORK_TREE do not redirect this catalog. Unset GIT_COMMON_DIR,
GIT_OBJECT_DIRECTORY, GIT_ALTERNATE_OBJECT_DIRECTORIES and GIT_REFERENCE_BACKEND
before use; ambient storage overrides are rejected.

Writer contention exits with status 75. Other failures exit with status 128.
";

#[derive(Debug)]
pub(crate) enum CliError {
    Catalog(CatalogError),
    Message(String),
    Io(io::Error),
    Json(serde_json::Error),
}

impl CliError {
    pub(crate) fn exit_code(&self) -> u8 {
        if matches!(self, Self::Catalog(CatalogError::Busy)) {
            75
        } else {
            128
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Catalog(error) => error.fmt(formatter),
            Self::Message(message) => formatter.write_str(message),
            Self::Io(error) => write!(formatter, "write catalog output: {error}"),
            Self::Json(error) => write!(formatter, "write catalog JSON: {error}"),
        }
    }
}

impl From<CatalogError> for CliError {
    fn from(error: CatalogError) -> Self {
        Self::Catalog(error)
    }
}

impl From<io::Error> for CliError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for CliError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

type Result<T> = std::result::Result<T, CliError>;

#[derive(Debug)]
enum Action {
    Init,
    Get {
        name: String,
    },
    List {
        after: Option<String>,
        limit: usize,
    },
    Put {
        name: String,
        revision: String,
        expected_generation: u64,
        request_id: String,
    },
    Delete {
        name: String,
        expected_generation: u64,
        request_id: String,
    },
    Pin {
        name: String,
        expected_generation: u64,
    },
}

#[derive(Debug)]
struct Options {
    repository: PathBuf,
    action: Action,
}

fn usage(message: impl Into<String>) -> CliError {
    CliError::Message(format!("{}; see `rit --rit-db --help`", message.into()))
}

fn utf8<'a>(value: &'a OsStr, option: &str) -> Result<&'a str> {
    value
        .to_str()
        .ok_or_else(|| usage(format!("{option} requires UTF-8 text")))
}

fn set_once<T>(slot: &mut Option<T>, value: T, option: &str) -> Result<()> {
    if slot.is_some() {
        return Err(usage(format!("{option} was specified more than once")));
    }
    *slot = Some(value);
    Ok(())
}

fn unsigned(value: &OsStr, option: &str) -> Result<u64> {
    let text = utf8(value, option)?;
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(usage(format!(
            "{option} must be an unsigned decimal integer"
        )));
    }
    text.parse()
        .map_err(|_| usage(format!("{option} is too large")))
}

fn required<T>(value: Option<T>, option: &str, action: &str) -> Result<T> {
    value.ok_or_else(|| usage(format!("{action} requires {option}")))
}

fn parse(args: &[OsString]) -> Result<Option<Options>> {
    let Some(first) = args.first() else {
        return Err(usage("a catalog action is required"));
    };
    let action = utf8(first, "catalog action")?;
    if matches!(action, "--help" | "-h") && args.len() == 1 {
        return Ok(None);
    }
    if !matches!(action, "init" | "put" | "get" | "list" | "delete" | "pin") {
        return Err(usage("expected init, put, get, list, delete, or pin"));
    }
    if args.len() == 2 && (args[1] == "--help" || args[1] == "-h") {
        return Ok(None);
    }
    let mut repository = None;
    let mut name = None;
    let mut revision = None;
    let mut expected_generation = None;
    let mut request_id = None;
    let mut after = None;
    let mut limit = None;
    let mut remaining = args[1..].iter();
    while let Some(option) = remaining.next() {
        let option = utf8(option, "option name")?;
        let allowed = match option {
            "--repo" => true,
            "--name" => matches!(action, "put" | "get" | "delete" | "pin"),
            "--revision" => action == "put",
            "--expect-generation" => matches!(action, "put" | "delete" | "pin"),
            "--request-id" => matches!(action, "put" | "delete"),
            "--after" | "--limit" => action == "list",
            _ => false,
        };
        if !allowed {
            return Err(usage(format!("{action} does not accept {option:?}")));
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
            "--name" => set_once(&mut name, utf8(value, option)?.to_owned(), option)?,
            "--revision" => {
                set_once(&mut revision, utf8(value, option)?.to_owned(), option)?;
            }
            "--expect-generation" => {
                set_once(&mut expected_generation, unsigned(value, option)?, option)?;
            }
            "--request-id" => {
                set_once(&mut request_id, utf8(value, option)?.to_owned(), option)?;
            }
            "--after" => set_once(&mut after, utf8(value, option)?.to_owned(), option)?,
            "--limit" => {
                let value = unsigned(value, option)?;
                if !(1..=1000).contains(&value) {
                    return Err(usage("--limit must be between 1 and 1000"));
                }
                set_once(&mut limit, value as usize, option)?;
            }
            _ => unreachable!("only applicable options reach their value parser"),
        }
    }
    let action = match action {
        "init" => Action::Init,
        "get" => Action::Get {
            name: required(name, "--name", action)?,
        },
        "list" => Action::List {
            after,
            limit: limit.unwrap_or(100),
        },
        "put" => Action::Put {
            name: required(name, "--name", action)?,
            revision: revision.unwrap_or_else(|| "HEAD".to_owned()),
            expected_generation: required(expected_generation, "--expect-generation", action)?,
            request_id: required(request_id, "--request-id", action)?,
        },
        "delete" => Action::Delete {
            name: required(name, "--name", action)?,
            expected_generation: required(expected_generation, "--expect-generation", action)?,
            request_id: required(request_id, "--request-id", action)?,
        },
        "pin" => Action::Pin {
            name: required(name, "--name", action)?,
            expected_generation: required(expected_generation, "--expect-generation", action)?,
        },
        _ => unreachable!("the action is validated before parsing options"),
    };
    Ok(Some(Options {
        repository: repository.unwrap_or_else(|| PathBuf::from(".")),
        action,
    }))
}

fn write_json(output: &mut impl Write, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")?;
    Ok(())
}

pub(crate) fn run(args: &[OsString]) -> Result<()> {
    let mut output = io::stdout().lock();
    let Some(options) = parse(args)? else {
        output.write_all(HELP.as_bytes())?;
        return Ok(());
    };
    match options.action {
        Action::Init => {
            let catalog = SnapshotCatalog::initialize(&options.repository)?;
            write_json(
                &mut output,
                &json!({
                    "schema_version": SCHEMA_VERSION,
                    "object_format": catalog.object_format().name(),
                    "database": catalog.database_path().display().to_string(),
                }),
            )
        }
        Action::Get { name } => {
            let catalog = SnapshotCatalog::open(&options.repository)?;
            let record = catalog.get(&name)?.ok_or_else(|| {
                CliError::Message(format!("saved snapshot {name:?} does not exist"))
            })?;
            write_json(&mut output, &json!(record))
        }
        Action::List { after, limit } => {
            let catalog = SnapshotCatalog::open(&options.repository)?;
            let records = catalog.list(after.as_deref(), limit)?;
            let next_after = records
                .last()
                .filter(|_| records.len() == limit)
                .map(|record| record.name.as_str());
            write_json(
                &mut output,
                &json!({ "records": &records, "next_after": next_after }),
            )
        }
        Action::Put {
            name,
            revision,
            expected_generation,
            request_id,
        } => {
            let catalog = SnapshotCatalog::open_writable(&options.repository)?;
            let result = catalog.put(&name, &revision, expected_generation, &request_id)?;
            write_json(&mut output, &json!(result))
        }
        Action::Delete {
            name,
            expected_generation,
            request_id,
        } => {
            let catalog = SnapshotCatalog::open_writable(&options.repository)?;
            let result = catalog.delete(&name, expected_generation, &request_id)?;
            write_json(&mut output, &json!(result))
        }
        Action::Pin {
            name,
            expected_generation,
        } => {
            // Pin does not mutate omdb. An existing catalog may be opened
            // read-only; only the native Git ref transaction changes storage.
            let catalog = SnapshotCatalog::open(&options.repository)?;
            let result = catalog.pin(&name, expected_generation)?;
            write_json(&mut output, &json!(result))
        }
    }
}
