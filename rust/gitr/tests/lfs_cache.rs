use gitr::lfs::{Pointer, Store, clean, hash_reader, smudge};
use std::fs;
use std::io::{self, Read};

#[test]
fn filter_roundtrip_binary_and_pointer_idempotence() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().join("objects"));
    let data: Vec<u8> = (0..600_000).map(|i| (i % 251) as u8).collect();
    let mut encoded = Vec::new();
    clean(&store, data.as_slice(), &mut encoded).unwrap();
    let pointer = Pointer::decode(&encoded).unwrap().unwrap();
    assert_eq!(pointer, hash_reader(data.as_slice()).unwrap());
    let mut twice = Vec::new();
    clean(&store, encoded.as_slice(), &mut twice).unwrap();
    assert_eq!(twice, encoded);
    let mut restored = Vec::new();
    smudge(&store, encoded.as_slice(), &mut restored, false).unwrap();
    assert_eq!(restored, data);
}

#[test]
fn known_sha256_and_empty_filter_match_lfs_spec() {
    let pointer = hash_reader("abc".as_bytes()).unwrap();
    assert_eq!(
        pointer.oid,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(pointer.size, 3);
    assert_eq!(
        String::from_utf8(pointer.encode().unwrap()).unwrap(),
        "version https://git-lfs.github.com/spec/v1\noid sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\nsize 3\n"
    );
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("objects"));
    let mut out = Vec::new();
    clean(&store, &b""[..], &mut out).unwrap();
    assert!(out.is_empty());
    assert!(!store.root().exists());
}

#[test]
fn corruption_and_missing_objects_fail_before_output() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("objects"));
    let pointer = store.ingest(&b"asset bytes"[..], None).unwrap();
    fs::write(store.path(&pointer).unwrap(), b"wrong bytes").unwrap();
    let mut out = Vec::new();
    assert!(
        smudge(
            &store,
            pointer.encode().unwrap().as_slice(),
            &mut out,
            false
        )
        .is_err()
    );
    assert!(out.is_empty());
    assert!(store.ingest(&b"asset bytes"[..], None).is_err());
    assert_eq!(
        fs::read(store.path(&pointer).unwrap()).unwrap(),
        b"wrong bytes"
    );
}

#[test]
fn partial_and_oversized_objects_are_not_published() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("objects"));
    let expected = hash_reader(&b"abcd"[..]).unwrap();
    for bytes in [b"abc".as_slice(), b"abcde".as_slice(), b"wxyz".as_slice()] {
        assert!(store.ingest(bytes, Some(&expected)).is_err());
        assert!(!store.path(&expected).unwrap().exists());
        assert_eq!(fs::read_dir(store.root()).unwrap().count(), 0);
    }
}

#[test]
fn failed_reader_leaves_no_partial_object() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("injected read failure"))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("objects"));
    assert!(store.ingest(Broken, None).is_err());
    assert_eq!(fs::read_dir(store.root()).unwrap().count(), 0);
}

#[test]
fn multiple_writers_publish_the_same_object_safely() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("objects"));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let store = store.clone();
            std::thread::spawn(move || store.ingest(&b"identical content"[..], None).unwrap())
        })
        .collect();
    let pointers: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert!(pointers.iter().all(|p| p == &pointers[0]));
    store.verify(&pointers[0]).unwrap();
    assert!(!fs::read_dir(store.root()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("incoming-")
    }));
}

#[test]
fn invalid_object_ids_and_unknown_extensions_fail_closed() {
    for oid in [
        "../file".to_owned(),
        "F".repeat(64),
        "0".repeat(63),
        "g".repeat(64),
    ] {
        assert!(Pointer::new(oid, 1).is_err());
    }
    let p = hash_reader(&b"abc"[..]).unwrap();
    let extended = format!(
        "version https://git-lfs.github.com/spec/v1\next-0-test sha256:{}\noid sha256:{}\nsize 3\n",
        p.oid, p.oid
    );
    assert!(Pointer::decode(extended.as_bytes()).is_err());
    let duplicate = format!(
        "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 3\nsize 4\n",
        p.oid
    );
    assert!(Pointer::decode(duplicate.as_bytes()).is_err());
}

#[test]
fn ordinary_small_and_large_blobs_pass_through_smudge() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("objects"));
    for data in [b"ordinary text".to_vec(), vec![0; 4096]] {
        let mut out = Vec::new();
        smudge(&store, data.as_slice(), &mut out, false).unwrap();
        assert_eq!(out, data);
    }
    let pointer = hash_reader(&b"not cached"[..]).unwrap().encode().unwrap();
    let mut out = Vec::new();
    smudge(&store, pointer.as_slice(), &mut out, true).unwrap();
    assert_eq!(out, pointer);
}

#[cfg(unix)]
#[test]
fn symlink_objects_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("objects"));
    let p = store.ingest(&b"data"[..], None).unwrap();
    fs::remove_file(store.path(&p).unwrap()).unwrap();
    let outside = dir.path().join("outside");
    fs::write(&outside, b"data").unwrap();
    std::os::unix::fs::symlink(&outside, store.path(&p).unwrap()).unwrap();
    assert!(store.verify(&p).is_err());
    assert!(store.ingest(&b"data"[..], None).is_err());
}
