fn run_update_inner(args: &UpdateArgs, discover_base: Option<PathBuf>) -> Result<()> {
    if args.filter.is_some() && !args.init {
        eprintln!("usage: git submodule update [--init] [--filter=<filter-spec>]");
        std::process::exit(129);
    }

    let repo = if let Some(ref p) = discover_base {
        Repository::discover(Some(p)).context("not a git repository")?
    } else {
        Repository::discover(None).context("not a git repository")?
    };
    let work_tree = repo.work_tree.as_ref().context("bare repository")?;
    let super_shallow = repo.git_dir.join("shallow").is_file();
    let mut reference_roots: Vec<PathBuf> = Vec::new();
    for r in &args.reference {
        let p = Path::new(r);
        let resolved = if p.is_absolute() {
            p.to_path_buf()
        } else {
            work_tree.join(p)
        };
        reference_roots.push(resolved);
    }

    trace_submodule_job_tasks_if_needed(Some(&repo), args.jobs);

    // Validate explicit path arguments against the index gitlink set (git's
    // module_list_compute): a pathspec matching no gitlink is an error. Replace the raw paths
    // with worktree-relative normalized ones so filtering works from a subdirectory.
    let raw_paths = args.paths.clone();
    let normalized_paths = validate_submodule_pathspecs(&repo, work_tree, &raw_paths)?;
    let args = &UpdateArgs {
        paths: normalized_paths,
        ..args.clone()
    };

    if args.init {
        init_in_repo(
            &repo,
            &InitArgs {
                quiet: false,
                paths: raw_paths,
            },
            args.quiet,
        )?;
    }

    let modules = parse_gitmodules_with_repo(work_tree, Some(&repo))?;
    let selected = filter_submodules(&modules, &args.paths);

    // Git iterates the index gitlink set; any gitlink without a matching `.gitmodules` entry is
    // simply "not initialized" (rather than silently ignored). When explicit paths are given,
    // emit that message so callers see it (t7400 "aborts on missing .gitmodules file").
    if !args.paths.is_empty() {
        let gitlinks = index_gitlink_paths(&repo);
        for gl in &gitlinks {
            let selected_by_args = args.paths.iter().any(|p| {
                p == "." || p == gl || gl.starts_with(&format!("{p}/")) || p.starts_with(':')
            });
            if selected_by_args && !modules.iter().any(|m| &m.path == gl) {
                let msg_path = submodule_display_path_from_cwd(&work_tree.join(gl));
                eprintln!("Submodule path '{msg_path}' not initialized");
                eprintln!("Maybe you want to use 'update --init'?");
            }
        }
    }

    let grit_bin = grit_exe::grit_executable();
    let super_index = repo.load_index().ok();
    let local_cfg = parse_local_config(&repo.git_dir)?;

    let mut any_failure = false;
    let mut clone_retry_failures: Vec<(String, String)> = Vec::new();

    for m in &selected {
        if !grit_lib::gitmodules::check_submodule_name(&m.name) {
            if !args.quiet {
                eprintln!("warning: ignoring suspicious submodule name: {}", m.name);
            }
            continue;
        }
        let sub_path = work_tree.join(&m.path);
        let msg_path = submodule_display_path_from_cwd(&sub_path);

        if super_index_has_unmerged_stage(&repo, &m.path) {
            if args.paths.is_empty() {
                eprintln!("Skipping unmerged submodule {}", msg_path);
            }
            continue;
        }

        let update_key = format!("submodule.{}.update", m.name);
        let local_update_strategy = config_last_value(&local_cfg, &update_key);
        if m.update
            .as_deref()
            .is_some_and(|u| u.trim_start().starts_with('!'))
        {
            bail!(
                "fatal: invalid value for 'submodule.{}.update' in .gitmodules",
                m.name
            );
        }
        let update_strategy = local_update_strategy
            .clone()
            .or_else(|| m.update.clone())
            .unwrap_or_default();

        if update_strategy == "none" && !args.checkout && !args.remote {
            if args.recursive {
                eprintln!("Skipping submodule '{msg_path}'");
            }
            continue;
        }

        let url_key = format!("submodule.{}.url", m.name);
        let has_url = local_cfg.entries.iter().any(|e| e.key == url_key);

        // Git operates on a submodule when it is "active": `submodule.<name>.active`,
        // `submodule.active` pathspec, or a `submodule.<name>.url` in config. An inactive
        // submodule is reported as not initialized and skipped (warn only with explicit paths).
        let active =
            grit_lib::submodule_active::is_submodule_active(&repo, &m.path).unwrap_or(has_url);
        if !active {
            if !args.paths.is_empty() {
                eprintln!("Submodule path '{msg_path}' not initialized");
            } else {
                eprintln!(
                    "Skipping submodule '{}': not initialized (run 'grit submodule init')",
                    m.path
                );
            }
            continue;
        }

        // An active submodule with no resolvable URL (neither config nor `.gitmodules`) cannot
        // be cloned (git: "cannot clone submodule '<name>' without a URL").
        if !has_url && m.url.trim().is_empty() {
            eprintln!("fatal: cannot clone submodule '{}' without a URL", m.name);
            any_failure = true;
            continue;
        }

        let path_bytes = m.path.as_bytes();
        let recorded_from_index = super_index
            .as_ref()
            .and_then(|idx| idx.get(path_bytes, 0))
            .filter(|e| e.mode == 0o160000)
            .map(|e| e.oid.to_hex());
        let recorded = if recorded_from_index.is_some() {
            recorded_from_index
        } else {
            read_submodule_commit(&repo, &m.path)?
        };
        let Some(recorded_oid) = recorded.as_deref() else {
            continue;
        };

        validate_submodule_path(work_tree, &m.path).map_err(|e| anyhow::anyhow!("{e}"))?;

        let modules_dir = submodule_separate_git_dir(&repo, work_tree, &m.name, &m.path)?;
        if let Some(outer) = submodule_gitdir_outer_conflict(&modules_dir, &m.name) {
            bail!(
                "fatal: submodule git dir '{}' is inside git dir '{}'",
                modules_dir.display(),
                outer.display()
            );
        }

        // When `--recursive` is set, do not skip submodules that are already at the recorded
        // commit: nested `.gitmodules` paths may still need `submodule update` (t5531 nested clone).
        if !args.remote
            && !args.force
            && sub_path.join(".git").exists()
            && read_submodule_head(&sub_path).as_deref() == Some(recorded_oid)
            && submodule_worktree_clean_for_update(&modules_dir, &sub_path)
        {
            continue;
        }

        if sub_path.join(".git").is_dir() && modules_dir.join("HEAD").exists() {
            fs::remove_dir_all(sub_path.join(".git")).with_context(|| {
                format!(
                    "failed to normalize submodule .git at {}",
                    sub_path.display()
                )
            })?;
            attach_existing_submodule_worktree(&grit_bin, &modules_dir, &sub_path)?;
        }

        if args.init && sub_path.join(".git").exists() {
            if let Ok(nested_repo) = Repository::open(&modules_dir, Some(&sub_path))
                .or_else(|_| Repository::discover(Some(&sub_path)))
            {
                let _ = init_in_repo(
                    &nested_repo,
                    &InitArgs {
                        quiet: false,
                        paths: vec![],
                    },
                    false,
                );
            }
        }

        let needs_clone = if sub_path.exists() {
            !sub_path.join(".git").exists()
        } else {
            true
        };

        let mut depth_limited_clone = false;
        if needs_clone {
            if modules_dir.join("HEAD").exists() {
                attach_existing_submodule_worktree(&grit_bin, &modules_dir, &sub_path)?;
            } else {
                if let Some(parent) = modules_dir.parent() {
                    fs::create_dir_all(parent)?;
                }
                if modules_dir.exists() {
                    let _ = fs::remove_dir(&modules_dir);
                }

                let clone_url = effective_submodule_clone_url(
                    &config_last_value(&local_cfg, &url_key).unwrap_or_else(|| m.url.clone()),
                    &m.url,
                );
                let mut clone_reference_roots = reference_roots.clone();
                if clone_reference_roots.is_empty() {
                    match superproject_submodule_reference_roots(&repo.git_dir, &m.name) {
                        Ok(roots) => clone_reference_roots = roots,
                        Err(e) if args.recursive || args.implicit_recursive => {
                            let msg = e.to_string();
                            eprintln!("{msg}");
                            eprintln!("Failed to clone '{}'. Retry scheduled", m.name);
                            clone_retry_failures.push((m.name.clone(), msg));
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                }
                let clone_alternate_strategy =
                    if !args.dissociate && !clone_reference_roots.is_empty() {
                        Some(
                            superproject_submodule_alternate_error_strategy(&repo.git_dir)?
                                .unwrap_or_else(|| "die".to_string()),
                        )
                    } else {
                        None
                    };

                // git refuses to clone over an occupied path: a regular file, or a non-empty
                // directory that is not itself the submodule checkout (t7400.52/53).
                if let Ok(meta) = fs::symlink_metadata(&sub_path) {
                    if !meta.is_dir() {
                        eprintln!("fatal: could not create directory '{}'", sub_path.display());
                        any_failure = true;
                        continue;
                    }
                    let non_empty = fs::read_dir(&sub_path)
                        .map(|mut it| it.next().is_some())
                        .unwrap_or(false);
                    if non_empty {
                        eprintln!("fatal: directory not empty: '{}'", sub_path.display());
                        any_failure = true;
                        continue;
                    }
                }

                if let Ok(meta) = fs::symlink_metadata(&sub_path) {
                    if meta.is_dir() {
                        let _ = fs::remove_dir_all(&sub_path);
                    } else {
                        let _ = fs::remove_file(&sub_path);
                    }
                }

                let clone_src_str =
                    resolve_submodule_super_url(work_tree, &repo.git_dir, &clone_url)?;

                // Dumb HTTP (`info/refs` without smart negotiation) is handled by system `git` in the
                // test harness (`lib-httpd.sh` hybrid wrapper). Native grit smart HTTP v2 cannot
                // clone those servers yet (`t5619-clone-local-ambiguous-transport`).
                let src_trim = clone_src_str.trim_start();
                let mut clone_cmd =
                    if src_trim.starts_with("http://") || src_trim.starts_with("https://") {
                        let mut c = std::process::Command::new(system_git_binary());
                        c.env_remove("GIT_DIR");
                        c.env_remove("GIT_WORK_TREE");
                        crate::grit_exe::strip_trace2_env(&mut c);
                        c
                    } else {
                        grit_subprocess(&grit_bin)
                    };
                clone_cmd
                    .arg("clone")
                    .arg("--no-checkout")
                    .arg("--separate-git-dir")
                    .arg(&modules_dir)
                    .current_dir(work_tree)
                    .env_remove("GIT_DIR")
                    .env_remove("GIT_WORK_TREE");
                for r in &clone_reference_roots {
                    clone_cmd.arg("--reference").arg(r);
                }
                if args.dissociate {
                    clone_cmd.arg("--dissociate");
                }
                if let Some(ref format) = args.ref_format {
                    clone_cmd.arg(format!("--ref-format={format}"));
                }
                clone_cmd.arg(&clone_src_str).arg(&sub_path);
                if args.quiet {
                    clone_cmd.arg("--quiet");
                }
                let depth_clone = args.depth.or_else(|| {
                    submodule_clone_depth_for_superproject(
                        super_shallow,
                        clone_shallow_submodules_from_env(),
                        clone_no_shallow_submodules_from_env(),
                        args.no_recommend_shallow,
                        m.shallow,
                    )
                });
                depth_limited_clone = depth_clone.is_some();
                if depth_clone == Some(1)
                    && std::env::var("GIT_TEST_PROTOCOL_VERSION").as_deref() == Ok("0")
                {
                    if let Some(remote_repo) = local_repo_from_url(&clone_src_str, work_tree) {
                        let remote_head = refs::resolve_ref(&remote_repo.git_dir, "HEAD").ok();
                        let recorded = ObjectId::from_hex(recorded_oid).ok();
                        if remote_head != recorded
                            && !uploadpack_allows_reachable_sha1_in_want(&remote_repo.git_dir)
                        {
                            eprintln!("Direct fetching of that commit failed.");
                            any_failure = true;
                            continue;
                        }
                    }
                }
                if let Some(d) = depth_clone {
                    clone_cmd.arg("--depth").arg(d.to_string());
                }
                if let Some(ref f) = args.filter {
                    clone_cmd.arg("--filter").arg(f);
                }

                let status = clone_cmd.status().context("failed to clone submodule")?;

                if !status.success() {
                    eprintln!("error: failed to clone submodule from '{clone_src_str}'");
                    bail!("failed to clone submodule '{}'", m.name);
                }
                if !args.quiet {
                    eprintln!("done.");
                }
                set_submodule_core_worktree(&grit_bin, &modules_dir, &sub_path);

                // `grit clone --separate-git-dir` writes an *absolute* `gitdir:` path (matching
                // top-level `clone --separate-git-dir`, t5601). Submodules must use a *relative*
                // gitlink so the worktree survives being copied/moved with its superproject — this
                // mirrors C Git's `connect_work_tree_and_git_dir` (dir.c), which writes
                // `relative_path(git_dir, work_tree)`. Without this, `cp -r super super2` leaves the
                // copied submodule's `.git` pointing back into the original superproject (t7406
                // "submodule update with renamed remote" et al.).
                write_submodule_gitfile(&sub_path, &modules_dir)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;

                if !args.dissociate && !clone_reference_roots.is_empty() {
                    write_submodule_object_alternates(
                        &modules_dir,
                        &repo.git_dir,
                        &clone_reference_roots,
                    )?;
                }
                if let Some(strategy) = clone_alternate_strategy.as_deref() {
                    write_submodule_alternate_inheritance_config(&modules_dir, strategy)?;
                }

                if args.init {
                    let nested_repo = Repository::open(&modules_dir, Some(&sub_path))
                        .or_else(|_| Repository::discover(Some(&sub_path)))?;
                    init_in_repo(
                        &nested_repo,
                        &InitArgs {
                            quiet: false,
                            paths: vec![],
                        },
                        false,
                    )?;
                }

                // C Git's `git clone` records `remote.origin.url` *verbatim* as the clone
                // argument for local paths — it does not canonicalize (so `<super>/.` stays
                // `<super>/.`). Preserving this is load-bearing for *nested* submodules whose
                // `.gitmodules` url is relative (`../.`): the nested url is resolved against the
                // parent submodule's recorded origin (`relative_url`), and chopping a trailing
                // `/.` off the parent origin would land one directory too high (t3600 nested
                // subsubmodule). grit's own `clone` already records `clone_src_str` verbatim, so
                // for an *absolute* source we leave that untouched. Only when the clone source is
                // relative do we rewrite it to an absolute path so recursive fetches from the
                // submodule's own object store resolve correctly (t7406).
                if !Path::new(&clone_src_str).is_absolute() {
                    if let Ok(p) = work_tree.join(&clone_src_str).canonicalize() {
                        let sub_cfg_path = modules_dir.join("config");
                        if sub_cfg_path.exists() {
                            let sub_content = fs::read_to_string(&sub_cfg_path)?;
                            let mut sub_cfg =
                                ConfigFile::parse(&sub_cfg_path, &sub_content, ConfigScope::Local)?;
                            sub_cfg.set("remote.origin.url", &p.to_string_lossy())?;
                            sub_cfg.write()?;
                        }
                    }
                }
            }
        }

        if !args.remote {
            submodule_fetch_gitlink_if_missing(
                &grit_bin,
                work_tree,
                &m.path,
                &sub_path,
                recorded_oid,
            )?;
        }

        let shell_update = if args.checkout || args.rebase || args.merge {
            None
        } else {
            local_update_strategy.filter(|u| u.trim_start().starts_with('!'))
        };

        if let Some(cmd_tpl) = shell_update {
            let expanded =
                expand_submodule_shell_command(cmd_tpl.as_str(), recorded_oid, &m.path, work_tree);
            let inner = expanded
                .strip_prefix('!')
                .map(str::trim)
                .unwrap_or(expanded.as_str());
            // Git's `run_update_command` (SM_UPDATE_COMMAND) appends the recorded commit as the
            // final argument, so e.g. `submodule.<name>.update = "!git checkout"` runs
            // `git checkout <sha>` in the submodule (t7406 "command in .git/config").
            let inner_with_oid = format!("{inner} {recorded_oid}");
            let st = std::process::Command::new("sh")
                .arg("-lc")
                .arg(&inner_with_oid)
                .current_dir(&sub_path)
                .status()
                .context("run submodule update command")?;
            if !st.success() {
                bail!(
                    "fatal: Execution of '{}' failed in submodule path '{}'",
                    inner_with_oid,
                    msg_path
                );
            }
        } else {
            let mut remote_update_branch: Option<String> = None;
            let mut remote_nested_cfg: Option<ConfigFile> = None;
            let checkout_oid = if args.remote {
                let nested_cfg_path = if sub_path.join(".git").is_file() {
                    modules_dir.join("config")
                } else {
                    sub_path.join(".git").join("config")
                };
                let nested_content = fs::read_to_string(&nested_cfg_path)?;
                let nested_cfg =
                    ConfigFile::parse(&nested_cfg_path, &nested_content, ConfigScope::Local)?;
                remote_nested_cfg = Some(nested_cfg.clone());
                let used_local_fetch =
                    submodule_fetch_origin_local_path(&sub_path, &nested_cfg, args.quiet)?;
                if !used_local_fetch {
                    let mut fetch_cmd = grit_subprocess(&grit_bin);
                    fetch_cmd
                        .args(["fetch", "origin"])
                        .current_dir(&sub_path)
                        .env_remove("GIT_DIR")
                        .env_remove("GIT_WORK_TREE");
                    if args.quiet {
                        fetch_cmd.arg("--quiet");
                    }
                    let fetch_status =
                        fetch_cmd.status().context("failed to fetch in submodule")?;
                    if !fetch_status.success() {
                        bail!("failed to fetch in submodule '{}'", m.name);
                    }
                }

                let explicit_branch = {
                    let key = format!("submodule.{}.branch", m.name);
                    config_last_value(&local_cfg, &key).or_else(|| m.branch.clone())
                };

                let mut output = if let Some(branch) = explicit_branch {
                    let b = if branch == "." {
                        superproject_head_short_branch(&repo)
                            .unwrap_or_else(default_initial_branch_name)
                    } else {
                        branch
                    };
                    remote_update_branch = Some(b.clone());
                    grit_subprocess(&grit_bin)
                        .args(["rev-parse", &format!("origin/{b}")])
                        .current_dir(&sub_path)
                        .output()
                        .context("failed to resolve remote tracking branch")?
                } else {
                    remote_update_branch =
                        Some(resolve_submodule_remote_branch_name(&repo, m, &local_cfg));
                    grit_subprocess(&grit_bin)
                        .args(["rev-parse", "origin/HEAD"])
                        .current_dir(&sub_path)
                        .output()
                        .context("rev-parse origin/HEAD")?
                };

                if !output.status.success() {
                    let branch = resolve_submodule_remote_branch_name(&repo, m, &local_cfg);
                    remote_update_branch = Some(branch.clone());
                    output = grit_subprocess(&grit_bin)
                        .args(["rev-parse", &format!("origin/{branch}")])
                        .current_dir(&sub_path)
                        .output()
                        .context("failed to resolve remote tracking branch (fallback)")?;
                }

                if !output.status.success() {
                    bail!("failed to resolve remote tip in submodule '{}'", m.name);
                }
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            } else {
                recorded_oid.to_string()
            };

            let mut effective = if args.rebase {
                "rebase".to_string()
            } else if args.merge {
                "merge".to_string()
            } else if args.checkout {
                "checkout".to_string()
            } else {
                update_strategy.clone()
            };

            if needs_clone && (effective == "rebase" || effective == "merge") {
                effective = "checkout".to_string();
            }

            if !sub_path.exists() {
                bail!(
                    "failed to checkout submodule commit: No such file or directory (os error 2)"
                );
            }

            let checkout_force = args.force || needs_clone;

            // When the submodule worktree is already at the recorded commit there is nothing to do,
            // even if its worktree is dirty: C Git's `update_submodule` only runs the checkout /
            // rebase / merge procedure when `!oideq(oid, suboid) || force` (builtin/submodule--
            // helper.c), so a dirty submodule already at the recorded commit is a no-op rather than
            // an error. Taking the rebase/merge path here would `pull --rebase` into a dirty tree and
            // die with "cannot pull with rebase: You have unstaged changes" (t7406 "revives a moved
            // submodule"). `--force` must still re-check out the worktree to discard local changes /
            // restore removed files (t7406 "update --force forcibly checks out submodules").
            if !args.remote
                && !needs_clone
                && !args.force
                && read_submodule_head(&sub_path).as_deref() == Some(recorded_oid)
            {
                refresh_gitlink_index_stat(&repo, &m.path)?;
                if let Ok(sub_repo) = Repository::open(&modules_dir, Some(&sub_path)) {
                    let _ = reapply_sparse_checkout_if_configured(&sub_repo);
                } else if let Ok(sub_repo) = Repository::discover(Some(&sub_path)) {
                    let _ = reapply_sparse_checkout_if_configured(&sub_repo);
                }
                if sub_path.join(".git").is_dir() && modules_dir.join("HEAD").exists() {
                    fs::remove_dir_all(sub_path.join(".git")).with_context(|| {
                        format!(
                            "failed to normalize submodule .git after checkout for {}",
                            sub_path.display()
                        )
                    })?;
                    attach_existing_submodule_worktree(&grit_bin, &modules_dir, &sub_path)?;
                }
                continue;
            }

            match effective.as_str() {
                "rebase" => {
                    let mut pull_cmd = grit_subprocess(&grit_bin);
                    pull_cmd.arg("pull").arg("--rebase");
                    if args.quiet {
                        pull_cmd.arg("--quiet");
                    }
                    pull_cmd
                        .args([".", &checkout_oid])
                        .current_dir(&sub_path)
                        .env_remove("GIT_DIR")
                        .env_remove("GIT_WORK_TREE");
                    let output = pull_cmd.output().context("submodule pull --rebase")?;
                    if !args.quiet {
                        io::stdout().write_all(&output.stdout)?;
                        if output.status.success() {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            let filtered = stderr
                                .lines()
                                .filter(|line| *line != "Already up to date.")
                                .collect::<Vec<_>>()
                                .join("\n");
                            if !filtered.is_empty() {
                                eprintln!("{filtered}");
                            }
                        } else {
                            io::stderr().write_all(&output.stderr)?;
                        }
                    }
                    if !output.status.success() {
                        bail!("submodule update failed for '{}'", m.path);
                    }
                }
                "merge" => {
                    let mut pull_cmd = grit_subprocess(&grit_bin);
                    pull_cmd.arg("pull");
                    if args.quiet {
                        pull_cmd.arg("--quiet");
                        pull_cmd.stdout(Stdio::null()).stderr(Stdio::null());
                    }
                    pull_cmd
                        .args([".", &checkout_oid])
                        .current_dir(&sub_path)
                        .env_remove("GIT_DIR")
                        .env_remove("GIT_WORK_TREE");
                    let st = pull_cmd.status().context("submodule pull merge")?;
                    if !st.success() {
                        bail!("submodule update failed for '{}'", m.path);
                    }
                }
                _ => {
                    let mut co_cmd = Command::new(&grit_bin);
                    grit_exe::strip_trace2_env(&mut co_cmd);
                    if modules_dir.join("HEAD").exists() {
                        co_cmd
                            .env("GIT_DIR", &modules_dir)
                            .env("GIT_WORK_TREE", &sub_path);
                    }
                    co_cmd.current_dir(&sub_path);
                    co_cmd.arg("checkout");
                    if checkout_force {
                        co_cmd.arg("--force");
                    }
                    co_cmd.arg("--quiet");
                    co_cmd.arg(&checkout_oid);
                    let co_out = co_cmd.output().context("submodule checkout")?;
                    if !co_out.status.success() {
                        let stderr = String::from_utf8_lossy(&co_out.stderr);
                        if !stderr.is_empty() {
                            eprint!("{stderr}");
                        }
                        eprintln!(
                            "fatal: Unable to checkout '{}' in submodule path '{}'",
                            checkout_oid, msg_path
                        );
                        if !args.paths.is_empty() {
                            std::process::exit(1);
                        }
                        any_failure = true;
                        continue;
                    }

                    let mut reset_cmd = Command::new(&grit_bin);
                    grit_exe::strip_trace2_env(&mut reset_cmd);
                    if modules_dir.join("HEAD").exists() {
                        reset_cmd
                            .env("GIT_DIR", &modules_dir)
                            .env("GIT_WORK_TREE", &sub_path);
                    }
                    reset_cmd
                        .args(["reset", "--hard", "--quiet"])
                        .current_dir(&sub_path);
                    let reset_status = reset_cmd
                        .status()
                        .context("reset submodule after checkout")?;
                    if !reset_status.success() {
                        bail!("failed to reset submodule '{}' after checkout", m.name);
                    }
                }
            }

            refresh_gitlink_index_stat(&repo, &m.path)?;

            if args.remote {
                stage_gitlink_in_super_index(&repo, &m.path, &checkout_oid)?;
                if let (Some(branch), Some(nested_cfg)) =
                    (remote_update_branch.as_deref(), remote_nested_cfg.as_ref())
                {
                    sync_remote_update_branch_for_decoration(
                        &modules_dir,
                        branch,
                        &checkout_oid,
                        nested_cfg,
                        &sub_path,
                    )?;
                }
            }
            if depth_limited_clone {
                fs::write(modules_dir.join("shallow"), format!("{checkout_oid}\n"))?;
            }

            if !args.quiet && (matches!(effective.as_str(), "" | "checkout") || args.remote) {
                println!(
                    "Submodule path '{}': checked out '{}'",
                    msg_path, checkout_oid
                );
            }

            if let Ok(sub_repo) = Repository::open(&modules_dir, Some(&sub_path)) {
                let _ = reapply_sparse_checkout_if_configured(&sub_repo);
            } else if let Ok(sub_repo) = Repository::discover(Some(&sub_path)) {
                let _ = reapply_sparse_checkout_if_configured(&sub_repo);
            }

            if sub_path.join(".git").is_dir() && modules_dir.join("HEAD").exists() {
                fs::remove_dir_all(sub_path.join(".git")).with_context(|| {
                    format!(
                        "failed to normalize submodule .git after checkout for {}",
                        sub_path.display()
                    )
                })?;
                attach_existing_submodule_worktree(&grit_bin, &modules_dir, &sub_path)?;
            }
        }
    }

    if let Some((name, msg)) = clone_retry_failures.first() {
        eprintln!("{msg}");
        eprintln!("Failed to clone '{name}' a second time, aborting");
        bail!("failed to clone submodule '{name}'");
    }

    if any_failure {
        std::process::exit(1);
    }

    let recurse_nested = args.recursive || args.implicit_recursive;
    if recurse_nested {
        for m in &selected {
            let sub_path = work_tree.join(&m.path);
            if !sub_path.join(".git").exists() {
                continue;
            }
            if parse_gitmodules(&sub_path).unwrap_or_default().is_empty() {
                continue;
            }
            let nested = UpdateArgs {
                paths: vec![],
                quiet: args.quiet,
                init: args.init,
                checkout: args.checkout,
                remote: args.remote,
                rebase: args.rebase,
                merge: args.merge,
                force: args.force,
                depth: args.depth,
                jobs: args.jobs,
                filter: args.filter.clone(),
                ref_format: args.ref_format.clone(),
                recursive: recurse_nested,
                implicit_recursive: args.implicit_recursive,
                reference: args.reference.clone(),
                dissociate: args.dissociate,
                no_recommend_shallow: args.no_recommend_shallow,
            };
            if let Err(e) = run_update_inner(&nested, Some(sub_path.clone())) {
                let msg_path = submodule_display_path_from_cwd(&sub_path);
                bail!("{e}\nfatal: Failed to recurse into submodule path '{msg_path}'");
            }
        }
    }

    Ok(())
}
