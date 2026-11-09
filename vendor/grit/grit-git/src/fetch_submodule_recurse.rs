//! Recursive `git fetch` into submodules (Git `fetch_submodules` / `submodule.c`).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{bail, Context, Result};
use grit_lib::config::ConfigFile;
use grit_lib::fetch_submodules::{
    collect_changed_submodules_for_fetch, is_submodule_active_for_fetch,
    might_have_submodules_to_fetch, parse_fetch_recurse_submodules_arg,
    submodule_git_dir_for_fetch, submodule_has_all_commits, ChangedSubmoduleFetch,
    FetchRecurseSubmodules,
};
use grit_lib::index::MODE_GITLINK;
use grit_lib::name_rev;
use grit_lib::objects::ObjectId;
use grit_lib::odb::Odb;
use grit_lib::repo::Repository;

use crate::commands::fetch::Args as FetchArgs;
use crate::commands::submodule::{get_default_remote_for_path, parse_gitmodules_with_repo};
use crate::grit_exe;

/// Resolve a changed submodule's git directory. Git addresses the submodule repo by *name*
/// (`.git/modules/<name>`), so when the recorded path has changed (e.g. `git mv` renamed the
/// submodule, t5526 #39) the path-based lookup misses and we fall back to the name. The work-tree
/// path is still tried first because populated submodules carry their gitdir via a `.git` gitfile.
fn changed_submodule_git_dir(
    repo: &Repository,
    path: &str,
    name: &str,
) -> Option<std::path::PathBuf> {
    if let Some(gd) = submodule_git_dir_for_fetch(repo, path) {
        return Some(gd);
    }
    let modules = grit_lib::submodule_gitdir::submodule_modules_git_dir(&repo.git_dir, name);
    if modules.join("HEAD").exists() {
        return Some(modules);
    }
    None
}

/// Run `git submodule--helper get-default-remote <path>` as a subprocess from the superproject and
/// capture the remote name, matching git's `oid_fetch_tasks` pass (submodule.c). Running it as a
/// child also produces the `trace: built-in: git submodule--helper get-default-remote <path>`
/// GIT_TRACE line that t5526 #40/#44 assert on. Returns `None` on any failure (caller falls back).
fn helper_get_default_remote(
    grit_bin: &Path,
    path: &str,
    super_work_tree: &Path,
) -> Option<String> {
    let out = std::process::Command::new(grit_bin)
        .current_dir(super_work_tree)
        .arg("submodule--helper")
        .arg("get-default-remote")
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn parse_gitmodules_file(work_tree: &Path) -> Option<ConfigFile> {
    let p = work_tree.join(".gitmodules");
    let content = std::fs::read_to_string(&p).ok()?;
    ConfigFile::parse(&p, &content, grit_lib::config::ConfigScope::Local).ok()
}

fn gitmodules_fetch_recurse_value(gm: &ConfigFile, submodule_name: &str) -> Option<String> {
    let key_lc = format!("submodule.{submodule_name}.fetchrecursesubmodules");
    for e in &gm.entries {
        if e.key == key_lc {
            return e.value.clone();
        }
    }
    None
}

fn effective_submodule_fetch_recurse(
    name: &str,
    cmd: FetchRecurseSubmodules,
    default: FetchRecurseSubmodules,
    config: &grit_lib::config::ConfigSet,
    gm: Option<&ConfigFile>,
) -> Result<FetchRecurseSubmodules> {
    if cmd != FetchRecurseSubmodules::Default {
        return Ok(cmd);
    }
    let key = format!("submodule.{name}.fetchRecurseSubmodules");
    let key_alt = format!("submodule.{name}.fetchrecursesubmodules");
    let from_config = config.get(&key).or_else(|| config.get(&key_alt));
    let from_gm = gm.and_then(|g| gitmodules_fetch_recurse_value(g, name));
    let raw = from_config.or_else(|| from_gm.clone());
    if let Some(v) = raw {
        return parse_fetch_recurse_submodules_arg(&key, v.trim()).map_err(|e| anyhow::anyhow!(e));
    }
    Ok(default)
}

fn fetch_parallel_job_count(args: &FetchArgs, config: &grit_lib::config::ConfigSet) -> usize {
    if let Some(j) = args.jobs {
        if j == 0 {
            return std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1);
        }
        return j;
    }
    if let Some(v) = config
        .get("submodule.fetchjobs")
        .or_else(|| config.get("submodule.fetchJobs"))
    {
        if let Ok(n) = v.trim().parse::<usize>() {
            if n == 0 {
                return std::thread::available_parallelism()
                    .map(|x| x.get())
                    .unwrap_or(1);
            }
            if n > 0 {
                return n;
            }
        }
    }
    if let Some(v) = config.get("fetch.parallel") {
        if let Ok(n) = v.trim().parse::<usize>() {
            if n == 0 {
                return std::thread::available_parallelism()
                    .map(|x| x.get())
                    .unwrap_or(1);
            }
            if n > 0 {
                return n;
            }
        }
    }
    1
}

