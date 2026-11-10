//! `grit-git completions <shell>` — emit shell completion scripts.
//!
//! grit uses manual pre-dispatch (see `main.rs`) and never assembles a full
//! clap `Command` tree at runtime, because building a parser for all 169
//! subcommands on every invocation would be wasteful. `clap_complete` does need
//! that tree, so [`build_cli`] reconstructs it on demand — only when generating
//! completions: every command in [`crate::KNOWN_COMMANDS`] becomes a
//! subcommand, and the commands that expose a clap `Args` struct contribute
//! their full option set (and any nested subcommands, e.g. `config get`) via
//! `augment_args`. Commands without an `Args` struct still complete by name.

use anyhow::{bail, Result};
use clap::{Arg, ArgAction, Command, ValueEnum, ValueHint};
use clap_complete::Shell;

use crate::commands;

/// Per-command one-line summaries, parsed from the same asset `grit-git help -a`
/// prints, so the completion menu shows identical descriptions.
const ALL_COMMANDS_HELP: &str = include_str!("help/assets/all_commands_help.txt");

/// `grit-git completions <shell>` — write a completion script for `<shell>` to stdout.
pub fn run(rest: &[String]) -> Result<()> {
    // A bare `-h`/`--help` (with no shell) prints usage rather than erroring.
    let shell_arg = rest.iter().find(|a| !a.starts_with('-'));
    let wants_help = rest.iter().any(|a| a == "-h" || a == "--help");

    let Some(shell_name) = shell_arg else {
        if wants_help {
            print!("{}", usage());
            return Ok(());
        }
        bail!("{}", usage());
    };

    let shell = <Shell as ValueEnum>::from_str(shell_name, true).map_err(|_| {
        anyhow::anyhow!(
            "unknown shell '{shell_name}'; expected one of: {}",
            shell_list()
        )
    })?;

    let mut cmd = build_cli();
    clap_complete::generate(shell, &mut cmd, "grit-git", &mut std::io::stdout());
    Ok(())
}

