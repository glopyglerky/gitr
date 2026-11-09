// Gitr-specific object database integration tests. SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

fn repository(algo: HashAlgo, enabled: bool) -> (tempfile::TempDir, Odb) {
    let directory = tempfile::tempdir().unwrap();
    let git = directory.path().join(".git");
    let objects = git.join("objects");
    fs::create_dir_all(&objects).unwrap();
    fs::write(git.join("config"), format!(
        "[core]\nrepositoryformatversion=1\n[extensions]\nobjectformat={}\n[gitr]\nredbObjects={}\n",
        algo.name(), enabled
    )).unwrap();
    let odb = Odb::new(&objects).with_config_git_dir(git);
    (directory, odb)
}

#[test]
fn redb_odb_is_explicit_and_reading_missing_data_creates_nothing() {
    let (_directory, odb) = repository(HashAlgo::Sha1, false);
    assert!(odb.redb_object_path().unwrap().is_none());
    let id = odb.write(ObjectKind::Blob, b"standard").unwrap();
    assert_eq!(odb.read(&id).unwrap().data, b"standard");
    assert!(!odb.config_git_dir().unwrap().join("gitr").exists());

    let (_directory, odb) = repository(HashAlgo::Sha1, true);
    let path = odb.redb_object_path().unwrap().unwrap();
    assert!(matches!(odb.read(&ObjectId::zero()), Err(Error::ObjectNotFound(_))));
    assert!(!odb.exists(&ObjectId::zero()));
    assert!(!path.parent().unwrap().exists());
}

#[test]
fn redb_odb_writes_both_formats_and_restores_a_missing_mirror() {
    for algo in [HashAlgo::Sha1, HashAlgo::Sha256] {
        let (_directory, odb) = repository(algo, true);
        let id = odb.write(ObjectKind::Blob, b"durable source").unwrap();
        let path = odb.redb_object_path().unwrap().unwrap();
        let store = RedbObjectStore::open(path, algo).unwrap();
        assert_eq!(store.read(&id).unwrap().unwrap().data, b"durable source");
        assert_eq!(Odb::read_loose_verify_oid(&odb.object_path(&id), &id).unwrap().data, b"durable source");
        fs::remove_file(odb.object_path(&id)).unwrap();
        assert_eq!(odb.read(&id).unwrap().data, b"durable source");
        assert!(odb.exists(&id));
        assert!(odb.exists_local(&id));
        assert_eq!(odb.write(ObjectKind::Blob, b"durable source").unwrap(), id);
        assert_eq!(Odb::read_loose_verify_oid(&odb.object_path(&id), &id).unwrap().data, b"durable source");
    }
}

#[test]
fn redb_odb_every_write_entry_point_uses_the_store() {
    let (_directory, odb) = repository(HashAlgo::Sha1, true);
    let ids = [
        odb.write(ObjectKind::Blob, b"one").unwrap(),
        odb.write_local(ObjectKind::Blob, b"two").unwrap(),
        odb.write_loose_materialize(ObjectKind::Blob, b"three").unwrap(),
        odb.write_raw(b"blob 04\0four").unwrap(),
        odb.write_raw_local(b"blob 4\0five").unwrap(),
    ];
    let store = RedbObjectStore::open(odb.redb_object_path().unwrap().unwrap(), HashAlgo::Sha1).unwrap();
    for id in ids {
        assert!(store.contains(&id).unwrap());
        assert!(odb.object_path(&id).is_file());
    }
    assert_eq!(store.read_raw(&ids[3]).unwrap().unwrap(), b"blob 04\0four");
}

#[test]
fn redb_odb_memory_overlay_never_creates_durable_objects() {
    let (_directory, odb) = repository(HashAlgo::Sha1, true);
    odb.enable_mem_overlay();
    let ids = [
        odb.write(ObjectKind::Blob, b"one").unwrap(),
        odb.write_local(ObjectKind::Blob, b"two").unwrap(),
        odb.write_loose_materialize(ObjectKind::Blob, b"three").unwrap(),
        odb.write_raw(b"blob 4\0four").unwrap(),
        odb.write_raw_local(b"blob 4\0five").unwrap(),
    ];
    for id in ids {
        assert!(odb.read(&id).is_ok());
        assert!(!odb.object_path(&id).exists());
    }
    assert!(!odb.redb_object_path().unwrap().unwrap().exists());
    odb.disable_mem_overlay();
    for id in ids {
        assert!(matches!(odb.read(&id), Err(Error::ObjectNotFound(_))));
    }
}

#[test]
fn redb_odb_quarantine_does_not_publish_into_main_database() {
    let (_directory, main) = repository(HashAlgo::Sha1, true);
    let quarantine = main.objects_dir().join("incoming-example");
    fs::create_dir(&quarantine).unwrap();
    let isolated = Odb::new(&quarantine)
        .with_config_git_dir(main.config_git_dir().unwrap().to_path_buf());
    assert!(isolated.redb_object_path().unwrap().is_none());
    let oid = isolated.write_local(ObjectKind::Blob, b"unapproved").unwrap();
    assert_eq!(isolated.read(&oid).unwrap().data, b"unapproved");
    assert!(!main.exists(&oid));
    assert!(!main.redb_object_path().unwrap().unwrap().exists());
}

