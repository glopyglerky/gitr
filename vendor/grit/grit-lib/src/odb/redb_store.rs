//! Gitr's durable immutable Git object store. SPDX-License-Identifier: MIT
//!
//! The table key is the unmodified Git SHA-1 or SHA-256 object ID. Values preserve
//! the exact header and payload bytes, including valid non-canonical headers
//! supplied through `hash-object --literally`. Fixed-size chunks avoid redb's
//! single-value limit. Each object and all its chunks commit in one immediately
//! durable transaction. SHA-256 also protects the payload of SHA-1 objects.
//!
//! This store does not own refs, indexes or existing pack files. [`super::Odb`]
//! selects it only for `gitr.redbObjects=true` and publishes standard loose files
//! after a successful database commit. No table from Gitr's catalog or action
//! database is opened here.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use redb::{
    ConcurrencyMode, Database, Durability, ReadableDatabase, ReadableTable, TableDefinition,
    TableHandle, WriteTransaction,
};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::objects::{HashAlgo, Object, ObjectId};

use super::{hash_bytes_with, parse_object_bytes};

/// Maximum bytes in one object chunk. Reads through `Odb` still return a whole
/// object because that is the imported engine's API; this is not a streaming API.
pub const CHUNK_BYTES: usize = 1024 * 1024;
const SCHEMA: u64 = 1;
const META: TableDefinition<&str, u64> = TableDefinition::new("gitr.object-meta.v1");
const OBJECTS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("gitr.git-objects.v1");
const CHUNKS: TableDefinition<(&[u8], u64), &[u8]> =
    TableDefinition::new("gitr.git-object-chunks.v1");

/// A shared handle to a single-hash-algorithm immutable object database.
/// Separate handles and processes coordinate through redb's `MultiWriter` mode.
#[derive(Clone)]
pub struct RedbObjectStore {
    db: Arc<Database>,
    path: PathBuf,
    algo: HashAlgo,
}

impl std::fmt::Debug for RedbObjectStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedbObjectStore")
            .field("path", &self.path)
            .field("algo", &self.algo)
            .finish_non_exhaustive()
    }
}

fn storage_error(error: impl std::fmt::Display) -> Error {
    Error::RedbObjectStore(error.to_string())
}

fn builder() -> redb::Builder {
    let mut builder = Database::builder();
    builder
        .set_concurrency_mode(ConcurrencyMode::MultiWriter)
        .set_cache_size(8 * 1024 * 1024);
    builder
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn check_path(path: &Path, create: bool) -> Result<()> {
    let directory = parent(path);
    if create {
        fs::create_dir_all(directory)?;
    }
    if !fs::symlink_metadata(directory)?.file_type().is_dir() {
        return Err(storage_error("database parent must be a real directory"));
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => Ok(()),
        Ok(_) => Err(storage_error("database must be a regular file, not a symlink")),
        Err(error) if create && error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::Io(error)),
    }
}

fn validate_schema(
    version: Option<u64>,
    algorithm: Option<u64>,
    chunk_size: Option<u64>,
    algo: HashAlgo,
) -> Result<()> {
    if version != Some(SCHEMA) {
        return Err(storage_error("missing or unsupported object schema"));
    }
    if algorithm != Some(u64::from(algo.oid_version())) {
        return Err(storage_error("object hash algorithm does not match repository"));
    }
    if chunk_size != Some(CHUNK_BYTES as u64) {
        return Err(storage_error("unsupported object chunk size"));
    }
    Ok(())
}

impl RedbObjectStore {
    /// Open an existing database without creating files, directories or tables.
    pub fn open(path: impl AsRef<Path>, algo: HashAlgo) -> Result<Self> {
        let path = path.as_ref();
        check_path(path, false)?;
        let db = builder().open(path).map_err(storage_error)?;
        let read = db.begin_read().map_err(storage_error)?;
        let meta = read.open_table(META).map_err(storage_error)?;
        validate_schema(
            meta.get("schema").map_err(storage_error)?.map(|v| v.value()),
            meta.get("hash").map_err(storage_error)?.map(|v| v.value()),
            meta.get("chunk-bytes")
                .map_err(storage_error)?
                .map(|v| v.value()),
            algo,
        )?;
        read.open_table(OBJECTS).map_err(storage_error)?;
        read.open_table(CHUNKS).map_err(storage_error)?;
        drop(meta);
        drop(read);
        Ok(Self {
            db: Arc::new(db),
            path: path.to_path_buf(),
            algo,
        })
    }

