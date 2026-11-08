//! Native Git storage and inspection commands, powered by Gitoxide.
//!
//! Repository discovery is deliberately isolated: repository-local configuration is
//! read, but includes, global configuration, helper programs and fsmonitor hooks are
//! not enabled. No production operation in this module launches `git`.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

use anyhow::{Context, Result, bail, ensure};
use clap::{ArgGroup, Args, Subcommand, ValueEnum};
use gix::{
    bstr::{BStr, BString, ByteSlice},
    objs::Write as ObjectWrite,
};

/// Supported native Git commands. Flatten this enum into the application CLI.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize a new repository (initial branch defaults to main).
    Init(Init),
    /// Hash exact input bytes, optionally writing a Git object. Filters are never run.
    HashObject(HashObject),
    /// Read object contents (up to 64 MiB), or inspect type/size without loading data.
    CatFile(CatFile),
    /// Resolve a single revision or inspect repository paths and format.
    RevParse(RevParse),
    /// List local references, preserving annotated tag object IDs.
    ShowRef(ShowRef),
    /// Show commit IDs and subjects in commit-time order.
    Log(Log),
    /// List cached index paths (repository-relative, including conflict stages).
    LsFiles(LsFiles),
    /// Show porcelain v1 status with rename detection disabled.
    Status(Status),
    /// Atomically update a direct reference only if its old object ID matches.
    UpdateRef(UpdateRef),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ObjectFormat {
    Sha1,
    Sha256,
}
impl ObjectFormat {
    fn kind(self) -> gix::hash::Kind {
        match self {
            Self::Sha1 => gix::hash::Kind::Sha1,
            Self::Sha256 => gix::hash::Kind::Sha256,
        }
    }
}

#[derive(Debug, Args)]
pub struct Init {
    #[arg(long)]
    pub bare: bool,
    #[arg(short = 'b', long, default_value = "main")]
    pub initial_branch: String,
    #[arg(long, value_enum, default_value = "sha1")]
    pub object_format: ObjectFormat,
    #[arg(default_value = ".")]
    pub target_directory: PathBuf,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ObjectType {
    Blob,
    Tree,
    Commit,
    Tag,
}
impl ObjectType {
    fn kind(self) -> gix::objs::Kind {
        match self {
            Self::Blob => gix::objs::Kind::Blob,
            Self::Tree => gix::objs::Kind::Tree,
            Self::Commit => gix::objs::Kind::Commit,
            Self::Tag => gix::objs::Kind::Tag,
        }
    }
}

#[derive(Debug, Args)]
#[command(group(ArgGroup::new("input").required(true).args(["stdin", "files"])))]
pub struct HashObject {
    #[arg(short = 'w')]
    pub write: bool,
    #[arg(long, conflicts_with = "files")]
    pub stdin: bool,
    #[arg(short = 't', value_enum, default_value = "blob")]
    pub object_type: ObjectType,
    /// Required for pathname input: hash exact bytes without attributes or filters.
    #[arg(long)]
    pub no_filters: bool,
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Args)]
#[command(group(ArgGroup::new("operation").required(true).args(["pretty", "size", "object_type"])))]
pub struct CatFile {
    #[arg(short = 'p')]
    pub pretty: bool,
    #[arg(short = 's')]
    pub size: bool,
    #[arg(short = 't')]
    pub object_type: bool,
    pub object: OsString,
}

#[derive(Debug, Args)]
#[command(group(ArgGroup::new("query").args(["show_toplevel", "git_dir", "git_common_dir", "is_bare_repository", "is_inside_work_tree", "show_object_format"])))]
pub struct RevParse {
    #[arg(long, conflicts_with = "revision")]
    pub show_toplevel: bool,
    #[arg(long, conflicts_with = "revision")]
    pub git_dir: bool,
    #[arg(long, conflicts_with = "revision")]
    pub git_common_dir: bool,
    #[arg(long, conflicts_with = "revision")]
    pub is_bare_repository: bool,
    #[arg(long, conflicts_with = "revision")]
    pub is_inside_work_tree: bool,
    #[arg(long, conflicts_with = "revision")]
    pub show_object_format: bool,
    #[arg(long, requires = "revision")]
    pub verify: bool,
    pub revision: Option<OsString>,
}