#[test]
fn redb_odb_existing_objects_and_alternates_are_read_without_migration() {
    let (_directory, main) = repository(HashAlgo::Sha1, true);
    let (alternate_directory, alternate) = repository(HashAlgo::Sha1, false);
    let oid = alternate.write(ObjectKind::Blob, b"alternate data").unwrap();
    fs::create_dir_all(main.objects_dir().join("info")).unwrap();
    fs::write(main.objects_dir().join("info/alternates"),
        format!("{}\n", alternate.objects_dir().display())).unwrap();
    assert_eq!(main.read(&oid).unwrap().data, b"alternate data");
    assert!(main.exists(&oid));
    assert!(!main.exists_local(&oid));
    assert!(!main.redb_object_path().unwrap().unwrap().exists());
    assert!(!alternate_directory.path().join(".git/gitr").exists());
    // A subsequent explicit write imports only the caller-supplied bytes into
    // the selected store. It does not mutate or migrate the alternate.
    main.write(ObjectKind::Blob, b"alternate data").unwrap();
    assert!(main.exists_local(&oid));
    assert!(!alternate_directory.path().join(".git/gitr").exists());
}

#[test]
fn redb_odb_rejects_a_conflicting_mirror_without_replacing_it() {
    let (_directory, odb) = repository(HashAlgo::Sha1, true);
    let oid = odb.hash(ObjectKind::Blob, b"wanted");
    let path = odb.object_path(&oid);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(b"blob 5\0other").unwrap();
    let conflicting = encoder.finish().unwrap();
    fs::write(&path, &conflicting).unwrap();
    assert!(matches!(odb.write(ObjectKind::Blob, b"wanted"), Err(Error::ObjectIdentityConflict(_))));
    assert_eq!(fs::read(&path).unwrap(), conflicting);
    assert!(!odb.redb_object_path().unwrap().unwrap().exists());
}

#[test]
fn redb_odb_invalid_configuration_cannot_silently_disable_storage() {
    let (_directory, odb) = repository(HashAlgo::Sha1, true);
    fs::write(odb.config_git_dir().unwrap().join("config"), "[gitr]\nredbObjects=perhaps\n").unwrap();
    assert!(matches!(odb.write(ObjectKind::Blob, b"x"), Err(Error::ConfigError(_))));
    assert!(odb.read(&ObjectId::zero()).is_err());
    assert!(!odb.config_git_dir().unwrap().join("gitr").exists());
}

#[test]
fn redb_odb_linked_worktrees_share_the_common_object_database() {
    let (_directory, main) = repository(HashAlgo::Sha1, true);
    let common = main.config_git_dir().unwrap();
    let linked = common.join("worktrees/linked");
    fs::create_dir_all(&linked).unwrap();
    fs::write(linked.join("commondir"), "../..\n").unwrap();
    let linked_odb = Odb::new(main.objects_dir()).with_config_git_dir(linked);
    assert_eq!(linked_odb.redb_object_path().unwrap(), main.redb_object_path().unwrap());
    let oid = linked_odb.write(ObjectKind::Blob, b"shared worktree").unwrap();
    assert_eq!(main.read(&oid).unwrap().data, b"shared worktree");
}

#[test]
fn redb_odb_simultaneous_mirror_publication_is_idempotent() {
    let (_directory, odb) = repository(HashAlgo::Sha1, true);
    // Initialize the database before racing independent mirror publications.
    odb.write(ObjectKind::Blob, b"initialize").unwrap();
    let handles: Vec<_> = (0..4).map(|_| {
        let odb = odb.clone();
        std::thread::spawn(move || odb.write(ObjectKind::Blob, b"same payload").unwrap())
    }).collect();
    let ids: Vec<_> = handles.into_iter().map(|handle| handle.join().unwrap()).collect();
    assert!(ids.iter().all(|id| *id == ids[0]));
    assert_eq!(Odb::read_loose_verify_oid(&odb.object_path(&ids[0]), &ids[0]).unwrap().data, b"same payload");
}

#[cfg(unix)]
#[test]
fn redb_odb_refuses_database_symlinks() {
    let (directory, odb) = repository(HashAlgo::Sha1, true);
    let target = directory.path().join("do-not-touch");
    fs::write(&target, b"untouched").unwrap();
    let path = odb.redb_object_path().unwrap().unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, path).unwrap();
    assert!(odb.write(ObjectKind::Blob, b"x").is_err());
    assert!(odb.read(&ObjectId::zero()).is_err());
    assert_eq!(fs::read(target).unwrap(), b"untouched");
}
