// SPDX-License-Identifier: MIT OR Apache-2.0
//! Repository-local snapshot identities persisted in the pinned omdb fork.
//!
//! Records and retry receipts commit atomically with immediate durability.
//! Deletions retain generation-bearing tombstones. Git objects stay in the Git
//! object database; a saved identity does not retain them against collection.
//! The filesystem containing the repository is trusted. These pathname checks
//! reject existing symlinks, but are not a sandbox against concurrent renames.

use std::{
    error::Error,
    fmt,
    fs::{self, File, OpenOptions},
    ops::Bound,
    path::{Path, PathBuf},
};

use omdb::{
    ConcurrencyMode, Database, Durability, ReadOnlyDatabase, ReadTransaction, ReadableDatabase,
    ReadableTable, TableDefinition,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sley::{ObjectFormat, ObjectId, RefPrecondition, ReferenceTarget, Repository};

use crate::snapshot::{Snapshot, open_repository};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_LIST_RECORDS: usize = 1000;
const MAX_IDENTIFIER_BYTES: usize = 128;
const MAX_RECORD_BYTES: usize = 4096;
const CACHE_BYTES: usize = 8 * 1024 * 1024;
const APPLICATION: &str = "rit.snapshot-catalog";
const STATE: TableDefinition<&str, &[u8]> = TableDefinition::new("rit.catalog.state.v1");
const RECORDS: TableDefinition<&str, &[u8]> = TableDefinition::new("rit.catalog.snapshots.v1");
const RECEIPTS: TableDefinition<&str, &[u8]> = TableDefinition::new("rit.catalog.receipts.v1");

pub type Result<T> = std::result::Result<T, CatalogError>;

#[derive(Debug)]
pub enum CatalogError {
    Busy,
    NotInitialized,
    AlreadyInitialized,
    ReadOnly,
    NotFound(String),
    Conflict { expected: u64, actual: u64 },
    RequestConflict,
    Invalid(String),
    Io(std::io::Error),
    Storage(omdb::Error),
    Git(sley::GitError),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => f.write_str("catalog writer is busy; retry the same request later"),
            Self::NotInitialized => {
                f.write_str("catalog is not initialized; run `rit --rit-db init`")
            }
            Self::AlreadyInitialized => f.write_str(
                "catalog path already exists; init never overwrites or adopts an existing file",
            ),
            Self::ReadOnly => f.write_str("catalog was opened read-only"),
            Self::NotFound(name) => {
                write!(f, "saved snapshot {name:?} does not exist or was deleted")
            }
            Self::Conflict { expected, actual } => write!(
                f,
                "stale catalog generation: expected {expected}, actual {actual}"
            ),
            Self::RequestConflict => {
                f.write_str("request ID was already used for different immutable input")
            }
            Self::Invalid(message) => f.write_str(message),
            Self::Io(error) => write!(f, "catalog filesystem: {error}"),
            Self::Storage(error) => write!(f, "catalog database: {error}"),
            Self::Git(error) => write!(f, "catalog snapshot: {error}"),
        }
    }
}

impl Error for CatalogError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Storage(error) => Some(error),
            Self::Git(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for CatalogError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<sley::GitError> for CatalogError {
    fn from(error: sley::GitError) -> Self {
        Self::Git(error)
    }
}

fn storage(error: impl Into<omdb::Error>) -> CatalogError {
    match error.into() {
        omdb::Error::WouldBlock | omdb::Error::DatabaseAlreadyOpen => CatalogError::Busy,
        error => CatalogError::Storage(error),
    }
}

fn invalid(message: impl Into<String>) -> CatalogError {
    CatalogError::Invalid(message.into())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotIdentity {
    pub object_format: String,
    pub commit_oid: String,
    pub root_tree_oid: String,
}

impl SnapshotIdentity {
    fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self {
            object_format: snapshot.object_format().name().to_owned(),
            commit_oid: snapshot.commit_id().to_string(),
            root_tree_oid: snapshot.tree_id().to_string(),
        }
    }

    fn validate(&self, format: ObjectFormat) -> Result<()> {
        if self.object_format != format.name() {
            return Err(invalid(
                "catalog snapshot object format does not match its repository",
            ));
        }
        let length = if format.name() == "sha1" { 40 } else { 64 };
        for oid in [&self.commit_oid, &self.root_tree_oid] {
            if oid.len() != length
                || !oid
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || oid.bytes().all(|b| b == b'0')
            {
                return Err(invalid("catalog contains a malformed canonical object ID"));
            }
        }
        Ok(())
    }
}

/// A slot in the catalog. `snapshot: None` is a deletion tombstone whose
/// generation must be supplied when recreating the name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRecord {
    pub name: String,
    pub generation: u64,
    pub snapshot: Option<SnapshotIdentity>,
}

