//! `grit verify-commit` — verify the GPG signature on commit object(s).
//!
//! Mirrors `git verify-commit`: for each commit the embedded `gpgsig` header is
//! verified, the gpg output is written to stderr (the raw `[GNUPG:]` status
//! lines under `--raw`, the human-readable lines otherwise), and the command
//! exits non-zero when any commit is unsigned, the signature is bad, or its
//! trust level is below `gpg.minTrustLevel`.

use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use grit_lib::objects::ObjectKind;
use grit_lib::repo::Repository;
use grit_lib::rev_parse::resolve_revision;
use grit_lib::signing::{self, GpgConfig};
use std::io::{self, Write};

/// Arguments for `grit verify-commit`.
#[derive(Debug, ClapArgs)]
#[command(about = "Check the GPG signature of commits")]
pub struct Args {
    /// Commit references to verify.
    #[arg(required = true)]
    pub commits: Vec<String>,

    /// Print the contents of the commit object before validation.
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,

    /// Print the raw gpg status output instead of the human-readable lines.
    #[arg(long = "raw")]
    pub raw: bool,
}

/// Run the `verify-commit` command.
pub fn run(args: Args) -> Result<()> {
    let repo = Repository::discover(None).context("not a git repository")?;
    let config = grit_lib::config::ConfigSet::load(Some(&repo.git_dir), true).unwrap_or_default();
    let gpg_cfg = GpgConfig::from_config(&config)?;

    let stdout = io::stdout();
    let mut out = stdout.lock();
    let stderr = io::stderr();
    let mut err = stderr.lock();

    let mut had_failure = false;

    for rev in &args.commits {
        let oid = match resolve_revision(&repo, rev) {
            Ok(oid) => oid,
            Err(e) => {
                writeln!(err, "error: {rev}: {e}")?;
                anyhow::bail!("could not verify commit '{rev}'");
            }
        };

        let obj = repo
            .odb
            .read(&oid)
            .with_context(|| format!("could not read object '{rev}'"))?;

        if obj.kind != ObjectKind::Commit {
            writeln!(
                err,
                "error: {}: object is a {}, not a commit",
                oid.to_hex(),
                obj.kind.as_str()
            )?;
            anyhow::bail!("could not verify commit '{rev}'");
        }

        // `verify-commit -v` writes the commit object (the signed payload, with
        // the gpgsig header stripped) to stdout before the gpg output.
        if args.verbose {
            match signing::extract_signed_payload(&obj.data) {
                Some((payload, _)) => out.write_all(&payload)?,
                None => out.write_all(&obj.data)?,
            }
            out.flush()?;
        }

        let sigc = signing::verify_commit(&gpg_cfg, &obj.data)?;

        // Emit the gpg output to stderr (raw status under --raw).
        let text = if args.raw {
            &sigc.gpg_status
        } else {
            &sigc.output
        };
        if !text.is_empty() {
            err.write_all(text.as_bytes())?;
        } else if sigc.result == 'N' {
            // No signature at all.
            writeln!(err, "error: {}: no signature found", oid.to_hex())?;
        }
        err.flush()?;

        if !sigc.verify_status(gpg_cfg.min_trust_level) {
            had_failure = true;
        }
    }

    if had_failure {
        anyhow::bail!("could not verify commit signature");
    }

    Ok(())
}
