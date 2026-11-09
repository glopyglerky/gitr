//! `grit update` — self-update grit to the latest release.
//!
//! This is a grit-specific (non-git) command. It runs the official installer at
//! <https://grit-scm.com/install>, which downloads the latest pre-built binary from
//! GitHub Releases for the current platform. By default it installs over the directory
//! of the currently-running `grit` binary so the in-use install is upgraded in place.

use anyhow::{bail, Context, Result};
use clap::Args as ClapArgs;
use std::path::PathBuf;
use std::process::Command;

/// URL of the canonical POSIX-sh installer (single source of truth for platform detection,
/// asset naming, and the install layout).
const INSTALL_URL: &str = "https://grit-scm.com/install";

/// Arguments for `grit update`.
#[derive(Debug, ClapArgs)]
#[command(about = "Update grit to the latest release (downloads from grit-scm.com)")]
pub struct Args {
    /// Install into this directory instead of the running binary's directory.
    #[arg(long = "dir", value_name = "DIR")]
    pub dir: Option<String>,
}

/// Run the `update` command.
pub fn run(args: Args) -> Result<()> {
    // The installer is a POSIX-sh pipeline that shells out to curl + tar.
    for tool in ["sh", "curl"] {
        if find_in_path(tool).is_none() {
            bail!(
                "`{tool}` is required to self-update. Install it, or run the installer manually:\n  curl -fsSL {INSTALL_URL} | sh"
            );
        }
    }

    // Where to install: explicit --dir, else the directory of the running grit binary so the
    // current install is replaced in place; else let the installer use its own default.
    let install_dir: Option<PathBuf> = match &args.dir {
        Some(d) => Some(PathBuf::from(d)),
        None => std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(PathBuf::from)),
    };

    eprintln!(
        "Updating grit (current: git version {})",
        crate::version_string()
    );
    match &install_dir {
        Some(d) => eprintln!("Install directory: {}", d.display()),
        None => eprintln!("Install directory: installer default ($HOME/.local/bin)"),
    }

    // Run `curl -fsSL <url> | sh`, exporting INSTALL_DIR so the installer targets our path.
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(format!("curl -fsSL {INSTALL_URL} | sh"));
    if let Some(dir) = &install_dir {
        cmd.env("INSTALL_DIR", dir);
    }

    let status = cmd.status().context("running the grit installer")?;
    if !status.success() {
        bail!(
            "grit update failed: the installer exited unsuccessfully ({status}). \
             You can retry with:\n  curl -fsSL {INSTALL_URL} | sh"
        );
    }
    Ok(())
}

/// Minimal `which`: first executable-or-regular file named `tool` on `$PATH`.
fn find_in_path(tool: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(tool))
        .find(|cand| cand.is_file())
}
