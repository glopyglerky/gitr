//! Database-resident, bounded Wasm actions. This is not a GitHub Actions runner.
//!
//! The database owns code, inputs, claims and results. Guest execution never holds
//! a database transaction or receives a database handle. A fresh interpreter has
//! no WASI, network, filesystem, environment, clock, subprocess or secret access.
//! Local cooperating processes serialize only short redb transactions; this is
//! not a replicated scheduler or a distributed exactly-once execution guarantee.

use anyhow::{Context, Result, bail, ensure};
use clap::Subcommand;
use gix::bstr::ByteSlice;
use redb::{ConcurrencyMode, Database, Durability, ReadableDatabase, TableDefinition};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::{self, Read, Write}, path::{Path, PathBuf}, time::{SystemTime, UNIX_EPOCH}};
use wasmi::{Caller, CompilationMode, Config, EnforcedLimits, Engine, Extern, ExternType, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, ValType};

pub const MAX_BYTES: usize = 1024 * 1024;
pub const PROFILE: &str = "gitr-actions-v1/wasmi-2.0.0/fuel-10000000/memory-16MiB/io-8MiB/calls-4096";
const FUEL: u64 = 10_000_000;
const MEMORY_BYTES: usize = 16 * 1024 * 1024;
const LEASE_MS: u64 = 60_000;
const OBJECTS: TableDefinition<&str, &[u8]> = TableDefinition::new("actions.objects.v1");
const JOBS: TableDefinition<&str, &[u8]> = TableDefinition::new("actions.jobs.v1");
const META: TableDefinition<&str, u64> = TableDefinition::new("actions.meta.v1");

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Validate and store a binary Wasm module, returning its SHA-256 identity.
    Install { module: PathBuf },
    /// Persist one immutable committed blob as an action input, without a checkout.
    Submit {
        job: String,
        module: String,
        #[arg(long, default_value = "HEAD")]
        revision: String,
        #[arg(long)]
        path: String,
    },
    /// Claim a named job, execute it outside all write transactions, and publish.
    Run { job: String },
    /// Read the durable state of a named job.
    Status { job: String },
    /// Fence a pending or running job so its old worker cannot publish.
    Cancel { job: String },
    /// Write a successful job's verified artifact bytes to stdout.
    Artifact { job: String },
    /// Execute one bounded JSON control request from stdin. Local process only.
    Control,
}

