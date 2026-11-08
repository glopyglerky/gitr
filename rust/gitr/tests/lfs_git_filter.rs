//! Exercise the native filters inside reference Git's real index/checkout flow.
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn git(root: &Path, args: &[&str]) -> Output {
    let result = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_AUTHOR_NAME", "Gitr test")
        .env("GIT_AUTHOR_EMAIL", "gitr-test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Gitr test")
        .env("GIT_COMMITTER_EMAIL", "gitr-test@example.invalid")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    result
}

#[test]
fn reference_git_add_checkout_and_worktree_share_native_lfs_cache() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    // Git interprets filter commands with its POSIX shell, also on Windows.
    let binary = env!("CARGO_BIN_EXE_gitr").replace('\\', "/");
    let quoted = format!("'{}'", binary.replace('\'', "'\\''"));
    git(
        &repo,
        &["config", "filter.lfs.clean", &format!("{quoted} lfs clean")],
    );
    git(
        &repo,
        &[
            "config",
            "filter.lfs.smudge",
            &format!("{quoted} lfs smudge"),
        ],
    );
    git(&repo, &["config", "filter.lfs.required", "true"]);
    fs::write(
        repo.join(".gitattributes"),
        "*.uasset filter=lfs diff=lfs merge=lfs -text\n",
    )
    .unwrap();
    let asset = vec![42u8; 2 * 1024 * 1024 + 13];
    fs::write(repo.join("world.uasset"), &asset).unwrap();
    fs::write(repo.join("empty.uasset"), []).unwrap();
    git(
        &repo,
        &["add", ".gitattributes", "world.uasset", "empty.uasset"],
    );
    let blob = git(&repo, &["show", ":world.uasset"]).stdout;
    let pointer = gitr::lfs::Pointer::decode(&blob).unwrap().unwrap();
    assert_eq!(pointer, gitr::lfs::hash_reader(asset.as_slice()).unwrap());
    assert!(git(&repo, &["show", ":empty.uasset"]).stdout.is_empty());
    git(
        &repo,
        &["commit", "-qm", "binary asset via native LFS filter"],
    );
    fs::remove_file(repo.join("world.uasset")).unwrap();
    git(&repo, &["checkout-index", "--all", "--force"]);
    assert_eq!(fs::read(repo.join("world.uasset")).unwrap(), asset);
    let worktree = dir.path().join("second-worktree");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-qb",
            "second",
            worktree.to_str().unwrap(),
        ],
    );
    assert_eq!(fs::read(worktree.join("world.uasset")).unwrap(), asset);
    assert_eq!(git(&repo, &["status", "--porcelain"]).stdout, b"");
    git(&repo, &["fsck", "--strict"]);
}
