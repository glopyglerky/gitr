//! The historical donor self-update command is unavailable in Gitr.

use anyhow::{bail, Result};
use clap::Args as ClapArgs;

/// Retained arguments for an explicit unsupported-command diagnostic.
#[derive(Debug, ClapArgs)]
#[command(about = "Self-update is not implemented in Gitr")]
pub struct Args {
    /// Requested installation directory.
    #[arg(long = "dir", value_name = "DIR")]
    pub dir: Option<String>,
}

pub fn run(_args: Args) -> Result<()> {
    bail!("self-update is not implemented in Gitr; install a qualified Gitr release through your existing installation method")
}