/// Transport-independent control surface. This is not an MCP protocol transport.
/// An eventual HTTP/MCP adapter must authorize the repository and operation before
/// calling this service. No caller-selected filesystem/database path is accepted.
#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Submit { job: String, module: String, revision: String, path: String },
    Status { job: String },
    Cancel { job: String },
    Run { job: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status { Queued, Running, Passed, Failed, Cancelled }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Spec {
    pub profile: String,
    pub module_sha256: String,
    pub commit_oid: String,
    pub blob_oid: String,
    pub path: String,
    pub input_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Job {
    pub id: String,
    pub spec: Spec,
    pub status: Status,
    pub attempt: u64,
    pub lease_until_ms: Option<u64>,
    pub artifact_sha256: Option<String>,
    pub exit_code: Option<i32>,
    pub failure: Option<String>,
}

/// Construction verifies the input against an actual Git snapshot, not a caller's
/// claimed commit/blob identifiers. Fields are intentionally private.
pub struct SnapshotInput {
    commit: String,
    blob: String,
    path: String,
    bytes: Vec<u8>,
}

impl SnapshotInput {
    pub fn from_git(repo: &gix::Repository, revision: &str, path: &str) -> Result<Self> {
        ensure!(!revision.is_empty() && revision.len() <= 4096, "invalid revision length");
        ensure!(!path.is_empty() && path.len() <= 4096 && !path.contains(['\0', '\\'])
            && !path.split('/').any(|part| part.is_empty() || part == "." || part == ".."), "invalid repository-relative input path");
        // Resolve the branch once. A later branch move cannot change this input.
        let commit = repo.rev_parse_single(revision.as_bytes().as_bstr())?.object()?
            .peel_to_kind(gix::objs::Kind::Commit)?.id.to_string();
        let spec = format!("{commit}:{path}");
        let id = repo.rev_parse_single(spec.as_bytes().as_bstr())?;
        let header = repo.find_header(id)?;
        ensure!(header.kind() == gix::objs::Kind::Blob && header.size() <= MAX_BYTES as u64,
            "action input must be a committed blob of at most 1 MiB");
        let object = id.object()?;
        ensure!(object.data.len() <= MAX_BYTES, "action input exceeds limit");
        Ok(Self { commit, blob: id.to_string(), path: path.to_owned(), bytes: object.data.clone() })
    }
}

pub struct ActionDb { db: Database }

#[derive(Clone)]
struct Claim { id: String, attempt: u64, spec: Spec }

pub fn database_path(repo: &gix::Repository) -> PathBuf {
    repo.common_dir().join("gitr").join("state.redb")
}

impl ActionDb {
    pub fn open(repo: &gix::Repository, create: bool) -> Result<Self> {
        let path = database_path(repo);
        let directory = path.parent().context("missing action database parent")?;
        if create {
            match fs::create_dir(directory) {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
                Err(error) => return Err(error).context("create action database directory"),
            }
        }
        ensure!(fs::symlink_metadata(directory)?.file_type().is_dir(), "action database parent must be a real directory");
        match fs::symlink_metadata(&path) {
            Ok(meta) => ensure!(meta.file_type().is_file(), "action database must be a regular file, not a symlink"),
            Err(error) if create && error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error).context("open action database; install a module first"),
        }
        let result = Self::open_path(&path, create)?;
        #[cfg(unix)]
        if create {
            // redb syncs its file. Also make new directory entries durable.
            fs::File::open(directory)?.sync_all()?;
            fs::File::open(repo.common_dir())?.sync_all()?;
        }
        Ok(result)
    }

    fn open_path(path: &Path, create: bool) -> Result<Self> {
        let mut builder = Database::builder();
        builder.set_concurrency_mode(ConcurrencyMode::MultiWriter).set_cache_size(8 * 1024 * 1024);
        let db = if create { builder.create(path)? } else { builder.open(path)? };
        if create {
            let mut tx = db.begin_write()?;
            tx.set_durability(Durability::Immediate);
            {
                let mut table = tx.open_table(META)?;
                let version = table.get("schema")?.map(|v| v.value());
                ensure!(version.is_none() || version == Some(1), "unsupported action schema");
                table.insert("schema", 1)?;
            }
            tx.open_table(OBJECTS)?;
            tx.open_table(JOBS)?;
            tx.commit()?;
        } else {
            let tx = db.begin_read()?;
            ensure!(tx.open_table(META)?.get("schema")?.map(|v| v.value()) == Some(1), "unsupported action schema");
        }
        Ok(Self { db })
    }

    pub fn install(&self, bytes: &[u8]) -> Result<String> {
        // Validation and eager compilation are deliberately outside a writer.
        prepare(bytes)?;
        let oid = digest(bytes);
        let mut tx = self.db.begin_write()?;
        tx.set_durability(Durability::Immediate);
        put_object(&tx, &oid, bytes)?;
        tx.commit()?;
        Ok(oid)
    }

    pub fn submit(&self, id: &str, module: &str, input: SnapshotInput) -> Result<Job> {
        key(id)?;
        oid(module)?;
        ensure!(input.bytes.len() <= MAX_BYTES, "action input exceeds limit");
        let spec = Spec { profile: PROFILE.into(), module_sha256: module.into(), commit_oid: input.commit,
            blob_oid: input.blob, path: input.path, input_sha256: digest(&input.bytes) };
        let mut tx = self.db.begin_write()?;
        tx.set_durability(Durability::Immediate);
        {
            let modules = tx.open_table(OBJECTS)?;
            let bytes = modules.get(module)?.context("module is not installed")?;
            ensure!(bytes.value().len() <= MAX_BYTES && digest(bytes.value()) == module, "stored module integrity failure");
        }
        let existing = { let table = tx.open_table(JOBS)?;
            table.get(id)?.map(|value| serde_json::from_slice::<Job>(value.value())).transpose()? };
        if let Some(existing) = existing {
            ensure!(existing.spec == spec, "idempotency key already belongs to a different action input or module");
            return Ok(existing);
        }
        put_object(&tx, &spec.input_sha256, &input.bytes)?;
        let job = Job { id: id.into(), spec, status: Status::Queued, attempt: 0,
            lease_until_ms: None, artifact_sha256: None, exit_code: None, failure: None };
        write_job(&tx, &job)?;
        tx.commit()?;
        Ok(job)
    }

    pub fn get(&self, id: &str) -> Result<Job> {
        key(id)?;
        let tx = self.db.begin_read()?;
        let table = tx.open_table(JOBS)?;
        let value = table.get(id)?.context("unknown action job")?;
        Ok(serde_json::from_slice(value.value())?)
    }

    fn claim(&self, id: &str, clock: impl FnOnce() -> Result<u64>) -> Result<Option<Claim>> {
        key(id)?;
        let mut tx = self.db.begin_write()?;
        tx.set_durability(Durability::Immediate);
        let now = clock()?; // Read the trusted host clock after acquiring the writer.
        let mut job = read_job(&tx, id)?;
        ensure!(job.spec.profile == PROFILE, "action requires a different runtime profile");
        match job.status {
            Status::Queued => (),
            Status::Running if job.lease_until_ms.is_some_and(|until| now >= until) => (),
            _ => return Ok(None),
        }
        job.attempt = job.attempt.checked_add(1).context("action attempt overflow")?;
        job.status = Status::Running;
        job.lease_until_ms = Some(now.checked_add(LEASE_MS).context("action lease overflow")?);
        write_job(&tx, &job)?;
        tx.commit()?;
        Ok(Some(Claim { id: id.into(), attempt: job.attempt, spec: job.spec }))
    }

    pub fn cancel(&self, id: &str) -> Result<Job> {
        key(id)?;
        let mut tx = self.db.begin_write()?;
        tx.set_durability(Durability::Immediate);
        let mut job = read_job(&tx, id)?;
        if matches!(job.status, Status::Queued | Status::Running) {
            job.status = Status::Cancelled;
            job.lease_until_ms = None;
            write_job(&tx, &job)?;
            tx.commit()?;
        }
        Ok(job)
    }

    pub fn run_once(&self, id: &str) -> Result<Job> {
        let Some(claim) = self.claim(id, now_ms)? else { return self.get(id); };
        // Immutable bytes are copied in bounded read transactions. No writer is
        // retained during validation, compilation, execution or host calls.
        let module = self.object(&claim.spec.module_sha256)?;
        let input = self.object(&claim.spec.input_sha256)?;
        let result = execute(&module, input);
        self.finish(&claim, result, now_ms)
    }

    fn finish(&self, claim: &Claim, result: Result<(i32, Vec<u8>)>, clock: impl FnOnce() -> Result<u64>) -> Result<Job> {
        let mut tx = self.db.begin_write()?;
        tx.set_durability(Durability::Immediate);
        let now = clock()?;
        let mut job = read_job(&tx, &claim.id)?;
        ensure!(job.status == Status::Running && job.attempt == claim.attempt && job.spec == claim.spec
            && job.lease_until_ms.is_some_and(|until| now < until), "worker claim is stale, expired or cancelled");
        match result {
            Ok((0, output)) => {
                ensure!(output.len() <= MAX_BYTES, "artifact exceeds limit");
                let id = digest(&output);
                put_object(&tx, &id, &output)?;
                job.status = Status::Passed;
                job.exit_code = Some(0);
                job.artifact_sha256 = Some(id);
            }
            Ok((code, _)) => {
                job.status = Status::Failed;
                job.exit_code = Some(code);
                job.failure = Some("guest returned a nonzero status; output discarded".into());
            }
            Err(_) => {
                job.status = Status::Failed;
                job.failure = Some("guest validation, trap or resource limit failure; output discarded".into());
            }
        }
        job.lease_until_ms = None;
        // Artifact and terminal receipt become visible together, or neither does.
        write_job(&tx, &job)?;
        tx.commit()?;
        Ok(job)
    }

    fn object(&self, id: &str) -> Result<Vec<u8>> {
        oid(id)?;
        let tx = self.db.begin_read()?;
        let table = tx.open_table(OBJECTS)?;
        let value = table.get(id)?.context("missing action object")?;
        ensure!(value.value().len() <= MAX_BYTES && digest(value.value()) == id, "action object integrity failure");
        Ok(value.value().to_vec())
    }

    pub fn artifact(&self, id: &str) -> Result<Vec<u8>> {
        let job = self.get(id)?;
        ensure!(job.status == Status::Passed, "only successful jobs have published artifacts");
        self.object(job.artifact_sha256.as_deref().context("successful action has no artifact")?)
    }
}

fn now_ms() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().try_into()?)
}
fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn oid(value: &str) -> Result<()> {
    ensure!(value.len() == 64 && value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)), "expected lowercase SHA-256 identity"); Ok(())
}
fn key(value: &str) -> Result<()> {
    ensure!(!value.is_empty() && value.len() <= 128 && value.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)), "job id must contain 1..128 ASCII letters, digits, dot, underscore or hyphen"); Ok(())
}
fn read_job(tx: &redb::WriteTransaction, id: &str) -> Result<Job> {
    let table = tx.open_table(JOBS)?;
    let value = table.get(id)?.context("unknown action job")?;
    Ok(serde_json::from_slice(value.value())?)
}
fn write_job(tx: &redb::WriteTransaction, job: &Job) -> Result<()> {
    let bytes = serde_json::to_vec(job)?;
    tx.open_table(JOBS)?.insert(job.id.as_str(), bytes.as_slice())?;
    Ok(())
}
fn put_object(tx: &redb::WriteTransaction, id: &str, bytes: &[u8]) -> Result<()> {
    ensure!(bytes.len() <= MAX_BYTES && digest(bytes) == id, "invalid action object");
    let mut table = tx.open_table(OBJECTS)?;
    {
        if let Some(existing) = table.get(id)? {
            ensure!(existing.value() == bytes, "action object collision or corruption");
            return Ok(());
        }
    }
    table.insert(id, bytes)?;
    Ok(())
}

