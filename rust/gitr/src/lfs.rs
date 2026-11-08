//! Standard Git LFS pointers and a bounded-memory, immutable object cache.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const VERSION: &str = "https://git-lfs.github.com/spec/v1";
pub const POINTER_LIMIT: usize = 1024;
const BUFFER_SIZE: usize = 128 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pointer {
    pub oid: String,
    pub size: u64,
}

impl Pointer {
    pub fn new(oid: String, size: u64) -> Result<Self> {
        ensure!(
            oid.len() == 64
                && oid
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "LFS object ID must contain exactly 64 lowercase hexadecimal characters"
        );
        ensure!(
            size <= i64::MAX as u64,
            "LFS size exceeds the supported 64-bit signed range"
        );
        Ok(Self { oid, size })
    }

    pub fn validate(&self) -> Result<()> {
        Self::new(self.oid.clone(), self.size).map(|_| ())
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        if self.size == 0 {
            ensure!(self.oid == empty_oid(), "invalid empty LFS object ID");
            return Ok(Vec::new());
        }
        Ok(format!(
            "version {VERSION}\noid sha256:{}\nsize {}\n",
            self.oid, self.size
        )
        .into_bytes())
    }

    /// Decode the supported v1 pointer. Unknown fields fail explicitly so that
    /// extensions are never silently discarded or incorrectly hydrated.
    pub fn decode(bytes: &[u8]) -> Result<Option<Self>> {
        if bytes.is_empty() {
            return Ok(Some(Self::new(empty_oid(), 0)?));
        }
        if bytes.len() >= POINTER_LIMIT {
            return Ok(None);
        }
        let Ok(text) = std::str::from_utf8(bytes) else {
            return Ok(None);
        };
        let mut lines = text.lines();
        let Some(first) = lines.next() else {
            return Ok(None);
        };
        if first != format!("version {VERSION}")
            && first != "version https://hawser.github.com/spec/v1"
        {
            return Ok(None);
        }
        let mut oid = None;
        let mut size = None;
        for line in lines {
            if let Some(value) = line.strip_prefix("oid sha256:") {
                ensure!(
                    oid.replace(value.to_owned()).is_none(),
                    "duplicate LFS object ID"
                );
            } else if let Some(value) = line.strip_prefix("size ") {
                ensure!(
                    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
                    "invalid LFS pointer size"
                );
                ensure!(
                    size.replace(value.parse::<u64>()?).is_none(),
                    "duplicate LFS size"
                );
            } else {
                bail!(
                    "unsupported LFS pointer field or extension; preserve this pointer with Git LFS"
                );
            }
        }
        Ok(Some(Self::new(
            oid.context("LFS pointer has no object ID")?,
            size.context("LFS pointer has no size")?,
        )?))
    }
}

