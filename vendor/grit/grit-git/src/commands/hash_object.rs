//! `grit hash-object` — compute object ID and optionally write to object store.

use anyhow::{bail, Context, Result};
use clap::Args as ClapArgs;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use grit_lib::config::ConfigSet;
use grit_lib::crlf::{self, ConversionConfig};
use grit_lib::fsck_standalone::fsck_object;
use grit_lib::objects::ObjectKind;
use grit_lib::odb::Odb;
use grit_lib::repo::Repository;

/// Arguments for `grit hash-object`.
#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Object type (blob, tree, commit, tag).
    #[arg(short = 't', default_value = "blob", value_name = "type")]
    pub object_type: String,

    /// Write the object to the object store.
    #[arg(short = 'w')]
    pub write: bool,

    /// Read object from stdin.
    #[arg(long)]
    pub stdin: bool,

    /// Read file paths from stdin (one per line).
    #[arg(long = "stdin-paths")]
    pub stdin_paths: bool,

    /// Skip clean/smudge filters (Git compatibility; Grit has no filter pipeline).
    #[arg(long = "no-filters")]
    pub no_filters: bool,

    /// Don't validate file content, just hash it (with --literally).
    #[arg(long)]
    pub literally: bool,

    /// Hash object as if it had this path, for attribute (filter) selection.
    #[arg(long = "path", value_name = "file")]
    pub path: Option<PathBuf>,

    /// File(s) to hash.
    pub files: Vec<PathBuf>,
}

/// Run `grit hash-object`.
pub fn run(args: Args) -> Result<()> {
    if args.stdin_paths {
        if args.stdin {
            bail!("Can't use --stdin-paths with --stdin");
        }
        if !args.files.is_empty() {
            bail!("Can't specify files with --stdin-paths");
        }
        if args.path.is_some() {
            bail!("Can't use --stdin-paths with --path");
        }
    }
    if args.path.is_some() && args.no_filters {
        bail!("Can't use --path with --no-filters");
    }

    let kind = ObjectKind::from_str(&args.object_type)
        .with_context(|| format!("unknown object type '{}'", args.object_type))?;
    validate_big_file_threshold_config()?;

    let use_filters = kind == ObjectKind::Blob && !args.no_filters;
    let repo = if args.write {
        Some(Repository::discover(None).context("not a git repository")?)
    } else if use_filters {
        Repository::discover(None).ok()
    } else {
        None
    };
    let filter_context = repo.as_ref().and_then(HashObjectFilterContext::load);

    // We only need the odb if -w is given (in which case `repo` is always `Some`).
    let odb = if let Some(repo) = repo.as_ref().filter(|_| args.write) {
        Some(odb_for_write(repo)?)
    } else {
        None
    };

    // `--path=<file>` overrides which path's attributes drive the clean filter. Git resolves it
    // relative to the prefix (subdirectory), i.e. into a work-tree-relative path, exactly like a
    // regular file argument (builtin/hash-object.c: `prefix_filename(prefix, vpath)`).
    let vpath = args.path.as_deref().map(|p| {
        filter_relative_path(
            p,
            filter_context
                .as_ref()
                .and_then(|c| c.repo.work_tree.as_deref()),
        )
    });

    if args.stdin {
        let mut data = Vec::new();
        std::io::stdin()
            .read_to_end(&mut data)
            .context("reading stdin")?;
        // With --stdin, the only attribute path available is --path (Git passes `vpath` to
        // hash_fd); without it, no filter applies.
        let data = if args.no_filters {
            data
        } else if let (Some(Some(rel)), Some(ctx)) = (vpath.as_ref(), filter_context.as_ref()) {
            convert_with_attrs(&data, rel, ctx)?
        } else {
            data
        };
        validate_object_data(kind, &data, args.literally)?;
        let oid = hash_and_maybe_write(kind, &data, odb.as_ref())?;
        println!("{oid}");
        for path in &args.files {
            let file_data = read_file_for_hash(
                path,
                kind,
                args.no_filters,
                filter_context.as_ref(),
                vpath.as_ref(),
            )?;
            validate_object_data(kind, &file_data, args.literally)?;
            let file_oid = hash_and_maybe_write(kind, &file_data, odb.as_ref())?;
            println!("{file_oid}");
        }
    } else if args.stdin_paths {
        // Read one path per line and emit one OID per line (matches Git; Git.pm keeps stdin
        // open across multiple writes — must not block on full-stream EOF before first line).
        let stdin = std::io::stdin().lock();
        for line in stdin.lines() {
            let line = line.context("reading stdin paths")?;
            if line.is_empty() {
                continue;
            }
            let path = PathBuf::from(line);
            let data =
                read_file_for_hash(&path, kind, args.no_filters, filter_context.as_ref(), None)?;
            validate_object_data(kind, &data, args.literally)?;
            let oid = hash_and_maybe_write(kind, &data, odb.as_ref())?;
            println!("{oid}");
        }
    } else {
        for path in &args.files {
            let data = read_file_for_hash(
                path,
                kind,
                args.no_filters,
                filter_context.as_ref(),
                vpath.as_ref(),
            )?;
            validate_object_data(kind, &data, args.literally)?;
            let oid = hash_and_maybe_write(kind, &data, odb.as_ref())?;
            println!("{oid}");
        }
    }

    Ok(())
}

