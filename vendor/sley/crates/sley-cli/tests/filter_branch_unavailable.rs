//! An unavailable history rewriter must not execute filters or mutate history.

#[cfg(unix)]
#[test]
fn rejected_filter_branch_preserves_repository_and_executes_no_helpers() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    let root = std::env::temp_dir().join(format!(
        "gitr-filter-unavailable-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let git = |args: &[&str]| {
        let output = sley_testkit::hermetic_git_command_with_identity(sley_testkit::oracle_git())
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    };
    git(&["init", "-q", "--initial-branch=main"]);
    fs::write(root.join("tracked"), b"original\n").unwrap();
    git(&["add", "tracked"]);
    git(&["commit", "-qm", "original"]);
    let head = git(&["rev-parse", "HEAD"]);
    let index = fs::read(root.join(".git/index")).unwrap();
    let trap_dir = root.join("trap-bin");
    fs::create_dir(&trap_dir).unwrap();
    let marker = root.join("helper-executed");
    for name in ["sh", "git", "git-filter-branch"] {
        let helper = trap_dir.join(name);
        fs::write(
            &helper,
            b"#!/bin/sh\nprintf invoked > \"$FILTER_BRANCH_TRAP\"\nexit 97\n",
        )
        .unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
    }
    for args in [
        vec![
            "filter-branch",
            "--force",
            "--tree-filter",
            "echo changed > tracked",
            "--",
            "HEAD",
        ],
        vec!["filter-branch", "-h"],
    ] {
        let output = Command::new(sley_testkit::sley_bin!())
            .current_dir(&root)
            .args(&args)
            .env("PATH", &trap_dir)
            .env("FILTER_BRANCH_TRAP", &marker)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", root.join("absent-config"))
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(if args[1] == "-h" { 129 } else { 128 })
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("filter-branch is unavailable"));
        assert!(
            !marker.exists(),
            "a shell or Git helper ran for a refused command"
        );
        assert_eq!(git(&["rev-parse", "HEAD"]), head);
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(fs::read(root.join("tracked")).unwrap(), b"original\n");
        assert!(!root.join(".git/refs/original").exists());
    }
    fs::remove_dir_all(&root).unwrap();
}
