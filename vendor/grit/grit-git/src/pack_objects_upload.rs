//! Spawn `pack-objects` for upload-pack (hook or direct `grit`), write rev-list stdin, stream stdout.

use anyhow::{bail, Context, Result};
use grit_lib::config::ConfigSet;
use grit_lib::objects::{parse_commit, parse_tag, ObjectId, ObjectKind};
use grit_lib::repo::Repository;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::{collections::HashSet, collections::VecDeque};

use crate::grit_exe::grit_executable;

fn resolve_hook_path(git_dir: &Path, hook: &str) -> PathBuf {
    let p = Path::new(hook);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        git_dir.join(p)
    }
}

/// Build and spawn `git pack-objects` (via hook or `grit pack-objects`).
///
/// When `thin` is true, omit objects the client already has (requires matching `have` lines).
/// For a fetch/clone with no common objects, pass `false` so the pack is self-contained.
pub fn spawn_pack_objects_upload(
    git_dir: &Path,
    thin: bool,
    filter_spec: Option<&str>,
) -> Result<Child> {
    spawn_pack_objects_upload_shallow(git_dir, thin, filter_spec, false, true, false, false)
}

/// Like [`spawn_pack_objects_upload`], but passes `--shallow` so `--shallow <oid>` stdin lines cut
/// the parent chains of those commits (used to produce a depth-limited shallow pack).
///
/// `progress` controls whether `pack-objects` is run with `--progress` (emitting the
/// `Total N (delta ...)` summary on stderr) or `--quiet`. Git's upload-pack passes `--progress`
/// only when the client did **not** request `no-progress`; honoring that keeps a non-tty fetch's
/// stderr clean (t5574).
pub fn spawn_pack_objects_upload_shallow(
    git_dir: &Path,
    thin: bool,
    filter_spec: Option<&str>,
    shallow: bool,
    progress: bool,
    omit_missing_promisor: bool,
    force_lazy_fetch: bool,
) -> Result<Child> {
    let protected = ConfigSet::load_protected(true).unwrap_or_default();
    let hook_raw = protected.get("uploadpack.packobjectshook");
    let grit = grit_executable();

    let mut cmd = if let Some(ref hook_path) = hook_raw {
        let hook_resolved = resolve_hook_path(git_dir, hook_path);
        let mut c = Command::new("sh");
        c.arg("-c")
            .arg("exec \"$0\" \"$@\"")
            .arg(&hook_resolved)
            .arg("git")
            .arg("pack-objects")
            .arg("--revs");
        if thin {
            c.arg("--thin");
        }
        if shallow {
            c.arg("--shallow");
        }
        if let Some(spec) = filter_spec.map(str::trim).filter(|s| !s.is_empty()) {
            c.arg(format!("--filter={spec}"));
        }
        c.arg("--stdout")
            .arg(if progress { "--progress" } else { "--quiet" })
            .arg("--delta-base-offset");
        c
    } else {
        let mut c = Command::new(&grit);
        c.arg("pack-objects").arg("--revs");
        if thin {
            c.arg("--thin");
        }
        if shallow {
            c.arg("--shallow");
        }
        if let Some(spec) = filter_spec.map(str::trim).filter(|s| !s.is_empty()) {
            c.arg(format!("--filter={spec}"));
        }
        c.arg("--stdout")
            .arg(if progress { "--progress" } else { "--quiet" })
            .arg("--delta-base-offset");
        c
    };

    if omit_missing_promisor {
        // Tells the spawned `pack-objects` to omit (rather than lazily fetch) objects missing from
        // the local promisor-enabled ODB when applying `--filter` — used when the fetching client
        // accepted an advertised promisor remote (`t5710`).
        cmd.env("GRIT_OMIT_MISSING_PROMISOR", "1");
    } else if force_lazy_fetch {
        // `upload-pack` disables lazy fetching by default, but when the server did not advertise a
        // promisor remote the filtered pack must back-fill omitted blobs before serving the client.
        cmd.env("GIT_NO_LAZY_FETCH", "0");
    }

    cmd.current_dir(git_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            if hook_raw.is_some() {
                anyhow::anyhow!("failed to spawn pack-objects hook")
            } else {
                anyhow::anyhow!("failed to spawn '{} pack-objects'", grit.display())
            }
        })
}

/// Write the stdin Git's `pack-objects --revs` expects (`--not` + exclusion commit OIDs).
pub fn write_pack_objects_revs_stdin(
    pin: &mut impl Write,
    wants: &[ObjectId],
    exclude_commits: &[ObjectId],
) -> Result<()> {
    write_pack_objects_revs_stdin_shallow(pin, wants, exclude_commits, &[])
}

