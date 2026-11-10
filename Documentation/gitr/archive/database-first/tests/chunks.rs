use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::thread;

use gitr::chunks::ChunkStore;
use gitr::lfs::{self, Pointer, Store};
use serde_json::{Value, json};
use tempfile::TempDir;

fn fixture() -> (TempDir, ChunkStore, Store) {
    let temp = TempDir::new().unwrap();
    let chunks = ChunkStore::new(temp.path().join("chunks"));
    let destination = Store::new(temp.path().join("lfs"));
    (temp, chunks, destination)
}

fn manifest(chunks: &ChunkStore, pointer: &Pointer) -> Value {
    serde_json::from_slice(&fs::read(chunks.manifest_path(pointer).unwrap()).unwrap()).unwrap()
}

fn chunk_pointer(manifest: &Value, index: usize) -> Pointer {
    let entry = &manifest["chunks"][index];
    Pointer::new(
        entry["oid"].as_str().unwrap().into(),
        entry["size"].as_u64().unwrap(),
    )
    .unwrap()
}

fn overwrite_manifest(chunks: &ChunkStore, pointer: &Pointer, value: &Value) {
    let path = chunks.manifest_path(pointer).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if !root.exists() {
        return found;
    }
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            found.extend(files(&entry.path()));
        } else {
            found.push(entry.path());
        }
    }
    found
}

fn assert_no_temporary_files(root: &Path) {
    for path in files(root) {
        assert!(
            !path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("incoming-"),
            "{}",
            path.display()
        );
    }
}