fn validate_big_file_threshold_config() -> Result<()> {
    let config = ConfigSet::load(None, true).unwrap_or_default();
    if let Some(raw) = config.get("core.bigFileThreshold") {
        if raw.trim_start().starts_with('-') {
            bail!(
                "bad numeric config value '{}' for 'core.bigfilethreshold'",
                raw
            );
        }
    }
    Ok(())
}

struct HashObjectFilterContext<'a> {
    repo: &'a Repository,
    config: ConfigSet,
    conv: ConversionConfig,
    attrs: Vec<crlf::AttrRule>,
}

impl<'a> HashObjectFilterContext<'a> {
    fn load(repo: &'a Repository) -> Option<Self> {
        let work_tree = repo.work_tree.as_deref()?;
        let config = ConfigSet::load(Some(&repo.git_dir), true).ok()?;
        let conv = ConversionConfig::from_config(&config);
        let attrs = crlf::load_gitattributes(work_tree);
        Some(Self {
            repo,
            config,
            conv,
            attrs,
        })
    }
}

fn read_file_for_hash(
    path: &Path,
    kind: ObjectKind,
    no_filters: bool,
    filter_context: Option<&HashObjectFilterContext<'_>>,
    vpath: Option<&Option<String>>,
) -> Result<Vec<u8>> {
    let raw = std::fs::read(path).with_context(|| format!("cannot read '{}'", path.display()))?;
    if kind != ObjectKind::Blob || no_filters {
        return Ok(raw);
    }
    let Some(ctx) = filter_context else {
        return Ok(raw);
    };
    // When `--path` is given, attribute lookup uses that virtual path instead of the file's own
    // path (builtin/hash-object.c passes `vpath` to convert). `Some(None)` means `--path` was set
    // but lies outside the work tree, so no attributes apply.
    let rel_path = match vpath {
        Some(Some(rel)) => rel.clone(),
        Some(None) => return Ok(raw),
        None => match filter_relative_path(path, ctx.repo.work_tree.as_deref()) {
            Some(rel) => rel,
            None => return Ok(raw),
        },
    };
    convert_with_attrs(&raw, &rel_path, ctx)
}

fn convert_with_attrs(
    raw: &[u8],
    rel_path: &str,
    ctx: &HashObjectFilterContext<'_>,
) -> Result<Vec<u8>> {
    let file_attrs = crlf::get_file_attrs(&ctx.attrs, rel_path, false, &ctx.config);
    crlf::convert_to_git(raw, rel_path, &ctx.conv, &file_attrs).map_err(|msg| anyhow::anyhow!(msg))
}

fn filter_relative_path(path: &Path, work_tree: Option<&Path>) -> Option<String> {
    let work_tree = work_tree?;
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let rel = abs.strip_prefix(work_tree).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

fn validate_object_data(kind: ObjectKind, data: &[u8], literally: bool) -> Result<()> {
    if literally {
        return Ok(());
    }
    if let Err(e) = fsck_object(kind, data) {
        if kind == ObjectKind::Tree && e.id == "badTree" {
            eprintln!("error: too-short tree object");
        }
        return Err(anyhow::anyhow!(grit_lib::error::Error::Message(format!(
            "error: object fails fsck: {}\nfatal: refusing to create malformed object",
            e.report_line()
        ))));
    }
    Ok(())
}

/// Object store used for `hash-object -w`.
///
/// When `GIT_OBJECT_DIRECTORY` is set, Git writes loose objects there instead of the repository’s
/// primary `objects/` directory (`t7700-repack` alternate-ODB setup).
fn odb_for_write(repo: &Repository) -> Result<Odb> {
    let Ok(raw) = std::env::var("GIT_OBJECT_DIRECTORY") else {
        return Ok(repo.odb.clone());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(repo.odb.clone());
    }
    let p = Path::new(trimmed);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .context("GIT_OBJECT_DIRECTORY is relative; need current directory")?
            .join(p)
    };
    Ok(Odb::new(&abs))
}

fn hash_and_maybe_write(
    kind: ObjectKind,
    data: &[u8],
    odb: Option<&Odb>,
) -> Result<grit_lib::objects::ObjectId> {
    if let Some(db) = odb {
        db.write_loose_materialize(kind, data)
            .context("writing object")
    } else {
        Ok(Odb::hash_object_data(kind, data))
    }
}