/// Like [`write_pack_objects_revs_stdin`], but also emits `--shallow <oid>` lines (consumed by
/// `pack-objects --shallow`) so the parent chains of `shallow_commits` are cut at the depth
/// boundary while their full trees remain in the pack.
pub fn write_pack_objects_revs_stdin_shallow(
    pin: &mut impl Write,
    wants: &[ObjectId],
    exclude_commits: &[ObjectId],
    shallow_commits: &[ObjectId],
) -> Result<()> {
    for s in shallow_commits {
        writeln!(pin, "--shallow {}", s.to_hex())?;
    }
    for w in wants {
        writeln!(pin, "{}", w.to_hex())?;
    }
    writeln!(pin, "--not")?;
    for h in exclude_commits {
        writeln!(pin, "{}", h.to_hex())?;
    }
    writeln!(pin)?;
    pin.flush()?;
    Ok(())
}

/// Compute commit OIDs to exclude for a depth-limited upload-pack response.
///
/// The returned OIDs are parent commits just beyond the requested depth from the wanted tips.
/// Passing these OIDs after `--not` to `pack-objects --revs` keeps history at or above the depth
/// boundary while allowing boundary commits themselves to be included.
pub fn compute_depth_exclude_commits(
    repo: &Repository,
    wants: &[ObjectId],
    depth: usize,
) -> Result<Vec<ObjectId>> {
    if depth == 0 || wants.is_empty() || depth >= i32::MAX as usize {
        return Ok(Vec::new());
    }

    let mut queue: VecDeque<(ObjectId, usize)> = VecDeque::new();
    let mut seen: std::collections::HashMap<ObjectId, usize> = std::collections::HashMap::new();

    for want in wants {
        if let Some(commit_oid) = peel_commit_oid(repo, *want)? {
            if seen.insert(commit_oid, 0).is_none() {
                queue.push_back((commit_oid, 0));
            }
        }
    }

    let mut excludes: HashSet<ObjectId> = HashSet::new();
    while let Some((oid, dist)) = queue.pop_front() {
        let obj = match repo.odb.read(&oid) {
            Ok(obj) => obj,
            Err(_) => continue,
        };
        if obj.kind != ObjectKind::Commit {
            continue;
        }
        let commit = parse_commit(&obj.data)?;
        if dist + 1 >= depth {
            excludes.extend(commit.parents.iter().copied());
            continue;
        }
        for parent in commit.parents {
            let next_dist = dist + 1;
            if seen
                .get(&parent)
                .is_some_and(|existing| *existing <= next_dist)
            {
                continue;
            }
            seen.insert(parent, next_dist);
            queue.push_back((parent, next_dist));
        }
    }

    let mut out: Vec<ObjectId> = excludes.into_iter().collect();
    out.sort_by_key(|oid| oid.to_hex());
    Ok(out)
}

/// Compute the shallow-boundary commits for a depth-limited upload-pack response.
///
/// These are the deepest in-depth commits (those at distance `depth - 1` from a wanted tip that
/// still have parents). Passing them as `--shallow <oid>` to `pack-objects` cuts their parent
/// chains so history stops at the boundary, while every object reachable from the wanted commits'
/// trees — including blobs that were first introduced beyond the boundary but are still referenced
/// by an in-depth tree — is fully included.
///
/// This is the correct shallow-pack mechanism: a plain `--not <boundary-parent>` exclusion would
/// drop blobs/trees shared between the in-depth commits and the cut-off history (e.g. a file added
/// in the first commit and never modified), producing an inconsistent shallow repo that fails
/// `git fsck` (t5537 "shallow fetches check connectivity before writing shallow file").
pub fn compute_depth_boundary_commits(
    repo: &Repository,
    wants: &[ObjectId],
    depth: usize,
) -> Result<Vec<ObjectId>> {
    if depth == 0 || wants.is_empty() || depth >= i32::MAX as usize {
        return Ok(Vec::new());
    }

    let mut queue: VecDeque<(ObjectId, usize)> = VecDeque::new();
    let mut seen: std::collections::HashMap<ObjectId, usize> = std::collections::HashMap::new();

    for want in wants {
        if let Some(commit_oid) = peel_commit_oid(repo, *want)? {
            if seen.insert(commit_oid, 0).is_none() {
                queue.push_back((commit_oid, 0));
            }
        }
    }

    let mut boundary: HashSet<ObjectId> = HashSet::new();
    while let Some((oid, dist)) = queue.pop_front() {
        let obj = match repo.odb.read(&oid) {
            Ok(obj) => obj,
            Err(_) => continue,
        };
        if obj.kind != ObjectKind::Commit {
            continue;
        }
        let commit = parse_commit(&obj.data)?;
        if dist + 1 >= depth {
            // This commit is at the depth boundary; its parents are cut off. Only commits that
            // actually have parents become shallow grafts (a root commit needs no graft).
            if !commit.parents.is_empty() {
                boundary.insert(oid);
            }
            continue;
        }
        for parent in commit.parents {
            let next_dist = dist + 1;
            if seen
                .get(&parent)
                .is_some_and(|existing| *existing <= next_dist)
            {
                continue;
            }
            seen.insert(parent, next_dist);
            queue.push_back((parent, next_dist));
        }
    }

    let mut out: Vec<ObjectId> = boundary.into_iter().collect();
    out.sort_by_key(|oid| oid.to_hex());
    Ok(out)
}