fn prepare(bytes: &[u8]) -> Result<Module> {
    ensure!(bytes.len() <= MAX_BYTES && bytes.starts_with(b"\0asm"), "expected binary Wasm of at most 1 MiB");
    let mut config = Config::default();
    config.consume_fuel(true).compilation_mode(CompilationMode::Eager).allow_start_fn(false)
        .wasm_multi_memory(false).set_max_recursion_depth(128).set_min_stack_height(128)
        .set_max_stack_height(256 * 1024).set_max_cached_stacks(0).enforced_limits(EnforcedLimits::strict());
    // Fresh engines bound accumulated compiled code, including rejected modules.
    let engine = Engine::new(&config);
    let module = Module::new(&engine, bytes)?;
    for import in module.imports() {
        ensure!(import.module() == "gitr_actions_v1", "unavailable action capability");
        let (params, results): (&[ValType], &[ValType]) = match import.name() {
            "input_len" => (&[], &[ValType::I32]),
            "input_read" => (&[ValType::I32, ValType::I32, ValType::I32], &[ValType::I32]),
            "output_write" => (&[ValType::I32, ValType::I32], &[ValType::I32]),
            _ => bail!("unavailable action capability"),
        };
        ensure!(matches!(import.ty(), ExternType::Func(ty) if ty.params() == params && ty.results() == results), "invalid action capability signature");
    }
    ensure!(matches!(module.get_export("run"), Some(ExternType::Func(ty)) if ty.params().is_empty() && ty.results() == [ValType::I32]), "expected run() -> i32 export");
    ensure!(matches!(module.get_export("memory"), Some(ExternType::Memory(ty)) if !ty.is_64() && ty.minimum() <= (MEMORY_BYTES / 65536) as u64), "expected bounded wasm32 memory export");
    Ok(module)
}

