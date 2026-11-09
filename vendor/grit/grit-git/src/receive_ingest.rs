//! Receive-side pack ingestion helpers shared by `receive-pack` and local `push`.
//!
//! Matches Git `receive-pack` behaviour: choose `unpack-objects` vs `index-pack` from
//! `receive.unpacklimit` / `transfer.unpacklimit`, and enforce `receive.maxInputSize`.

use anyhow::{bail, Context, Result};
use grit_lib::config::ConfigSet;
use grit_lib::objects::ObjectId;
use grit_lib::odb::Odb;
use grit_lib::promisor::repo_treats_promisor_packs;
use grit_lib::receive_pack::{max_input_size_from_config, should_use_unpack_objects};
use grit_lib::unpack_objects::{unpack_objects, UnpackOptions};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::grit_exe;

/// Ingest a pack into `git_dir` using the same unpack path as Git `receive-pack`.
///
/// When `strict` is false (e.g. `receive-pack --skip-connectivity-check`), `unpack-objects` runs
/// without `--strict` so thin packs can store tips without requiring all bases in the ODB.
pub fn ingest_received_pack(
    git_dir: &Path,
    pack: &[u8],
    remote_cfg: &ConfigSet,
    strict: bool,
) -> Result<()> {
    ingest_received_pack_with_shallow(git_dir, pack, remote_cfg, strict, &HashSet::new())
}

/// Like [`ingest_received_pack`], but treats `shallow_boundaries` as grafts during the `--strict`
/// connectivity walk (their parents are not required). Mirrors `receive-pack` running
/// `unpack-objects --shallow-file <tmp>` for a shallow push.
pub fn ingest_received_pack_with_shallow(
    git_dir: &Path,
    pack: &[u8],
    remote_cfg: &ConfigSet,
    strict: bool,
    shallow_boundaries: &HashSet<ObjectId>,
) -> Result<()> {
    let max_input_bytes = max_input_size_from_config(remote_cfg);

    if should_use_unpack_objects(pack, remote_cfg) {
        let allow_promisor_missing = strict
            && (repo_treats_promisor_packs(git_dir, remote_cfg) || has_promisor_pack(git_dir));
        if allow_promisor_missing {
            return ingest_promisor_pack_in_process(
                git_dir,
                pack,
                max_input_bytes,
                strict,
                shallow_boundaries,
            );
        }
        ingest_via_unpack_objects_subprocess(
            git_dir,
            pack,
            max_input_bytes,
            strict,
            allow_promisor_missing,
            shallow_boundaries,
        )
    } else {
        ingest_via_index_pack_subprocess(git_dir, pack, max_input_bytes)
    }
}

fn ingest_promisor_pack_in_process(
    git_dir: &Path,
    pack: &[u8],
    max_input: Option<u64>,
    strict: bool,
    shallow_boundaries: &HashSet<ObjectId>,
) -> Result<()> {
    let objects_dir = git_dir.join("objects");
    let odb = Odb::new(&objects_dir);
    let opts = UnpackOptions {
        dry_run: false,
        quiet: true,
        strict,
        allowed_missing: Default::default(),
        allow_promisor_missing_references: true,
        max_input_bytes: max_input,
        shallow_boundaries: shallow_boundaries.clone(),
    };
    unpack_objects(&mut &pack[..], &odb, &opts)
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Write shallow boundary OIDs to a temporary file under `git_dir` for `--shallow-file`.
///
/// Returns `None` when the set is empty (no file is needed). The caller removes the file after the
/// subprocess completes.
fn write_temp_shallow_file(git_dir: &Path, boundaries: &HashSet<ObjectId>) -> Option<PathBuf> {
    if boundaries.is_empty() {
        return None;
    }
    let path = git_dir.join(format!("shallow_unpack_{}", std::process::id()));
    let mut body = String::new();
    for oid in boundaries {
        body.push_str(&oid.to_hex());
        body.push('\n');
    }
    std::fs::write(&path, body).ok()?;
    Some(path)
}

fn ingest_via_unpack_objects_subprocess(
    git_dir: &Path,
    pack: &[u8],
    max_input: Option<u64>,
    strict: bool,
    allow_promisor_missing: bool,
    shallow_boundaries: &HashSet<ObjectId>,
) -> Result<()> {
    let shallow_file = write_temp_shallow_file(git_dir, shallow_boundaries);
    let mut cmd = Command::new(grit_exe::grit_executable());
    grit_exe::strip_trace2_env(&mut cmd);
    cmd.arg(format!("--git-dir={}", git_dir.display()));
    if strict {
        cmd.args(["unpack-objects", "-q", "--strict"]);
    } else {
        cmd.args(["unpack-objects", "-q"]);
    }
    if let Some(ref sf) = shallow_file {
        cmd.arg("--shallow-file").arg(sf);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .env("GIT_DIR", git_dir.as_os_str());
    if allow_promisor_missing {
        cmd.env("GRIT_ALLOW_PROMISOR_MISSING_REFERENCES", "1");
    }
    if let Some(n) = max_input {
        cmd.arg(format!("--max-input-size={n}"));
    }
    let mut child = cmd.spawn().context("spawn grit unpack-objects")?;
    let mut stdin = child.stdin.take().context("unpack-objects stdin")?;
    stdin
        .write_all(pack)
        .context("write pack to unpack-objects stdin")?;
    drop(stdin);
    let out = child
        .wait_with_output()
        .context("wait for unpack-objects")?;
    if let Some(ref sf) = shallow_file {
        let _ = std::fs::remove_file(sf);
    }
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    bail!("unpack-objects abnormal exit: {stderr}");
}

fn has_promisor_pack(git_dir: &Path) -> bool {
    std::fs::read_dir(git_dir.join("objects").join("pack"))
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("promisor"))
}

fn ingest_via_index_pack_subprocess(
    git_dir: &Path,
    pack: &[u8],
    max_input: Option<u64>,
) -> Result<()> {
    let mut cmd = Command::new(grit_exe::grit_executable());
    grit_exe::strip_trace2_env(&mut cmd);
    cmd.arg(format!("--git-dir={}", git_dir.display()))
        .args(["index-pack", "--stdin", "--fix-thin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_DIR", git_dir.as_os_str());
    if let Some(n) = max_input {
        cmd.arg(format!("--max-input-size={n}"));
    }
    let mut child = cmd.spawn().context("spawn grit index-pack")?;
    let mut stdin = child.stdin.take().context("index-pack stdin")?;
    stdin
        .write_all(pack)
        .context("write pack to index-pack stdin")?;
    drop(stdin);
    let out = child.wait_with_output().context("wait for index-pack")?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    bail!("index-pack abnormal exit: {stderr}");
}
