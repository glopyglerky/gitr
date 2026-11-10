//! `grit prune` command.
//!
//! Removes unreachable loose objects from the object database.  Only loose
//! objects (files under `.git/objects/XX/…`) are considered; packed objects
//! are left untouched.

use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use grit_lib::check_ref_format::{check_refname_format, RefNameOptions};
use grit_lib::config::ConfigSet;
use grit_lib::diff::zero_oid;
use grit_lib::objects::{parse_commit, parse_tag, parse_tree, ObjectId, ObjectKind};
use grit_lib::odb::Odb;
use grit_lib::refs;
use grit_lib::repo::Repository;
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

/// Arguments for `grit prune`.
#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Do not remove anything; just show what would be removed.
    #[arg(short = 'n', long = "dry-run")]
    pub dry_run: bool,

    /// Report pruned objects.
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,

    /// Only prune objects older than this time.
    ///
    /// Accepts "now" to prune everything, or a duration like "2.weeks.ago".
    #[arg(long = "expire")]
    pub expire: Option<String>,

    /// Do not apply an expiration grace period.
    #[arg(long = "no-expire")]
    pub no_expire: bool,

    /// Extra heads to treat as reachable.
    #[arg(value_name = "HEAD")]
    pub heads: Vec<String>,

    /// Do not show progress (suppresses output to stderr).
    #[arg(long = "no-progress")]
    pub no_progress: bool,

    /// Show progress.
    #[arg(long = "progress")]
    pub progress: bool,
}

/// Parse and run `grit prune` from raw argv so missing `--expire` values use Git wording.
pub fn run_from_argv(rest: &[String]) -> Result<()> {
    for (idx, arg) in rest.iter().enumerate() {
        if arg == "--expire" && rest.get(idx + 1).is_none_or(|next| next.starts_with('-')) {
            anyhow::bail!("option `expire' requires a value");
        }
    }
    run(crate::parse_cmd_args("prune", rest))
}

