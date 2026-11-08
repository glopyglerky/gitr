//! Local content-defined deduplication without changing Git LFS objects.
//!
//! FastCDC chooses boundaries; SHA-256 identifies and verifies the bytes. A
//! manifest is published only after every chunk is cached. Hydration verifies
//! each chunk and the whole LFS object before publishing the reconstructed file.
use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use fastcdc::v2020::{Normalization, StreamCDC};
use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::lfs::{
    self, Pointer, Store, ensure_directory, ensure_real_directory, sync_directory,
    sync_verified_file,
};

const MIN_SIZE: usize = 64 * 1024;
const AVERAGE_SIZE: usize = 256 * 1024;
const MAX_SIZE: usize = 1024 * 1024;
const MAX_CHUNKS: usize = 1_000_000;
const MAX_MANIFEST_BYTES: u64 = 128 * 1024 * 1024;
const ALGORITHM: &str = "fastcdc-v2020";
const IMPLEMENTATION: &str = "fastcdc-5.0.0";

#[derive(Clone, Debug)]
pub struct ChunkStore {
    root: PathBuf,
}

/// Reused bytes were found and verified before ingestion. Ingested bytes were
/// submitted to the immutable store; a concurrent import may win publication.
#[derive(Clone, Debug, Serialize)]
pub struct ImportReport {
    pub pointer: Pointer,
    pub chunks: usize,
    pub reused_chunks: usize,
    pub reused_bytes: u64,
    pub ingested_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Profile {
    min_size: usize,
    average_size: usize,
    max_size: usize,
    normalization: u8,
    seed: u64,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            min_size: MIN_SIZE,
            average_size: AVERAGE_SIZE,
            max_size: MAX_SIZE,
            normalization: 1,
            seed: 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Chunk {
    oid: String,
    size: u64,
    offset: u64,
}

impl Chunk {
    fn pointer(&self) -> Result<Pointer> {
        Pointer::new(self.oid.clone(), self.size)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    algorithm: String,
    implementation: String,
    hash_algorithm: String,
    profile: Profile,
    pointer: Pointer,
    #[serde(deserialize_with = "deserialize_chunks")]
    chunks: Vec<Chunk>,
}

impl Manifest {
    fn validate(&self, expected: &Pointer) -> Result<()> {
        expected.validate()?;
        self.pointer.validate()?;
        ensure!(
            &self.pointer == expected,
            "chunk manifest does not match the requested LFS pointer"
        );
        ensure!(
            self.schema_version == 1,
            "unsupported chunk manifest schema version"
        );
        ensure!(
            self.algorithm == ALGORITHM
                && self.implementation == IMPLEMENTATION
                && self.hash_algorithm == "sha256",
            "unsupported chunk manifest algorithm or implementation"
        );
        ensure!(
            self.profile == Profile::default(),
            "unsupported chunking profile"
        );
        ensure!(
            self.chunks.len() <= MAX_CHUNKS,
            "chunk manifest exceeds the chunk limit"
        );
        let mut offset = 0u64;
        for (index, chunk) in self.chunks.iter().enumerate() {
            chunk.pointer()?;
            ensure!(
                chunk.offset == offset,
                "chunk manifest has non-contiguous offsets"
            );
            ensure!(
                chunk.size > 0 && chunk.size <= MAX_SIZE as u64,
                "invalid chunk length"
            );
            if index + 1 < self.chunks.len() {
                ensure!(
                    chunk.size >= MIN_SIZE as u64,
                    "non-final chunk is shorter than the profile minimum"
                );
            }
            offset = offset
                .checked_add(chunk.size)
                .context("chunk manifest length overflow")?;
            ensure!(
                offset <= self.pointer.size,
                "chunks exceed the original LFS size"
            );
        }
        ensure!(
            offset == self.pointer.size,
            "chunk manifest length does not match the original LFS size"
        );
        if self.pointer.size == 0 {
            ensure!(
                self.pointer == lfs::hash_reader(io::empty())?,
                "invalid empty-object hash in chunk manifest"
            );
        }
        Ok(())
    }
}

impl ChunkStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn manifest_path(&self, pointer: &Pointer) -> Result<PathBuf> {
        pointer.validate()?;
        Ok(self
            .root
            .join("manifests")
            .join(&pointer.oid[..2])
            .join(format!("{}.json", pointer.oid)))
    }

    /// Chunk a stream using the fixed, versioned profile. The data buffer is at
    /// most 1 MiB plus one returned chunk. Metadata is capped at one million
    /// chunks and 128 MiB of JSON. Failed imports may retain verified chunks,
    /// which remain safe to reuse; they never publish an incomplete manifest.
    pub fn import(&self, reader: impl Read) -> Result<ImportReport> {
        ensure_directory(&self.root)?;
        let objects = Store::new(self.root.join("objects"));
        ensure_directory(objects.root())?;
        let mut whole_hash = Sha256::new();
        let mut size = 0u64;
        let mut chunks = Vec::new();
        let mut reused_chunks = 0;
        let mut reused_bytes = 0u64;
        let mut ingested_bytes = 0u64;
        let chunker = StreamCDC::with_level_and_seed(
            RetryInterrupted(reader),
            MIN_SIZE,
            AVERAGE_SIZE,
            MAX_SIZE,
            Normalization::Level1,
            0,
        );
        for result in chunker {
            let chunk = result.context("cannot read source for content-defined chunking")?;
            ensure!(
                chunks.len() < MAX_CHUNKS,
                "input exceeds the chunk-count limit"
            );
            ensure!(
                chunk.offset == size
                    && chunk.length == chunk.data.len()
                    && chunk.length > 0
                    && chunk.length <= MAX_SIZE,
                "chunker returned an invalid boundary"
            );
            let pointer = lfs::hash_reader(chunk.data.as_slice())?;
            match fs::symlink_metadata(objects.path(&pointer)?) {
                Ok(_) => {
                    objects
                        .verify_durable(&pointer)
                        .context("existing chunk is corrupt; it was not overwritten")?;
                    reused_chunks += 1;
                    reused_bytes = reused_bytes
                        .checked_add(pointer.size)
                        .context("reused chunk size overflow")?;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    objects.ingest(chunk.data.as_slice(), Some(&pointer))?;
                    ingested_bytes = ingested_bytes
                        .checked_add(pointer.size)
                        .context("ingested chunk size overflow")?;
                }
                Err(error) => return Err(error).context("cannot inspect existing chunk"),
            }
            whole_hash.update(&chunk.data);
            chunks
                .try_reserve(1)
                .context("cannot allocate bounded chunk metadata")?;
            chunks.push(Chunk {
                oid: pointer.oid,
                size: pointer.size,
                offset: size,
            });
            size = size
                .checked_add(pointer.size)
                .context("source size overflow")?;
        }
        let oid = whole_hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let pointer = Pointer::new(oid, size)?;
        let manifest = Manifest {
            schema_version: 1,
            algorithm: ALGORITHM.into(),
            implementation: IMPLEMENTATION.into(),
            hash_algorithm: "sha256".into(),
            profile: Profile::default(),
            pointer: pointer.clone(),
            chunks,
        };
        manifest.validate(&pointer)?;
        self.publish_manifest(&manifest)?;
        Ok(ImportReport {
            pointer,
            chunks: manifest.chunks.len(),
            reused_chunks,
            reused_bytes,
            ingested_bytes,
        })
    }

    /// Reconstruct the ordinary whole-file LFS object. Its SHA-256 and size
    /// must match before the destination store publishes anything. Chunks and
    /// manifests remain immutable and are never rewritten during hydration.
    pub fn hydrate(&self, pointer: &Pointer, destination: &Store) -> Result<()> {
        let manifest = self.load_manifest(pointer)?;
        match fs::symlink_metadata(destination.path(pointer)?) {
            Ok(_) => {
                return destination
                    .verify_durable(pointer)
                    .context("existing destination object is corrupt; it was not overwritten");
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("cannot inspect destination LFS object"),
        }
        let reader = ChunkReader {
            objects: Store::new(self.root.join("objects")),
            chunks: manifest.chunks.into_iter(),
            current: None,
            remaining: 0,
        };
        destination
            .ingest(reader, Some(pointer))
            .context("cannot hydrate the complete LFS object")?;
        Ok(())
    }

    fn load_manifest(&self, pointer: &Pointer) -> Result<Manifest> {
        let path = self.manifest_path(pointer)?;
        ensure_real_directory(&self.root)?;
        ensure_real_directory(&self.root.join("manifests"))?;
        ensure_real_directory(path.parent().context("chunk manifest has no parent")?)?;
        let metadata = fs::symlink_metadata(&path).context("chunk manifest is not cached")?;
        ensure!(
            metadata.file_type().is_file(),
            "chunk manifest must be a regular file, not a symlink"
        );
        ensure!(
            metadata.len() <= MAX_MANIFEST_BYTES,
            "chunk manifest exceeds the JSON size limit"
        );
        let mut reader = BufReader::new(File::open(path)?).take(MAX_MANIFEST_BYTES + 1);
        let manifest: Manifest =
            serde_json::from_reader(&mut reader).context("invalid chunk manifest JSON")?;
        ensure!(
            reader.limit() > 0,
            "chunk manifest exceeds the JSON size limit"
        );
        manifest.validate(pointer)?;
        Ok(manifest)
    }

    fn publish_manifest(&self, manifest: &Manifest) -> Result<()> {
        let path = self.manifest_path(&manifest.pointer)?;
        ensure_directory(&self.root.join("manifests"))?;
        let parent = path.parent().context("chunk manifest has no parent")?;
        ensure_directory(parent)?;
        let mut temp = tempfile::Builder::new()
            .prefix("incoming-")
            .tempfile_in(parent)?;
        {
            let limited = LimitedWriter {
                inner: &mut temp,
                remaining: MAX_MANIFEST_BYTES,
            };
            let mut writer = BufWriter::new(limited);
            serde_json::to_writer(&mut writer, manifest)
                .context("cannot write bounded chunk manifest")?;
            writer.flush()?;
        }
        temp.as_file().sync_all()?;
        match temp.persist_noclobber(&path) {
            Ok(_) => {}
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                let existing = self.load_manifest(&manifest.pointer)?;
                ensure!(
                    &existing == manifest,
                    "an incompatible manifest already exists; it was not overwritten"
                );
                sync_verified_file(&File::open(&path)?)?;
            }
            Err(error) => return Err(error.error).context("cannot publish chunk manifest"),
        }
        // Complete the winner's publication barrier as well when a concurrent
        // import installed this same manifest before reaching its own sync.
        sync_directory(parent)?;
        Ok(())
    }
}

struct ChunkReader {
    objects: Store,
    chunks: std::vec::IntoIter<Chunk>,
    current: Option<File>,
    remaining: u64,
}

impl Read for ChunkReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            if let Some(file) = self.current.as_mut() {
                if self.remaining == 0 {
                    self.current = None;
                    continue;
                }
                let limit = bytes.len().min(self.remaining as usize);
                let count = file.read(&mut bytes[..limit])?;
                if count == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "verified chunk was truncated during hydration",
                    ));
                }
                self.remaining -= count as u64;
                return Ok(count);
            }
            let Some(chunk) = self.chunks.next() else {
                return Ok(0);
            };
            let pointer = chunk.pointer().map_err(io::Error::other)?;
            self.current = Some(
                self.objects
                    .open_verified(&pointer)
                    .map_err(io::Error::other)?,
            );
            self.remaining = chunk.size;
        }
    }
}

fn deserialize_chunks<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Chunk>, D::Error> {
    struct BoundedChunks;
    impl<'de> Visitor<'de> for BoundedChunks {
        type Value = Vec<Chunk>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a bounded array of chunk references")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            let mut chunks = Vec::new();
            while let Some(chunk) = sequence.next_element()? {
                if chunks.len() == MAX_CHUNKS {
                    return Err(de::Error::custom(
                        "chunk manifest exceeds the chunk-count limit",
                    ));
                }
                chunks
                    .try_reserve(1)
                    .map_err(|_| de::Error::custom("cannot allocate bounded chunk metadata"))?;
                chunks.push(chunk);
            }
            Ok(chunks)
        }
    }
    deserializer.deserialize_seq(BoundedChunks)
}

struct LimitedWriter<W> {
    inner: W,
    remaining: u64,
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > self.remaining {
            return Err(io::Error::other(
                "chunk manifest exceeds the JSON size limit",
            ));
        }
        let count = self.inner.write(bytes)?;
        self.remaining -= count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

struct RetryInterrupted<R>(R);

impl<R: Read> Read for RetryInterrupted<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.0.read(bytes) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => return result,
            }
        }
    }
}