    /// Create a database or open a previously initialized one. Existing data is
    /// never replaced. An unrelated schema or hash format is rejected.
    pub fn create(path: impl AsRef<Path>, algo: HashAlgo) -> Result<Self> {
        let path = path.as_ref();
        check_path(path, true)?;
        let db = builder().create(path).map_err(storage_error)?;
        let mut write = db.begin_write().map_err(storage_error)?;
        write
            .set_durability(Durability::Immediate)
            .map_err(storage_error)?;
        let table_names: Vec<_> = write.list_tables().map_err(storage_error)?
            .map(|table| table.name().to_owned()).collect();
        let has_tables = !table_names.is_empty()
            || write
                .list_multimap_tables()
                .map_err(storage_error)?
                .next()
                .is_some();
        {
            let mut meta = write.open_table(META).map_err(storage_error)?;
            if has_tables {
                if !table_names.iter().any(|name| name == OBJECTS.name())
                    || !table_names.iter().any(|name| name == CHUNKS.name())
                {
                    return Err(storage_error("missing object tables in existing database"));
                }
                validate_schema(
                    meta.get("schema").map_err(storage_error)?.map(|v| v.value()),
                    meta.get("hash").map_err(storage_error)?.map(|v| v.value()),
                    meta.get("chunk-bytes")
                        .map_err(storage_error)?
                        .map(|v| v.value()),
                    algo,
                )?;
            } else {
                meta.insert("schema", SCHEMA).map_err(storage_error)?;
                meta.insert("hash", u64::from(algo.oid_version()))
                    .map_err(storage_error)?;
                meta.insert("chunk-bytes", CHUNK_BYTES as u64)
                    .map_err(storage_error)?;
            }
        }
        write.open_table(OBJECTS).map_err(storage_error)?;
        write.open_table(CHUNKS).map_err(storage_error)?;
        write.commit().map_err(storage_error)?;
        #[cfg(unix)]
        {
            fs::File::open(parent(path))?.sync_all()?;
            fs::File::open(parent(parent(path)))?.sync_all()?;
        }
        Ok(Self {
            db: Arc::new(db),
            path: path.to_path_buf(),
            algo,
        })
    }

    /// The exact database location, separate from catalogs and action state.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Insert one complete Git store representation. Success means its header
    /// and every chunk are durable. Repeated identical writes are idempotent.
    pub fn insert_raw(&self, raw: &[u8]) -> Result<ObjectId> {
        // A single-item batch keeps identical validation and rollback behavior.
        let mut ids = self.insert_batch(&[raw])?;
        ids.pop()
            .ok_or_else(|| storage_error("object insertion returned no identity"))
    }

    /// Insert a batch atomically. If any input or an existing record is invalid,
    /// none of this batch becomes visible. No filesystem mirrors are written by
    /// this low-level API; use `Odb` for ordinary Git interoperability.
    pub fn insert_batch(&self, objects: &[&[u8]]) -> Result<Vec<ObjectId>> {
        let mut write = self.db.begin_write().map_err(storage_error)?;
        write
            .set_durability(Durability::Immediate)
            .map_err(storage_error)?;
        let mut ids = Vec::with_capacity(objects.len());
        for raw in objects {
            parse_object_bytes(raw)?;
            let oid = hash_bytes_with(self.algo, raw);
            self.insert_in(&write, &oid, raw)?;
            ids.push(oid);
        }
        write.commit().map_err(storage_error)?;
        Ok(ids)
    }