fn random_bytes(length: usize) -> Vec<u8> {
    // Fixed data keeps boundary/reuse assertions reproducible on every host.
    let mut state = 0x1066_2026_1123_4455u64;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

#[test]
fn small_file_roundtrips_and_reimport_does_not_rewrite_verified_chunks() {
    let (temp, chunks, destination) = fixture();
    let data = b"small source asset";
    let first = chunks.import(data.as_slice()).unwrap();
    assert_eq!(first.pointer, lfs::hash_reader(data.as_slice()).unwrap());
    assert_eq!(first.chunks, 1);
    assert_eq!(first.reused_bytes, 0);
    assert_eq!(first.ingested_bytes, data.len() as u64);
    let saved = manifest(&chunks, &first.pointer);
    assert_eq!(saved["schema_version"], 1);
    assert_eq!(saved["algorithm"], "fastcdc-v2020");
    assert_eq!(saved["implementation"], "fastcdc-5.0.0");
    assert_eq!(
        saved["profile"],
        json!({
            "min_size": 65536, "average_size": 262144, "max_size": 1048576, "normalization": 1, "seed": 0
        })
    );
    let objects = Store::new(temp.path().join("chunks/objects"));
    let object_path = objects.path(&chunk_pointer(&saved, 0)).unwrap();
    let metadata_before = fs::metadata(&object_path).unwrap();
    let manifest_time = fs::metadata(chunks.manifest_path(&first.pointer).unwrap())
        .unwrap()
        .modified()
        .unwrap();
    let second = chunks.import(data.as_slice()).unwrap();
    assert_eq!(second.reused_bytes, data.len() as u64);
    assert_eq!(second.reused_chunks, 1);
    assert_eq!(second.ingested_bytes, 0);
    let metadata_after = fs::metadata(&object_path).unwrap();
    assert_eq!(
        metadata_before.modified().unwrap(),
        metadata_after.modified().unwrap()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(metadata_before.ino(), metadata_after.ino());
    }
    assert_eq!(
        manifest_time,
        fs::metadata(chunks.manifest_path(&first.pointer).unwrap())
            .unwrap()
            .modified()
            .unwrap()
    );
    chunks.hydrate(&first.pointer, &destination).unwrap();
    assert_eq!(
        fs::read(destination.path(&first.pointer).unwrap()).unwrap(),
        data
    );
    destination.verify(&first.pointer).unwrap();
    chunks.hydrate(&first.pointer, &destination).unwrap();
    assert_no_temporary_files(temp.path());
}

#[test]
fn empty_file_has_an_empty_manifest_and_hydrates_to_the_empty_lfs_object() {
    let (_temp, chunks, destination) = fixture();
    let report = chunks.import(io::empty()).unwrap();
    assert_eq!(report.pointer, lfs::hash_reader(io::empty()).unwrap());
    assert_eq!(report.chunks, 0);
    assert_eq!(report.ingested_bytes, 0);
    assert_eq!(report.reused_bytes, 0);
    assert!(
        manifest(&chunks, &report.pointer)["chunks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    chunks.hydrate(&report.pointer, &destination).unwrap();
    assert!(
        fs::read(destination.path(&report.pointer).unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn insertion_reuses_unchanged_content_and_hydrates_exact_original_bytes() {
    let (temp, chunks, destination) = fixture();
    let original = random_bytes(8 * 1024 * 1024);
    let first = chunks.import(original.as_slice()).unwrap();
    let mut edited = original.clone();
    edited.splice(
        1024 * 1024 + 12345..1024 * 1024 + 12345,
        b"inserted asset metadata\n".repeat(137),
    );
    let second = chunks.import(edited.as_slice()).unwrap();
    assert_ne!(first.pointer.oid, second.pointer.oid);
    assert_eq!(second.pointer, lfs::hash_reader(edited.as_slice()).unwrap());
    assert!(
        second.reused_bytes > (original.len() / 2) as u64,
        "only {} bytes were reused",
        second.reused_bytes
    );
    assert!(second.ingested_bytes > 0);
    assert_eq!(
        second.reused_bytes + second.ingested_bytes,
        edited.len() as u64
    );
    chunks.hydrate(&first.pointer, &destination).unwrap();
    chunks.hydrate(&second.pointer, &destination).unwrap();
    assert_eq!(
        fs::read(destination.path(&first.pointer).unwrap()).unwrap(),
        original
    );
    assert_eq!(
        fs::read(destination.path(&second.pointer).unwrap()).unwrap(),
        edited
    );
    assert_no_temporary_files(temp.path());
}

#[test]
fn read_fragmentation_and_interrupted_reads_do_not_change_chunk_boundaries() {
    struct Fragmented<'a> {
        bytes: &'a [u8],
        interrupt_once: bool,
    }
    impl Read for Fragmented<'_> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if self.interrupt_once {
                self.interrupt_once = false;
                return Err(io::ErrorKind::Interrupted.into());
            }
            let count = output.len().min(113).min(self.bytes.len());
            output[..count].copy_from_slice(&self.bytes[..count]);
            self.bytes = &self.bytes[count..];
            Ok(count)
        }
    }
    let (_temp, chunks, _destination) = fixture();
    let bytes = random_bytes(2 * 1024 * 1024 + 37);
    let first = chunks.import(bytes.as_slice()).unwrap();
    let second = chunks
        .import(Fragmented {
            bytes: &bytes,
            interrupt_once: true,
        })
        .unwrap();
    assert_eq!(first.pointer, second.pointer);
    assert_eq!(first.chunks, second.chunks);
    assert_eq!(second.reused_bytes, bytes.len() as u64);
    assert_eq!(second.ingested_bytes, 0);
}

#[test]
fn missing_corrupt_and_truncated_chunks_never_publish_a_whole_object() {
    for case in ["missing", "corrupt", "truncated"] {
        let (temp, chunks, destination) = fixture();
        let data = random_bytes(2 * 1024 * 1024);
        let report = chunks.import(data.as_slice()).unwrap();
        let saved = manifest(&chunks, &report.pointer);
        let objects = Store::new(temp.path().join("chunks/objects"));
        let first = chunk_pointer(&saved, 0);
        let path = objects.path(&first).unwrap();
        match case {
            "missing" => fs::remove_file(&path).unwrap(),
            "corrupt" => {
                let mut bytes = fs::read(&path).unwrap();
                bytes[0] ^= 1;
                fs::write(&path, bytes).unwrap();
            }
            "truncated" => File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(1)
                .unwrap(),
            _ => unreachable!(),
        }
        assert!(
            chunks.hydrate(&report.pointer, &destination).is_err(),
            "{case}"
        );
        assert!(!destination.path(&report.pointer).unwrap().exists());
        if case != "missing" {
            let bad_bytes = fs::read(&path).unwrap();
            assert!(chunks.import(data.as_slice()).is_err());
            assert_eq!(fs::read(&path).unwrap(), bad_bytes);
        }
        assert_no_temporary_files(temp.path());
    }
}

#[test]
fn manifests_reject_invalid_schema_algorithms_profiles_ids_sizes_and_offsets() {
    let (temp, chunks, destination) = fixture();
    let data = b"tiny asset";
    let report = chunks.import(data.as_slice()).unwrap();
    let good = manifest(&chunks, &report.pointer);
    let mut mutations = Vec::new();
    for (name, value) in [
        ("schema_version", json!(99)),
        ("algorithm", json!("unknown")),
        ("implementation", json!("fastcdc-unknown")),
        ("hash_algorithm", json!("sha1")),
    ] {
        let mut changed = good.clone();
        changed[name] = value;
        mutations.push(changed);
    }
    let mut profile = good.clone();
    profile["profile"]["seed"] = json!(1);
    mutations.push(profile);
    let mut traversal = good.clone();
    traversal["chunks"][0]["oid"] = json!("../../outside");
    mutations.push(traversal);
    let mut offset = good.clone();
    offset["chunks"][0]["offset"] = json!(1);
    mutations.push(offset);
    let mut overlength = good.clone();
    overlength["chunks"][0]["size"] = json!(1048577);
    mutations.push(overlength);
    let mut zero = good.clone();
    zero["chunks"][0]["size"] = json!(0);
    mutations.push(zero);
    let mut original_size = good.clone();
    original_size["pointer"]["size"] = json!(100);
    mutations.push(original_size);
    let mut missing = good.clone();
    missing["chunks"] = json!([]);
    mutations.push(missing);
    for mutation in mutations {
        overwrite_manifest(&chunks, &report.pointer, &mutation);
        assert!(chunks.hydrate(&report.pointer, &destination).is_err());
        assert!(!destination.path(&report.pointer).unwrap().exists());
    }
    overwrite_manifest(&chunks, &report.pointer, &good);
    chunks.hydrate(&report.pointer, &destination).unwrap();
    assert_no_temporary_files(temp.path());
}

#[test]
fn valid_individual_chunks_cannot_forge_a_different_whole_file_hash() {
    let (temp, chunks, destination) = fixture();
    let data = random_bytes(2 * 1024 * 1024);
    let report = chunks.import(data.as_slice()).unwrap();
    let mut forged = manifest(&chunks, &report.pointer);
    let different = Pointer::new("f".repeat(64), report.pointer.size).unwrap();
    assert_ne!(different.oid, report.pointer.oid);
    forged["pointer"] = serde_json::to_value(&different).unwrap();
    overwrite_manifest(&chunks, &different, &forged);
    let error = format!(
        "{:#}",
        chunks.hydrate(&different, &destination).unwrap_err()
    );
    assert!(error.contains("hash or size"), "{error}");
    assert!(!destination.path(&different).unwrap().exists());
    assert_no_temporary_files(temp.path());
}

#[test]
fn concurrent_identical_imports_publish_one_manifest_and_verified_objects() {
    let (temp, chunks, destination) = fixture();
    let chunks = Arc::new(chunks);
    let data = Arc::new(random_bytes(2 * 1024 * 1024 + 19));
    let barrier = Arc::new(Barrier::new(4));
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let chunks = chunks.clone();
            let data = data.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                chunks.import(data.as_slice()).unwrap()
            })
        })
        .collect();
    let reports: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert!(
        reports
            .iter()
            .all(|report| report.pointer == reports[0].pointer)
    );
    assert_eq!(files(&temp.path().join("chunks/manifests")).len(), 1);
    chunks.hydrate(&reports[0].pointer, &destination).unwrap();
    assert_eq!(
        fs::read(destination.path(&reports[0].pointer).unwrap()).unwrap(),
        *data
    );
    assert_no_temporary_files(temp.path());
}

