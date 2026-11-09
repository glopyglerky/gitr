use anyhow::{Context, Result, ensure};
use clap::{Args, Parser, Subcommand};
use gitr::{actions, assets, chunks, db, git, lfs, remote};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version,
    about = "Native Rust Git operations, verified game assets and repository search"
)]
struct Cli {
    /// Run as if started in this directory.
    #[arg(short = 'C', global = true)]
    directory: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(flatten)]
    Git(git::Command),
    /// Standard Git LFS pointers, local cache, HTTPS transfers and asset locks.
    Lfs {
        /// Explicit LFS objects directory. Defaults to this repository's common cache.
        #[arg(long, global = true)]
        storage: Option<PathBuf>,
        #[command(subcommand)]
        command: LfsCommand,
    },
    /// Inspect large assets or print game engine .gitattributes rules.
    Assets {
        #[command(subcommand)]
        command: AssetsCommand,
    },
    /// Transactional, commit-addressed repository graph and lexical vector search.
    Db {
        #[command(subcommand)]
        command: db::Command,
    },
    /// Report repository storage and compatibility information without changing it.
    Doctor,
    /// Database-resident bounded Wasm actions and local JSON control.
    Actions {
        #[command(subcommand)]
        command: actions::Command,
    },
}

#[derive(Subcommand)]
enum AssetsCommand {
    /// Store verified reusable chunks and an atomic full-file manifest locally.
    Import {
        file: PathBuf,
        /// Explicit chunk cache directory. Defaults to the repository common directory.
        #[arg(long)]
        cache: Option<PathBuf>,
    },
    /// Reconstruct a chunked asset into the standard LFS cache, verifying its full hash.
    Hydrate {
        #[command(flatten)]
        object: ObjectArgs,
        #[arg(long)]
        cache: Option<PathBuf>,
        /// Explicit destination LFS objects directory.
        #[arg(long)]
        storage: Option<PathBuf>,
    },
    /// Scan file sizes with .gitignore rules, without reading file contents.
    Scan {
        #[arg(long, default_value_t = 50 * 1024 * 1024)]
        threshold_bytes: u64,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Print rules for review. Existing files are never edited automatically.
    Attributes {
        #[arg(value_parser = ["unreal", "unity", "generic"])]
        engine: String,
    },
}

#[derive(Args)]
struct ObjectArgs {
    /// Full lowercase SHA-256 object ID.
    oid: String,
    /// Expected size in bytes.
    size: u64,
}

impl ObjectArgs {
    fn pointer(&self) -> Result<lfs::Pointer> {
        lfs::Pointer::new(self.oid.clone(), self.size)
    }
}

#[derive(Args)]
struct NetworkArgs {
    /// Explicit LFS server base URL, normally ending in .git/info/lfs.
    #[arg(long)]
    endpoint: String,
    /// Permit HTTP to a literal loopback IP for local services and tests only.
    #[arg(long)]
    allow_insecure_loopback: bool,
}

impl NetworkArgs {
    fn client(&self) -> Result<remote::Client> {
        let authorization = match std::env::var("GITR_LFS_AUTHORIZATION") {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => None,
            Err(_) => anyhow::bail!("GITR_LFS_AUTHORIZATION is not valid Unicode"),
        };
        remote::Client::new(&self.endpoint, authorization, self.allow_insecure_loopback)
    }
}

#[derive(Subcommand)]
enum LfsCommand {
    /// Git clean filter. Read stdin, verify/cache bytes, write a standard pointer.
    Clean,
    /// Local Git smudge filter. A missing object is an error; use fetch first.
    Smudge {
        /// Leave pointers in place for selective/lazy hydration.
        #[arg(long)]
        skip: bool,
    },
    /// Hash a file and print its standard pointer without storing it.
    Pointer { file: PathBuf },
    /// Store a file and print its standard pointer.
    Import { file: PathBuf },
    /// Check a cached object's hash and length.
    Verify(ObjectArgs),
    /// Fetch one known object using the standard Git LFS basic batch protocol.
    Fetch {
        #[command(flatten)]
        object: ObjectArgs,
        #[command(flatten)]
        network: NetworkArgs,
    },
    /// Upload one verified cached object using the standard Git LFS protocol.
    Upload {
        #[command(flatten)]
        object: ObjectArgs,
        #[command(flatten)]
        network: NetworkArgs,
    },
    /// Acquire a server-side asset lock. Requires server locking support.
    Lock {
        path: String,
        #[command(flatten)]
        network: NetworkArgs,
    },
    /// List all locks reported by the server.
    Locks {
        #[command(flatten)]
        network: NetworkArgs,
    },
    /// Release a server-side asset lock by its opaque ID.
    Unlock {
        id: String,
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        network: NetworkArgs,
    },
    /// Query which server locks belong to this authenticated user.
    VerifyLocks {
        #[command(flatten)]
        network: NetworkArgs,
    },
}

fn main() -> std::process::ExitCode {
    // The worker never parses repository options, discovers Git or opens redb.
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() == 2 && args[1] == actions::runner::WORKER_ARGUMENT {
        return match actions::runner::worker_stdio() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(_) => std::process::ExitCode::FAILURE,
        };
    }
    match run(Cli::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
            {
                return std::process::ExitCode::SUCCESS;
            }
            eprintln!("gitr: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    if let Some(path) = cli.directory {
        std::env::set_current_dir(path).context("cannot change directory")?;
    }
    match cli.command {
        Command::Actions { command } => actions::run(command),
        Command::Git(command) => git::run(command),
        Command::Db { command } => db::run(command),
        Command::Lfs { storage, command } => run_lfs(storage, command),
        Command::Assets { command } => match command {
            AssetsCommand::Import { file, cache } => {
                print_json(&chunk_store(cache)?.import(File::open(file)?)?)
            }
            AssetsCommand::Hydrate {
                object,
                cache,
                storage,
            } => chunk_store(cache)?.hydrate(&object.pointer()?, &store(storage)?),
            AssetsCommand::Attributes { engine } => {
                io::stdout()
                    .lock()
                    .write_all(assets::attributes(&engine)?.as_bytes())?;
                Ok(())
            }
            AssetsCommand::Scan {
                threshold_bytes,
                limit,
            } => {
                let repo = git::discover()?;
                let root = repo
                    .workdir()
                    .context("bare repositories have no worktree to scan")?;
                print_json(&assets::scan(root, threshold_bytes, limit)?)
            }
        },
        Command::Doctor => {
            let repo = git::discover()?;
            print_json(&serde_json::json!({
                "git_dir": repo.git_dir(),
                "common_dir": repo.common_dir(),
                "worktree": repo.workdir(),
                "object_format": repo.object_hash().to_string(),
                "shallow": repo.is_shallow()?,
                "native_engine": "gitoxide/gix 0.90.0",
                "catalog": db::catalog_path(&repo),
                "catalog_backend": "redb 4.5.0; local cooperating writer processes",
                "embedding": "512-dimensional lexical feature vectors; no learned semantic model",
                "lfs_cache": default_store()?.root(),
                "git_command_parity": "partial; see Documentation/gitr/compatibility.md",
                "distributed_ha": "not implemented",
            }))
        }
    }
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, value)?;
    writeln!(stdout)?;
    Ok(())
}