#[derive(Debug, Args)]
pub struct ShowRef {
    /// Include HEAD if it resolves to an object.
    #[arg(long)]
    pub head: bool,
    #[arg(long)]
    pub heads: bool,
    #[arg(long)]
    pub tags: bool,
}

#[derive(Debug, Args)]
pub struct Log {
    #[arg(short = 'n', long, default_value_t = 20)]
    pub max_count: usize,
    /// Abbreviate commit IDs; otherwise full IDs are printed.
    #[arg(long)]
    pub oneline: bool,
    #[arg(long)]
    pub first_parent: bool,
    #[arg(default_value = "HEAD")]
    pub revision: OsString,
}

#[derive(Debug, Args)]
pub struct LsFiles {
    /// Use NUL terminators and preserve path bytes without quoting.
    #[arg(short = 'z')]
    pub nul: bool,
    #[arg(short = 's', long)]
    pub stage: bool,
    /// Explicitly select the cached index (the only supported mode).
    #[arg(long)]
    pub cached: bool,
}

#[derive(Debug, Args)]
pub struct Status {
    /// Select porcelain v1 (the only supported output format).
    #[arg(long)]
    pub porcelain: bool,
    #[arg(short = 'z')]
    pub nul: bool,
    /// Rename detection is intentionally disabled in this release.
    #[arg(long)]
    pub no_renames: bool,
}

#[derive(Debug, Args)]
pub struct UpdateRef {
    /// Full reference name, for example refs/heads/main.
    pub reference: OsString,
    pub new_oid: String,
    /// Expected old ID; all zeroes means the reference must not exist.
    pub expected_old_oid: String,
    #[arg(short = 'm', default_value = "gitr: guarded reference update")]
    pub message: String,
}

fn isolated_options() -> gix::open::Options {
    gix::open::Options::isolated()
        .config_overrides(["core.fsmonitor=false"])
        .strict_config(true)
        .bail_if_untrusted(true)
}

fn discover_native(path: &Path) -> gix::Result<gix::Repository> {
    gix::ThreadSafeRepository::discover_opts(
        path,
        Default::default(),
        gix::sec::trust::Mapping {
            full: isolated_options(),
            reduced: isolated_options(),
        },
    )
    .map(|repo| repo.to_thread_local())
}

/// Discover from the current directory without loading global config or helpers.
pub fn discover() -> Result<gix::Repository> {
    discover_at(std::env::current_dir()?)
}

/// Discover from `path`, including Git files used by linked worktrees.
pub fn discover_at(path: impl AsRef<Path>) -> Result<gix::Repository> {
    discover_native(path.as_ref()).context("discover Git repository")
}

/// Execute a native command. All object and path output uses raw bytes or Git quoting.
pub fn run(command: Command) -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    match command {
        Command::Init(args) => init(args, &mut out),
        Command::HashObject(args) => hash_object(args, &mut out),
        Command::CatFile(args) => cat_file(args, &mut out),
        Command::RevParse(args) => rev_parse(args, &mut out),
        Command::ShowRef(args) => show_ref(args, &mut out),
        Command::Log(args) => log(args, &mut out),
        Command::LsFiles(args) => ls_files(args, &mut out),
        Command::Status(args) => status(args, &mut out),
        Command::UpdateRef(args) => update_ref(args),
    }
}

fn init(args: Init, out: &mut impl Write) -> Result<()> {
    // Validate before creating any directory or repository metadata.
    ensure!(
        !args.initial_branch.starts_with('-'),
        "initial branch cannot start with '-'"
    );
    let branch = format!("refs/heads/{}", args.initial_branch);
    gix::validate::reference::branch_name(branch.as_bytes().as_bstr())
        .context("invalid initial branch name")?;
    let repo = gix::ThreadSafeRepository::init_opts(
        &args.target_directory,
        if args.bare {
            gix::create::Kind::Bare
        } else {
            gix::create::Kind::WithWorktree
        },
        gix::create::Options {
            object_hash: Some(args.object_format.kind()),
            ..Default::default()
        },
        isolated_options()
            .config_overrides([format!("init.defaultBranch={}", args.initial_branch)]),
    )?
    .to_thread_local();
    writeln!(
        out,
        "Initialized empty Git repository in {}",
        repo.git_dir().display()
    )?;
    Ok(())
}

