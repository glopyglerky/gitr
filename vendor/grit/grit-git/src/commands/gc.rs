//! `grit gc` — repository housekeeping.
//!
//! Runs [`prune_packed_objects`](grit_lib::prune_packed::prune_packed_objects),
//! [`pack_refs`](crate::commands::pack_refs), optional [`reflog`](grit_lib::reflog) expiry
//! (`gc.reflogExpire`, `gc.reflogExpireUnreachable`), optional [`commit-graph`](crate::commands::commit_graph) writes
//! (`gc.writeCommitGraph`), and [`repack`](crate::commands::repack) **`-d -l`** to pack objects
//! (including **`--aggressive`** and cruft / keep-largest-pack forwarding).

use crate::commands::pack_refs;
use crate::commands::prune;
use crate::commands::repack;
use crate::commands::update_server_info;
use crate::grit_exe;
use crate::{trace2_emit_git_subcommand_argv, trace_run_command_git_invocation};
use anyhow::{bail, Context, Result};
use clap::Args as ClapArgs;
use grit_lib::config::ConfigSet;
use grit_lib::hooks::{run_hook, HookResult};
use grit_lib::objects::ObjectId;
use grit_lib::pack::read_pack_index;
use grit_lib::promisor::{promisor_pack_object_ids, repo_treats_promisor_packs};
use grit_lib::prune_packed::{prune_packed_objects, PrunePackedOptions};
use grit_lib::reflog::{expire_reflog, expire_reflog_unreachable, list_reflog_refs};
use grit_lib::repo::Repository;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

/// Arguments for `grit gc`.
#[derive(Debug, ClapArgs)]
#[command(about = "Cleanup unnecessary files and optimize the local repository")]
pub struct Args {
    /// More aggressive optimization (accepted; repack tuning not wired yet).
    #[arg(long)]
    pub aggressive: bool,

    /// Only run if [`gc.auto`](https://git-scm.com/docs/git-config#Documentation/git-config.txt-gcauto)
    /// heuristics say housekeeping is needed.
    #[arg(long)]
    pub auto: bool,

    /// Suppress informational messages (including auto-gc notices).
    #[arg(long, short = 'q')]
    pub quiet: bool,

    /// Show progress even when stderr is not a terminal (accepted for tests).
    #[arg(long = "no-quiet")]
    pub no_quiet: bool,

    /// Force even if another gc may be running (bypasses `gc.pid` checks).
    #[arg(long)]
    pub force: bool,

    /// Prune unreachable loose objects older than the given date (Git `--prune[=<date>]`).
    ///
    /// Bare `--prune` uses `gc.pruneExpire` when set, otherwise `2.weeks.ago`.
    #[arg(
        long = "prune",
        num_args = 0..=1,
        value_name = "DATE",
        default_missing_value = "",
        overrides_with = "no_prune"
    )]
    pub prune: Option<Option<String>>,

    /// Do not run loose-object pruning (`git prune`).
    #[arg(long = "no-prune", overrides_with = "prune")]
    pub no_prune: bool,

    /// Detach to background (accepted; always runs in foreground in grit).
    #[arg(long)]
    pub detach: bool,

    #[arg(long = "no-detach")]
    pub no_detach: bool,

    /// Skip pack-refs / reflog expire (used by `git maintenance run` background gc child).
    #[arg(long = "skip-foreground-tasks", hide = true)]
    pub skip_foreground_tasks: bool,

    /// Cruft pack options (forwarded to `grit repack` / pack-objects).
    #[arg(long)]
    pub cruft: bool,

    #[arg(long = "no-cruft")]
    pub no_cruft: bool,

    #[arg(long = "max-cruft-size", value_name = "SIZE")]
    pub max_cruft_size: Option<String>,

    #[arg(long = "expire-to", value_name = "DIR")]
    pub expire_to: Option<String>,

    #[arg(long = "keep-largest-pack")]
    pub keep_largest_pack: bool,
}

