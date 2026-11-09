//! Bounded-memory canonical blob hashing and durable publication.
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};

use flate2::{read::ZlibDecoder, write::ZlibEncoder};
use sha1::{Digest, Sha1};
use sha2::Sha256;

use super::Odb;
use crate::error::{Error, Result};
use crate::objects::{HashAlgo, ObjectId};

const BUFFER: usize = 64 * 1024;

/// Hash a raw Git representation while also recording its secondary checksum.
pub(super) fn digests(algo: HashAlgo, input: &mut dyn Read) -> Result<(ObjectId, [u8; 32], u64)> {
    let mut sha1 = Sha1::new();
    let mut sha256 = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; BUFFER];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 { break; }
        size = size.checked_add(count as u64).ok_or_else(|| Error::CorruptObject("object length overflow".into()))?;
        if algo == HashAlgo::Sha1 { sha1.update(&buffer[..count]); }
        sha256.update(&buffer[..count]);
    }
    let secondary: [u8; 32] = sha256.finalize().into();
    let oid = match algo {
        HashAlgo::Sha1 => ObjectId::from_bytes(sha1.finalize().as_slice())?,
        HashAlgo::Sha256 => ObjectId::from_bytes(&secondary)?,
    };
    Ok((oid, secondary, size))
}

pub(super) fn raw_blob<'a>(snapshot: &'a mut File, size: u64) -> Result<impl Read + 'a> {
    snapshot.seek(SeekFrom::Start(0))?;
    Ok(io::Cursor::new(format!("blob {size}\0").into_bytes()).chain(snapshot))
}

fn snapshot(input: &mut dyn Read) -> Result<(File, u64)> {
    let mut file = tempfile::tempfile()?;
    let size = io::copy(input, &mut file)?;
    file.seek(SeekFrom::Start(0))?;
    Ok((file, size))
}

impl Odb {
    /// Hash an unfiltered blob with fixed-size buffers. Spooling takes a stable
    /// snapshot so stdin and concurrently resized files get the correct header.
    pub fn hash_blob_stream(algo: HashAlgo, input: &mut dyn Read) -> Result<ObjectId> {
        let (mut file, size) = snapshot(input)?;
        let result = digests(algo, &mut raw_blob(&mut file, size)?)?;
        Ok(result.0)
    }

    /// Durably publish a canonical blob using bounded source buffers. redb is
    /// selected by the same repository setting as the other object write APIs.
    /// This API deliberately refuses memory overlays instead of allocating the
    /// entire stream behind a method advertised as bounded-memory.
    pub fn write_blob_stream(&self, input: &mut dyn Read) -> Result<ObjectId> {
        if self.mem_overlay.lock().map_err(|_| Error::CorruptObject("object overlay lock poisoned".into()))?.is_some() {
            return Err(Error::Message("streaming writes cannot target an in-memory object overlay".into()));
        }
        let (mut file, size) = snapshot(input)?;
        let (oid, _, _) = digests(self.hash_algo(), &mut raw_blob(&mut file, size)?)?;
        if let Some(store) = self.redb_store(true)? {
            let stored = store.insert_blob_stream(&mut file, size)?;
            if stored != oid { return Err(Error::ObjectIdentityConflict(oid.to_hex())); }
        }
        self.publish_blob_stream(&oid, &mut file, size)?;
        Ok(oid)
    }

    fn publish_blob_stream(&self, oid: &ObjectId, snapshot: &mut File, size: u64) -> Result<()> {
        let path = self.object_path(oid);
        let directory = path.parent().ok_or_else(|| Error::CorruptObject("object path has no parent".into()))?;
        fs::create_dir_all(directory)?;
        let synchronize = |file: &File| -> Result<()> {
            file.sync_all()?;
            #[cfg(unix)] {
                File::open(directory)?.sync_all()?;
                File::open(&self.objects_dir)?.sync_all()?;
            }
            Ok(())
        };
        if let Some(existing) = verify_mirror(&path, snapshot, size, oid)? {
            synchronize(&existing)?;
            return Ok(());
        }
        let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
        {
            let mut compressed = ZlibEncoder::new(temporary.as_file_mut(), self.loose_compression()?);
            io::copy(&mut raw_blob(snapshot, size)?, &mut compressed)?;
            compressed.finish()?.flush()?;
        }
        temporary.as_file().sync_all()?;
        match temporary.persist_noclobber(&path) {
            Ok(published) => synchronize(&published),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                let winner = verify_mirror(&path, snapshot, size, oid)?
                    .ok_or_else(|| Error::ObjectIdentityConflict(oid.to_hex()))?;
                synchronize(&winner)
            }
            Err(error) => Err(Error::Io(error.error)),
        }
    }
}

fn verify_mirror(path: &std::path::Path, snapshot: &mut File, size: u64, oid: &ObjectId) -> Result<Option<File>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::Io(error)),
        Ok(meta) if !meta.file_type().is_file() => return Err(Error::ObjectIdentityConflict(oid.to_hex())),
        Ok(_) => {}
    }
    let file = File::open(path)?;
    let mut compressed = ZlibDecoder::new(&file);
    let mut expected = raw_blob(snapshot, size)?;
    let mut actual_bytes = [0u8; BUFFER];
    let mut expected_bytes = [0u8; BUFFER];
    loop {
        let count = expected.read(&mut expected_bytes)?;
        if count == 0 {
            if compressed.read(&mut actual_bytes[..1])? != 0 {
                return Err(Error::ObjectIdentityConflict(oid.to_hex()));
            }
            break;
        }
        compressed.read_exact(&mut actual_bytes[..count])?;
        if actual_bytes[..count] != expected_bytes[..count] {
            return Err(Error::ObjectIdentityConflict(oid.to_hex()));
        }
    }
    drop(compressed);
    Ok(Some(file))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streaming_blob_matches_git_hash_and_rejects_a_corrupt_mirror() {
        for algo in [HashAlgo::Sha1, HashAlgo::Sha256] {
            let temp = tempfile::tempdir().unwrap();
            let git_dir = temp.path().join("repo.git");
            fs::create_dir_all(git_dir.join("objects")).unwrap();
            let format = if algo == HashAlgo::Sha256 { "[extensions]\nobjectformat=sha256\n" } else { "" };
            fs::write(git_dir.join("config"), format!("[gitr]\nredbObjects=true\n{format}")).unwrap();
            let odb = Odb::new(&git_dir.join("objects")).with_config_git_dir(git_dir);
            let bytes = vec![b'z'; 3 * super::super::redb_store::CHUNK_BYTES + 17];
            let oid = odb.write_blob_stream(&mut bytes.as_slice()).unwrap();
            assert_eq!(oid, Odb::hash_blob_stream(algo, &mut bytes.as_slice()).unwrap());
            assert_eq!(odb.read(&oid).unwrap().data, bytes);
            assert_eq!(odb.write_blob_stream(&mut bytes.as_slice()).unwrap(), oid);
            fs::write(odb.object_path(&oid), b"corrupt").unwrap();
            assert!(odb.write_blob_stream(&mut bytes.as_slice()).is_err());
        }
    }
}