#[test]
fn source_io_failure_leaves_no_manifest_or_temporary_file() {
    struct Failing<'a> {
        bytes: &'a [u8],
        remaining: usize,
    }
    impl Read for Failing<'_> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::other("injected source read failure"));
            }
            let count = self.remaining.min(output.len()).min(self.bytes.len());
            output[..count].copy_from_slice(&self.bytes[..count]);
            self.bytes = &self.bytes[count..];
            self.remaining -= count;
            Ok(count)
        }
    }
    let (temp, chunks, _destination) = fixture();
    let data = random_bytes(3 * 1024 * 1024);
    let error = chunks
        .import(Failing {
            bytes: &data,
            remaining: 1100000,
        })
        .unwrap_err();
    assert!(format!("{error:#}").contains("injected source read failure"));
    assert!(files(&temp.path().join("chunks/manifests")).is_empty());
    assert_no_temporary_files(temp.path());
}

#[test]
fn oversized_manifest_is_rejected_before_parsing_or_allocating_its_body() {
    let (_temp, chunks, destination) = fixture();
    let report = chunks.import(b"asset".as_slice()).unwrap();
    let path = chunks.manifest_path(&report.pointer).unwrap();
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_len(128 * 1024 * 1024 + 1)
        .unwrap();
    let error = chunks
        .hydrate(&report.pointer, &destination)
        .unwrap_err()
        .to_string();
    assert!(error.contains("JSON size limit"));
    assert!(!destination.path(&report.pointer).unwrap().exists());
}

