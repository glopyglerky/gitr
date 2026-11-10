//! `grit sh-setup` — shell setup helpers.
//!
//! Helper for shell scripts that provides common setup functions like
//! `die`, `require_work_tree`, `cd_to_toplevel`, etc.  Gitr does not ship generated shell implementations.
//!
//!     . "$(grit --exec-path)/grit-sh-setup"

use anyhow::Result;
use clap::Args as ClapArgs;

/// Arguments for `grit sh-setup`.
#[derive(Debug, ClapArgs)]
#[command(about = "Shell setup helper compatibility is not implemented")]
pub struct Args {
    /// Arguments (unused, accepted for compatibility).
    #[arg(value_name = "ARG", num_args = 0.., trailing_var_arg = true)]
    pub args: Vec<String>,
}

/// Refuse the omitted shell-library interface explicitly.
pub fn run(_args: Args) -> Result<()> {
    anyhow::bail!("git-sh-setup shell library is not included in the Rust-only Gitr distribution")
}