fn hash_object(args: HashObject, out: &mut impl Write) -> Result<()> {
    let repo = match discover_native(&std::env::current_dir()?) {
        Ok(repo) => Some(repo),
        Err(error) if !args.write && error.classify().is_not_found() => None,
        Err(error) => return Err(error).context("discover repository for hash-object"),
    };
    ensure!(
        args.stdin || args.no_filters,
        "pathname hashing requires --no-filters: attribute-aware conversion is not implemented; --stdin hashes raw bytes"
    );
    let hash_kind = repo
        .as_ref()
        .map_or(gix::hash::Kind::Sha1, |repo| repo.object_hash());
    let object_kind = args.object_type.kind();
    if args.stdin {
        let stdin = io::stdin();
        let id = hash_stream(
            &mut stdin.lock(),
            repo.as_ref(),
            hash_kind,
            object_kind,
            args.write,
        )?;
        writeln!(out, "{id}")?;
    } else {
        for path in args.files {
            let mut input =
                File::open(&path).with_context(|| format!("open {}", path.display()))?;
            let id = hash_stream(
                &mut input,
                repo.as_ref(),
                hash_kind,
                object_kind,
                args.write,
            )?;
            writeln!(out, "{id}")?;
        }
    }
    Ok(())
}

fn hash_stream(
    input: &mut dyn Read,
    repo: Option<&gix::Repository>,
    hash_kind: gix::hash::Kind,
    object_kind: gix::objs::Kind,
    write: bool,
) -> Result<gix::hash::ObjectId> {
    // The object header requires a trustworthy length. Spooling also prevents a
    // concurrently resized input from creating a corrupt loose object. Memory use
    // stays bounded even for multi-gigabyte input and for unknown-length stdin.
    let mut snapshot = tempfile::tempfile().context("create input snapshot")?;
    let size = io::copy(input, &mut snapshot).context("snapshot object input")?;
    snapshot.seek(SeekFrom::Start(0))?;
    if object_kind != gix::objs::Kind::Blob {
        const STRUCTURED_OBJECT_LIMIT: u64 = 16 * 1024 * 1024;
        ensure!(
            size <= STRUCTURED_OBJECT_LIMIT,
            "structured objects above 16 MiB are not supported"
        );
        let mut bytes = Vec::with_capacity(size as usize);
        snapshot.read_to_end(&mut bytes)?;
        gix::objs::ObjectRef::from_bytes(&bytes, object_kind, hash_kind)
            .context("invalid structured Git object")?;
        snapshot.seek(SeekFrom::Start(0))?;
    }
    if write {
        repo.context("writing an object requires a repository")?
            .objects
            .write_stream(object_kind, size, &mut snapshot)
            .context("write Git object")
    } else {
        gix::objs::compute_stream_hash(
            hash_kind,
            object_kind,
            &mut snapshot,
            size,
            &mut gix::progress::Discard,
            &AtomicBool::new(false),
        )
        .context("hash Git object")
    }
}