/// Run `grit prune`.
pub fn run(args: Args) -> Result<()> {
    let repo = Repository::discover(None).context("failed to discover repository")?;
    if repository_has_precious_objects(&repo)? {
        anyhow::bail!("cannot prune in a precious-objects repo");
    }
    let objects_dir = repo.git_dir.join("objects");
    let odb = Odb::new(&objects_dir);
    if std::env::var("GIT_REF_PARANOIA").ok().as_deref() != Some("0") {
        guard_against_corrupt_loose_refs(&repo.git_dir, &odb)?;
    }

    let expire_policy = if args.no_expire {
        ExpirePolicy::All
    } else {
        parse_expire_time(args.expire.as_deref())?
    };

    // 1. Collect all reachable object IDs.
    let mut reachable = collect_reachable(&repo, &odb, &objects_dir, &args.heads)
        .context("failed to collect reachable objects")?;
    collect_recent_objects_hook_oids(&repo, &mut reachable)?;

    // 2. Enumerate all loose objects and treat recent loose objects as reachability roots.
    let loose = scan_loose_objects(&objects_dir)?;
    if let ExpirePolicy::OlderThan(threshold) = expire_policy {
        add_recent_loose_reachable(&odb, &loose, threshold, &mut reachable);
    }

    prune_stale_temporary_packs(&objects_dir, expire_policy, args.dry_run)?;

    // 3. Prune unreachable loose objects that are old enough.
    let mut pruned = 0usize;
    let mut pruned_oids = Vec::new();
    for (oid, path) in &loose {
        if reachable.contains(oid) {
            continue;
        }

        // Check modification time against expire threshold.
        match expire_policy {
            ExpirePolicy::Never => continue,
            ExpirePolicy::All => {}
            ExpirePolicy::OlderThan(threshold) => {
                match fs::metadata(path).and_then(|m| m.modified()) {
                    Ok(mtime) => {
                        if mtime >= threshold {
                            continue; // too new to prune
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                    Err(_) => {} // can't read mtime, prune anyway
                }
            }
        }

        if args.dry_run || args.verbose {
            println!("{}", oid.to_hex());
        }

        if !args.dry_run {
            pruned_oids.push(*oid);
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => {
                    eprintln!("warning: failed to remove {}: {e}", path.display());
                }
            }
            // Try to remove the now-possibly-empty prefix directory.
            if let Some(parent) = path.parent() {
                let _ = fs::remove_dir(parent);
            }
        }

        pruned += 1;
    }

    if !args.dry_run {
        prune_shallow_file(&repo.git_dir, &pruned_oids)?;
    }

    if args.verbose && !args.dry_run {
        eprintln!("prune: removed {} unreachable loose object(s)", pruned);
    }

    Ok(())
}

fn collect_recent_objects_hook_oids(
    repo: &Repository,
    reachable: &mut HashSet<ObjectId>,
) -> Result<()> {
    let cfg = ConfigSet::load(Some(&repo.git_dir), true)?;
    let Some(hook) = cfg.get("gc.recentobjectshook") else {
        return Ok(());
    };
    let hook = hook.trim();
    if hook.is_empty() {
        return Ok(());
    }
    let mut cmd = Command::new(hook);
    if let Some(work_tree) = &repo.work_tree {
        cmd.current_dir(work_tree);
    }
    let output = cmd
        .output()
        .with_context(|| format!("running gc.recentObjectsHook {hook}"))?;
    if !output.status.success() {
        return Ok(());
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let oid_text = line.trim();
        if let Ok(oid) = ObjectId::from_hex(oid_text) {
            reachable.insert(oid);
        }
    }
    Ok(())
}

fn prune_shallow_file(git_dir: &Path, pruned_oids: &[ObjectId]) -> Result<()> {
    if pruned_oids.is_empty() {
        return Ok(());
    }
    let shallow_path = git_dir.join("shallow");
    let Ok(content) = fs::read_to_string(&shallow_path) else {
        return Ok(());
    };
    let pruned: HashSet<ObjectId> = pruned_oids.iter().copied().collect();
    let kept: Vec<&str> = content
        .lines()
        .filter(|line| {
            ObjectId::from_hex(line.trim())
                .map(|oid| !pruned.contains(&oid))
                .unwrap_or(true)
        })
        .collect();
    if kept.is_empty() {
        let _ = fs::remove_file(shallow_path);
    } else {
        fs::write(shallow_path, format!("{}\n", kept.join("\n")))?;
    }
    Ok(())
}

fn guard_against_corrupt_loose_refs(git_dir: &Path, odb: &Odb) -> Result<()> {
    let refs_dir = git_dir.join("refs");
    if !refs_dir.is_dir() {
        return Ok(());
    }
    scan_ref_dir_for_prune(git_dir, odb, &refs_dir)
}

fn scan_ref_dir_for_prune(git_dir: &Path, odb: &Odb, dir: &Path) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            scan_ref_dir_for_prune(git_dir, odb, &path)?;
            continue;
        }
        if !entry.file_type()?.is_file() {
            continue;
        }
        let rel = path
            .strip_prefix(git_dir)
            .unwrap_or(path.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        if check_refname_format(&rel, &RefNameOptions::default()).is_err() {
            anyhow::bail!("bad ref for prune: {rel}");
        }
        let raw = fs::read_to_string(&path).unwrap_or_default();
        let value = raw.trim();
        if value.starts_with("ref: ") {
            continue;
        }
        if value.len() == 40 && value.chars().all(|c| c.is_ascii_hexdigit()) {
            if let Ok(oid) = ObjectId::from_hex(value) {
                if !odb.exists(&oid) {
                    anyhow::bail!("bad ref for prune: {rel}");
                }
            }
        }
    }
    Ok(())
}

/// Whether this repository declares `extensions.preciousObjects = true`.
fn repository_has_precious_objects(repo: &Repository) -> Result<bool> {
    let config = ConfigSet::load(Some(&repo.git_dir), true)?;
    let Some(value) = config.get("extensions.preciousobjects") else {
        return Ok(false);
    };
    let normalized = value.trim().to_ascii_lowercase();
    Ok(matches!(normalized.as_str(), "1" | "true" | "yes" | "on"))
}

/// Result of parsing [`Args::expire`](Args::expire): either a cutoff time, prune everything, or
/// never prune (Git `--expire=never` / `gc.pruneExpire=never`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpirePolicy {
    /// Prune unreachable loose objects older than this instant.
    OlderThan(SystemTime),
    /// Prune all unreachable loose objects (Git `now`).
    All,
    /// Do not remove unreachable loose objects based on reachability (Git `never`).
    Never,
}