struct Host { input: Vec<u8>, output: Vec<u8>, limits: StoreLimits, calls: usize, io_bytes: usize }
impl Host {
    fn charge(&mut self, bytes: usize) -> std::result::Result<(), wasmi::Error> {
        self.calls += 1;
        self.io_bytes = self.io_bytes.checked_add(bytes).ok_or_else(|| wasmi::Error::new("host byte overflow"))?;
        if self.calls > 4096 || self.io_bytes > 8 * MAX_BYTES { return Err(wasmi::Error::new("host work budget exceeded")); }
        Ok(())
    }
}
fn memory(caller: &Caller<'_, Host>) -> std::result::Result<Memory, wasmi::Error> {
    caller.get_export("memory").and_then(Extern::into_memory).ok_or_else(|| wasmi::Error::new("missing memory"))
}
fn range(pointer: i32, length: i32) -> std::result::Result<std::ops::Range<usize>, wasmi::Error> {
    if pointer < 0 || length < 0 || length as usize > MAX_BYTES { return Err(wasmi::Error::new("invalid host buffer")); }
    let start = pointer as usize;
    let end = start.checked_add(length as usize).ok_or_else(|| wasmi::Error::new("host buffer overflow"))?;
    Ok(start..end)
}
fn execute(bytes: &[u8], input: Vec<u8>) -> Result<(i32, Vec<u8>)> {
    ensure!(input.len() <= MAX_BYTES, "action input exceeds limit");
    let module = prepare(bytes)?;
    let engine = module.engine();
    let limits = StoreLimitsBuilder::new().memory_size(MEMORY_BYTES).memories(1).tables(1)
        .table_elements(4096).instances(1).trap_on_grow_failure(true).build();
    let mut store = Store::new(engine, Host { input, output: Vec::new(), limits, calls: 0, io_bytes: 0 });
    store.limiter(|state| &mut state.limits);
    store.set_fuel(FUEL)?;
    let mut linker = Linker::<Host>::new(engine);
    linker.func_wrap("gitr_actions_v1", "input_len", |mut caller: Caller<'_, Host>| -> std::result::Result<i32, wasmi::Error> {
        caller.data_mut().charge(0)?;
        Ok(caller.data().input.len() as i32)
    })?;
    linker.func_wrap("gitr_actions_v1", "input_read", |mut caller: Caller<'_, Host>, offset: i32, pointer: i32, length: i32| -> std::result::Result<i32, wasmi::Error> {
        let source = range(offset, length)?;
        let destination = range(pointer, length)?;
        caller.data_mut().charge(length as usize)?;
        let memory = memory(&caller)?;
        let (guest, state) = memory.data_and_store_mut(&mut caller);
        let input = state.input.get(source).ok_or_else(|| wasmi::Error::new("input out of bounds"))?;
        guest.get_mut(destination).ok_or_else(|| wasmi::Error::new("guest buffer out of bounds"))?.copy_from_slice(input);
        Ok(length)
    })?;
    linker.func_wrap("gitr_actions_v1", "output_write", |mut caller: Caller<'_, Host>, pointer: i32, length: i32| -> std::result::Result<i32, wasmi::Error> {
        let source = range(pointer, length)?;
        caller.data_mut().charge(length as usize)?;
        let memory = memory(&caller)?;
        let (guest, state) = memory.data_and_store_mut(&mut caller);
        if state.output.len().checked_add(length as usize).is_none_or(|n| n > MAX_BYTES) { return Err(wasmi::Error::new("artifact budget exceeded")); }
        let bytes = guest.get(source).ok_or_else(|| wasmi::Error::new("guest buffer out of bounds"))?;
        state.output.extend_from_slice(bytes);
        Ok(length)
    })?;
    let instance = linker.instantiate_and_start(&mut store, &module)?;
    let result = instance.get_typed_func::<(), i32>(&store, "run")?.call(&mut store, ())?;
    Ok((result, std::mem::take(&mut store.data_mut().output)))
}