#[test]
fn malformed_existing_manifest_is_not_overwritten_by_a_reimport() {
    let (_temp, chunks, _destination) = fixture();
    let data = b"asset";
    let report = chunks.import(data.as_slice()).unwrap();
    let path = chunks.manifest_path(&report.pointer).unwrap();
    fs::write(&path, b"broken manifest").unwrap();
    assert!(chunks.import(data.as_slice()).is_err());
    assert_eq!(fs::read(path).unwrap(), b"broken manifest");
    let traversal = Pointer {
        oid: "../outside".into(),
        size: 1,
    };
    assert!(chunks.manifest_path(&traversal).is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_manifests_chunks_and_cache_roots_are_rejected() {
    use std::os::unix::fs::symlink;
    let (temp, chunks, destination) = fixture();
    let data = b"asset";
    let report = chunks.import(data.as_slice()).unwrap();
    let saved = manifest(&chunks, &report.pointer);
    let path = chunks.manifest_path(&report.pointer).unwrap();
    let original = temp.path().join("original-manifest");
    fs::rename(&path, &original).unwrap();
    symlink(&original, &path).unwrap();
    assert!(chunks.hydrate(&report.pointer, &destination).is_err());
    assert!(chunks.import(data.as_slice()).is_err());
    fs::remove_file(&path).unwrap();
    fs::rename(original, &path).unwrap();
    let objects = Store::new(temp.path().join("chunks/objects"));
    let object_path = objects.path(&chunk_pointer(&saved, 0)).unwrap();
    let original = temp.path().join("original-chunk");
    fs::rename(&object_path, &original).unwrap();
    symlink(&original, &object_path).unwrap();
    assert!(chunks.hydrate(&report.pointer, &destination).is_err());
    assert!(chunks.import(data.as_slice()).is_err());
    let alias = temp.path().join("alias");
    symlink(temp.path().join("chunks"), &alias).unwrap();
    let alias = ChunkStore::new(alias);
    assert!(alias.import(data.as_slice()).is_err());
    assert!(alias.hydrate(&report.pointer, &destination).is_err());
    assert!(!destination.path(&report.pointer).unwrap().exists());
}