impl SnapshotRecord {
    fn validate(&self, name: &str, format: ObjectFormat, generation: u64) -> Result<()> {
        identifier(&self.name, "snapshot name")?;
        if self.name != name || self.generation == 0 || self.generation > generation {
            return Err(invalid("catalog record name or generation is inconsistent"));
        }
        if let Some(identity) = &self.snapshot {
            identity.validate(format)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MutationResult {
    pub request_id: String,
    pub replayed: bool,
    pub record: SnapshotRecord,
}

/// A conservative Git reachability anchor for one exact catalog generation.
/// Pins are not automatically released when the name is replaced or deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PinResult {
    pub name: String,
    pub generation: u64,
    pub commit_oid: String,
    pub refname: String,
    pub already_pinned: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    application: String,
    schema_version: u32,
    object_format: String,
    generation: u64,
}

impl Header {
    fn validate(&self, format: ObjectFormat) -> Result<()> {
        if self.application != APPLICATION || self.schema_version != SCHEMA_VERSION {
            return Err(invalid(
                "unsupported Rit catalog application or schema; no automatic migration",
            ));
        }
        if self.object_format != format.name() {
            return Err(invalid(
                "catalog object format does not match its repository",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MutationRequest {
    name: String,
    expected_generation: u64,
    // None is delete. Put always contains the resolved immutable identity.
    snapshot: Option<SnapshotIdentity>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    request: MutationRequest,
    record: SnapshotRecord,
}

impl Receipt {
    fn validate(&self, format: ObjectFormat, generation: u64) -> Result<()> {
        self.record
            .validate(&self.request.name, format, generation)?;
        if self.record.snapshot != self.request.snapshot
            || self.record.generation <= self.request.expected_generation
        {
            return Err(invalid("catalog receipt is inconsistent"));
        }
        Ok(())
    }
}

enum Access {
    ReadOnly(ReadOnlyDatabase),
    Writable(Database),
}

struct Location {
    repository_path: PathBuf,
    common_dir: PathBuf,
    directory: PathBuf,
    database: PathBuf,
    format: ObjectFormat,
}

impl Location {
    fn resolve(path: &Path) -> Result<Self> {
        let repository = open_repository(path)?;
        let common_dir = fs::canonicalize(repository.common_dir())?;
        let directory = common_dir.join("rit");
        Ok(Self {
            repository_path: fs::canonicalize(repository.git_dir())?,
            database: directory.join("metadata.omdb"),
            common_dir,
            directory,
            format: repository.object_format(),
        })
    }

    fn check(&self) -> Result<()> {
        check_path(&self.directory, true)?;
        check_path(&self.database, false)
    }
}

/// Explicitly opened local catalog. Multiple handles/processes may read and
/// contend for the one omdb writer. `try_begin_write` returns Busy on contention;
/// opening, storage I/O and successful commits remain synchronous.
pub struct SnapshotCatalog {
    location: Location,
    access: Access,
}

impl SnapshotCatalog {
    /// Create a new catalog, including in an unborn repository. An existing
    /// file is never initialized or overwritten, even when empty or incomplete.
    /// Initialization currently requires Unix directory synchronization.
    pub fn initialize(path: &Path) -> Result<Self> {
        #[cfg(not(unix))]
        return Err(invalid(
            "catalog initialization requires a qualified directory synchronization implementation for this platform",
        ));

        #[cfg(unix)]
        {
            let location = Location::resolve(path)?;
            match fs::create_dir(&location.directory) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            check_path(&location.directory, true)?;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&location.database)
                .map_err(|error| {
                    if error.kind() == std::io::ErrorKind::AlreadyExists {
                        CatalogError::AlreadyInitialized
                    } else {
                        error.into()
                    }
                })?;
            // Make the reserved name durable before any process can validate
            // an application header and acknowledge its own mutation. Syncing
            // only after the header commit leaves an initialization race.
            file.sync_all()?;
            File::open(&location.directory)?.sync_all()?;
            File::open(&location.common_dir)?.sync_all()?;
            let backend = omdb::backends::FileBackend::new(file).map_err(storage)?;
            let db = builder().create_with_backend(backend).map_err(storage)?;
            let mut write = db.try_begin_write().map_err(storage)?;
            write
                .set_durability(Durability::Immediate)
                .map_err(storage)?;
            let header = Header {
                application: APPLICATION.into(),
                schema_version: SCHEMA_VERSION,
                object_format: location.format.name().into(),
                generation: 0,
            };
            {
                let mut state = write.open_table(STATE).map_err(storage)?;
                state
                    .insert("header", encode(&header)?.as_slice())
                    .map_err(storage)?;
                write.open_table(RECORDS).map_err(storage)?;
                write.open_table(RECEIPTS).map_err(storage)?;
            }
            write.commit().map_err(storage)?;
            Ok(Self {
                location,
                access: Access::Writable(db),
            })
        }
    }

    /// Open an existing catalog without modifying it. Recovery, when required,
    /// needs an explicit writable open; reads never create or migrate a store.
    pub fn open(path: &Path) -> Result<Self> {
        let location = Location::resolve(path)?;
        location.check()?;
        let db = builder()
            .open_read_only(&location.database)
            .map_err(storage)?;
        Self::checked(location, Access::ReadOnly(db))
    }

    /// Open an existing store for mutations. omdb may recover its own committed
    /// state after an interrupted writer; unknown Rit schemas are never adopted.
    pub fn open_writable(path: &Path) -> Result<Self> {
        let location = Location::resolve(path)?;
        location.check()?;
        // Refuse a clean foreign/schema-mismatched database before a writable
        // open can change even omdb's bookkeeping. An interrupted own database
        // may require omdb recovery before any header can be read; the explicit
        // writable path permits that recovery, then validates the header again.
        match builder().open_read_only(&location.database) {
            Ok(db) => {
                let read = db.begin_read().map_err(storage)?;
                read_header(&read, location.format)?;
                read.open_table(RECORDS).map_err(storage)?;
                read.open_table(RECEIPTS).map_err(storage)?;
            }
            Err(omdb::DatabaseError::RepairAborted) => {}
            Err(error) => return Err(storage(error)),
        }
        let db = builder().open(&location.database).map_err(storage)?;
        Self::checked(location, Access::Writable(db))
    }

    fn checked(location: Location, access: Access) -> Result<Self> {
        let catalog = Self { location, access };
        let read = catalog.begin_read()?;
        catalog.header(&read)?;
        read.open_table(RECORDS).map_err(storage)?;
        read.open_table(RECEIPTS).map_err(storage)?;
        Ok(catalog)
    }

    pub fn database_path(&self) -> &Path {
        &self.location.database
    }

    pub fn object_format(&self) -> ObjectFormat {
        self.location.format
    }

    fn begin_read(&self) -> Result<ReadTransaction> {
        match &self.access {
            Access::ReadOnly(db) => db.begin_read().map_err(storage),
            Access::Writable(db) => db.begin_read().map_err(storage),
        }
    }

    fn header(&self, read: &ReadTransaction) -> Result<Header> {
        read_header(read, self.location.format)
    }

    /// Return a live record or a tombstone. None means this name never existed.
    pub fn get(&self, name: &str) -> Result<Option<SnapshotRecord>> {
        identifier(name, "snapshot name")?;
        let read = self.begin_read()?;
        let header = self.header(&read)?;
        let table = read.open_table(RECORDS).map_err(storage)?;
        table
            .get(name)
            .map_err(storage)?
            .map(|value| {
                let record: SnapshotRecord = decode(value.value())?;
                record.validate(name, self.location.format, header.generation)?;
                Ok(record)
            })
            .transpose()
    }

    /// Bounded lexical page, including tombstones. `after` is exclusive.
    pub fn list(&self, after: Option<&str>, limit: usize) -> Result<Vec<SnapshotRecord>> {
        if limit == 0 || limit > MAX_LIST_RECORDS {
            return Err(invalid(format!(
                "catalog list limit must be 1..={MAX_LIST_RECORDS}"
            )));
        }
        if let Some(after) = after {
            identifier(after, "list cursor")?;
        }
        let read = self.begin_read()?;
        let header = self.header(&read)?;
        let table = read.open_table(RECORDS).map_err(storage)?;
        let range = table
            .range::<&str>((Bound::Excluded(after.unwrap_or("")), Bound::Unbounded))
            .map_err(storage)?;
        let mut records = Vec::with_capacity(limit);
        for item in range.take(limit) {
            let (name, value) = item.map_err(storage)?;
            let record: SnapshotRecord = decode(value.value())?;
            record.validate(name.value(), self.location.format, header.generation)?;
            records.push(record);
        }
        Ok(records)
    }

    /// Resolve and verify the commit and root tree before taking the writer.
    /// Retry a moving ref using the original full commit ID to keep input exact.
    pub fn put(
        &self,
        name: &str,
        revision: &str,
        expected_generation: u64,
        request_id: &str,
    ) -> Result<MutationResult> {
        identifier(name, "snapshot name")?;
        identifier(request_id, "request ID")?;
        if !matches!(self.access, Access::Writable(_)) {
            return Err(CatalogError::ReadOnly);
        }
        // A completed request remains replayable using its full immutable ID
        // even if its Git objects have since been pruned. This returns the
        // original receipt, never a claim that the content is still readable.
        if let Some(result) = self.replay_put(name, revision, expected_generation, request_id)? {
            return Ok(result);
        }
        let snapshot = Snapshot::open(&self.location.repository_path, revision)?;
        let identity = SnapshotIdentity::from_snapshot(&snapshot);
        identity.validate(self.location.format)?;
        self.mutate(
            request_id,
            MutationRequest {
                name: name.into(),
                expected_generation,
                snapshot: Some(identity),
            },
        )
    }

    fn replay_put(
        &self,
        name: &str,
        revision: &str,
        expected_generation: u64,
        request_id: &str,
    ) -> Result<Option<MutationResult>> {
        let oid_length = if self.location.format.name() == "sha1" {
            40
        } else {
            64
        };
        if revision.len() != oid_length
            || !revision
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Ok(None);
        }
        let read = self.begin_read()?;
        let header = self.header(&read)?;
        let receipts = read.open_table(RECEIPTS).map_err(storage)?;
        let Some(value) = receipts.get(request_id).map_err(storage)? else {
            return Ok(None);
        };
        let receipt: Receipt = decode(value.value())?;
        receipt.validate(self.location.format, header.generation)?;
        if receipt.request.name != name
            || receipt.request.expected_generation != expected_generation
            || receipt
                .request
                .snapshot
                .as_ref()
                .is_none_or(|identity| identity.commit_oid != revision)
        {
            return Err(CatalogError::RequestConflict);
        }
        Ok(Some(MutationResult {
            request_id: request_id.into(),
            replayed: true,
            record: receipt.record,
        }))
    }

    /// Delete a live record, keeping its generation and retry receipt. Deleting
    /// an absent/already deleted slot fails unless replaying its original receipt.
    pub fn delete(
        &self,
        name: &str,
        expected_generation: u64,
        request_id: &str,
    ) -> Result<MutationResult> {
        identifier(name, "snapshot name")?;
        identifier(request_id, "request ID")?;
        self.mutate(
            request_id,
            MutationRequest {
                name: name.into(),
                expected_generation,
                snapshot: None,
            },
        )
    }

    fn mutate(&self, request_id: &str, request: MutationRequest) -> Result<MutationResult> {
        let Access::Writable(db) = &self.access else {
            return Err(CatalogError::ReadOnly);
        };
        let mut write = db.try_begin_write().map_err(storage)?;
        write
            .set_durability(Durability::Immediate)
            .map_err(storage)?;
        let record;
        {
            let mut state = write.open_table(STATE).map_err(storage)?;
            let mut header: Header = decode(
                state
                    .get("header")
                    .map_err(storage)?
                    .ok_or_else(|| invalid("missing Rit catalog header"))?
                    .value(),
            )?;
            header.validate(self.location.format)?;
            let mut receipts = write.open_table(RECEIPTS).map_err(storage)?;
            if let Some(value) = receipts.get(request_id).map_err(storage)? {
                let receipt: Receipt = decode(value.value())?;
                receipt.validate(self.location.format, header.generation)?;
                if receipt.request != request {
                    return Err(CatalogError::RequestConflict);
                }
                return Ok(MutationResult {
                    request_id: request_id.into(),
                    replayed: true,
                    record: receipt.record,
                });
            }
            let mut records = write.open_table(RECORDS).map_err(storage)?;
            let current: Option<SnapshotRecord> = records
                .get(request.name.as_str())
                .map_err(storage)?
                .map(|value| decode(value.value()))
                .transpose()?;
            if let Some(current) = &current {
                current.validate(&request.name, self.location.format, header.generation)?;
            }
            let actual = current.as_ref().map_or(0, |record| record.generation);
            if actual != request.expected_generation {
                return Err(CatalogError::Conflict {
                    expected: request.expected_generation,
                    actual,
                });
            }
            if request.snapshot.is_none()
                && current
                    .as_ref()
                    .is_none_or(|record| record.snapshot.is_none())
            {
                return Err(CatalogError::NotFound(request.name));
            }
            header.generation = header
                .generation
                .checked_add(1)
                .ok_or_else(|| invalid("catalog generation exhausted"))?;
            record = SnapshotRecord {
                name: request.name.clone(),
                generation: header.generation,
                snapshot: request.snapshot.clone(),
            };
            let receipt = Receipt {
                request,
                record: record.clone(),
            };
            records
                .insert(record.name.as_str(), encode(&record)?.as_slice())
                .map_err(storage)?;
            receipts
                .insert(request_id, encode(&receipt)?.as_slice())
                .map_err(storage)?;
            state
                .insert("header", encode(&header)?.as_slice())
                .map_err(storage)?;
        }
        write.commit().map_err(storage)?;
        Ok(MutationResult {
            request_id: request_id.into(),
            replayed: false,
            record,
        })
    }

    /// Reopen saved Git content and check the recorded immutable identity.
    /// Metadata can survive object pruning; content access then fails explicitly.
    pub fn snapshot(&self, name: &str) -> Result<Snapshot> {
        let identity = self
            .get(name)?
            .and_then(|record| record.snapshot)
            .ok_or_else(|| CatalogError::NotFound(name.into()))?;
        let snapshot = Snapshot::open(&self.location.repository_path, &identity.commit_oid)?;
        if SnapshotIdentity::from_snapshot(&snapshot) != identity {
            return Err(invalid(
                "saved snapshot identity does not match Git content",
            ));
        }
        Ok(snapshot)
    }

    /// Pin the exact live generation in Git's ordinary reachability namespace.
    ///
    /// The deterministic name makes a retry after an interrupted ref commit
    /// recoverable without extending or migrating the omdb schema. This is a
    /// create-only ref update: foreign refs are never replaced, and deletion of
    /// the catalog record does not release a successful pin. A later, separately
    /// qualified release protocol must own that transition.
    ///
    /// Git GC must not run concurrently with pin publication. Git's own
    /// pruning rules do not make a newly created ref atomic with an already
    /// running GC's reachability walk.
    pub fn pin(&self, name: &str, expected_generation: u64) -> Result<PinResult> {
        identifier(name, "snapshot name")?;
        let original = self
            .get(name)?
            .ok_or_else(|| CatalogError::NotFound(name.into()))?;
        if original.generation != expected_generation {
            return Err(CatalogError::Conflict {
                expected: expected_generation,
                actual: original.generation,
            });
        }
        let identity = original
            .snapshot
            .as_ref()
            .ok_or_else(|| CatalogError::NotFound(name.into()))?;
        let repository = open_repository(&self.location.repository_path)?;
        verify_owned_object_store(&repository)?;
        let snapshot = Snapshot::open(&self.location.repository_path, &identity.commit_oid)?;
        if SnapshotIdentity::from_snapshot(&snapshot) != *identity {
            return Err(invalid(
                "saved snapshot identity does not match Git content",
            ));
        }
        let oid = ObjectId::from_hex(self.location.format, &identity.commit_oid)?;
        let refname = pin_ref_name(name, original.generation);

        // A saved identity alone is insufficient: verify the full reachable
        // graph before installing a ref. The second walk must open a fresh
        // object store so decoded-object/pack caches cannot conceal removal
        // during publication; this still cannot coordinate with Git GC.
        verify_reachable(&repository, oid)?;
        let refs = repository.references();
        let already_pinned = match refs.read_ref(&refname)? {
            Some(ReferenceTarget::Direct(existing)) if existing == oid => true,
            Some(_) => return Err(invalid("snapshot pin ref exists with a different target")),
            None => {
                let mut transaction = refs.transaction();
                transaction.update_to(
                    &refname,
                    ReferenceTarget::Direct(oid),
                    RefPrecondition::MustNotExist,
                    None,
                );
                match transaction.commit() {
                    Ok(()) => false,
                    Err(error) => {
                        // A second process can win the create-only race. A
                        // matching ref is the same idempotent publication.
                        if refs.read_ref(&refname)? != Some(ReferenceTarget::Direct(oid)) {
                            return Err(error.into());
                        }
                        true
                    }
                }
            }
        };
        let post_repository = open_repository(&self.location.repository_path)?;
        verify_owned_object_store(&post_repository)?;
        verify_reachable(&post_repository, oid)?;
        if self.get(name)?.as_ref() != Some(&original) {
            return Err(invalid(
                "catalog record changed during pin publication; pin retained conservatively",
            ));
        }
        if post_repository.references().read_ref(&refname)? != Some(ReferenceTarget::Direct(oid)) {
            return Err(invalid("snapshot pin ref changed during verification"));
        }
        Ok(PinResult {
            name: name.into(),
            generation: original.generation,
            commit_oid: identity.commit_oid.clone(),
            refname,
            already_pinned,
        })
    }
}

// A ref in this repository cannot retain objects owned by a different repo's
// GC. Explicit pins therefore require a self-contained local object store.
// Reject promisor packs too: they describe a store that can omit descendants.
fn verify_owned_object_store(repository: &Repository) -> Result<()> {
    let objects = repository.common_dir().join("objects");
    if !fs::symlink_metadata(&objects)?.file_type().is_dir() {
        return Err(invalid(
            "snapshot pins require a local Git objects directory",
        ));
    }
    for entry in fs::read_dir(&objects)? {
        if entry?.file_type()?.is_symlink() {
            return Err(invalid(
                "snapshot pins reject symlinked Git object storage directories",
            ));
        }
    }
    let alternates = objects.join("info/alternates");
    match fs::symlink_metadata(alternates) {
        Ok(_) => {
            return Err(invalid(
                "snapshot pins do not support Git object alternates",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let pack = objects.join("pack");
    match fs::symlink_metadata(&pack) {
        Ok(metadata) if metadata.file_type().is_dir() => {
            for item in fs::read_dir(pack)? {
                let item = item?;
                if item.file_type()?.is_symlink() {
                    return Err(invalid(
                        "snapshot pins require locally owned Git pack files",
                    ));
                }
                let path = item.path();
                if path
                    .extension()
                    .is_some_and(|extension| extension == "promisor")
                {
                    return Err(invalid("snapshot pins do not support Git promisor packs"));
                }
            }
        }
        Ok(_) => return Err(invalid("snapshot pins require a local Git pack directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

// Encode name bytes, not their ref syntax. Components stay below 100 bytes,
// including for the longest allowed name, and no name can inject a ref path.
fn pin_ref_name(name: &str, generation: u64) -> String {
    use std::fmt::Write as _;

    let mut path = String::from("refs/rit/retained");
    for (index, chunk) in name.as_bytes().chunks(48).enumerate() {
        path.push('/');
        if index == 0 {
            path.push('n');
        }
        for byte in chunk {
            write!(&mut path, "{byte:02x}").expect("String formatting is infallible");
        }
    }
    write!(&mut path, "/g{generation:020}").expect("String formatting is infallible");
    path
}

// This is intentionally an explicit, potentially expensive check: it walks
// every reachable commit, tree and blob, including parent history, and hashes
// their uncompressed bytes. Gitlinks are external repositories and excluded.
fn verify_reachable(repository: &Repository, commit: ObjectId) -> Result<()> {
    // The pack walk is strict about absent objects, including promisor misses.
    // The fsck walk separately enforces each Git graph edge's expected type,
    // parsing and the hash of every reachable object.
    let plan = repository
        .reachable_pack_plan()
        .root(commit)
        .build()?
        .ok_or_else(|| invalid("snapshot pin has no reachable Git objects"))?;
    let objects = repository.common_dir().join("objects");
    for oid in plan.object_ids() {
        // A loose object may itself be a symlink into another repository's
        // GC domain even when its containing directory is locally owned.
        let hex = oid.to_string();
        let loose = objects.join(&hex[..2]).join(&hex[2..]);
        match fs::symlink_metadata(loose) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(invalid(format!(
                    "snapshot pin object {oid} is a symlink into external storage"
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let findings = sley_fsck::check_connectivity(
        repository.object_database(),
        repository.object_format(),
        &[commit],
        &[],
        sley_fsck::ConnectivityOptions::default(),
    );
    if let Some(error) = findings.errors().next() {
        return Err(invalid(format!(
            "invalid snapshot pin Git graph: {}",
            error.message()
        )));
    }
    Ok(())
}

fn read_header(read: &ReadTransaction, format: ObjectFormat) -> Result<Header> {
    let state = read.open_table(STATE).map_err(storage)?;
    let value = state
        .get("header")
        .map_err(storage)?
        .ok_or_else(|| invalid("missing Rit catalog header"))?;
    let header: Header = decode(value.value())?;
    header.validate(format)?;
    Ok(header)
}

fn builder() -> omdb::Builder {
    let mut builder = Database::builder();
    builder
        .set_cache_size(CACHE_BYTES)
        .set_concurrency_mode(ConcurrencyMode::MultiWriter);
    builder
}

fn check_path(path: &Path, directory: bool) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            CatalogError::NotInitialized
        } else {
            error.into()
        }
    })?;
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err(invalid(
            "catalog paths must be real directories and regular files, not symlinks",
        ));
    }
    Ok(())
}

fn identifier(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_BYTES
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(invalid(format!(
            "{label} must be 1..={MAX_IDENTIFIER_BYTES} ASCII letters, digits, '.', '_' or '-', starting with a letter or digit"
        )));
    }
    Ok(())
}

fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| invalid(format!("encode catalog record: {error}")))?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(invalid("catalog record exceeds the size limit"));
    }
    Ok(bytes)
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(invalid("catalog record exceeds the size limit"));
    }
    serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("invalid catalog record: {error}")))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier, Mutex};

    fn repository() -> (tempfile::TempDir, String) {
        let temp = tempfile::tempdir().unwrap();
        let repo = sley::Repository::init(temp.path()).unwrap();
        let tree = repo
            .write_raw_object(sley::GitObjectType::Tree, vec![])
            .unwrap();
        let commit = repo.write_raw_object(
            sley::GitObjectType::Commit,
            format!("tree {tree}\nauthor Catalog <catalog@example.invalid> 1 +0000\ncommitter Catalog <catalog@example.invalid> 1 +0000\n\ntest\n").into_bytes(),
        ).unwrap();
        (temp, commit.to_string())
    }

    fn writable(catalog: &SnapshotCatalog) -> &Database {
        let Access::Writable(db) = &catalog.access else {
            panic!("expected writer")
        };
        db
    }

    #[test]
    fn held_writer_returns_busy_and_failed_cas_does_not_consume_receipt() {
        let (temp, commit) = repository();
        let catalog = SnapshotCatalog::initialize(temp.path()).unwrap();
        let held = writable(&catalog).try_begin_write().unwrap();
        assert!(matches!(
            catalog.put("job", &commit, 0, "retry"),
            Err(CatalogError::Busy)
        ));
        drop(held);
        let first = catalog.put("job", &commit, 0, "retry").unwrap();
        assert_eq!(first.record.generation, 1);
        assert!(!first.replayed);
        assert!(matches!(
            catalog.put("job", &commit, 0, "cas"),
            Err(CatalogError::Conflict { .. })
        ));
        assert_eq!(
            catalog
                .put("job", &commit, 1, "cas")
                .unwrap()
                .record
                .generation,
            2
        );
        let reader = SnapshotCatalog::open(temp.path()).unwrap();
        assert!(matches!(
            reader.put("job", &commit, 0, "retry"),
            Err(CatalogError::ReadOnly)
        ));
    }

    #[test]
    fn simultaneous_writers_publish_one_generation() {
        let (temp, commit) = repository();
        let catalog = Arc::new(SnapshotCatalog::initialize(temp.path()).unwrap());
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for id in ["one", "two"] {
            let catalog = Arc::clone(&catalog);
            let barrier = Arc::clone(&barrier);
            let commit = commit.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                catalog.put("job", &commit, 0, id)
            }));
        }
        barrier.wait();
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert!(
            results
                .iter()
                .filter_map(|result| result.as_ref().err())
                .all(|error| matches!(error, CatalogError::Busy | CatalogError::Conflict { .. }))
        );
        assert_eq!(catalog.get("job").unwrap().unwrap().generation, 1);
    }

    #[test]
    fn unknown_headers_and_missing_tables_are_rejected_without_adoption() {
        for field in [
            "application",
            "schema_version",
            "object_format",
            "missing_header",
            "missing_table",
        ] {
            let (temp, _) = repository();
            let catalog = SnapshotCatalog::initialize(temp.path()).unwrap();
            let db_path = catalog.database_path().to_owned();
            let write = writable(&catalog).begin_write().unwrap();
            if field == "missing_table" {
                write.delete_table(RECEIPTS).unwrap();
            } else {
                let mut table = write.open_table(STATE).unwrap();
                if field == "missing_header" {
                    table.remove("header").unwrap();
                } else {
                    let mut header: serde_json::Value =
                        serde_json::from_slice(table.get("header").unwrap().unwrap().value())
                            .unwrap();
                    header[field] = match field {
                        "application" => serde_json::json!("foreign"),
                        "schema_version" => serde_json::json!(2),
                        _ => serde_json::json!("sha256"),
                    };
                    table
                        .insert("header", serde_json::to_vec(&header).unwrap().as_slice())
                        .unwrap();
                }
            }
            write.commit().unwrap();
            drop(catalog);
            let before = fs::read(&db_path).unwrap();
            assert!(SnapshotCatalog::open(temp.path()).is_err(), "{field}");
            assert!(
                SnapshotCatalog::open_writable(temp.path()).is_err(),
                "{field}"
            );
            assert_eq!(fs::read(&db_path).unwrap(), before, "{field}");
        }
    }

    #[test]
    fn invalid_records_tree_mismatch_and_generation_overflow_fail_closed() {
        let (temp, commit) = repository();
        let catalog = SnapshotCatalog::initialize(temp.path()).unwrap();
        let original = catalog.put("job", &commit, 0, "first").unwrap().record;
        for malformed in [
            serde_json::json!({"name":"job", "generation":0, "snapshot":null}),
            serde_json::json!({"name":"other", "generation":1, "snapshot":null}),
            serde_json::json!({"name":"job", "generation":1, "snapshot":{"object_format":"sha1", "commit_oid":"short", "root_tree_oid":"short"}}),
        ] {
            let write = writable(&catalog).begin_write().unwrap();
            write
                .open_table(RECORDS)
                .unwrap()
                .insert("job", serde_json::to_vec(&malformed).unwrap().as_slice())
                .unwrap();
            write.commit().unwrap();
            assert!(catalog.get("job").is_err());
            assert!(catalog.list(None, 1).is_err());
            assert!(catalog.put("job", &commit, 1, "invalid").is_err());
        }
        let mut changed = original.clone();
        changed.snapshot.as_mut().unwrap().root_tree_oid = "1".repeat(40);
        let write = writable(&catalog).begin_write().unwrap();
        write
            .open_table(RECORDS)
            .unwrap()
            .insert("job", encode(&changed).unwrap().as_slice())
            .unwrap();
        write.commit().unwrap();
        assert!(
            catalog
                .snapshot("job")
                .unwrap_err()
                .to_string()
                .contains("does not match Git content")
        );
        assert!(
            catalog
                .pin("job", original.generation)
                .unwrap_err()
                .to_string()
                .contains("does not match Git content")
        );
        assert!(
            open_repository(temp.path())
                .unwrap()
                .references()
                .read_ref(&pin_ref_name("job", original.generation))
                .unwrap()
                .is_none()
        );

        let write = writable(&catalog).begin_write().unwrap();
        {
            let mut state = write.open_table(STATE).unwrap();
            let mut header: Header = decode(state.get("header").unwrap().unwrap().value()).unwrap();
            header.generation = u64::MAX;
            state
                .insert("header", encode(&header).unwrap().as_slice())
                .unwrap();
            write
                .open_table(RECORDS)
                .unwrap()
                .insert("job", encode(&original).unwrap().as_slice())
                .unwrap();
        }
        write.commit().unwrap();
        assert!(
            catalog
                .put("job", &commit, 1, "exhausted")
                .unwrap_err()
                .to_string()
                .contains("exhausted")
        );
        assert_eq!(catalog.get("job").unwrap().unwrap(), original);
        let read = catalog.begin_read().unwrap();
        assert!(
            read.open_table(RECEIPTS)
                .unwrap()
                .get("exhausted")
                .unwrap()
                .is_none()
        );
    }

    #[derive(Default)]
    struct Image {
        bytes: Vec<u8>,
        remaining: Option<usize>,
    }

    #[derive(Clone, Default)]
    struct FaultBackend(Arc<Mutex<Image>>);

    impl fmt::Debug for FaultBackend {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("FaultBackend")
        }
    }