fn empty_oid() -> String {
    hex(&Sha256::digest([]))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 15) as usize] as char);
    }
    output
}

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self, pointer: &Pointer) -> Result<PathBuf> {
        pointer.validate()?;
        Ok(self
            .root
            .join(&pointer.oid[0..2])
            .join(&pointer.oid[2..4])
            .join(&pointer.oid))
    }

    /// Publish only after hash, length, file flush and atomic no-clobber install.
    /// An existing object is verified, never replaced, including under races.
    pub fn ingest(&self, mut reader: impl Read, expected: Option<&Pointer>) -> Result<Pointer> {
        if let Some(expected) = expected {
            expected.validate()?;
        }
        ensure_directory(&self.root)?;
        let mut temp = tempfile::Builder::new()
            .prefix("incoming-")
            .tempfile_in(&self.root)?;
        let mut hash = Sha256::new();
        let mut size = 0u64;
        let mut buffer = vec![0u8; BUFFER_SIZE];
        loop {
            // Read at most one byte beyond the claimed length before rejecting.
            let read_limit = expected
                .map(|p| (p.size.saturating_sub(size) + 1).min(BUFFER_SIZE as u64) as usize)
                .unwrap_or(BUFFER_SIZE);
            let count = match reader.read(&mut buffer[..read_limit]) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if count == 0 {
                break;
            }
            size = size
                .checked_add(count as u64)
                .context("LFS size overflow")?;
            if let Some(expected) = expected {
                ensure!(
                    size <= expected.size,
                    "download exceeds declared LFS object size"
                );
            }
            ensure!(size <= i64::MAX as u64, "LFS object exceeds supported size");
            hash.update(&buffer[..count]);
            temp.write_all(&buffer[..count])?;
        }
        let pointer = Pointer::new(hex(&hash.finalize()), size)?;
        if let Some(expected) = expected {
            ensure!(
                &pointer == expected,
                "LFS object hash or size does not match its pointer"
            );
        }
        temp.as_file().sync_all()?;
        let path = self.path(&pointer)?;
        let first = self.root.join(&pointer.oid[0..2]);
        let second = first.join(&pointer.oid[2..4]);
        ensure_directory(&first)?;
        ensure_directory(&second)?;
        match temp.persist_noclobber(&path) {
            Ok(_) => sync_directory(&second)?,
            Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => {
                self.verify(&pointer)
                    .context("existing LFS cache object is corrupt; it was not overwritten")?;
            }
            Err(e) => return Err(e.error).context("cannot publish LFS object"),
        }
        Ok(pointer)
    }

    pub fn verify(&self, pointer: &Pointer) -> Result<()> {
        self.open_verified(pointer).map(|_| ())
    }

    /// Return the same verified file handle, positioned at zero. Gitr never
    /// modifies a published object. The cache must be owned by a trusted user.
    pub fn open_verified(&self, pointer: &Pointer) -> Result<File> {
        let path = self.path(pointer)?;
        ensure_real_directory(&self.root)?;
        ensure_real_directory(&self.root.join(&pointer.oid[0..2]))?;
        ensure_real_directory(&self.root.join(&pointer.oid[0..2]).join(&pointer.oid[2..4]))?;
        let meta = fs::symlink_metadata(&path)
            .context("LFS object is not cached; fetch it before smudging")?;
        ensure!(
            meta.file_type().is_file(),
            "LFS object must be a regular file, not a symlink"
        );
        ensure!(meta.len() == pointer.size, "LFS object length mismatch");
        let mut file = File::open(&path)?;
        let actual = hash_reader(&mut file)?;
        ensure!(&actual == pointer, "LFS object hash or size mismatch");
        file.seek(SeekFrom::Start(0))?;
        Ok(file)
    }
}

fn ensure_real_directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.file_type().is_dir(),
        "LFS cache directory must not be a symlink: {}",
        path.display()
    );
    Ok(())
}

fn ensure_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => ensure_real_directory(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path)?;
            ensure_real_directory(path)
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

pub fn hash_reader(mut reader: impl Read) -> Result<Pointer> {
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = vec![0; BUFFER_SIZE];
    loop {
        let count = match reader.read(&mut buffer) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .context("LFS object size overflow")?;
        hash.update(&buffer[..count]);
    }
    Pointer::new(hex(&hash.finalize()), size)
}

/// The filter passes existing valid pointers through exactly, including their
/// spelling. It refuses unknown extensions instead of losing their metadata.
pub fn clean(store: &Store, mut input: impl Read, mut output: impl Write) -> Result<()> {
    let mut prefix = Vec::with_capacity(POINTER_LIMIT);
    input
        .by_ref()
        .take(POINTER_LIMIT as u64)
        .read_to_end(&mut prefix)?;
    if Pointer::decode(&prefix)?.is_some() {
        output.write_all(&prefix)?;
    } else {
        let pointer = store.ingest(io::Cursor::new(prefix).chain(input), None)?;
        output.write_all(&pointer.encode()?)?;
    }
    Ok(())
}

/// Local-only filter. A missing or corrupt object is an error, never an empty
/// successful checkout. Fetch is a separate explicit network operation.
pub fn smudge(
    store: &Store,
    mut input: impl Read,
    mut output: impl Write,
    skip: bool,
) -> Result<()> {
    let mut prefix = Vec::with_capacity(POINTER_LIMIT);
    input
        .by_ref()
        .take(POINTER_LIMIT as u64)
        .read_to_end(&mut prefix)?;
    if skip || prefix.is_empty() {
        output.write_all(&prefix)?;
        io::copy(&mut input, &mut output)?;
    } else if let Some(pointer) = Pointer::decode(&prefix)? {
        let mut file = store.open_verified(&pointer)?;
        io::copy(&mut file, &mut output)?;
    } else {
        output.write_all(&prefix)?;
        io::copy(&mut input, &mut output)?;
    }
    Ok(())
}