/// Compute exclusion commits for `--deepen=<n>` when the client is already shallow.
///
/// The client advertises shallow boundary commits via `shallow <oid>` lines. Relative deepening
/// should extend history by `deepen` commits beyond the nearest advertised boundary, rather than
/// applying `deepen` as an absolute depth from the new tip.
pub fn compute_relative_deepen_exclude_commits(
    repo: &Repository,
    wants: &[ObjectId],
    shallow_boundaries: &HashSet<ObjectId>,
    deepen: usize,
) -> Result<Vec<ObjectId>> {
    if deepen == 0 || wants.is_empty() || shallow_boundaries.is_empty() {
        return Ok(Vec::new());
    }

    let mut min_boundary_distance: Option<usize> = None;
    for want in wants {
        let Some(commit_oid) = peel_commit_oid(repo, *want)? else {
            continue;
        };
        let mut queue: VecDeque<(ObjectId, usize)> = VecDeque::from([(commit_oid, 0)]);
        let mut seen: HashSet<ObjectId> = HashSet::new();
        while let Some((oid, dist)) = queue.pop_front() {
            if !seen.insert(oid) {
                continue;
            }
            if shallow_boundaries.contains(&oid) {
                min_boundary_distance = Some(match min_boundary_distance {
                    Some(cur) => cur.min(dist),
                    None => dist,
                });
                continue;
            }
            let Ok(obj) = repo.odb.read(&oid) else {
                continue;
            };
            if obj.kind != ObjectKind::Commit {
                continue;
            }
            let commit = parse_commit(&obj.data)?;
            for parent in commit.parents {
                queue.push_back((parent, dist + 1));
            }
        }
    }

    let effective_depth = min_boundary_distance
        .map(|d| d + 1 + deepen)
        .unwrap_or(deepen);
    compute_depth_exclude_commits(repo, wants, effective_depth)
}

fn peel_commit_oid(repo: &Repository, mut oid: ObjectId) -> Result<Option<ObjectId>> {
    loop {
        let obj = match repo.odb.read(&oid) {
            Ok(obj) => obj,
            Err(_) => return Ok(None),
        };
        match obj.kind {
            ObjectKind::Commit => return Ok(Some(oid)),
            ObjectKind::Tag => {
                let tag = parse_tag(&obj.data)?;
                oid = tag.object;
            }
            _ => return Ok(None),
        }
    }
}

pub(crate) fn write_sideband_64k(w: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    const MAX_PAYLOAD: usize = 65515;
    for chunk in payload.chunks(MAX_PAYLOAD) {
        let len = 4 + 1 + chunk.len();
        write!(w, "{len:04x}")?;
        w.write_all(&[1u8])?;
        w.write_all(chunk)?;
    }
    Ok(())
}

/// A valid empty PACK v2 (header + SHA1 trailer) for upload-pack when there is nothing to send.
pub fn empty_packfile_v2_bytes() -> Vec<u8> {
    use sha1::{Digest, Sha1};
    let mut buf = Vec::new();
    buf.extend_from_slice(b"PACK");
    buf.extend_from_slice(&2u32.to_be_bytes());
    buf.extend_from_slice(&0u32.to_be_bytes());
    let mut hasher = Sha1::new();
    hasher.update(&buf);
    buf.extend_from_slice(hasher.finalize().as_slice());
    buf
}

/// Read pack bytes from `child` and write to `out`, optionally wrapping in side-band-64k (v0).
pub fn drain_pack_objects_child(
    mut child: Child,
    out: &mut impl Write,
    sideband: bool,
) -> Result<()> {
    let mut pack_out = child.stdout.take().context("pack-objects stdout")?;
    let stderr_child = child.stderr.take();
    let stderr_handle = std::thread::spawn(move || {
        if let Some(mut e) = stderr_child {
            let mut buf = Vec::new();
            let _ = e.read_to_end(&mut buf);
            buf
        } else {
            Vec::new()
        }
    });

    const CHUNK: usize = 32000;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = pack_out.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if sideband {
            write_sideband_64k(out, &buf[..n])?;
        } else {
            out.write_all(&buf[..n])?;
        }
    }

    let status = child.wait()?;
    let err_bytes = stderr_handle.join().unwrap_or_default();
    if !err_bytes.is_empty() {
        let _ = io::stderr().write_all(&err_bytes);
    }
    if !status.success() {
        // Mirror `upload-pack.c`: `error("git upload-pack: git-pack-objects died with error.")`.
        // The child's own diagnostic (e.g. "unable to read <oid>" or "bad tree object <oid>") has
        // already been forwarded to our stderr above; this adds the "pack-objects died" line the
        // client/test greps for (t5530 packing and enumeration errors).
        bail!("git upload-pack: git-pack-objects died with error.");
    }
    Ok(())
}
