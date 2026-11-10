//! Regression tests for native CLI ref and signature safety boundaries.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::{fs, process::Command};

#[test]
fn tag_verification_never_accepts_armor_without_a_valid_signature() {
    let root = tempfile::tempdir().unwrap();
    let git_dir = root.path().join(".git");
    fs::create_dir_all(git_dir.join("objects")).unwrap();
    fs::create_dir_all(git_dir.join("refs/tags")).unwrap();
    fs::write(git_dir.join("HEAD"), b"ref: refs/heads/main\n").unwrap();
    fs::write(git_dir.join("config"), b"[core]\nrepositoryformatversion = 0\n").unwrap();
    let odb = grit_lib::odb::Odb::new(&git_dir.join("objects")).with_config_git_dir(git_dir.clone());
    let blob = odb.write(grit_lib::objects::ObjectKind::Blob, b"payload").unwrap();
    let raw = format!("object {blob}\ntype blob\ntag forged\ntagger Example <example@example.invalid> 1700000000 +0000\n\nmessage\n-----BEGIN PGP SIGNATURE-----\nnot-a-signature\n-----END PGP SIGNATURE-----\n");
    let tag = odb.write(grit_lib::objects::ObjectKind::Tag, raw.as_bytes()).unwrap();
    fs::write(git_dir.join("refs/tags/forged"), format!("{tag}\n")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_grit-git"))
        .args(["tag", "-v", "forged"])
        .current_dir(root.path())
        .env_clear()
        .env("PATH", root.path().join("no-programs"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", root.path().join("absent-global"))
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_CONFIG_COUNT")
        .output().unwrap();
    assert!(!output.status.success(), "unverified armor was reported valid");
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("gpg") || error.contains("signature"), "unexpected failure: {error}");
}