/// Run `grit gc`.
pub fn run(args: Args) -> Result<()> {
    let repo = Repository::discover(None).context("not a git repository")?;
    let cfg = ConfigSet::load(Some(&repo.git_dir), true).unwrap_or_default();
    validate_gc_prune_expire_config(&repo.git_dir, &cfg)?;

    let quiet = args.quiet && !args.no_quiet;

    // Tri-state `--detach` (Git `opts.detach`, default -1): `--detach` -> 1, `--no-detach` -> 0,
    // otherwise fall back to `gc.autodetach` (default true) under `--auto`. grit always runs in
    // the foreground, but this controls user-facing messages and progress (t6500 34/35).
    let effective_detach = if args.detach {
        1
    } else if args.no_detach {
        0
    } else if args.auto {
        if cfg
            .get_bool("gc.autodetach")
            .and_then(|r| r.ok())
            .unwrap_or(true)
        {
            1
        } else {
            0
        }
    } else {
        // Non-auto gc never daemonizes.
        0
    };

    // Auto-gc should not spam stderr with repack/pack-objects summaries (matches `git gc --auto`),
    // but when it runs in the foreground (`--no-detach` / `gc.autodetach=false`) it should still
    // surface progress like a normal gc (t6500 35). Detached/background auto gc stays quiet.
    let quiet_effective = quiet || (args.auto && effective_detach > 0);

    if args.auto {
        if !need_to_gc(&repo, &cfg) && !reftable_auto_pack_needed(&repo) {
            return Ok(());
        }
        let hook_ok = matches!(
            run_hook(&repo, "pre-auto-gc", &[], None),
            HookResult::Success | HookResult::NotFound
        );
        if !hook_ok {
            return Ok(());
        }
        if !args.quiet {
            if effective_detach > 0 {
                eprintln!("Auto packing the repository in background for optimum performance.");
            } else {
                eprintln!("Auto packing the repository for optimum performance.");
            }
            eprintln!("See \"git help gc\" for manual housekeeping.");
        }

        // When detaching (the default), a previous failed gc leaves `gc.log`; Git's
        // report_last_gc_error() refuses to run again while a recent, non-empty log exists
        // (git/builtin/gc.c:791-832), skipping silently (exit 0) after printing a warning.
        if effective_detach > 0 && report_last_gc_error(&repo.git_dir, &cfg) {
            return Ok(());
        }
    }

    // Be quiet on `--auto` when another gc already holds the lock (git/builtin/gc.c:992-1001):
    // exit 0 without doing anything instead of erroring out.
    let _gc_pid_guard = match acquire_gc_pid(&repo.git_dir, args.force, args.auto)? {
        Some(guard) => guard,
        None => return Ok(()),
    };

    let objects_dir = repo.git_dir.join("objects");
    if !args.no_prune {
        let opts = PrunePackedOptions {
            dry_run: false,
            quiet: quiet_effective,
        };
        prune_packed_objects(&objects_dir, opts).map_err(|e| anyhow::anyhow!("{e}"))?;
        prune_stale_tmp_objects(&objects_dir)?;
    }

    if !args.skip_foreground_tasks {
        trace_run_command_git_invocation(&["pack-refs", "--all", "--prune"]);
        pack_refs::run(pack_refs::Args {
            all: true,
            prune: true,
            no_prune: false,
            auto: args.auto,
            include: Vec::new(),
            no_include: false,
            exclude: Vec::new(),
            no_exclude: false,
        })?;
    }

    run_repack_for_gc(&repo, &cfg, quiet_effective, &args)?;
    prune_packed_objects(
        &objects_dir,
        PrunePackedOptions {
            dry_run: false,
            quiet: quiet_effective,
        },
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    clean_invalid_pack_garbage_for_gc(&objects_dir)?;

    if !args.skip_foreground_tasks && !args.auto {
        // Expire reflogs before pruning so unreachable objects that are retained only by old
        // reflog entries can be removed during `gc --prune=now`.
        run_reflog_expire_for_gc(&repo, &cfg)?;
        run_reflog_expire_unreachable_for_gc(&repo, &cfg)?;
    }

    if !args.no_prune {
        if let Some(expire) = gc_prune_expire_cli(&args, &cfg) {
            let precious_objects = cfg
                .get_bool("extensions.preciousobjects")
                .and_then(|r| r.ok())
                .unwrap_or(false);
            if precious_objects {
                return Ok(());
            }
            if quiet_effective {
                trace_run_command_git_invocation(&[
                    "prune",
                    "--expire",
                    expire.as_str(),
                    "--no-progress",
                ]);
            } else {
                trace_run_command_git_invocation(&["prune", "--expire", expire.as_str()]);
            }
            let old_ignore_reflogs = std::env::var_os("GRIT_PRUNE_IGNORE_REFLOGS");
            if expire == "now" {
                std::env::set_var("GRIT_PRUNE_IGNORE_REFLOGS", "1");
            }
            prune::run(prune::Args {
                dry_run: false,
                verbose: false,
                expire: Some(expire),
                no_expire: false,
                heads: Vec::new(),
                no_progress: quiet_effective,
                progress: false,
            })
            .context("git prune during gc")?;
            match old_ignore_reflogs {
                Some(v) => std::env::set_var("GRIT_PRUNE_IGNORE_REFLOGS", v),
                None => std::env::remove_var("GRIT_PRUNE_IGNORE_REFLOGS"),
            }
        }
    }
    // Git enables the commit-graph progress meter when `!quiet && !daemonized`
    // (git/builtin/gc.c:1056-1058). For a foreground auto gc (`--no-detach` / `gc.autodetach=false`)
    // that means progress should be requested even though grit forces the repack summaries quiet.
    let commit_graph_progress =
        args.no_quiet || (!quiet && effective_detach <= 0 && !args.skip_foreground_tasks);
    run_commit_graph_for_gc(&repo, &cfg, quiet_effective, commit_graph_progress)?;

    Ok(())
}

fn clean_invalid_pack_garbage_for_gc(objects_dir: &Path) -> Result<()> {
    let pack_dir = objects_dir.join("pack");
    let rd = match fs::read_dir(&pack_dir) {
        Ok(rd) => rd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    for entry in rd {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("idx") {
            continue;
        }
        let pack_path = path.with_extension("pack");
        if pack_path.is_file() || read_pack_index(&path).is_ok() {
            continue;
        }
        let keep_path = path.with_extension("keep");
        let _ = fs::remove_file(&path);
        if keep_path.is_file() {
            let _ = fs::remove_file(keep_path);
        }
    }
    Ok(())
}

fn reftable_auto_pack_needed(repo: &Repository) -> bool {
    if !grit_lib::reftable::is_reftable_repo(&repo.git_dir) {
        return false;
    }
    grit_lib::reftable::ReftableStack::open(&repo.git_dir)
        .map(|stack| stack.table_names().len() > 2)
        .unwrap_or(false)
}

/// Expire argument for `git prune` during `gc`: `None` means skip pruning (Git `never`).
fn validate_gc_prune_expire_config(git_dir: &Path, cfg: &ConfigSet) -> Result<()> {
    let Some(value) = cfg.get("gc.pruneexpire") else {
        return Ok(());
    };
    if gc_prune_expire_value_is_valid(&value) {
        return Ok(());
    }
    let config_path = git_dir.join("config");
    let line = fs::read_to_string(&config_path)
        .ok()
        .and_then(|text| {
            text.lines()
                .position(|line| line.to_ascii_lowercase().contains("pruneexpire"))
                .map(|idx| idx + 1)
        })
        .unwrap_or(1);
    bail!(
        "Invalid gc.pruneexpire: '{value}'\nfatal: bad config variable 'gc.pruneexpire' in file '.git/config' at line {line}"
    )
}

fn gc_prune_expire_value_is_valid(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    if matches!(normalized.as_str(), "now" | "all" | "never") || normalized.contains('-') {
        return true;
    }
    let normalized = normalized.replace('.', " ");
    let parts: Vec<&str> = normalized.split_whitespace().collect();
    if parts.len() < 2 {
        return false;
    }
    if parts[0].parse::<u64>().is_err() {
        return false;
    }
    matches!(
        parts[1].trim_end_matches('s'),
        "second" | "minute" | "hour" | "day" | "week" | "month" | "year"
    )
}

fn gc_prune_expire_cli(args: &Args, cfg: &ConfigSet) -> Option<String> {
    let from_flag = match &args.prune {
        Some(Some(s)) if s.is_empty() => cfg.get("gc.pruneexpire").clone(),
        Some(Some(s)) => Some(s.clone()),
        Some(None) => cfg.get("gc.pruneexpire").clone(),
        None => cfg.get("gc.pruneexpire").clone(),
    };
    let expire = from_flag.unwrap_or_else(|| "2.weeks.ago".to_string());
    let normalized = expire.trim().to_ascii_lowercase();
    if normalized == "never" {
        None
    } else {
        Some(expire)
    }
}

fn gc_hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Writes `<git-dir>/gc.pid` while gc runs; removed on drop. Matches Git’s stale checks (12h, host, `kill(pid,0)`).
struct GcPidGuard {
    path: PathBuf,
}

impl Drop for GcPidGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Acquire the `gc.pid` lock. Returns `Ok(None)` when the lock is held by a live process AND
/// `auto` is set — Git's "be quiet on --auto" behavior (git/builtin/gc.c:992-1001): the caller
/// should then exit 0 doing nothing. For non-auto gc a held lock is a hard error.
fn acquire_gc_pid(git_dir: &Path, force: bool, auto: bool) -> Result<Option<GcPidGuard>> {
    let pid_path = git_dir.join("gc.pid");
    if !force {
        match check_or_clear_stale_gc_pid(&pid_path)? {
            GcLockState::Free => {}
            GcLockState::Held(host) => {
                if auto {
                    // Another gc holds the lock; auto gc exits silently.
                    return Ok(None);
                }
                bail!("gc is already running on machine {host}");
            }
        }
    }
    let my_pid = std::process::id();
    let host = gc_hostname();
    fs::write(&pid_path, format!("{my_pid} {host}\n"))?;
    Ok(Some(GcPidGuard { path: pid_path }))
}

/// Result of inspecting an existing `gc.pid` lock file.
enum GcLockState {
    /// No (valid, live) lock — safe to take it.
    Free,
    /// The lock is held by a live process on the named host.
    Held(String),
}

fn check_or_clear_stale_gc_pid(pid_path: &Path) -> Result<GcLockState> {
    let meta = match fs::metadata(pid_path) {
        Ok(m) => m,
        Err(_) => return Ok(GcLockState::Free),
    };
    let age_secs = SystemTime::now()
        .duration_since(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH))
        .map(|d| d.as_secs())
        .unwrap_or(u64::MAX);
    if age_secs > 12 * 3600 {
        let _ = fs::remove_file(pid_path);
        return Ok(GcLockState::Free);
    }
    let contents = fs::read_to_string(pid_path)?;
    let mut parts = contents.split_whitespace();
    let Some(pid_s) = parts.next() else {
        let _ = fs::remove_file(pid_path);
        return Ok(GcLockState::Free);
    };
    let Some(locking_host) = parts.next() else {
        let _ = fs::remove_file(pid_path);
        return Ok(GcLockState::Free);
    };
    let Ok(foreign_pid) = pid_s.parse::<u32>() else {
        let _ = fs::remove_file(pid_path);
        return Ok(GcLockState::Free);
    };
    let my_host = gc_hostname();
    if locking_host != my_host {
        return Ok(GcLockState::Held(locking_host.to_string()));
    }
    #[cfg(unix)]
    {
        if grit_lib::unix_process::pid_is_alive(foreign_pid) {
            return Ok(GcLockState::Held(locking_host.to_string()));
        }
        let _ = fs::remove_file(pid_path);
        Ok(GcLockState::Free)
    }
    #[cfg(not(unix))]
    {
        let _ = foreign_pid;
        Ok(GcLockState::Held(locking_host.to_string()))
    }
}

/// Mirror Git's `report_last_gc_error` (git/builtin/gc.c:791-832): if `gc.log` exists, is
/// non-empty, and its mtime is at or after `now - gc.logexpiry` (default `1.day.ago`), print the
/// warning and signal that the gc should be skipped. Returns `true` when the gc must be skipped.
fn report_last_gc_error(git_dir: &Path, cfg: &ConfigSet) -> bool {
    let log_path = git_dir.join("gc.log");
    let meta = match fs::metadata(&log_path) {
        Ok(m) => m,
        Err(_) => return false,
    };

    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let expire_spec = cfg
        .get("gc.logexpiry")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "1.day.ago".to_string());

    if expire_spec.eq_ignore_ascii_case("never") {
        // A "never"-expiring log is always considered recent: never auto-run again.
        // fall through to the non-empty check below.
    }
    let expire_time = if expire_spec.eq_ignore_ascii_case("never") {
        i64::MIN
    } else {
        grit_lib::git_date::approx::approxidate_careful(&expire_spec, None) as i64
    };

    // Older than the expiry cutoff -> ignore the stale log and let gc proceed.
    if mtime < expire_time {
        return false;
    }

    let contents = fs::read_to_string(&log_path).unwrap_or_default();
    if contents.is_empty() {
        return false;
    }

    eprint!(
        "warning: The last gc run reported the following. Please correct the root cause\n\
         and remove {}\n\
         Automatic cleanup will not be performed until the file is removed.\n\n\
         {}",
        log_path.display(),
        contents
    );
    true
}