    impl Image {
        fn before_mutation(&mut self) -> std::io::Result<()> {
            if let Some(remaining) = &mut self.remaining {
                if *remaining == 0 {
                    return Err(std::io::Error::other("injected storage failure"));
                }
                *remaining -= 1;
            }
            Ok(())
        }
    }

    impl omdb::StorageBackend for FaultBackend {
        fn len(&self) -> std::io::Result<u64> {
            Ok(self.0.lock().unwrap().bytes.len() as u64)
        }
        fn read(&self, offset: u64, out: &mut [u8]) -> std::io::Result<()> {
            let offset =
                usize::try_from(offset).map_err(|_| std::io::Error::other("offset overflow"))?;
            let end = offset
                .checked_add(out.len())
                .ok_or_else(|| std::io::Error::other("range overflow"))?;
            let image = self.0.lock().unwrap();
            let bytes = image
                .bytes
                .get(offset..end)
                .ok_or_else(|| std::io::Error::other("read outside image"))?;
            out.copy_from_slice(bytes);
            Ok(())
        }
        fn write(&self, offset: u64, bytes: &[u8]) -> std::io::Result<()> {
            let offset =
                usize::try_from(offset).map_err(|_| std::io::Error::other("offset overflow"))?;
            let end = offset
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("range overflow"))?;
            let mut image = self.0.lock().unwrap();
            image.before_mutation()?;
            let target = image
                .bytes
                .get_mut(offset..end)
                .ok_or_else(|| std::io::Error::other("write outside image"))?;
            target.copy_from_slice(bytes);
            Ok(())
        }
        fn set_len(&self, len: u64) -> std::io::Result<()> {
            let len = usize::try_from(len).map_err(|_| std::io::Error::other("length overflow"))?;
            if len > 16 * 1024 * 1024 {
                return Err(std::io::Error::other("test image too large"));
            }
            let mut image = self.0.lock().unwrap();
            image.before_mutation()?;
            image.bytes.resize(len, 0);
            Ok(())
        }
        fn sync_data(&self) -> std::io::Result<()> {
            self.0.lock().unwrap().before_mutation()
        }
    }

    #[test]
    fn injected_io_failures_recover_record_and_receipt_together() {
        let (temp, commit) = repository();
        let catalog = SnapshotCatalog::initialize(temp.path()).unwrap();
        let path = catalog.database_path().to_owned();
        drop(catalog);
        let baseline = fs::read(path).unwrap();
        let snapshot = Snapshot::open(temp.path(), &commit).unwrap();
        let identity = SnapshotIdentity::from_snapshot(&snapshot);
        let mut failures = 0;
        let mut successes = 0;
        // Fail each of the first 24 write/resize/sync calls. The final boundary
        // exceeds this small transaction's I/O count and exercises success.
        for boundary in 0..24 {
            let backend = FaultBackend(Arc::new(Mutex::new(Image {
                bytes: baseline.clone(),
                remaining: None,
            })));
            let db = Database::builder()
                .set_cache_size(CACHE_BYTES)
                .create_with_backend(backend.clone())
                .unwrap();
            let catalog = SnapshotCatalog {
                location: Location::resolve(temp.path()).unwrap(),
                access: Access::Writable(db),
            };
            backend.0.lock().unwrap().remaining = Some(boundary);
            let result = catalog.mutate(
                "durable",
                MutationRequest {
                    name: "job".into(),
                    expected_generation: 0,
                    snapshot: Some(identity.clone()),
                },
            );
            if result.is_err() {
                failures += 1;
            } else {
                successes += 1;
            }
            drop(catalog);
            backend.0.lock().unwrap().remaining = None;
            let db = Database::builder()
                .set_cache_size(CACHE_BYTES)
                .create_with_backend(backend)
                .unwrap();
            let recovered = SnapshotCatalog::checked(
                Location::resolve(temp.path()).unwrap(),
                Access::Writable(db),
            )
            .unwrap();
            let record = recovered.get("job").unwrap();
            let receipt_present = {
                let read = recovered.begin_read().unwrap();
                read.open_table(RECEIPTS)
                    .unwrap()
                    .get("durable")
                    .unwrap()
                    .is_some()
            };
            assert_eq!(record.is_some(), receipt_present, "boundary {boundary}");
            if result.is_ok() {
                assert!(receipt_present, "lost acknowledged boundary {boundary}");
            }
            let retry = recovered.put("job", &commit, 0, "durable").unwrap();
            assert_eq!(retry.replayed, receipt_present, "boundary {boundary}");
            assert_eq!(retry.record.generation, 1, "boundary {boundary}");
        }
        assert!(
            failures > 0 && successes > 0,
            "failures={failures}, successes={successes}"
        );
    }
}