fn shell_list() -> String {
    Shell::value_variants()
        .iter()
        .filter_map(|s| s.to_possible_value().map(|v| v.get_name().to_string()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn usage() -> String {
    format!(
        "usage: grit-git completions <{}>\n",
        shell_list().replace(", ", "|")
    )
}

/// Assemble the full clap `Command` tree, used only for completion generation.
pub(crate) fn build_cli() -> Command {
    let mut cli = Command::new("grit-git")
        .about("A Git implementation in Rust")
        .disable_help_subcommand(true);

    cli = augment_globals(cli);

    for &name in crate::KNOWN_COMMANDS {
        let mut sub = augment_command(name, Command::new(name));
        if let Some(desc) = description_for(name) {
            sub = sub.about(desc);
        }
        cli = cli.subcommand(robust_subcommand(name, sub));
    }

    // `completions` is grit-specific (not in KNOWN_COMMANDS); register it so the
    // generated script can complete `grit-git completions <shell>` too.
    cli.subcommand(
        Command::new("completions")
            .about("Generate shell completion scripts")
            .arg(
                Arg::new("shell")
                    .required(true)
                    .value_parser(clap::value_parser!(Shell)),
            ),
    )
}

/// Guard against latent clap conflicts in a command's `Args`.
///
/// In debug builds clap runs duplicate-argument assertions when a command tree
/// is built. A few of grit's `Args` structs have conflicts that only trip these
/// debug-only checks — e.g. `config`'s `global` `--default` collides with
/// `config get`'s own `--default` once the subcommand tree is assembled. Rather
/// than let one such command abort the whole generator, probe each subcommand in
/// isolation and fall back to name-only completion when it fails. Release builds
/// compile out clap's assertions, so they always get the full option set.
fn robust_subcommand(name: &'static str, sub: Command) -> Command {
    #[cfg(debug_assertions)]
    {
        let probe = sub.clone();
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let ok =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| probe.debug_assert())).is_ok();
        std::panic::set_hook(prev_hook);
        if !ok {
            let mut bare = Command::new(name);
            if let Some(desc) = description_for(name) {
                bare = bare.about(desc);
            }
            return bare;
        }
    }
    let _ = name;
    sub
}

/// grit's global options, accepted before the subcommand (see `extract_globals`).
fn augment_globals(cmd: Command) -> Command {
    cmd.arg(
        Arg::new("version")
            .short('v')
            .long("version")
            .action(ArgAction::SetTrue)
            .help("Show the grit version"),
    )
    .arg(
        Arg::new("change-dir")
            .short('C')
            .value_name("path")
            .value_hint(ValueHint::DirPath)
            .help("Run as if grit was started in <path>"),
    )
    .arg(
        Arg::new("config")
            .short('c')
            .value_name("name=value")
            .help("Pass a configuration parameter to the command"),
    )
    .arg(
        Arg::new("config-env")
            .long("config-env")
            .value_name("name=envvar")
            .help("Like -c but the value is the name of an environment variable"),
    )
    .arg(
        Arg::new("exec-path")
            .long("exec-path")
            .value_name("path")
            .value_hint(ValueHint::DirPath)
            .num_args(0..=1)
            .help("Path to wherever your core Git programs are installed"),
    )
    .arg(
        Arg::new("git-dir")
            .long("git-dir")
            .value_name("path")
            .value_hint(ValueHint::DirPath)
            .help("Set the path to the repository (.git directory)"),
    )
    .arg(
        Arg::new("work-tree")
            .long("work-tree")
            .value_name("path")
            .value_hint(ValueHint::DirPath)
            .help("Set the path to the working tree"),
    )
    .arg(
        Arg::new("namespace")
            .long("namespace")
            .value_name("name")
            .help("Set the Git namespace"),
    )
    .arg(
        Arg::new("bare")
            .long("bare")
            .action(ArgAction::SetTrue)
            .help("Treat the repository as a bare repository"),
    )
    .arg(
        Arg::new("paginate")
            .short('p')
            .long("paginate")
            .action(ArgAction::SetTrue)
            .help("Pipe all output into less (or $PAGER)"),
    )
    .arg(
        Arg::new("no-pager")
            .short('P')
            .long("no-pager")
            .action(ArgAction::SetTrue)
            .help("Do not pipe Git output into a pager"),
    )
}

/// Attach a command's clap `Args` (options + nested subcommands) when it has one.
///
/// The set mirrors [`crate::print_completion_helper`]; commands without an `Args`
/// struct (manually parsed, e.g. `merge-tree`) fall through as bare subcommands.
fn augment_command(name: &str, cmd: Command) -> Command {
    macro_rules! aug {
        ($ty:path) => {
            <$ty as clap::Args>::augment_args(cmd)
        };
    }
    match name {
        "add" => aug!(commands::add::Args),
        "am" => aug!(commands::am::Args),
        "apply" => aug!(commands::apply::Args),
        "bisect" => aug!(commands::bisect::Args),
        "blame" => aug!(commands::blame::Args),
        "branch" => aug!(commands::branch::Args),
        "cat-file" => aug!(commands::cat_file::Args),
        "check-ignore" => aug!(commands::check_ignore::Args),
        "checkout" => aug!(commands::checkout::Args),
        "cherry-pick" => aug!(commands::cherry_pick::Args),
        "clean" => aug!(commands::clean::Args),
        "clone" => aug!(commands::clone::Args),
        "commit" => aug!(commands::commit::Args),
        "config" => aug!(commands::config::Args),
        "describe" => aug!(commands::describe::Args),
        "diff" => aug!(commands::diff::Args),
        "fetch" => aug!(commands::fetch::Args),
        "for-each-ref" => aug!(commands::for_each_ref::Args),
        "format-patch" => aug!(commands::format_patch::Args),
        "fsck" => aug!(commands::fsck::Args),
        "gc" => aug!(commands::gc::Args),
        "grep" => aug!(commands::grep::Args),
        "init" => aug!(commands::init::Args),
        "log" => aug!(commands::log::Args),
        "ls-files" => aug!(commands::ls_files::Args),
        "ls-remote" => aug!(commands::ls_remote::Args),
        "ls-tree" => aug!(commands::ls_tree::Args),
        "merge" => aug!(commands::merge::Args),
        "merge-base" => aug!(commands::merge_base::Args),
        "multi-pack-index" => aug!(commands::multi_pack_index::Args),
        "mv" => aug!(commands::mv::Args),
        "notes" => aug!(commands::notes::Args),
        "pull" => aug!(commands::pull::Args),
        "push" => aug!(commands::push::Args),
        "rebase" => aug!(commands::rebase::Args),
        "reflog" => aug!(commands::reflog::Args),
        "remote" => aug!(commands::remote::Args),
        "reset" => aug!(commands::reset::Args),
        "restore" => aug!(commands::restore::Args),
        "rev-list" => aug!(commands::rev_list::Args),
        "rev-parse" => aug!(commands::rev_parse::Args),
        "revert" => aug!(commands::revert::Args),
        "rm" => aug!(commands::rm::Args),
        "show" => aug!(commands::show::Args),
        "show-ref" => aug!(commands::show_ref::Args),
        "sparse-checkout" => aug!(commands::sparse_checkout::Args),
        "stash" => aug!(commands::stash::Args),
        "status" => aug!(commands::status::Args),
        "submodule" => aug!(commands::submodule::Args),
        "switch" => aug!(commands::switch::Args),
        "symbolic-ref" => aug!(commands::symbolic_ref::Args),
        "tag" => aug!(commands::tag::Args),
        "update-index" => aug!(commands::update_index::Args),
        "update-ref" => aug!(commands::update_ref::Args),
        "version" => aug!(commands::version::Args),
        "worktree" => aug!(commands::worktree::Args),
        _ => cmd,
    }
}

/// One-line description for `name`, parsed from the `help -a` asset.
///
/// Command rows are indented exactly three spaces (`   add    Add file ...`);
/// section headers have no leading whitespace and are skipped.
fn description_for(name: &str) -> Option<&'static str> {
    for line in ALL_COMMANDS_HELP.lines() {
        let Some(row) = line.strip_prefix("   ") else {
            continue;
        };
        if row.starts_with(' ') {
            continue;
        }
        let mut parts = row.splitn(2, char::is_whitespace);
        if parts.next() != Some(name) {
            continue;
        }
        let desc = parts.next().unwrap_or("").trim();
        if !desc.is_empty() {
            return Some(desc);
        }
    }
    None
}
