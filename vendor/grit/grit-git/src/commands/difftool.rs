//! `grit difftool` — launch an external diff tool.
//!
//! Parses difftool-specific options, then delegates to [`grit_lib::difftool`].

use crate::explicit_exit::SilentNonZeroExit;
use anyhow::{Context, Result};
use grit_lib::config::ConfigSet;
use grit_lib::difftool::{parse_difftool_argv, run_difftool, DifftoolEnv};
use grit_lib::error::Error;
use grit_lib::repo::Repository;
use std::io::{self};

/// Run `grit difftool` from raw argv (after the subcommand name).
pub fn run_from_argv(argv: Vec<String>) -> Result<()> {
    if argv.len() == 1 {
        match argv[0].as_str() {
            "-h" | "--help" | "--help-all" => {
                if let Some(syn) =
                    crate::commands::upstream_synopsis_help::synopsis_for_builtin("difftool")
                {
                    crate::commands::upstream_synopsis_help::print_upstream_synopsis_stdout_and_exit(
                        "difftool",
                        syn,
                        if argv[0] == "--help" { 0 } else { 129 },
                    );
                }
            }
            _ => {}
        }
    }

    let opts = parse_difftool_argv(&argv).map_err(|e| anyhow::anyhow!("{e}"))?;

    if opts.tool_help {
        let config = ConfigSet::new();
        let mut stdout = io::stdout().lock();
        grit_lib::difftool::print_tool_help(&config, &mut stdout)?;
        return Ok(());
    }

    let env = DifftoolEnv {
        git_diff_tool: std::env::var("GIT_DIFF_TOOL")
            .ok()
            .filter(|s| !s.is_empty()),
        git_difftool_no_prompt: std::env::var("GIT_DIFFTOOL_NO_PROMPT")
            .ok()
            .is_some_and(|s| !s.is_empty()),
        git_difftool_prompt: std::env::var("GIT_DIFFTOOL_PROMPT")
            .ok()
            .is_some_and(|s| !s.is_empty()),
        git_mergetool_gui: match std::env::var("GIT_MERGETOOL_GUI").ok().as_deref() {
            Some("true") => Some(true),
            Some("false") => Some(false),
            _ => None,
        },
        display: std::env::var("DISPLAY").ok(),
    };

    let repo = if opts.no_index {
        None
    } else {
        Some(Repository::discover(None).context("not a git repository")?)
    };

    let config = if let Some(ref r) = repo {
        ConfigSet::load(Some(&r.git_dir), true)?
    } else {
        ConfigSet::load(None, true)?
    };

    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let result = run_difftool(repo.as_ref(), &opts, &env, &config, &mut stdin, &mut stdout)
        .map_err(|e| match e {
            Error::Message(msg) => anyhow::anyhow!("{msg}"),
            other => anyhow::anyhow!("{other}"),
        })?;

    if result.exit_code != 0 {
        return Err(SilentNonZeroExit {
            code: result.exit_code,
        }
        .into());
    }
    Ok(())
}

/// Legacy clap entry — forwards raw argv captured by main.
pub fn run(args: Vec<String>) -> Result<()> {
    run_from_argv(args)
}