fn too_many_packs_for_gc(repo: &Repository, cfg: &ConfigSet) -> bool {
    let pack_limit = cfg
        .get("gc.autopacklimit")
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(50);
    if pack_limit <= 0 {
        return false;
    }
    let pack_dir = repo.git_dir.join("objects").join("pack");
    count_local_pack_files(&pack_dir) >= pack_limit as usize
}

/// Packs to keep when auto-gc does a full repack due to `gc.autoPackLimit` (Git `need_to_gc`).
fn auto_gc_keep_packs(repo: &Repository, cfg: &ConfigSet) -> Result<Vec<String>> {
    let pack_limit = cfg
        .get("gc.autopacklimit")
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(50);
    if pack_limit <= 0 {
        return Ok(Vec::new());
    }

    if let Some(limit_s) = cfg.get("gc.bigpackthreshold") {
        // `gc.bigPackThreshold` accepts size suffixes (`2g`, `1m`); plain `parse::<u64>` would drop
        // them and silently treat the threshold as 0 (= keep largest pack).
        let limit = parse_byte_size_with_suffix(&limit_s).unwrap_or(0);
        if limit > 0 {
            let mut keep = find_base_packs(&repo.git_dir, limit)?;
            if keep.len() as i32 >= pack_limit {
                keep = find_base_packs(&repo.git_dir, 0)?;
            }
            return Ok(keep);
        }
    }

    find_base_packs(&repo.git_dir, 0)
}

