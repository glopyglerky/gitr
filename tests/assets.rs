use gitr::assets::{attributes, scan};
use std::{fs, process::Command};

#[test]
fn bounded_scan_respects_gitignore_and_ranks_largest_files() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .arg(dir.path())
            .status()
            .unwrap()
            .success()
    );
    fs::write(dir.path().join(".gitignore"), "generated/\n").unwrap();
    fs::create_dir(dir.path().join("generated")).unwrap();
    fs::write(dir.path().join("generated/huge.bin"), vec![0; 4096]).unwrap();
    fs::write(dir.path().join("a.uasset"), vec![0; 200]).unwrap();
    fs::write(dir.path().join("b.fbx"), vec![0; 300]).unwrap();
    fs::write(dir.path().join("c.rs"), vec![0; 100]).unwrap();
    let report = scan(dir.path(), 50, 2).unwrap();
    assert_eq!(report.large_files, 3);
    assert_eq!(report.omitted_large_files, 1);
    assert_eq!(report.largest[0].path, "b.fbx");
    assert_eq!(report.largest[1].category, "unreal-binary");
    assert_eq!(report.files, 4);
}

#[test]
fn unity_profile_does_not_turn_text_assets_or_meta_into_lfs() {
    let rules = attributes("unity").unwrap();
    assert!(
        !rules
            .lines()
            .any(|line| line.starts_with("*.meta ") || line.starts_with("*.unity "))
    );
    assert!(
        attributes("unreal")
            .unwrap()
            .contains("*.uasset filter=lfs")
    );
}

#[cfg(unix)]
#[test]
fn symlinked_files_and_directories_are_not_scanned() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("asset"), vec![0; 1000]).unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("external")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("asset"), dir.path().join("link")).unwrap();
    assert_eq!(scan(dir.path(), 0, 10).unwrap().files, 0);
}