fn cat_file(args: CatFile, out: &mut impl Write) -> Result<()> {
    let repo = discover()?;
    let id = repo.rev_parse_single(gix::path::os_str_into_bstr(&args.object)?)?;
    let header = repo.find_header(id)?;
    if args.object_type {
        writeln!(out, "{}", header.kind())?;
        return Ok(());
    }
    if args.size {
        writeln!(out, "{}", header.size())?;
        return Ok(());
    }
    const MAX_PRETTY_OBJECT_BYTES: u64 = 64 * 1024 * 1024;
    ensure!(
        header.size() <= MAX_PRETTY_OBJECT_BYTES,
        "cat-file -p currently supports objects up to 64 MiB; inspect larger objects with -s/-t, or use Git LFS for large asset payloads"
    );
    let object = id.object()?;
    if object.kind == gix::objs::Kind::Tree {
        let tree = gix::objs::TreeRefIter::from_bytes(&object.data, repo.object_hash());
        for entry in tree {
            let entry = entry?;
            let mode = entry.mode.value();
            let kind = match mode {
                0o040000 => "tree",
                0o160000 => "commit",
                _ => "blob",
            };
            write!(out, "{mode:06o} {kind} {}\t", entry.oid)?;
            out.write_all(gix::quote::ansi_c::quote(entry.filename).as_bytes())?;
            out.write_all(b"\n")?;
        }
    } else {
        out.write_all(&object.data)?;
    }
    Ok(())
}

fn print_path(out: &mut impl Write, path: &Path) -> Result<()> {
    out.write_all(gix::path::os_str_into_bstr(path.as_os_str())?.as_bytes())?;
    out.write_all(b"\n")?;
    Ok(())
}

fn print_git_directory(out: &mut impl Write, path: &Path) -> Result<()> {
    let cwd = std::env::current_dir()?.canonicalize()?;
    let path = path.canonicalize()?;
    if path == cwd {
        print_path(out, Path::new("."))
    } else if path == cwd.join(".git") {
        print_path(out, Path::new(".git"))
    } else {
        print_path(out, &path)
    }
}

fn rev_parse(args: RevParse, out: &mut impl Write) -> Result<()> {
    let repo = discover()?;
    if args.show_toplevel {
        print_path(
            out,
            &repo
                .workdir()
                .context("bare repository has no working tree")?
                .canonicalize()?,
        )?;
    } else if args.git_dir {
        print_git_directory(out, repo.git_dir())?;
    } else if args.git_common_dir {
        print_git_directory(out, repo.common_dir())?;
    } else if args.is_bare_repository {
        writeln!(out, "{}", repo.is_bare())?;
    } else if args.is_inside_work_tree {
        let here = std::env::current_dir()?.canonicalize()?;
        let inside = repo
            .workdir()
            .is_some_and(|dir| dir.canonicalize().is_ok_and(|dir| here.starts_with(dir)))
            && !here.starts_with(repo.git_dir().canonicalize()?);
        writeln!(out, "{inside}")?;
    } else if args.show_object_format {
        writeln!(out, "{}", repo.object_hash())?;
    } else {
        let spec = args
            .revision
            .context("supply a revision or a repository query option")?;
        writeln!(
            out,
            "{}",
            repo.rev_parse_single(gix::path::os_str_into_bstr(&spec)?)?
        )?;
    }
    Ok(())
}

