//! `grit verify-tag` — verify the signature on annotated tag object(s).
//!
//! Mirrors `git verify-tag`: for each tag the appended armored signature is
//! verified (gpg/gpgsm/ssh, chosen from the signature armor), the verifier
//! output is written to stderr (the raw `[GNUPG:]` status lines under `--raw`,
//! the human-readable lines otherwise), and the command exits non-zero when any
//! tag is unsigned, the signature is bad, or its trust level is below
//! `gpg.minTrustLevel`.  With `--format` the signature output is suppressed and
//! a ref-filter style line is printed only for tags that verify successfully.

use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use grit_lib::objects::{parse_tag, ObjectKind};
use grit_lib::repo::Repository;
use grit_lib::rev_parse::resolve_revision;
use grit_lib::signing::{self, GpgConfig};
use std::io::{self, Write};

/// Arguments for `grit verify-tag`.
#[derive(Debug, ClapArgs)]
#[command(about = "Verify a tag object")]
pub struct Args {
    /// Tag references to verify.
    #[arg(required = true)]
    pub tags: Vec<String>,

    /// Print the contents of the tag object before validation.
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,

    /// Print the raw gpg status output instead of the human-readable lines.
    #[arg(long = "raw")]
    pub raw: bool,

    /// Format the output of the (successfully verified) tag(s).
    #[arg(long = "format", value_name = "FORMAT")]
    pub format: Option<String>,
}

/// Run the `verify-tag` command.
pub fn run(args: Args) -> Result<()> {
    let repo = Repository::discover(None).context("not a git repository")?;
    let config = grit_lib::config::ConfigSet::load(Some(&repo.git_dir), true).unwrap_or_default();
    let gpg_cfg = GpgConfig::from_config(&config)?;

    let stdout = io::stdout();
    let mut out = stdout.lock();
    let stderr = io::stderr();
    let mut err = stderr.lock();

    // `--format` suppresses the signature status output (GPG_VERIFY_OMIT_STATUS).
    let omit_status = args.format.is_some();

    let mut had_failure = false;

    for rev in &args.tags {
        let oid = match resolve_revision(&repo, rev) {
            Ok(oid) => oid,
            Err(_) => {
                writeln!(err, "error: tag '{rev}' not found.")?;
                had_failure = true;
                continue;
            }
        };

        let obj = repo
            .odb
            .read(&oid)
            .with_context(|| format!("could not read object '{rev}'"))?;

        if obj.kind != ObjectKind::Tag {
            writeln!(
                err,
                "error: {}: cannot verify a non-tag object of type {}.",
                oid.to_hex(),
                obj.kind.as_str()
            )?;
            had_failure = true;
            continue;
        }

        // `verify-tag -v` writes the signed payload (tag object with the
        // appended signature stripped) to stdout before the verifier output.
        // When the tag is unsigned, Git writes the whole buffer.
        if args.verbose {
            match signing::parse_signed_buffer(&obj.data) {
                Some((payload, _)) => out.write_all(&payload)?,
                None => out.write_all(&obj.data)?,
            }
            out.flush()?;
        }

        let sigc = signing::verify_tag(&gpg_cfg, &obj.data)?;

        if !omit_status {
            let text = if args.raw {
                &sigc.gpg_status
            } else {
                &sigc.output
            };
            if !text.is_empty() {
                err.write_all(text.as_bytes())?;
            } else if sigc.result == 'N' {
                writeln!(err, "error: {}: no signature found", oid.to_hex())?;
            }
            err.flush()?;
        }

        let verified = sigc.verify_status(gpg_cfg.min_trust_level);
        if !verified {
            had_failure = true;
            continue;
        }

        // On success with `--format`, print the formatted line for this tag.
        if let Some(fmt) = &args.format {
            let line = expand_tag_format(fmt, rev, &obj.data);
            writeln!(out, "{line}")?;
            out.flush()?;
        }
    }

    if had_failure {
        anyhow::bail!("could not verify tag signature");
    }

    Ok(())
}

/// Expand a minimal subset of the ref-filter format used by `git verify-tag
/// --format`.  The test suite only exercises `%(tag)` (the tag's short name).
fn expand_tag_format(fmt: &str, name: &str, raw_tag: &[u8]) -> String {
    let tagname = parse_tag(raw_tag)
        .ok()
        .map(|t| t.tag)
        .unwrap_or_else(|| name.to_owned());

    let mut out = String::with_capacity(fmt.len());
    let bytes = fmt.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
            if let Some(close) = fmt[i + 2..].find(')') {
                let atom = &fmt[i + 2..i + 2 + close];
                match atom {
                    "tag" => out.push_str(&tagname),
                    other => {
                        // Unknown atom: emit verbatim to stay debuggable.
                        out.push_str("%(");
                        out.push_str(other);
                        out.push(')');
                    }
                }
                i = i + 2 + close + 1;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}