fn gc_cruft_packs_enabled(cfg: &ConfigSet, gc_args: &Args) -> bool {
    if gc_args.no_cruft {
        return false;
    }
    if gc_args.cruft {
        return true;
    }
    !cfg.get("gc.cruftpacks")
        .map(|s| {
            let t = s.trim().to_lowercase();
            t == "false" || t == "0" || t == "off" || t == "no"
        })
        .unwrap_or(false)
}

/// Parse `1M`, `2G`, `1048576` into bytes for `gc.maxCruftSize` / `--max-cruft-size`.
fn parse_byte_size_with_suffix(raw: &str) -> Option<u64> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let upper = s.to_ascii_uppercase();
    let (digits, mult) = if upper.ends_with("K") {
        (&s[..s.len() - 1], 1024u64)
    } else if upper.ends_with("M") {
        (&s[..s.len() - 1], 1024u64 * 1024)
    } else if upper.ends_with("G") {
        (&s[..s.len() - 1], 1024u64 * 1024 * 1024)
    } else {
        (s, 1u64)
    };
    let n: u64 = digits.trim().parse().ok()?;
    Some(n.saturating_mul(mult))
}

/// Repack for `git gc`: matches `git/builtin/gc.c` `add_repack_all_option` + `need_to_gc` repack args.
fn run_repack_for_gc(
    repo: &Repository,
    cfg: &ConfigSet,
    quiet: bool,
    gc_args: &Args,
) -> Result<()> {
    let objects_dir = repo.git_dir.join("objects");
    if repo_treats_promisor_packs(&repo.git_dir, cfg) {
        let mut promisor_ids: Vec<ObjectId> =
            promisor_pack_object_ids(&objects_dir).into_iter().collect();
        promisor_ids.sort_by_key(|o| o.to_hex());
        promisor_ids.dedup();
        if !promisor_ids.is_empty() {
            return run_promisor_merge_repack_gc(repo, quiet, gc_args, &promisor_ids);
        }
    }

    let work_dir = repo.work_tree.as_deref().unwrap_or(&repo.git_dir);
    let mut repack_trace: Vec<String> = vec!["repack".into(), "-d".into(), "-l".into()];
    let mut cmd = Command::new(grit_exe::grit_executable());
    cmd.current_dir(work_dir).args(["repack", "-d", "-l"]);

    if gc_args.auto && !too_many_packs_for_gc(repo, cfg) {
        repack_trace.push("--no-write-bitmap-index".into());
        cmd.arg("--no-write-bitmap-index");
    } else {
        let cfg_prune = cfg.get("gc.pruneexpire");
        let prune_expire: std::borrow::Cow<'_, str> = match &gc_args.prune {
            Some(Some(s)) => std::borrow::Cow::Borrowed(s.as_str()),
            Some(None) => std::borrow::Cow::Owned(
                cfg_prune
                    .clone()
                    .unwrap_or_else(|| "2.weeks.ago".to_string()),
            ),
            None => std::borrow::Cow::Owned(
                cfg_prune
                    .clone()
                    .unwrap_or_else(|| "2.weeks.ago".to_string()),
            ),
        };
        let prune_expire = prune_expire.as_ref();
        let cruft_on = gc_cruft_packs_enabled(cfg, gc_args);
        let expire_to = gc_args
            .expire_to
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let max_cruft_cli = gc_args
            .max_cruft_size
            .as_deref()
            .and_then(parse_byte_size_with_suffix);
        let max_cruft_cfg = cfg
            .get("gc.maxcruftsize")
            .as_deref()
            .and_then(parse_byte_size_with_suffix);
        let max_cruft = max_cruft_cli.or(max_cruft_cfg);

        if gc_args.no_prune || prune_expire.eq_ignore_ascii_case("never") {
            // `git gc --no-prune` / `--prune=never` must not loosen and then delete
            // unreachable objects; leave the repack as the base `repack -d -l` pass.
        } else if prune_expire == "now" && !(cruft_on && expire_to.is_some()) {
            repack_trace.push("-a".into());
            cmd.arg("-a");
        } else if cruft_on {
            repack_trace.push("--cruft".into());
            cmd.arg("--cruft");
            let exp_arg = format!("--cruft-expiration={prune_expire}");
            repack_trace.push(exp_arg.clone());
            cmd.arg(exp_arg);
            if let Some(n) = max_cruft {
                let m = format!("--max-cruft-size={n}");
                repack_trace.push(m.clone());
                cmd.arg(m);
            }
            if let Some(ref et) = expire_to {
                let e = format!("--expire-to={et}");
                repack_trace.push(e.clone());
                cmd.arg(e);
            }
        } else {
            repack_trace.push("-A".into());
            cmd.arg("-A");
            let unpack_arg = format!("--unpack-unreachable={prune_expire}");
            repack_trace.push(unpack_arg.clone());
            cmd.arg(unpack_arg);
        }

        let keep = if gc_args.auto {
            auto_gc_keep_packs(repo, cfg)?
        } else if gc_args.keep_largest_pack {
            find_base_packs(&repo.git_dir, 0)?
        } else if let Some(limit_s) = cfg.get("gc.bigpackthreshold") {
            let limit = parse_byte_size_with_suffix(&limit_s).unwrap_or(0);
            if limit > 0 {
                find_base_packs(&repo.git_dir, limit)?
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        for name in keep {
            repack_trace.push("--keep-pack".into());
            repack_trace.push(name.clone());
            cmd.arg("--keep-pack").arg(name);
        }
    }

    if let Some(ref f) = cfg.get("gc.repackfilter") {
        if !f.is_empty() {
            let opt = format!("--filter={f}");
            repack_trace.push(opt.clone());
            cmd.arg(opt);
        }
    }
    if let Some(ref to) = cfg.get("gc.repackfilterto") {
        if !to.is_empty() {
            repack_trace.push("--filter-to".into());
            repack_trace.push(to.clone());
            cmd.arg("--filter-to").arg(to);
        }
    }

    if quiet {
        repack_trace.push("-q".into());
        cmd.arg("-q");
    }
    if gc_args.aggressive {
        repack_trace.push("--aggressive".into());
        cmd.arg("--aggressive");
    }
    let trace_refs: Vec<&str> = repack_trace.iter().map(|s| s.as_str()).collect();
    trace_run_command_git_invocation(&trace_refs);
    let mut trace2_argv = vec!["git".to_string()];
    trace2_argv.extend(repack_trace.iter().cloned());
    trace2_emit_git_subcommand_argv(&trace2_argv);
    let status = cmd.status().context("failed to run grit repack for gc")?;
    if !status.success() {
        eprintln!("warning: repack returned non-zero status");
    }
    Ok(())
}

/// Remove stale `objects/tmp_*` files left from interrupted object writes (Git-compatible naming).
fn prune_stale_tmp_objects(objects_dir: &Path) -> Result<()> {
    const MAX_AGE: Duration = Duration::from_secs(14 * 24 * 3600);
    let rd = match fs::read_dir(objects_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let now = SystemTime::now();
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("tmp_") {
            continue;
        }
        let path = entry.path();
        let meta = match fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if now.duration_since(modified).unwrap_or(Duration::ZERO) > MAX_AGE {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

fn parse_pack_objects_stdout_hash(stdout: &[u8]) -> Result<String> {
    let line = std::str::from_utf8(stdout)
        .context("pack-objects stdout not utf-8")?
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.is_empty() {
        bail!("pack-objects did not print a pack hash on stdout");
    }
    Ok(line.to_string())
}

/// Like [`parse_pack_objects_stdout_hash`] but returns `None` when pack-objects
/// wrote no pack (empty object set), which is not an error.
fn parse_optional_pack_objects_stdout_hash(stdout: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(stdout).ok()?;
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

fn apply_gc_pack_objects_args(cmd: &mut Command, quiet: bool, gc_args: &Args) {
    if quiet {
        cmd.arg("-q");
    }
    if gc_args.aggressive {
        cmd.arg("-f");
        cmd.arg("--window").arg("250");
        cmd.arg("--depth").arg("250");
    }
    if gc_args.cruft {
        cmd.arg("--cruft");
    }
    if gc_args.no_cruft {
        cmd.arg("--no-cruft");
    }
}

fn is_cruft_pack(pack_dir: &Path, stem: &str) -> bool {
    pack_dir.join(format!("{stem}.mtimes")).exists()
}

/// Packs to retain during `git gc` repack: all packs at least `limit` bytes, or the single largest
/// when `limit == 0` (Git `find_base_packs`).
fn find_base_packs(git_dir: &Path, limit: u64) -> Result<Vec<String>> {
    let pack_dir = git_dir.join("objects").join("pack");
    let rd = match fs::read_dir(&pack_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };

    let mut large = Vec::new();
    let mut largest: Option<(u64, String)> = None;

    for entry in rd {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".pack") {
            continue;
        }
        let stem = name
            .strip_suffix(".pack")
            .unwrap_or(name.as_str())
            .to_string();
        if is_cruft_pack(&pack_dir, &stem) {
            continue;
        }
        let meta = match fs::metadata(entry.path()) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let sz = meta.len();
        if limit > 0 && sz >= limit {
            large.push(name.clone());
        }
        match &mut largest {
            None => largest = Some((sz, name)),
            Some((best_sz, best_name)) if sz > *best_sz => {
                *best_sz = sz;
                *best_name = name;
            }
            _ => {}
        }
    }

    if limit > 0 {
        large.sort();
        return Ok(large);
    }

    Ok(largest.into_iter().map(|(_, n)| n).collect())
}

/// Merge all promisor-pack objects into one promisor pack, then repack non-promisor objects.
fn run_promisor_merge_repack_gc(
    repo: &Repository,
    quiet: bool,
    gc_args: &Args,
    promisor_ids: &[ObjectId],
) -> Result<()> {
    let work_dir = repo.work_tree.as_deref().unwrap_or(&repo.git_dir);
    let grit_bin = grit_exe::grit_executable();
    let pack_base = if repo.work_tree.is_some() {
        ".git/objects/pack/pack"
    } else {
        "objects/pack/pack"
    };

    let mut cmd1 = Command::new(&grit_bin);
    cmd1.current_dir(work_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .arg("pack-objects")
        .arg(pack_base);
    apply_gc_pack_objects_args(&mut cmd1, quiet, gc_args);

    let mut child = cmd1
        .spawn()
        .context("spawn pack-objects for promisor merge")?;
    {
        let mut stdin = child.stdin.take().context("pack-objects stdin")?;
        for oid in promisor_ids {
            writeln!(stdin, "{}", oid.to_hex())?;
        }
    }
    let out1 = child
        .wait_with_output()
        .context("wait pack-objects promisor merge")?;
    if !out1.status.success() {
        bail!(
            "pack-objects (promisor merge) failed with status {}",
            out1.status
        );
    }
    let hash1 = parse_pack_objects_stdout_hash(&out1.stdout)?;
    let promisor_pack_name = format!("pack-{hash1}.pack");
    let pack_dir = repo.git_dir.join("objects/pack");
    fs::write(pack_dir.join(format!("pack-{hash1}.promisor")), b"")?;

    let mut cmd2 = Command::new(&grit_bin);
    cmd2.current_dir(work_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .arg("pack-objects")
        .arg("--all")
        .arg("--exclude-promisor-objects")
        .arg(pack_base);
    apply_gc_pack_objects_args(&mut cmd2, quiet, gc_args);

    let out2 = cmd2
        .output()
        .context("run pack-objects for non-promisor gc")?;
    if !out2.status.success() {
        bail!(
            "pack-objects (non-promisor) failed with status {}",
            out2.status
        );
    }
    // The non-promisor pass can legitimately enumerate zero objects (everything
    // reachable is already in promisor packs), in which case pack-objects writes
    // no pack and prints no hash. That is not an error: simply drop the
    // superseded non-promisor packs and keep the freshly written promisor pack.
    let main_pack_name = match parse_optional_pack_objects_stdout_hash(&out2.stdout) {
        Some(hash2) => format!("pack-{hash2}.pack"),
        None => String::new(),
    };

    let mut keep: Vec<String> = vec![promisor_pack_name];
    if !main_pack_name.is_empty() {
        keep.push(main_pack_name);
    }
    repack::remove_superseded_packs_multi(&pack_dir, &keep)?;
    update_server_info::refresh_objects_info_packs(repo)?;
    Ok(())
}

/// Apply `gc.reflogExpire` to all reflogs (Git default **90** days when unset).
fn run_reflog_expire_for_gc(repo: &Repository, cfg: &ConfigSet) -> Result<()> {
    let raw = cfg
        .get("gc.reflogexpire")
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_else(|| "90".to_string());
    if raw == "never" || raw == "false" {
        return Ok(());
    }
    trace_run_command_git_invocation(&["reflog", "expire", "--all"]);
    let days: u64 = raw.parse().unwrap_or(90);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| anyhow::anyhow!("system time error: {e}"))?
        .as_secs() as i64;
    let cutoff = now.saturating_sub((days as i64).saturating_mul(86_400));

    let refs = list_reflog_refs(&repo.git_dir).map_err(|e| anyhow::anyhow!("{e}"))?;
    for refname in refs {
        expire_reflog(&repo.git_dir, &refname, Some(cutoff)).map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    Ok(())
}

/// Apply `gc.reflogExpireUnreachable` to all reflogs (Git default **30** days when unset).
fn run_reflog_expire_unreachable_for_gc(repo: &Repository, cfg: &ConfigSet) -> Result<()> {
    let raw = cfg
        .get("gc.reflogexpireunreachable")
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_else(|| "30".to_string());
    if raw == "never" || raw == "false" {
        return Ok(());
    }
    trace_run_command_git_invocation(&["reflog", "expire", "--all"]);
    let days: u64 = raw.parse().unwrap_or(30);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| anyhow::anyhow!("system time error: {e}"))?
        .as_secs() as i64;
    let cutoff = now.saturating_sub((days as i64).saturating_mul(86_400));

    let refs = list_reflog_refs(&repo.git_dir).map_err(|e| anyhow::anyhow!("{e}"))?;
    for refname in refs {
        expire_reflog_unreachable(repo, &repo.git_dir, &refname, Some(cutoff))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    Ok(())
}

/// Run `grit commit-graph write` when `gc.writeCommitGraph` is true (Git default: on).
///
/// `want_progress` requests the commit-graph progress meter (Git enables it when the gc is not
/// quiet and not daemonized); otherwise `quiet` forces `--no-progress`.
fn run_commit_graph_for_gc(
    repo: &Repository,
    cfg: &ConfigSet,
    quiet: bool,
    want_progress: bool,
) -> Result<()> {
    let write_graph = cfg
        .get_bool("gc.writecommitgraph")
        .and_then(|r| r.ok())
        .unwrap_or(true);
    if !write_graph {
        return Ok(());
    }

    let work_dir = repo.work_tree.as_deref().unwrap_or(&repo.git_dir);
    let progress_flag = if want_progress {
        Some("--progress")
    } else if quiet {
        Some("--no-progress")
    } else {
        None
    };

    let mut trace_args = vec!["commit-graph", "write", "--reachable", "--changed-paths"];
    if let Some(flag) = progress_flag {
        trace_args.push(flag);
    }
    trace_run_command_git_invocation(&trace_args);

    let mut cmd = Command::new(grit_exe::grit_executable());
    cmd.current_dir(work_dir)
        .args(["commit-graph", "write", "--reachable", "--changed-paths"]);
    if let Some(flag) = progress_flag {
        cmd.arg(flag);
    }
    if quiet {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    let status = cmd
        .status()
        .context("failed to run grit commit-graph write for gc")?;
    if !status.success() && !quiet {
        eprintln!("warning: commit-graph write returned non-zero status");
    }
    Ok(())
}

/// Rounded threshold matching Git’s `gc.auto` interpretation (`DIV_ROUND_UP(limit, 256) * 256`).
fn gc_auto_threshold(gc_auto: i32) -> usize {
    if gc_auto <= 0 {
        return 0;
    }
    ((gc_auto as usize).saturating_add(255) / 256) * 256
}

/// Git’s `ODB_COUNT_OBJECTS_APPROXIMATE`: count loose objects under `objects/17/` and multiply by 256.
fn approximate_loose_object_count(objects_dir: &Path) -> usize {
    let shard = objects_dir.join("17");
    let Ok(rd) = fs::read_dir(&shard) else {
        return 0;
    };
    let mut n = 0usize;
    for entry in rd.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        if fname.len() == 38 && fname.chars().all(|c| c.is_ascii_hexdigit()) {
            n += 1;
        }
    }
    n.saturating_mul(256)
}

fn count_local_pack_files(pack_dir: &Path) -> usize {
    let Ok(rd) = fs::read_dir(pack_dir) else {
        return 0;
    };
    rd.flatten()
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("pack"))
        })
        .count()
}

/// Returns whether automatic gc should do work (loose object count or pack count over limits).
/// Exposed for `git maintenance` auto conditions (`t7900`).
pub(crate) fn need_to_gc(repo: &Repository, cfg: &ConfigSet) -> bool {
    let gc_auto = cfg
        .get("gc.auto")
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(6700);
    if gc_auto <= 0 {
        return false;
    }

    if too_many_packs_for_gc(repo, cfg) {
        return true;
    }

    let threshold = gc_auto_threshold(gc_auto);
    let loose = approximate_loose_object_count(&repo.git_dir.join("objects"));
    loose > threshold
}