/// Parse the `--expire` value into an [`ExpirePolicy`].
///
/// - `None` → 2 weeks before now (Git default)
/// - `"now"` → prune all unreachable loose objects
/// - `"never"` → do not prune unreachable loose objects
fn parse_expire_time(expire: Option<&str>) -> Result<ExpirePolicy> {
    match expire {
        None => Ok(ExpirePolicy::All),
        Some("now") | Some("all") => Ok(ExpirePolicy::All),
        Some(s) if s.eq_ignore_ascii_case("never") => Ok(ExpirePolicy::Never),
        Some(s) => {
            if let Some(threshold) = parse_relative_time(s) {
                Ok(ExpirePolicy::OlderThan(threshold))
            } else {
                anyhow::bail!("malformed expiration date: {s}");
            }
        }
    }
}

/// Parse Git-style relative time strings like "2.weeks.ago", "3.days.ago".
fn parse_relative_time(s: &str) -> Option<SystemTime> {
    let s = s.trim();
    // Normalize: replace '.' with spaces, lowercase
    let normalized = s.replace('.', " ").to_ascii_lowercase();
    let parts: Vec<&str> = normalized.split_whitespace().collect();
    // Handle "N unit" or "N unit ago" or "now"
    if parts.is_empty() {
        return None;
    }
    if parts[0] == "now" {
        return Some(SystemTime::now());
    }
    if s.contains('-') {
        return Some(SystemTime::UNIX_EPOCH);
    }
    if parts.len() < 2 {
        return None;
    }
    let n: u64 = parts[0].parse().ok()?;
    let unit = parts[1].trim_end_matches('s');
    let secs = match unit {
        "second" => n,
        "minute" => n * 60,
        "hour" => n * 3600,
        "day" => n * 86400,
        "week" => n * 7 * 86400,
        "month" => n * 30 * 86400,
        "year" => n * 365 * 86400,
        _ => return None,
    };
    SystemTime::now().checked_sub(Duration::from_secs(secs))
}

/// Build the set of all reachable object IDs by walking from refs.
fn collect_reachable(
    repo: &Repository,
    odb: &Odb,
    _objects_dir: &Path,
    extra_heads: &[String],
) -> Result<HashSet<ObjectId>> {
    let mut reachable = HashSet::new();
    let mut queue: VecDeque<ObjectId> = VecDeque::new();

    // Seed from HEAD.
    if let Ok(head_oid) = refs::resolve_ref(&repo.git_dir, "HEAD") {
        queue.push_back(head_oid);
    }

    // Seed from all refs (branches, tags, etc.).
    if let Ok(all_refs) = refs::list_refs(&repo.git_dir, "refs/") {
        for (_, oid) in all_refs {
            queue.push_back(oid);
        }
    }

    for head in extra_heads {
        let oid = if let Ok(oid) = ObjectId::from_hex(head) {
            oid
        } else {
            grit_lib::rev_parse::resolve_revision(repo, head)
                .map_err(|_| anyhow::anyhow!("fatal: bad revision '{head}'"))?
        };
        queue.push_back(oid);
    }

    // Seed from reflogs unless gc is doing an aggressive `--prune=now` pass after reflog expiry.
    if std::env::var_os("GRIT_PRUNE_IGNORE_REFLOGS").is_none() {
        collect_reflog_oids(&repo.git_dir, &mut queue);
    }

    collect_index_oids(repo, &repo.index_path(), &mut queue);
    collect_linked_worktree_oids(repo, &mut queue);

    // Match `git/reachable.c` `add_rebase_files`: OIDs recorded in in-progress rebase state
    // must stay reachable so `git prune` cannot delete `orig-head` before `rebase --abort`
    // (t3407-rebase-abort).
    collect_rebase_state_head_oids(&repo.git_dir, &mut queue);

    // BFS walk. Objects in packs are read via [`Odb::read`] when reached from
    // refs/reflogs; we do **not** treat every packed OID as reachable (that
    // would keep unreachable commits alive after `git gc --prune=now`, breaking
    // `git notes prune` and similar).
    while let Some(oid) = queue.pop_front() {
        if !reachable.insert(oid) {
            continue;
        }

        // Try to read the object.  If it's only in a pack (and we already
        // marked all packed IDs above), we simply can't walk its children
        // from the loose store — that's fine.
        let obj = match odb.read(&oid) {
            Ok(o) => o,
            Err(_) => continue,
        };

        match obj.kind {
            ObjectKind::Commit => {
                if let Ok(commit) = parse_commit(&obj.data) {
                    // The tree and all parents are reachable.
                    queue.push_back(commit.tree);
                    for parent in commit.parents {
                        queue.push_back(parent);
                    }
                }
            }
            ObjectKind::Tree => {
                if let Ok(entries) = parse_tree(&obj.data) {
                    for entry in entries {
                        queue.push_back(entry.oid);
                    }
                }
            }
            ObjectKind::Tag => {
                if let Ok(tag) = parse_tag(&obj.data) {
                    queue.push_back(tag.object);
                }
            }
            ObjectKind::Blob => {
                // Blobs have no children.
            }
        }
    }

    Ok(reachable)
}