    fn insert_in(&self, write: &WriteTransaction, oid: &ObjectId, raw: &[u8]) -> Result<()> {
        let mut objects = write.open_table(OBJECTS).map_err(storage_error)?;
        let mut chunks = write.open_table(CHUNKS).map_err(storage_error)?;
        let digest = Sha256::digest(raw);
        let mut record = [0u8; 40];
        record[..8].copy_from_slice(&(raw.len() as u64).to_be_bytes());
        record[8..].copy_from_slice(digest.as_slice());
        let previous = objects
            .get(oid.as_bytes())
            .map_err(storage_error)?
            .map(|v| v.value().to_vec());
        if let Some(previous) = previous {
            if previous.as_slice() != record.as_slice() {
                return Err(Error::ObjectIdentityConflict(oid.to_hex()));
            }
            // Compare bytes as well as hashes. Neither a SHA-1 collision nor a
            // damaged payload can silently become a successful duplicate write.
            for (index, wanted) in raw.chunks(CHUNK_BYTES).enumerate() {
                let stored = chunks
                    .get((oid.as_bytes(), index as u64))
                    .map_err(storage_error)?
                    .ok_or_else(|| Error::ObjectIdentityConflict(oid.to_hex()))?;
                if stored.value() != wanted {
                    return Err(Error::ObjectIdentityConflict(oid.to_hex()));
                }
            }
            if chunks
                .range((oid.as_bytes(), raw.len().div_ceil(CHUNK_BYTES) as u64)..=(oid.as_bytes(), u64::MAX))
                .map_err(storage_error)?
                .next().transpose().map_err(storage_error)?.is_some()
            {
                return Err(Error::ObjectIdentityConflict(oid.to_hex()));
            }
            return Ok(());
        }
        if chunks.range((oid.as_bytes(), 0)..=(oid.as_bytes(), u64::MAX))
            .map_err(storage_error)?.next().transpose().map_err(storage_error)?.is_some()
        {
            return Err(Error::ObjectIdentityConflict(oid.to_hex()));
        }
        for (index, chunk) in raw.chunks(CHUNK_BYTES).enumerate() {
            chunks
                .insert((oid.as_bytes(), index as u64), chunk)
                .map_err(storage_error)?;
        }
        objects
            .insert(oid.as_bytes(), record.as_slice())
            .map_err(storage_error)?;
        Ok(())
    }

    /// Read and verify exact stored bytes. Absence is distinct from a malformed
    /// database, missing chunk, hash mismatch, or storage error.
    pub fn read_raw(&self, oid: &ObjectId) -> Result<Option<Vec<u8>>> {
        if oid.algo() != self.algo {
            return Err(storage_error("object identity has the wrong hash algorithm"));
        }
        let read = self.db.begin_read().map_err(storage_error)?;
        let objects = read.open_table(OBJECTS).map_err(storage_error)?;
        let Some(record) = objects.get(oid.as_bytes()).map_err(storage_error)? else {
            return Ok(None);
        };
        let record = record.value();
        if record.len() != 40 {
            return Err(Error::CorruptObject(format!(
                "invalid redb object record for {oid}"
            )));
        }
        let mut size_bytes = [0u8; 8];
        size_bytes.copy_from_slice(&record[..8]);
        let size = usize::try_from(u64::from_be_bytes(size_bytes))
            .map_err(|_| storage_error("object is too large for this address space"))?;
        let mut raw = Vec::new();
        raw.try_reserve_exact(size)
            .map_err(|error| storage_error(format!("object allocation failed: {error}")))?;
        let chunks = read.open_table(CHUNKS).map_err(storage_error)?;
        let count = size.div_ceil(CHUNK_BYTES);
        for index in 0..count {
            let chunk = chunks
                .get((oid.as_bytes(), index as u64))
                .map_err(storage_error)?
                .ok_or_else(|| Error::CorruptObject(format!("missing redb object chunk for {oid}")))?;
            let expected = (size - raw.len()).min(CHUNK_BYTES);
            if chunk.value().len() != expected {
                return Err(Error::CorruptObject(format!(
                    "invalid redb object chunk length for {oid}"
                )));
            }
            raw.extend_from_slice(chunk.value());
        }
        if chunks
            .range((oid.as_bytes(), count as u64)..=(oid.as_bytes(), u64::MAX))
            .map_err(storage_error)?
            .next().transpose().map_err(storage_error)?.is_some()
        {
            return Err(Error::CorruptObject(format!(
                "unexpected trailing redb object chunk for {oid}"
            )));
        }
        if Sha256::digest(&raw).as_slice() != &record[8..]
            || hash_bytes_with(self.algo, &raw) != *oid
        {
            return Err(Error::CorruptObject(format!(
                "redb object hash mismatch for {oid}"
            )));
        }
        parse_object_bytes(&raw)?;
        Ok(Some(raw))
    }