fn default_store() -> Result<lfs::Store> {
    let repo = git::discover()?;
    // Never silently ignore a custom cache and report assets missing.
    if let Some(storage) = repo.config_snapshot().string("lfs.storage") {
        ensure!(
            storage.is_empty(),
            "custom lfs.storage is not auto-resolved yet; pass --storage pointing to its objects directory"
        );
    }
    let lfs_dir = repo.common_dir().join("lfs");
    if let Ok(meta) = fs::symlink_metadata(&lfs_dir) {
        ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "repository LFS directory must not be a symlink"
        );
    }
    Ok(lfs::Store::new(lfs_dir.join("objects")))
}

fn store(storage: Option<PathBuf>) -> Result<lfs::Store> {
    storage.map(lfs::Store::new).map_or_else(default_store, Ok)
}

fn chunk_store(cache: Option<PathBuf>) -> Result<chunks::ChunkStore> {
    if let Some(cache) = cache {
        return Ok(chunks::ChunkStore::new(cache));
    }
    let repo = git::discover()?;
    let directory = repo.common_dir().join("gitr");
    if let Ok(meta) = fs::symlink_metadata(&directory) {
        ensure!(
            meta.file_type().is_dir(),
            "gitr metadata directory must not be a symlink"
        );
    }
    Ok(chunks::ChunkStore::new(directory.join("chunks")))
}

fn run_lfs(storage: Option<PathBuf>, command: LfsCommand) -> Result<()> {
    match command {
        LfsCommand::Clean => lfs::clean(&store(storage)?, io::stdin().lock(), io::stdout().lock()),
        LfsCommand::Smudge { skip } => lfs::smudge(
            &store(storage)?,
            io::stdin().lock(),
            io::stdout().lock(),
            skip,
        ),
        LfsCommand::Pointer { file } => {
            let pointer = lfs::hash_reader(File::open(file)?)?;
            io::stdout().lock().write_all(&pointer.encode()?)?;
            Ok(())
        }
        LfsCommand::Import { file } => {
            let pointer = store(storage)?.ingest(File::open(file)?, None)?;
            io::stdout().lock().write_all(&pointer.encode()?)?;
            Ok(())
        }
        LfsCommand::Verify(object) => store(storage)?.verify(&object.pointer()?),
        LfsCommand::Fetch { object, network } => network
            .client()?
            .download(&store(storage)?, &object.pointer()?),
        LfsCommand::Upload { object, network } => network
            .client()?
            .upload(&store(storage)?, &object.pointer()?),
        LfsCommand::Lock { path, network } => print_json(&network.client()?.lock(&path)?),
        LfsCommand::Locks { network } => print_json(&network.client()?.list_locks()?),
        LfsCommand::Unlock { id, force, network } => {
            print_json(&network.client()?.unlock(&id, force)?)
        }
        LfsCommand::VerifyLocks { network } => print_json(&network.client()?.verify_locks()?),
    }
}
