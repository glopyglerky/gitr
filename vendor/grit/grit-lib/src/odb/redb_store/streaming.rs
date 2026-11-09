//! Streaming insertion with the same atomic, immutable record format.
use std::fs::File;
use std::io::Read;

use super::{RedbObjectStore, CHUNKS, CHUNK_BYTES, OBJECTS, storage_error};
use crate::error::{Error, Result};
use crate::objects::ObjectId;
use crate::odb::streaming::{digests, raw_blob};
use redb::{Durability, ReadableTable};

impl RedbObjectStore {
    /// Insert the canonical representation of a snapshotted blob. The caller
    /// owns this private temporary file; arbitrary concurrently mutable files
    /// should be passed through `Odb::write_blob_stream`, which snapshots first.
    pub(crate) fn insert_blob_stream(&self, snapshot: &mut File, size: u64) -> Result<ObjectId> {
        let (oid, checksum, raw_size) = digests(self.algo, &mut raw_blob(snapshot, size)?)?;
        let expected_size = size.checked_add(format!("blob {size}\0").len() as u64)
            .ok_or_else(|| storage_error("object length overflow"))?;
        if raw_size != expected_size { return Err(storage_error("blob snapshot length changed")); }
        let mut record = [0u8; 40];
        record[..8].copy_from_slice(&raw_size.to_be_bytes());
        record[8..].copy_from_slice(&checksum);
        let mut transaction = self.db.begin_write().map_err(storage_error)?;
        transaction.set_durability(Durability::Immediate).map_err(storage_error)?;
        {
            let mut objects = transaction.open_table(OBJECTS).map_err(storage_error)?;
            let mut chunks = transaction.open_table(CHUNKS).map_err(storage_error)?;
            let existing = objects.get(oid.as_bytes()).map_err(storage_error)?.map(|v| v.value().to_vec());
            if existing.as_ref().is_some_and(|old| old.as_slice() != record.as_slice()) {
                return Err(Error::ObjectIdentityConflict(oid.to_hex()));
            }
            if existing.is_none() && chunks.range((oid.as_bytes(), 0)..=(oid.as_bytes(), u64::MAX))
                .map_err(storage_error)?.next().transpose().map_err(storage_error)?.is_some() {
                return Err(Error::ObjectIdentityConflict(oid.to_hex()));
            }
            let mut stream = raw_blob(snapshot, size)?;
            let mut buffer = vec![0u8; CHUNK_BYTES];
            let count = raw_size.div_ceil(CHUNK_BYTES as u64);
            for index in 0..count {
                let len = (raw_size - index * CHUNK_BYTES as u64).min(CHUNK_BYTES as u64) as usize;
                stream.read_exact(&mut buffer[..len])?;
                if existing.is_some() {
                    let stored = chunks.get((oid.as_bytes(), index)).map_err(storage_error)?
                        .ok_or_else(|| Error::ObjectIdentityConflict(oid.to_hex()))?;
                    if stored.value() != &buffer[..len] { return Err(Error::ObjectIdentityConflict(oid.to_hex())); }
                } else {
                    chunks.insert((oid.as_bytes(), index), &buffer[..len]).map_err(storage_error)?;
                }
            }
            if stream.read(&mut buffer[..1])? != 0 { return Err(storage_error("blob snapshot length changed")); }
            if chunks.range((oid.as_bytes(), count)..=(oid.as_bytes(), u64::MAX))
                .map_err(storage_error)?.next().transpose().map_err(storage_error)?.is_some() {
                return Err(Error::ObjectIdentityConflict(oid.to_hex()));
            }
            objects.insert(oid.as_bytes(), record.as_slice()).map_err(storage_error)?;
        }
        transaction.commit().map_err(storage_error)?;
        Ok(oid)
    }
}