fn trace_parallel_tasks(n: usize) {
    if let Ok(trace_val) = std::env::var("GIT_TRACE") {
        if trace_val.is_empty() || trace_val == "0" || trace_val.eq_ignore_ascii_case("false") {
            return;
        }
        let line = format!("run_processes_parallel: preparing to run up to {n} tasks\n");
        crate::write_git_trace(&trace_val, &line);
    }
}

fn forward_parent_fetch_flags(cmd: FetchRecurseSubmodules, args: &FetchArgs) -> Vec<String> {
    let mut v = Vec::new();
    if args.dry_run {
        v.push("--dry-run".to_string());
    }
    if args.quiet {
        v.push("-q".to_string());
    }
    if args.no_write_fetch_head {
        v.push("--no-write-fetch-head".to_string());
    }
    match cmd {
        FetchRecurseSubmodules::On => v.push("--recurse-submodules".to_string()),
        FetchRecurseSubmodules::Off => v.push("--no-recurse-submodules".to_string()),
        FetchRecurseSubmodules::OnDemand => v.push("--recurse-submodules=on-demand".to_string()),
        FetchRecurseSubmodules::Default => {}
    }
    v
}

struct SubmoduleFetchWork {
    /// Directory for the child process (`submodule/` when checked out, else the git dir).
    process_cwd: std::path::PathBuf,
    /// Pass `--work-tree=.` only when `process_cwd` is the git dir (unpopulated module).
    work_tree_dot: bool,
    /// Path relative to the repo whose index listed this submodule (super or nested).
    display_path: String,
    remote: String,
    default_token: &'static str,
    /// Submodule git dir, used for the post-fetch "are the needed commits present?" check.
    git_dir: std::path::PathBuf,
    /// Superproject commit that recorded the change — drives the `at commit <oid>` display only.
    at_commit: Option<ObjectId>,
    /// Commits the superproject now references; if a normal fetch doesn't bring them in (commit
    /// outside the standard refspec), a follow-up by-oid fetch is issued. Empty for index-only
    /// recursion (git's `oid_fetch_tasks` second pass).
    needed_commits: Vec<ObjectId>,
}