    /// Read a verified Git object, or `None` when it has never been stored here.
    pub fn read(&self, oid: &ObjectId) -> Result<Option<Object>> {
        self.read_raw(oid)?
            .map(|raw| parse_object_bytes(&raw))
            .transpose()
    }

    /// Check existence without hiding corruption. This verifies the stored bytes
    /// rather than merely trusting a metadata key.
    pub fn contains(&self, oid: &ObjectId) -> Result<bool> {
        self.read_raw(oid).map(|raw| raw.is_some())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::odb::build_store_bytes;
    use crate::objects::ObjectKind;

    #[test]
    fn redb_objects_preserve_both_hash_formats_chunks_and_reopen() {
        for algo in [HashAlgo::Sha1, HashAlgo::Sha256] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("objects.redb");
            let raw = build_store_bytes(ObjectKind::Blob, &vec![b'x'; CHUNK_BYTES * 2 + 19]);
            let store = RedbObjectStore::create(&path, algo).unwrap();
            let oid = store.insert_raw(&raw).unwrap();
            assert_eq!(oid.algo(), algo);
            assert_eq!(store.insert_raw(&raw).unwrap(), oid);
            assert_eq!(store.read_raw(&oid).unwrap().unwrap(), raw);
            drop(store);
            let reopened = RedbObjectStore::open(&path, algo).unwrap();
            assert_eq!(reopened.read_raw(&oid).unwrap().unwrap(), raw);
            assert!(reopened.read(&ObjectId::null(algo)).unwrap().is_none());
            let other = if algo == HashAlgo::Sha1 { HashAlgo::Sha256 } else { HashAlgo::Sha1 };
            assert!(RedbObjectStore::open(&path, other).is_err());
        }
    }

    #[test]
    fn redb_objects_failed_batch_does_not_publish_earlier_items() {
        let dir = tempfile::tempdir().unwrap();
        let store = RedbObjectStore::create(dir.path().join("objects.redb"), HashAlgo::Sha1).unwrap();
        let first = b"blob 3\0one";
        assert!(store.insert_batch(&[first, b"blob 99\0short"]).is_err());
        assert!(!store.contains(&hash_bytes_with(HashAlgo::Sha1, first)).unwrap());
        assert_eq!(store.insert_raw(first).unwrap(), hash_bytes_with(HashAlgo::Sha1, first));
    }

    #[test]
    fn redb_objects_keep_exact_noncanonical_header() {
        let dir = tempfile::tempdir().unwrap();
        let store = RedbObjectStore::create(dir.path().join("objects.redb"), HashAlgo::Sha1).unwrap();
        let raw = b"blob 003\0abc";
        let oid = store.insert_raw(raw).unwrap();
        assert_eq!(store.read_raw(&oid).unwrap().unwrap(), raw);
        assert_eq!(store.read(&oid).unwrap().unwrap().data, b"abc");
        assert_ne!(oid, hash_bytes_with(HashAlgo::Sha1, b"blob 3\0abc"));
    }

    #[test]
    fn redb_objects_corrupt_chunks_refuse_read_and_duplicate_write() {
        let dir = tempfile::tempdir().unwrap();
        let store = RedbObjectStore::create(dir.path().join("objects.redb"), HashAlgo::Sha1).unwrap();
        let raw = b"blob 3\0abc";
        let oid = store.insert_raw(raw).unwrap();
        let write = store.db.begin_write().unwrap();
        {
            let mut chunks = write.open_table(CHUNKS).unwrap();
            chunks.insert((oid.as_bytes(), 0), b"blob 3\0xyz".as_slice()).unwrap();
        }
        write.commit().unwrap();
        assert!(matches!(store.read(&oid), Err(Error::CorruptObject(_))));
        assert!(matches!(store.insert_raw(raw), Err(Error::ObjectIdentityConflict(_))));
        let read = store.db.begin_read().unwrap();
        let chunks = read.open_table(CHUNKS).unwrap();
        assert_eq!(chunks.get((oid.as_bytes(), 0)).unwrap().unwrap().value(), b"blob 3\0xyz");
    }