fn show_ref(args: ShowRef, out: &mut impl Write) -> Result<()> {
    let repo = discover()?;
    let mut rows = BTreeMap::<BString, gix::hash::ObjectId>::new();
    if args.head
        && let Ok(head) = repo.head_id()
    {
        rows.insert("HEAD".into(), head.detach());
    }
    let references = repo.references()?.all()?.collect::<gix::Result<Vec<_>>>()?;
    for mut reference in references {
        let name = reference.name().as_bstr().to_owned();
        if (args.heads || args.tags)
            && !(args.heads && name.starts_with(b"refs/heads/")
                || args.tags && name.starts_with(b"refs/tags/"))
        {
            continue;
        }
        let object = reference.follow_to_object()?;
        rows.insert(name, object.detach());
    }
    ensure!(!rows.is_empty(), "no references found");
    for (name, id) in rows {
        write!(out, "{id} ")?;
        out.write_all(name.as_bytes())?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

fn log(args: Log, out: &mut impl Write) -> Result<()> {
    let repo = discover()?;
    let id = repo
        .rev_parse_single(gix::path::os_str_into_bstr(&args.revision)?)?
        .object()?
        .peel_to_kind(gix::objs::Kind::Commit)?
        .id;
    let walk = repo
        .rev_walk([id])
        .sorting(gix::revision::walk::Sorting::ByCommitTime(
            Default::default(),
        ));
    let walk = if args.first_parent {
        walk.first_parent_only()
    } else {
        walk
    };
    for entry in walk.all()?.take(args.max_count) {
        let entry = entry?;
        let commit = entry.object()?;
        if args.oneline {
            write!(out, "{} ", entry.id().shorten()?)?;
        } else {
            write!(out, "{} ", entry.id)?;
        }
        // Git's subject joins the first paragraph, which may span multiple lines.
        let message = commit.message_raw()?;
        let mut subject = Vec::new();
        for line in message.lines().skip_while(|line| line.trim().is_empty()) {
            if line.trim().is_empty() {
                break;
            }
            if !subject.is_empty() {
                subject.push(b' ');
            }
            subject.extend_from_slice(line.trim());
        }
        out.write_all(&subject)?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

fn write_filename(out: &mut impl Write, path: &BStr, nul: bool) -> Result<()> {
    if nul {
        out.write_all(path.as_bytes())?;
        out.write_all(b"\0")?;
    } else {
        out.write_all(gix::quote::ansi_c::quote(path).as_bytes())?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

fn ls_files(args: LsFiles, out: &mut impl Write) -> Result<()> {
    let repo = discover()?;
    let index = repo.index_or_empty()?;
    ensure!(
        !index.is_sparse(),
        "sparse index listing is not implemented"
    );
    for entry in index.entries() {
        if args.stage {
            write!(
                out,
                "{:06o} {} {}\t",
                entry.mode.bits(),
                entry.id,
                entry.stage_raw()
            )?;
        }
        write_filename(out, entry.path(&index), args.nul)?;
    }
    Ok(())
}

fn status(args: Status, out: &mut impl Write) -> Result<()> {
    let repo = discover()?;
    ensure!(repo.workdir().is_some(), "status requires a working tree");
    let config = repo.config_snapshot();
    ensure!(
        config.plumbing().sections_by_name("filter").is_none(),
        "status with configured clean/process filters is not implemented; gitr does not execute repository filter commands"
    );
    let index = repo.index_or_empty()?;
    ensure!(!index.is_sparse(), "sparse index status is not implemented");
    ensure!(
        !index
            .entries()
            .iter()
            .any(|entry| entry.mode == gix::index::entry::Mode::COMMIT),
        "submodule status is not implemented"
    );
    let mut conflicts = BTreeMap::<BString, u8>::new();
    for entry in index.entries() {
        let stage = entry.stage_raw();
        if stage != 0 {
            *conflicts.entry(entry.path(&index).to_owned()).or_default() |= 1 << (stage - 1);
        }
    }
    let mut rows = BTreeMap::<BString, [u8; 2]>::new();
    let mut untracked = BTreeSet::<BString>::new();
    use gix::status::{Item, index_worktree::iter::Summary};
    let status = repo
        .status(gix::progress::Discard)?
        .index(index.into())
        .untracked_files(gix::status::UntrackedFiles::Files)
        .tree_index_track_renames(gix::status::tree_index::TrackRenames::Disabled)
        .index_worktree_rewrites(None)
        .into_iter(std::iter::empty::<BString>())?;
    for item in status {
        match item? {
            Item::IndexWorktree(item) => {
                let Some(summary) = item.summary() else {
                    continue;
                };
                let path = item.rela_path().to_owned();
                if summary == Summary::Added {
                    untracked.insert(path);
                    continue;
                }
                let row = rows.entry(path).or_insert(*b"  ");
                match summary {
                    Summary::Added => unreachable!("handled above"),
                    Summary::Removed => row[1] = b'D',
                    Summary::TypeChange => row[1] = b'T',
                    Summary::Modified => row[1] = b'M',
                    Summary::IntentToAdd => row[1] = b'A',
                    Summary::Conflict => {}
                    Summary::Renamed | Summary::Copied => {
                        bail!("unexpected worktree rename result")
                    }
                }
            }
            Item::TreeIndex(change) => {
                use gix::diff::index::Change;
                let (path, status) = match change {
                    Change::Addition { location, .. } => (location.into_owned(), b'A'),
                    Change::Deletion { location, .. } => (location.into_owned(), b'D'),
                    Change::Modification {
                        location,
                        previous_entry_mode,
                        entry_mode,
                        ..
                    } => (
                        location.into_owned(),
                        if previous_entry_mode.bits() & 0o170000 != entry_mode.bits() & 0o170000 {
                            b'T'
                        } else {
                            b'M'
                        },
                    ),
                    Change::Rewrite { .. } => bail!("unexpected index rename result"),
                };
                rows.entry(path).or_insert(*b"  ")[0] = status;
            }
        }
    }
    for (path, stages) in conflicts {
        let status = match stages {
            1 => *b"DD",
            2 => *b"AU",
            3 => *b"UD",
            4 => *b"UA",
            5 => *b"DU",
            6 => *b"AA",
            7 => *b"UU",
            _ => unreachable!(),
        };
        rows.insert(path, status);
    }
    for (path, status) in rows {
        out.write_all(&status)?;
        out.write_all(b" ")?;
        write_filename(out, path.as_bstr(), args.nul)?;
    }
    for path in untracked {
        out.write_all(b"?? ")?;
        write_filename(out, path.as_bstr(), args.nul)?;
    }
    Ok(())
}

fn update_ref(args: UpdateRef) -> Result<()> {
    let repo = discover()?;
    let name_bytes = gix::path::os_str_into_bstr(&args.reference)?;
    ensure!(
        name_bytes.starts_with(b"refs/"),
        "use a fully qualified reference under refs/; symbolic HEAD updates are not supported"
    );
    let name: gix::refs::FullName = name_bytes.try_into().context("invalid reference name")?;
    let new =
        gix::hash::ObjectId::from_hex(args.new_oid.as_bytes()).context("invalid new object ID")?;
    let old = gix::hash::ObjectId::from_hex(args.expected_old_oid.as_bytes())
        .context("invalid expected old object ID")?;
    ensure!(
        new.kind() == repo.object_hash() && old.kind() == repo.object_hash(),
        "object IDs must use the repository hash format"
    );
    ensure!(
        !new.is_null(),
        "reference deletion is not supported by guarded update-ref"
    );
    let target = repo
        .find_header(new)
        .context("new reference target does not exist")?;
    if name_bytes.starts_with(b"refs/heads/") {
        gix::validate::reference::branch_name(name_bytes).context("invalid branch reference")?;
        ensure!(
            target.kind() == gix::objs::Kind::Commit,
            "a branch must point directly to a commit"
        );
    }
    use gix::refs::transaction::{PreviousValue, RefEdit};
    let expected = if old.is_null() {
        PreviousValue::MustNotExist
    } else {
        PreviousValue::MustExistAndMatch(old.into())
    };
    // PreviousValue is checked while holding the reference lock; never replace it
    // with a separate read/compare/write sequence. A symbolic reference cannot
    // match an expected object target, so it is not accidentally overwritten.
    ensure!(
        !args.message.contains(['\0', '\n', '\r']),
        "reflog message must be a single line without NUL bytes"
    );
    let edit = RefEdit::update_with_log(
        name,
        new,
        expected,
        gix::refs::transaction::LogChange {
            message: args.message.into(),
            force_create_reflog: true,
            ..Default::default()
        },
    )
    .with_deref(false);
    let committer = repo.committer().transpose()?.context(
        "guarded updates require repository-local user.name and user.email for the reflog",
    )?;
    let applied = repo
        .edit_references_as([edit], Some(committer))
        .context("reference compare-and-swap failed")?;
    // Gitoxide 0.90 permits MustNotExist when the existing target already equals
    // the requested target. Its receipt retains that value from inside the lock,
    // and this case changes neither the reference nor its reflog. Convert that
    // idempotent no-op into the strict creation failure promised by our CLI.
    if old.is_null() {
        ensure!(
            applied.iter().all(|edit| matches!(
                edit.change,
                gix::refs::transaction::Change::Update {
                    expected: PreviousValue::MustNotExist,
                    ..
                }
            )),
            "reference compare-and-swap failed: the reference already exists"
        );
    }
    Ok(())
}
