//! `filter-branch` requires a native implementation before it can be enabled.
//!
//! The donor delegated to the installed Git shell program. Gitr never treats
//! that execution as a Rust port and fails explicitly instead.
use anyhow::Result;
use clap::Args as ClapArgs;

/// Arguments retained for command discovery and clear failure reporting.
#[derive(Debug, ClapArgs)]
#[command(about = "Rewrite branches (native implementation not yet available)")]
pub struct Args {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

/// Report the missing native command without invoking an external Git program.
pub fn run(_args: Args) -> Result<()> {
    anyhow::bail!("filter-branch is not implemented natively; Gitr does not delegate to the system Git shell program")
}