pub fn control(repo: &gix::Repository, request: Request) -> Result<Job> {
    let create = matches!(request, Request::Submit { .. });
    let db = ActionDb::open(repo, create)?;
    match request {
        Request::Submit { job, module, revision, path } => db.submit(&job, &module, SnapshotInput::from_git(repo, &revision, &path)?),
        Request::Status { job } => db.get(&job),
        Request::Cancel { job } => db.cancel(&job),
        Request::Run { job } => db.run_once(&job),
    }
}
fn bounded_read(input: impl Read, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    input.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "input exceeds byte limit");
    Ok(bytes)
}
fn print_json(value: &impl Serialize) -> Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, value)?;
    writeln!(out)?;
    Ok(())
}
pub fn run(command: Command) -> Result<()> {
    let repo = crate::git::discover()?;
    if let Command::Control = command {
        let bytes = bounded_read(io::stdin().lock(), 32 * 1024)?;
        return print_json(&control(&repo, serde_json::from_slice(&bytes)?)?);
    }
    let create = matches!(command, Command::Install { .. } | Command::Submit { .. });
    let db = ActionDb::open(&repo, create)?;
    match command {
        Command::Install { module } => print_json(&serde_json::json!({"module_sha256": db.install(&bounded_read(fs::File::open(module)?, MAX_BYTES)?)?, "profile": PROFILE})),
        Command::Submit { job, module, revision, path } => print_json(&db.submit(&job, &module, SnapshotInput::from_git(&repo, &revision, &path)?)?),
        Command::Run { job } => {
            let result = db.run_once(&job)?;
            print_json(&result)?;
            ensure!(result.status == Status::Passed, "action has not passed");
            Ok(())
        }
        Command::Status { job } => print_json(&db.get(&job)?),
        Command::Cancel { job } => print_json(&db.cancel(&job)?),
        Command::Artifact { job } => { io::stdout().lock().write_all(&db.artifact(&job)?)?; Ok(()) },
        Command::Control => unreachable!("handled above"),
    }
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
