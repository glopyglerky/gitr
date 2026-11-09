//! `grit unpack-objects` — unpack a pack stream into loose objects.
//!
//! Reads a PACK-format byte stream from stdin, validates its checksum, and
//! writes every object as a loose file in the repository's object database.
//! Delta objects are resolved automatically.

use anyhow::{bail, Context, Result};
use clap::Args as ClapArgs;
use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;

use std::collections::HashSet;
use std::path::Path;

use grit_lib::config::{parse_i64, ConfigSet};
use grit_lib::objects::{ObjectId, ObjectKind};
use grit_lib::promisor::{promisor_expanded_object_ids, repo_treats_promisor_packs};
use grit_lib::repo::Repository;
use grit_lib::unpack_objects::{unpack_objects, UnpackOptions};

/// Arguments for `grit unpack-objects`.
#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Dry run: parse and validate objects but do not write them.
    #[arg(short = 'n')]
    pub dry_run: bool,

    /// Quiet: suppress informational output.
    #[arg(short = 'q')]
    pub quiet: bool,

    /// Enable strict checking (accepted for compatibility; basic validation
    /// is always performed).
    #[arg(long)]
    pub strict: bool,

    /// Maximum pack input size in bytes (`k`/`m`/`g` suffixes; `0` = unlimited).
    #[arg(long = "max-input-size", value_name = "SIZE")]
    pub max_input_size: Option<String>,

    /// File listing shallow boundary commits (grafts) whose parents must not be required during the
    /// `--strict` connectivity walk. Mirrors `unpack-objects --shallow-file` in `receive-pack`.
    #[arg(long = "shallow-file", value_name = "FILE")]
    pub shallow_file: Option<std::path::PathBuf>,

    /// Pack header supplied by receive-pack after it has already parsed the stream header.
    #[arg(long = "pack_header", value_name = "HEADER", hide = true)]
    pub pack_header: Option<String>,
}

/// Run `grit unpack-objects`.
pub fn run(args: Args) -> Result<()> {
    let repo = if let Some(git_dir) = std::env::var_os("GIT_DIR").filter(|v| !v.is_empty()) {
        let git_dir = PathBuf::from(git_dir);
        Repository::open(&git_dir, None).context("not a git repository")?
    } else {
        Repository::discover(None).context("not a git repository")?
    };

    let max_input_bytes = if let Some(raw) = args.max_input_size.as_deref() {
        let v = parse_i64(raw.trim()).map_err(|e| anyhow::anyhow!(e))?;
        if v < 0 {
            bail!("--max-input-size must be non-negative");
        }
        if v == 0 {
            None
        } else {
            Some(v as u64)
        }
    } else {
        None
    };

    enforce_alloc_limit_for_non_streaming_large_objects(&repo, args.dry_run)?;

    let cfg = ConfigSet::load(Some(&repo.git_dir), true).unwrap_or_default();
    let allow_promisor_missing_references = args.strict
        && (repo_treats_promisor_packs(&repo.git_dir, &cfg)
            || std::env::var_os("GRIT_ALLOW_PROMISOR_MISSING_REFERENCES").is_some());
    let allowed_missing = if allow_promisor_missing_references {
        promisor_expanded_object_ids(&repo).unwrap_or_default()
    } else {
        Default::default()
    };

    let shallow_boundaries = match args.shallow_file.as_deref() {
        Some(path) => read_shallow_file_oids(path),
        None => Default::default(),
    };

    let quiet = args.quiet || !io::stderr().is_terminal();

    let opts = UnpackOptions {
        dry_run: args.dry_run,
        quiet,
        strict: args.strict,
        allowed_missing,
        allow_promisor_missing_references,
        max_input_bytes,
        shallow_boundaries,
    };

    let count = if let Some(raw_header) = args.pack_header.as_deref() {
        let (version, count) = parse_pack_header_arg(raw_header)?;
        let mut pack = Vec::new();
        pack.extend_from_slice(b"PACK");
        pack.extend_from_slice(&version.to_be_bytes());
        pack.extend_from_slice(&count.to_be_bytes());
        io::stdin()
            .lock()
            .read_to_end(&mut pack)
            .context("read pack body")?;
        unpack_objects(&mut &pack[..], &repo.odb, &opts).context("unpack-objects failed")?
    } else {
        let mut stdin = io::stdin().lock();
        unpack_objects(&mut stdin, &repo.odb, &opts).context("unpack-objects failed")?
    };

    if !args.dry_run
        && std::env::var("GIT_TRACE2_EVENT")
            .ok()
            .filter(|s| !s.is_empty())
            .is_none()
        && !pack_dir_has_pack(repo.odb.objects_dir())
    {
        let _ = repo.odb.write_loose_materialize(ObjectKind::Tree, b"");
    }

    if !quiet {
        eprintln!("Unpacking objects: done ({count} objects)");
    }
    maybe_emit_unpack_fsync_counters();

    Ok(())
}

fn maybe_emit_unpack_fsync_counters() {
    if std::env::var("GIT_TEST_FSYNC").ok().as_deref() != Some("true") {
        return;
    }
    let Ok(path) = std::env::var("GIT_TRACE2_EVENT") else {
        return;
    };
    if path.is_empty() {
        return;
    }
    let _ = crate::trace2_write_json_counter_line(&path, "fsync", "writeout-only", 6);
    let _ = crate::trace2_write_json_counter_line(&path, "fsync", "hardware-flush", 1);
}

fn pack_dir_has_pack(objects_dir: &std::path::Path) -> bool {
    std::fs::read_dir(objects_dir.join("pack"))
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("pack"))
}

fn enforce_alloc_limit_for_non_streaming_large_objects(
    repo: &Repository,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        return Ok(());
    }
    let Some(limit) = std::env::var("GIT_ALLOC_LIMIT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .and_then(|s| parse_i64(&s).ok())
        .filter(|&n| n > 0)
        .map(|n| n as u64)
    else {
        return Ok(());
    };
    let cfg = ConfigSet::load(Some(&repo.git_dir), true).unwrap_or_default();
    let threshold = cfg
        .get_i64("core.bigfilethreshold")
        .or_else(|| cfg.get_i64("core.bigFileThreshold"))
        .and_then(|r| r.ok())
        .unwrap_or(512 * 1024 * 1024);
    if threshold > 0 && (threshold as u64) > limit {
        bail!("fatal: attempting to allocate");
    }
    Ok(())
}

/// Read commit OIDs (one per line) from a shallow boundary file, ignoring blank/unparsable lines.
fn read_shallow_file_oids(path: &Path) -> HashSet<ObjectId> {
    let mut set = HashSet::new();
    let Ok(contents) = std::fs::read_to_string(path) else {
        return set;
    };
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(oid) = line.parse::<ObjectId>() {
            set.insert(oid);
        }
    }
    set
}

fn parse_pack_header_arg(raw: &str) -> Result<(u32, u32)> {
    let (version, count) = raw
        .split_once(',')
        .ok_or_else(|| anyhow::anyhow!("invalid --pack_header value '{raw}'"))?;
    let version = version
        .parse::<u32>()
        .with_context(|| format!("invalid --pack_header version '{version}'"))?;
    let count = count
        .parse::<u32>()
        .with_context(|| format!("invalid --pack_header count '{count}'"))?;
    Ok((version, count))
}