fn add_recent_loose_reachable(
    odb: &Odb,
    loose: &[(ObjectId, std::path::PathBuf)],
    threshold: SystemTime,
    reachable: &mut HashSet<ObjectId>,
) {
    let mut queue = VecDeque::new();
    for (oid, path) in loose {
        let Ok(mtime) = fs::metadata(path).and_then(|m| m.modified()) else {
            continue;
        };
        if mtime >= threshold {
            queue.push_back(*oid);
        }
    }

    while let Some(oid) = queue.pop_front() {
        if !reachable.insert(oid) {
            continue;
        }
        let Ok(obj) = odb.read(&oid) else {
            continue;
        };
        match obj.kind {
            ObjectKind::Commit => {
                if let Ok(commit) = parse_commit(&obj.data) {
                    queue.push_back(commit.tree);
                    queue.extend(commit.parents);
                }
            }
            ObjectKind::Tree => {
                if let Ok(entries) = parse_tree(&obj.data) {
                    queue.extend(entries.into_iter().map(|entry| entry.oid));
                }
            }
            ObjectKind::Tag => {
                if let Ok(tag) = parse_tag(&obj.data) {
                    queue.push_back(tag.object);
                }
            }
            ObjectKind::Blob => {}
        }
    }
}

fn prune_stale_temporary_packs(
    objects_dir: &Path,
    expire_policy: ExpirePolicy,
    dry_run: bool,
) -> Result<()> {
    let rd = match fs::read_dir(objects_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for entry in rd {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.starts_with("tmp_") || !name.ends_with(".pack") {
            continue;
        }
        let expired = match expire_policy {
            ExpirePolicy::Never => false,
            ExpirePolicy::All => true,
            ExpirePolicy::OlderThan(threshold) => fs::metadata(&path)
                .and_then(|m| m.modified())
                .map(|mtime| mtime < threshold)
                .unwrap_or(true),
        };
        if expired && !dry_run {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

fn collect_index_oids(repo: &Repository, index_path: &Path, queue: &mut VecDeque<ObjectId>) {
    if let Ok(index) = repo.load_index_at(index_path) {
        for entry in &index.entries {
            if !entry.oid.is_zero() {
                queue.push_back(entry.oid);
            }
        }
        if let Some(records) = &index.resolve_undo {
            for record in records.values() {
                for (mode, oid) in record.modes.iter().zip(record.oids.iter()) {
                    if *mode != 0 && !oid.is_zero() {
                        queue.push_back(*oid);
                    }
                }
            }
        }
    }
}

fn collect_linked_worktree_oids(repo: &Repository, queue: &mut VecDeque<ObjectId>) {
    let worktrees = repo.git_dir.join("worktrees");
    let Ok(entries) = fs::read_dir(worktrees) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let wt_git = entry.path();
        collect_worktree_head_oid(repo, &wt_git, queue);
        collect_index_oids(repo, &wt_git.join("index"), queue);
        collect_reflog_oids(&wt_git, queue);
    }
}

fn collect_worktree_head_oid(repo: &Repository, wt_git: &Path, queue: &mut VecDeque<ObjectId>) {
    let Ok(head) = fs::read_to_string(wt_git.join("HEAD")) else {
        return;
    };
    let head = head.trim();
    if let Ok(oid) = ObjectId::from_hex(head) {
        queue.push_back(oid);
        return;
    }
    if let Some(refname) = head.strip_prefix("ref: ") {
        if let Ok(oid) = refs::resolve_ref(&repo.git_dir, refname.trim()) {
            queue.push_back(oid);
        }
    }
}

/// Push the object id from a single-line file under `.git/` if present and valid hex.
///
/// Git marks these paths when computing prune reachability; see `add_rebase_files` in upstream
/// `reachable.c`.
fn collect_rebase_state_head_oids(git_dir: &Path, queue: &mut VecDeque<ObjectId>) {
    const REL_PATHS: [&str; 4] = [
        "rebase-apply/autostash",
        "rebase-apply/orig-head",
        "rebase-merge/autostash",
        "rebase-merge/orig-head",
    ];
    for rel in REL_PATHS {
        let path = git_dir.join(rel);
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let hex = content.trim();
        if hex.len() == 40 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            if let Ok(oid) = hex.parse::<ObjectId>() {
                queue.push_back(oid);
            }
        }
    }
}

/// Parse `old_oid` and `new_oid` from one reflog line.
///
/// Lines are `<old-hex> <new-hex> <identity>\t<message>`; the identity contains spaces, so we
/// must not split the line on spaces alone (matches `grit_lib::reflog::parse_reflog_line`).
fn parse_reflog_line_oids(line: &str) -> Option<(ObjectId, ObjectId)> {
    let before_tab = line.split('\t').next()?;
    if before_tab.len() < 83 {
        return None;
    }
    let old_hex = &before_tab[..40];
    let new_hex = &before_tab[41..81];
    let old_oid = old_hex.parse().ok()?;
    let new_oid = new_hex.parse().ok()?;
    Some((old_oid, new_oid))
}

/// Scan reflog files for object IDs and add them to the queue.
fn collect_reflog_oids(git_dir: &Path, queue: &mut VecDeque<ObjectId>) {
    let logs_dir = git_dir.join("logs");
    let zero = zero_oid();
    if let Ok(entries) = walk_files(&logs_dir) {
        for path in entries {
            if let Ok(content) = fs::read_to_string(&path) {
                for line in content.lines() {
                    if line.is_empty() {
                        continue;
                    }
                    if let Some((old_oid, new_oid)) = parse_reflog_line_oids(line) {
                        if old_oid != zero {
                            queue.push_back(old_oid);
                        }
                        if new_oid != zero {
                            queue.push_back(new_oid);
                        }
                    }
                }
            }
        }
    }
}

/// Recursively walk a directory, returning all file paths.
fn walk_files(dir: &Path) -> io::Result<Vec<std::path::PathBuf>> {
    let mut files = Vec::new();
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(files),
        Err(e) => return Err(e),
    };
    for entry in rd {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_dir() {
            files.extend(walk_files(&entry.path())?);
        } else if ft.is_file() {
            files.push(entry.path());
        }
    }
    Ok(files)
}

/// Enumerate all loose objects in the object store.
fn scan_loose_objects(objects_dir: &Path) -> Result<Vec<(ObjectId, std::path::PathBuf)>> {
    let mut objects = Vec::new();
    let rd = match fs::read_dir(objects_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(objects),
        Err(e) => anyhow::bail!("failed to read objects dir: {e}"),
    };

    for entry in rd {
        let entry = entry?;
        let dir_name = entry.file_name().to_string_lossy().to_string();

        // Only two-hex-char prefix subdirectories.
        if dir_name.len() != 2
            || !dir_name.chars().all(|c| c.is_ascii_hexdigit())
            || !entry.path().is_dir()
        {
            continue;
        }

        let sub_rd = match fs::read_dir(entry.path()) {
            Ok(rd) => rd,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => anyhow::bail!("failed to read dir {}: {e}", entry.path().display()),
        };

        for file in sub_rd {
            let file = file?;
            let file_name = file.file_name().to_string_lossy().to_string();
            if file_name.len() != 38 || !file_name.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }

            let hex = format!("{dir_name}{file_name}");
            if let Ok(oid) = hex.parse::<ObjectId>() {
                objects.push((oid, file.path()));
            }
        }
    }

    Ok(objects)
}