/// After a successful top-level `fetch`, recurse into submodules like Git's `fetch_submodules`.
pub(crate) fn recursive_fetch_submodules_after_fetch(
    super_git_dir: &Path,
    config: &grit_lib::config::ConfigSet,
    args: &FetchArgs,
    cmd_recurse: FetchRecurseSubmodules,
) -> Result<()> {
    let Some(record) = crate::fetch_submodule_record::take_fetch_submodule_record() else {
        return Ok(());
    };
    let cwd = std::env::current_dir().context("cwd")?;
    let repo = Repository::discover(Some(cwd.as_path())).context("open repository")?;
    let work_tree = repo.work_tree.as_ref().context("bare repository")?;
    if !might_have_submodules_to_fetch(work_tree, super_git_dir) {
        return Ok(());
    }
    let gm_file = parse_gitmodules_file(work_tree);

    let positive = grit_lib::fetch_submodules::merge_tips_for_changed_walk(
        &record.submodule_commits.borrow(),
        &record.tips_after.borrow(),
    );
    let negative: Vec<String> = record.tips_before.iter().map(|o| o.to_hex()).collect();
    let mut changed_list = collect_changed_submodules_for_fetch(&repo, &positive, &negative)?;
    let mut changed_by_name: HashMap<String, ChangedSubmoduleFetch> = HashMap::new();
    for c in changed_list.drain(..) {
        changed_by_name.insert(c.name.clone(), c);
    }

    let default_child = if let Some(ref s) = args.recurse_submodules_default {
        parse_fetch_recurse_submodules_arg("--recurse-submodules-default", s)
            .map_err(|e| anyhow::anyhow!(e))?
    } else {
        FetchRecurseSubmodules::OnDemand
    };

    let mut filtered_changed: HashMap<String, ChangedSubmoduleFetch> = HashMap::new();
    for (name, cs) in changed_by_name {
        if !is_submodule_active_for_fetch(&repo, config, &cs.super_oid, &cs.path, &name) {
            continue;
        }
        let Some(gd) = changed_submodule_git_dir(&repo, &cs.path, &name) else {
            continue;
        };
        let odb = Odb::new(&gd.join("objects"));
        if submodule_has_all_commits(&odb, &cs.new_commits)? {
            continue;
        }
        filtered_changed.insert(name, cs);
    }
    let changed_by_name = filtered_changed;

    let modules = parse_gitmodules_with_repo(work_tree, Some(&repo))?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut work: Vec<SubmoduleFetchWork> = Vec::new();

    let index_path = repo.index_path();
    let index = repo.load_index_at(&index_path).ok();
    if let Some(ref idx) = index {
        for e in &idx.entries {
            if e.stage() != 0 || e.mode != MODE_GITLINK {
                continue;
            }
            let path = String::from_utf8_lossy(&e.path).to_string();
            // Git's `fetch_task_create` falls back to a synthetic submodule (name == path) when an
            // index gitlink has no `.gitmodules` entry but the work tree is populated
            // (`get_non_gitmodules_submodule` / `default_name_or_path`). This lets the *index* task
            // handle it with a plain "Fetching submodule X" (no `at commit` annotation), matching
            // t5526 #36 where the superproject commit removed `.gitmodules`.
            let synthetic;
            let sm = match modules.iter().find(|m| m.path == path) {
                Some(m) => m,
                None => {
                    let abs_sm = work_tree.join(&path);
                    if !abs_sm.join(".git").exists() {
                        continue;
                    }
                    synthetic = crate::commands::submodule::SubmoduleInfo {
                        name: path.clone(),
                        path: path.clone(),
                        url: String::new(),
                        shallow: None,
                        update: None,
                        branch: None,
                        ignore: None,
                    };
                    &synthetic
                }
            };
            // NOTE: Git's `get_fetch_task_from_index` does *not* gate on submodule activeness — only
            // the changed-task path checks `is_tree_submodule_active`. So we do not filter the index
            // entry here (t5526 #36 unsets `submodule.<n>.url` yet still fetches the populated gitlink).
            let mode = effective_submodule_fetch_recurse(
                &sm.name,
                cmd_recurse,
                default_child,
                config,
                gm_file.as_ref(),
            )?;
            let include = match mode {
                FetchRecurseSubmodules::Off => false,
                FetchRecurseSubmodules::On => true,
                FetchRecurseSubmodules::OnDemand | FetchRecurseSubmodules::Default => {
                    changed_by_name.contains_key(&sm.name)
                }
            };
            if !include {
                continue;
            }
            let Some(gd) = submodule_git_dir_for_fetch(&repo, &path) else {
                let abs = work_tree.join(&path);
                if abs.is_dir()
                    && std::fs::read_dir(&abs)
                        .map(|d| d.count() > 0)
                        .unwrap_or(false)
                {
                    bail!("Could not access submodule '{path}'");
                }
                continue;
            };
            if seen.insert(sm.name.clone()) {
                let abs_sm = work_tree.join(&path);
                let populated = abs_sm.join(".git").exists();
                let remote = if populated {
                    get_default_remote_for_path(&path)?
                } else {
                    crate::commands::submodule::get_default_remote_from_git_dir(&gd)
                };
                let (process_cwd, work_tree_dot) = if populated {
                    (abs_sm, false)
                } else {
                    (gd.clone(), true)
                };
                work.push(SubmoduleFetchWork {
                    process_cwd,
                    work_tree_dot,
                    display_path: path.clone(),
                    remote,
                    default_token: if mode == FetchRecurseSubmodules::On {
                        "yes"
                    } else {
                        "on-demand"
                    },
                    git_dir: gd.clone(),
                    // Git's index task only handles a *populated* gitlink ("Fetching submodule X",
                    // no annotation). An index entry whose work tree is absent has no populated
                    // repo handle, so it falls through to the changed task which annotates it
                    // `at commit <super_oid>` — e.g. a nested deepsubmodule reached while the parent
                    // submodule is itself unpopulated (`--work-tree=.`), t5526 #27/#28.
                    at_commit: if populated {
                        None
                    } else {
                        changed_by_name.get(&sm.name).map(|c| c.super_oid)
                    },
                    // If the superproject's new commits reference commits outside the submodule's
                    // standard refspec, a by-OID follow-up fetch brings them in (git's
                    // `oid_fetch_tasks` second pass) — needed for name-conflicted submodules (t5526
                    // #52) where the populated submodule's remote differs from the recorded URL.
                    needed_commits: changed_by_name
                        .get(&sm.name)
                        .map(|c| c.new_commits.clone())
                        .unwrap_or_default(),
                });
            }
        }
    }

    let mut changed_names: Vec<String> = changed_by_name.keys().cloned().collect();
    changed_names.sort();
    for name in changed_names {
        if seen.contains(&name) {
            continue;
        }
        let Some(cs) = changed_by_name.get(&name) else {
            continue;
        };
        if !is_submodule_active_for_fetch(&repo, config, &cs.super_oid, &cs.path, &name) {
            continue;
        }
        let mode = effective_submodule_fetch_recurse(
            &name,
            cmd_recurse,
            default_child,
            config,
            gm_file.as_ref(),
        )?;
        let include = match mode {
            FetchRecurseSubmodules::Off => false,
            FetchRecurseSubmodules::On => true,
            FetchRecurseSubmodules::OnDemand | FetchRecurseSubmodules::Default => true,
        };
        if !include {
            continue;
        }
        let Some(gd) = submodule_git_dir_for_fetch(&repo, &cs.path) else {
            bail!(
                "Could not access submodule '{}' at commit {}",
                cs.path,
                cs.super_oid.to_hex()
            );
        };
        seen.insert(name.clone());
        let abs_sm = work_tree.join(&cs.path);
        let populated = abs_sm.join(".git").exists();
        // When the submodule work tree is gone (e.g. the current index has no submodules but a
        // newly-fetched superproject commit changes one), `get_default_remote_for_path` cannot
        // walk the work tree — read the default remote from the module git dir directly.
        let remote = if populated {
            get_default_remote_for_path(&cs.path)?
        } else {
            crate::commands::submodule::get_default_remote_from_git_dir(&gd)
        };
        let (process_cwd, work_tree_dot) = if populated {
            (abs_sm, false)
        } else {
            (gd.clone(), true)
        };
        work.push(SubmoduleFetchWork {
            process_cwd,
            work_tree_dot,
            display_path: cs.path.clone(),
            remote,
            default_token: "on-demand",
            git_dir: gd.clone(),
            at_commit: Some(cs.super_oid),
            needed_commits: cs.new_commits.clone(),
        });
    }

    let jobs = fetch_parallel_job_count(args, config).max(1);
    trace_parallel_tasks(jobs);

    let grit_bin = grit_exe::grit_executable();
    let prefix_raw = args.submodule_prefix.as_deref().unwrap_or("");
    let prefix_trim = prefix_raw.trim_end_matches('/');
    let forward = forward_parent_fetch_flags(cmd_recurse, args);

    for w in work {
        let full_prefix = if prefix_trim.is_empty() {
            format!("{}/", w.display_path)
        } else {
            format!("{prefix_trim}/{}/", w.display_path)
        };
        let stderr_path = if prefix_trim.is_empty() {
            w.display_path.clone()
        } else {
            format!("{prefix_trim}/{}", w.display_path)
        };
        if !args.quiet {
            match &w.at_commit {
                Some(super_oid) => {
                    let abbrev = short_oid(super_oid, super_git_dir);
                    eprintln!("Fetching submodule {stderr_path} at commit {abbrev}");
                }
                None => eprintln!("Fetching submodule {stderr_path}"),
            }
        }

        // First, a normal fetch (default refspec). `--work-tree=.` is a *global* git option (must
        // precede the `fetch` subcommand) used when the submodule work tree is gone so the child
        // doesn't chdir into a stale core.worktree (git submodule.c `get_fetch_task_from_changed`).
        let build_argv = |extra_oids: &[ObjectId]| -> Vec<String> {
            let mut argv: Vec<String> = Vec::new();
            if w.work_tree_dot {
                argv.push("--work-tree=.".into());
            }
            argv.push("fetch".into());
            argv.extend(forward.clone());
            argv.push("--recurse-submodules-default".into());
            argv.push(w.default_token.to_string());
            argv.push(format!("--submodule-prefix={full_prefix}"));
            argv.push(w.remote.clone());
            for o in extra_oids {
                argv.push(o.to_hex());
            }
            argv
        };

        let run_child = |argv: &[String]| -> Result<bool> {
            let trace_argv: Vec<String> = std::iter::once("git".to_string())
                .chain(argv.iter().cloned())
                .collect();
            crate::trace2_emit_git_subcommand_argv(&trace_argv);
            let status = std::process::Command::new(&grit_bin)
                .current_dir(&w.process_cwd)
                .args(argv)
                .status()
                .with_context(|| format!("submodule fetch {}", w.display_path))?;
            Ok(status.success())
        };

        let argv = build_argv(&[]);
        if !run_child(&argv)? {
            bail!("submodule fetch failed for {}", w.display_path);
        }

        // Git's second `oid_fetch_tasks` pass: if a referenced commit is still missing after the
        // normal fetch (commit lives outside the standard refspec), retry by explicit OID.
        if !w.needed_commits.is_empty() {
            let odb = Odb::new(&w.git_dir.join("objects"));
            let missing: Vec<ObjectId> = w
                .needed_commits
                .iter()
                .filter(|oid| !odb.exists(oid))
                .copied()
                .collect();
            if !missing.is_empty() {
                // For the by-OID pass git re-derives the remote by running
                // `git submodule--helper get-default-remote <path>` as a subprocess from the
                // superproject (submodule.c `get_next_submodule` oid_fetch_tasks branch). Mirror
                // that — both to match the resolved remote and to emit the GIT_TRACE line the test
                // checks (t5526 #40). Fall back to the precomputed remote if the helper fails.
                let oid_remote = helper_get_default_remote(&grit_bin, &w.display_path, work_tree)
                    .unwrap_or_else(|| w.remote.clone());
                let mut argv = build_argv(&missing);
                // Replace the remote token (just before the trailing OIDs) with the helper's answer.
                let oid_pos = argv.len() - missing.len();
                argv[oid_pos - 1] = oid_remote;
                if !run_child(&argv)? {
                    bail!("submodule fetch failed for {}", w.display_path);
                }
            }
        }
    }

    Ok(())
}

fn short_oid(oid: &ObjectId, _super_git_dir: &Path) -> String {
    name_rev::abbrev_oid(*oid, 7)
}