    #[test]
    fn redb_objects_missing_chunk_is_not_an_absent_object() {
        let dir = tempfile::tempdir().unwrap();
        let store = RedbObjectStore::create(dir.path().join("objects.redb"), HashAlgo::Sha256).unwrap();
        let oid = store.insert_raw(b"blob 3\0abc").unwrap();
        let write = store.db.begin_write().unwrap();
        {
            write.open_table(CHUNKS).unwrap().remove((oid.as_bytes(), 0)).unwrap();
        }
        write.commit().unwrap();
        assert!(matches!(store.contains(&oid), Err(Error::CorruptObject(_))));
    }

    #[test]
    fn redb_objects_concurrent_handles_preserve_all_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objects.redb");
        let store = RedbObjectStore::create(&path, HashAlgo::Sha1).unwrap();
        let handles: Vec<_> = (0..4).map(|worker| {
            let path = path.clone();
            std::thread::spawn(move || {
                let own = RedbObjectStore::open(path, HashAlgo::Sha1).unwrap();
                for index in 0..8 {
                    let raw = build_store_bytes(ObjectKind::Blob, format!("{worker}-{index}").as_bytes());
                    let id = own.insert_raw(&raw).unwrap();
                    assert_eq!(own.read_raw(&id).unwrap().unwrap(), raw);
                    own.insert_raw(b"blob 6\0shared").unwrap();
                }
            })
        }).collect();
        for handle in handles { handle.join().unwrap(); }
        for worker in 0..4 {
            for index in 0..8 {
                let raw = build_store_bytes(ObjectKind::Blob, format!("{worker}-{index}").as_bytes());
                assert_eq!(store.read_raw(&hash_bytes_with(HashAlgo::Sha1, &raw)).unwrap().unwrap(), raw);
            }
        }
    }

    #[test]
    fn redb_objects_open_does_not_create_and_refuses_unrelated_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing/objects.redb");
        assert!(RedbObjectStore::open(&path, HashAlgo::Sha1).is_err());
        assert!(!path.parent().unwrap().exists());
        let path = dir.path().join("other.redb");
        let db = builder().create(&path).unwrap();
        let write = db.begin_write().unwrap();
        let unrelated: TableDefinition<&str, u64> = TableDefinition::new("unrelated");
        {
            write.open_table(unrelated).unwrap().insert("kept", 7).unwrap();
        }
        write.commit().unwrap();
        assert!(RedbObjectStore::create(&path, HashAlgo::Sha1).is_err());
        let read = db.begin_read().unwrap();
        assert_eq!(read.open_table(unrelated).unwrap().get("kept").unwrap().unwrap().value(), 7);
        assert!(read.open_table(META).is_err());
    }

    #[test]
    fn redb_odb_corruption_is_not_hidden_by_valid_loose_mirror() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join(".git");
        let objects = git_dir.join("objects");
        fs::create_dir_all(&objects).unwrap();
        fs::write(git_dir.join("config"), "[gitr]\nredbObjects=true\n").unwrap();
        let odb = crate::odb::Odb::new(&objects).with_config_git_dir(git_dir.clone());
        let oid = odb.write(ObjectKind::Blob, b"abc").unwrap();
        assert!(odb.object_path(&oid).is_file());
        let store = RedbObjectStore::open(git_dir.join("gitr/objects.redb"), HashAlgo::Sha1).unwrap();
        let write = store.db.begin_write().unwrap();
        {
            write.open_table(CHUNKS).unwrap()
                .insert((oid.as_bytes(), 0), b"blob 3\0xyz".as_slice()).unwrap();
        }
        write.commit().unwrap();
        assert!(matches!(odb.read(&oid), Err(Error::CorruptObject(_))));
        assert!(!odb.exists(&oid));
        assert!(odb.write(ObjectKind::Blob, b"abc").is_err());
        assert_eq!(crate::odb::Odb::read_loose_verify_oid(&odb.object_path(&oid), &oid).unwrap().data, b"abc");
    }
}
