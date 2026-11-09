//! `grit fast-export` — export repository as a fast-import stream.

use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use grit_lib::fast_export::{export_stream, FastExportOptions};
use grit_lib::repo::Repository;
use std::io;

/// Arguments for `grit fast-export`.
#[derive(Debug, ClapArgs)]
#[command(about = "Export repository as fast-import stream")]
pub struct Args {
    /// Export all local branches (`refs/heads/`) and reachable objects.
    #[arg(long)]
    pub all: bool,

    /// Anonymize paths, identities, messages, and opaque OIDs.
    #[arg(long)]
    pub anonymize: bool,

    /// Map a token in anonymized output (`from` or `from:to`).
    #[arg(long = "anonymize-map", value_name = "MAP")]
    pub anonymize_map: Vec<String>,

    /// Raw arguments for compatibility (`--all`, `--anonymize`, etc. may appear here).
    #[arg(value_name = "ARG", num_args = 0.., allow_hyphen_values = true, trailing_var_arg = true)]
    pub args: Vec<String>,
}

/// Run `grit fast-export`.
pub fn run(args: Args) -> Result<()> {
    let repo = Repository::discover(None).context("not a git repository")?;

    let mut all = args.all;
    let mut anonymize = args.anonymize;
    let mut maps = args.anonymize_map;
    let mut no_data = false;
    let mut use_done_feature = false;
    let mut revisions = Vec::new();
    let mut paths = Vec::new();
    let mut after_dashdash = false;

    for a in &args.args {
        if after_dashdash {
            paths.push(a.clone());
            continue;
        }
        if a == "--" {
            after_dashdash = true;
            continue;
        }
        if let Some(map) = a.strip_prefix("--anonymize-map=") {
            maps.push(map.to_string());
            continue;
        }
        match a.as_str() {
            "--all" => all = true,
            "--anonymize" => anonymize = true,
            "--no-data" => no_data = true,
            "--use-done-feature" => use_done_feature = true,
            _ if a.starts_with("--") => {}
            _ => {
                revisions.push(a.clone());
            }
        }
    }

    if revisions.is_empty() && !all {
        revisions.push("HEAD".to_string());
    }

    if !all && revisions.len() == 1 && revisions[0] == "HEAD" {
        if let Some(head) = grit_lib::refs::read_head(&repo.git_dir)? {
            revisions[0] = head;
        }
    }

    if all {
        revisions.clear();
    }

    let opts = FastExportOptions {
        all,
        anonymize,
        anonymize_maps: maps,
        no_data,
        use_done_feature,
        revisions,
        paths,
    };

    let stdout = io::stdout().lock();
    export_stream(&repo, stdout, &opts).map_err(|e| anyhow::anyhow!("{e}"))
}
