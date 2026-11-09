//! Differential tests use C Git only as an independent fixture/oracle.
//! The production Git module must not invoke it.
use std::{
    ffi::OsStr,
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
use tempfile::TempDir;

fn command(program: impl AsRef<OsStr>, cwd: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(program);
    cmd.current_dir(cwd)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", cwd.join("absent-global-config"))
        .env("GIT_AUTHOR_DATE", "2026-01-02T03:04:05+0000")
        .env("GIT_COMMITTER_DATE", "2026-01-02T03:04:05+0000")
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn execute(mut cmd: Command, input: Option<&[u8]>) -> Output {
    let mut child = cmd.spawn().expect("start command");
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input)
            .expect("write input");
    }
    drop(child.stdin.take());
    child.wait_with_output().expect("command output")
}

fn success(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn git(cwd: &Path, args: &[&str]) -> Vec<u8> {
    success(execute(command("git", cwd, args), None))
}
fn gitr(cwd: &Path, args: &[&str]) -> Vec<u8> {
    success(execute(
        command(env!("CARGO_BIN_EXE_gitr"), cwd, args),
        None,
    ))
}
fn native_output(cwd: &Path, args: &[&str]) -> Output {
    execute(command(env!("CARGO_BIN_EXE_gitr"), cwd, args), None)
}
fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap().trim_end().to_owned()
}
fn fixture(format: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(
        dir.path(),
        &[
            "init",
            "-q",
            "-b",
            "main",
            &format!("--object-format={format}"),
        ],
    );
    git(dir.path(), &["config", "user.name", "Compatibility Test"]);
    git(
        dir.path(),
        &["config", "user.email", "compatibility@example.invalid"],
    );
    git(dir.path(), &["config", "core.autocrlf", "false"]);
    dir
}
fn commit(cwd: &Path, message: &str) -> String {
    git(cwd, &["add", "-A"]);
    git(cwd, &["commit", "-qm", message]);
    text(git(cwd, &["rev-parse", "HEAD"]))
}
fn index_blob(cwd: &Path, name: &[u8], contents: &[u8]) {
    let oid = text(success(execute(
        command("git", cwd, &["hash-object", "-w", "--stdin"]),
        Some(contents),
    )));
    let mut entry = format!("100644 {oid}\t").into_bytes();
    entry.extend_from_slice(name);
    entry.push(0);
    // Exercise Git path bytes without requiring the host filesystem to accept them.
    success(execute(
        command(
            "git",
            cwd,
            &[
                "-c",
                "core.protectNTFS=false",
                "update-index",
                "--add",
                "-z",
                "--index-info",
            ],
        ),
        Some(&entry),
    ));
}
fn commit_index(cwd: &Path, message: &str) {
    let tree = text(git(cwd, &["write-tree"]));
    let commit = text(git(cwd, &["commit-tree", &tree, "-m", message]));
    git(cwd, &["update-ref", "HEAD", &commit]);
}
fn compare(cwd: &Path, args: &[&str]) {
    assert_eq!(gitr(cwd, args), git(cwd, args), "arguments {args:?}");
}

#[test]
fn initializes_git_readable_sha1_sha256_and_bare_repositories() {
    for format in ["sha1", "sha256"] {
        for bare in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut args = vec!["init", "--object-format", format, "-b", "assets", "repo"];
            if bare {
                args.insert(1, "--bare");
            }
            gitr(dir.path(), &args);
            let repo = dir.path().join("repo");
            assert_eq!(
                text(git(&repo, &["symbolic-ref", "HEAD"])),
                "refs/heads/assets"
            );
            compare(&repo, &["rev-parse", "--is-bare-repository"]);
            compare(&repo, &["rev-parse", "--show-object-format"]);
            git(&repo, &["fsck", "--strict"]);
        }
    }
}

#[test]
fn invalid_initial_branch_does_not_create_repository() {
    let dir = tempfile::tempdir().unwrap();
    let result = native_output(dir.path(), &["init", "-b", "../bad", "repo"]);
    assert!(!result.status.success());
    assert!(!dir.path().join("repo").exists());
}

#[test]
fn hashes_raw_files_and_stdin_with_git_object_identity() {
    let bytes: Vec<u8> = (0..2_097_193).map(|n| (n % 251) as u8).collect();
    for format in ["sha1", "sha256"] {
        let dir = fixture(format);
        let cwd = dir.path();
        fs::write(cwd.join("asset.bin"), &bytes).unwrap();
        compare(cwd, &["hash-object", "--no-filters", "asset.bin"]);
        let id = text(gitr(
            cwd,
            &["hash-object", "--no-filters", "-w", "asset.bin"],
        ));
        assert_eq!(git(cwd, &["cat-file", "-p", &id]), bytes);
        let got = success(execute(
            command(
                env!("CARGO_BIN_EXE_gitr"),
                cwd,
                &["hash-object", "-w", "--stdin"],
            ),
            Some(&bytes),
        ));
        let expected = success(execute(
            command("git", cwd, &["hash-object", "--stdin"]),
            Some(&bytes),
        ));
        assert_eq!(got, expected);
        git(cwd, &["fsck", "--strict"]);
    }
}

#[test]
fn raw_hashing_outside_a_repository_uses_sha1() {
    let dir = tempfile::tempdir().unwrap();
    let args = ["hash-object", "--stdin"];
    let input = b"outside repository\0binary\n";
    let actual = success(execute(
        command(env!("CARGO_BIN_EXE_gitr"), dir.path(), &args),
        Some(input),
    ));
    let expected = success(execute(command("git", dir.path(), &args), Some(input)));
    assert_eq!(actual, expected);
}

#[test]
fn pathname_hashing_requires_explicit_no_filters() {
    let dir = fixture("sha1");
    fs::create_dir(dir.path().join("nested")).unwrap();
    fs::write(
        dir.path().join("nested/.gitattributes"),
        b"*.bin filter=evil\n",
    )
    .unwrap();
    fs::write(dir.path().join("nested/asset.bin"), b"bytes").unwrap();
    git(
        dir.path(),
        &["config", "filter.evil.clean", "touch executed"],
    );
    let output = native_output(dir.path(), &["hash-object", "nested/asset.bin"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--no-filters"));
    compare(
        dir.path(),
        &["hash-object", "--no-filters", "nested/asset.bin"],
    );
    assert!(!dir.path().join("executed").exists());
}

#[test]
fn reads_loose_and_packed_blob_tree_commit_and_tag_objects() {
    for format in ["sha1", "sha256"] {
        let dir = fixture(format);
        let cwd = dir.path();
        index_blob(cwd, b"nested/asset.bin", b"binary\0contents\xff\n");
        index_blob(cwd, b"quoted\tfile\n", b"tabbed\n");
        commit_index(cwd, "first subject\n\nbody");
        git(cwd, &["tag", "-a", "release", "-m", "annotated release"]);
        for packed in [false, true] {
            if packed {
                git(cwd, &["gc", "--prune=now"]);
            }
            for spec in [
                "HEAD",
                "HEAD^{tree}",
                "HEAD:nested",
                "HEAD:nested/asset.bin",
                "refs/tags/release",
            ] {
                for mode in ["-p", "-s", "-t"] {
                    compare(cwd, &["cat-file", mode, spec]);
                }
            }
            compare(cwd, &["show-ref"]);
            compare(cwd, &["show-ref", "--head"]);
            compare(cwd, &["show-ref", "--tags"]);
            compare(cwd, &["rev-parse", "HEAD^{tree}"]);
        }
    }
}

#[test]
fn object_metadata_reads_do_not_require_pretty_printing_large_blob() {
    let dir = fixture("sha1");
    let path = dir.path().join("large.bin");
    fs::File::create(&path)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let id = text(git(
        dir.path(),
        &["hash-object", "--no-filters", "-w", "large.bin"],
    ));
    compare(dir.path(), &["cat-file", "-s", &id]);
    compare(dir.path(), &["cat-file", "-t", &id]);
    let result = native_output(dir.path(), &["cat-file", "-p", &id]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("64 MiB"));
    assert!(result.stdout.is_empty());
}

#[test]
fn validates_structured_objects_before_writing() {
    let dir = fixture("sha256");
    let result = execute(
        command(
            env!("CARGO_BIN_EXE_gitr"),
            dir.path(),
            &["hash-object", "-w", "--stdin", "-t", "commit"],
        ),
        Some(b"not a commit"),
    );
    assert!(!result.status.success());
    let valid = b"tree 6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321\nauthor Test <test@example.invalid> 1 +0000\ncommitter Test <test@example.invalid> 1 +0000\n\nsubject\n";
    let actual = success(execute(
        command(
            env!("CARGO_BIN_EXE_gitr"),
            dir.path(),
            &["hash-object", "--stdin", "-t", "commit"],
        ),
        Some(valid),
    ));
    let expected = success(execute(
        command(
            "git",
            dir.path(),
            &["hash-object", "--stdin", "-t", "commit"],
        ),
        Some(valid),
    ));
    assert_eq!(actual, expected);
}

#[test]
fn discovers_nested_and_linked_worktrees_and_common_storage() {
    let dir = fixture("sha256");
    let cwd = dir.path();
    fs::write(cwd.join("file"), b"main\n").unwrap();
    commit(cwd, "worktree base");
    fs::create_dir_all(cwd.join("nested/inside")).unwrap();
    let nested = cwd.join("nested/inside");
    compare(&nested, &["rev-parse", "HEAD"]);
    compare(&nested, &["rev-parse", "--show-toplevel"]);
    compare(&nested, &["rev-parse", "--is-inside-work-tree"]);
    let linked_dir = tempfile::tempdir().unwrap();
    let linked = linked_dir.path().join("play");
    let mut add = command("git", cwd, &["worktree", "add", "-q", "-b", "play"]);
    add.arg(&linked);
    success(execute(add, None));
    compare(&linked, &["rev-parse", "HEAD"]);
    compare(&linked, &["rev-parse", "--show-toplevel"]);
    compare(&linked, &["ls-files", "-s", "-z"]);
    let common = text(gitr(&linked, &["rev-parse", "--git-common-dir"]));
    assert_eq!(
        fs::canonicalize(common).unwrap(),
        fs::canonicalize(cwd.join(".git")).unwrap()
    );
    fs::write(linked.join("file"), b"linked modification\n").unwrap();
    assert_eq!(gitr(&linked, &["status", "--porcelain"]), b" M file\n");
    assert!(gitr(cwd, &["status", "--porcelain"]).is_empty());
}

#[test]
fn log_and_index_listing_match_supported_git_modes() {
    for format in ["sha1", "sha256"] {
        let dir = fixture(format);
        fs::write(dir.path().join("first"), b"one\n").unwrap();
        commit(dir.path(), "subject one\ncontinued\n\nbody");
        fs::write(dir.path().join("second"), b"two\n").unwrap();
        commit(dir.path(), "subject two");
        compare(dir.path(), &["log", "--oneline", "-n", "2"]);
        compare(dir.path(), &["ls-files"]);
        compare(dir.path(), &["ls-files", "-s", "-z"]);
        assert_eq!(
            gitr(dir.path(), &["log", "-n", "2"]),
            git(dir.path(), &["log", "--format=%H %s", "-n", "2"])
        );
    }
}

#[test]
fn status_matches_staged_worktree_deleted_and_untracked_changes() {
    for format in ["sha1", "sha256"] {
        let dir = fixture(format);
        let cwd = dir.path();
        for name in ["both", "deleted", "staged", "working"] {
            fs::write(cwd.join(name), b"old\n").unwrap();
        }
        fs::write(cwd.join(".gitignore"), b"ignored/\n").unwrap();
        commit(cwd, "base");
        fs::write(cwd.join("both"), b"staged version\n").unwrap();
        fs::write(cwd.join("staged"), b"staged version\n").unwrap();
        fs::write(cwd.join("new"), b"new file\n").unwrap();
        git(cwd, &["add", "both", "staged", "new"]);
        fs::write(cwd.join("both"), b"working version\n").unwrap();
        fs::write(cwd.join("working"), b"working version\n").unwrap();
        fs::remove_file(cwd.join("deleted")).unwrap();
        fs::create_dir(cwd.join("untracked")).unwrap();
        fs::write(cwd.join("untracked/file"), b"untracked\n").unwrap();
        fs::create_dir(cwd.join("ignored")).unwrap();
        fs::write(cwd.join("ignored/cache"), b"ignored\n").unwrap();
        let index_before = fs::read(cwd.join(".git/index")).unwrap();
        let actual = gitr(cwd, &["status", "--porcelain", "--no-renames", "-z"]);
        assert_eq!(
            fs::read(cwd.join(".git/index")).unwrap(),
            index_before,
            "read-only status must not refresh the index"
        );
        let expected = git(
            cwd,
            &[
                "status",
                "--porcelain",
                "--no-renames",
                "--untracked-files=all",
                "-z",
            ],
        );
        assert_eq!(actual, expected);
    }
}

#[test]
fn status_and_ls_files_handle_unborn_and_missing_index() {
    let dir = fixture("sha1");
    fs::write(dir.path().join("file"), b"pending\n").unwrap();
    compare(dir.path(), &["ls-files", "-z"]);
    compare(dir.path(), &["status", "--porcelain", "--no-renames", "-z"]);
    git(dir.path(), &["add", "file"]);
    compare(dir.path(), &["status", "--porcelain", "--no-renames", "-z"]);
    commit(dir.path(), "first");
    fs::remove_file(dir.path().join(".git/index")).unwrap();
    compare(dir.path(), &["ls-files", "-z"]);
    compare(dir.path(), &["status", "--porcelain", "--no-renames", "-z"]);
}

#[test]
fn status_handles_unmerged_stages_without_executing_hooks() {
    let dir = fixture("sha1");
    let cwd = dir.path();
    fs::write(cwd.join("file"), b"base\n").unwrap();
    commit(cwd, "base");
    git(cwd, &["checkout", "-qb", "side"]);
    fs::write(cwd.join("file"), b"side\n").unwrap();
    commit(cwd, "side");
    git(cwd, &["checkout", "-q", "main"]);
    fs::write(cwd.join("file"), b"main\n").unwrap();
    commit(cwd, "main");
    assert!(
        !execute(command("git", cwd, &["merge", "side"]), None)
            .status
            .success()
    );
    compare(cwd, &["status", "--porcelain", "--no-renames", "-z"]);
    compare(cwd, &["ls-files", "-s", "-z"]);
}

#[test]
fn status_refuses_configured_filters_and_disables_fsmonitor() {
    let dir = fixture("sha1");
    let cwd = dir.path();
    fs::write(cwd.join("file"), b"base\n").unwrap();
    commit(cwd, "base");
    git(cwd, &["config", "core.fsmonitor", "touch executed"]);
    assert!(gitr(cwd, &["status", "--porcelain"]).is_empty());
    assert!(!cwd.join("executed").exists());
    git(cwd, &["config", "filter.evil.clean", "touch executed"]);
    fs::write(cwd.join(".gitattributes"), b"file filter=evil\n").unwrap();
    fs::write(cwd.join("file"), b"modified\n").unwrap();
    assert!(
        !native_output(cwd, &["status", "--porcelain"])
            .status
            .success()
    );
    assert!(!cwd.join("executed").exists());
}

#[test]
fn preserves_non_utf8_index_tree_and_status_path_bytes() {
    let dir = fixture("sha256");
    let name = b"asset-\xff\t.bin";
    index_blob(dir.path(), name, b"old\n");
    commit_index(dir.path(), "byte path");
    compare(dir.path(), &["ls-files"]);
    compare(dir.path(), &["ls-files", "-z"]);
    compare(dir.path(), &["ls-files", "-s", "-z"]);
    compare(dir.path(), &["cat-file", "-p", "HEAD^{tree}"]);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        if let Err(error) = fs::write(dir.path().join(OsStr::from_bytes(name)), b"new\n") {
            // APFS rejects invalid UTF-8 with EILSEQ (92), even on Unix.
            if error.kind() == std::io::ErrorKind::InvalidInput
                || (cfg!(target_os = "macos") && error.raw_os_error() == Some(92))
            {
                eprintln!("filesystem rejects this byte path: {error}");
                return;
            }
            panic!("create byte-path worktree fixture: {error}");
        }
        compare(dir.path(), &["status", "--porcelain", "--no-renames"]);
        compare(dir.path(), &["status", "--porcelain", "--no-renames", "-z"]);
    }
}

#[test]
fn guarded_updates_require_existing_valid_targets_and_expected_old_value() {
    for format in ["sha1", "sha256"] {
        let dir = fixture(format);
        let cwd = dir.path();
        fs::write(cwd.join("file"), b"base\n").unwrap();
        let base = commit(cwd, "base");
        let zero = "0".repeat(base.len());
        gitr(cwd, &["update-ref", "refs/heads/created", &base, &zero]);
        assert_eq!(text(git(cwd, &["rev-parse", "refs/heads/created"])), base);
        assert!(
            !native_output(cwd, &["update-ref", "refs/heads/created", &base, &zero])
                .status
                .success()
        );
        assert!(
            !native_output(
                cwd,
                &[
                    "update-ref",
                    "refs/heads/created",
                    &"1".repeat(base.len()),
                    &base
                ]
            )
            .status
            .success()
        );
        let blob = text(git(cwd, &["hash-object", "--no-filters", "-w", "file"]));
        assert!(
            !native_output(cwd, &["update-ref", "refs/heads/created", &blob, &base])
                .status
                .success()
        );
        assert!(
            !native_output(cwd, &["update-ref", "../escape", &base, &zero])
                .status
                .success()
        );
        git(cwd, &["fsck", "--strict"]);
    }
}

#[test]
fn competing_guarded_writers_have_exactly_one_winner() {
    for format in ["sha1", "sha256"] {
        let dir = fixture(format);
        let cwd = dir.path();
        fs::write(cwd.join("file"), b"base\n").unwrap();
        let base = commit(cwd, "base");
        let tree = text(git(cwd, &["rev-parse", "HEAD^{tree}"]));
        let first = text(git(
            cwd,
            &["commit-tree", &tree, "-p", &base, "-m", "writer one"],
        ));
        let second = text(git(
            cwd,
            &["commit-tree", &tree, "-p", &base, "-m", "writer two"],
        ));
        let one = command(
            env!("CARGO_BIN_EXE_gitr"),
            cwd,
            &["update-ref", "refs/heads/main", &first, &base],
        )
        .spawn()
        .unwrap();
        let two = command(
            env!("CARGO_BIN_EXE_gitr"),
            cwd,
            &["update-ref", "refs/heads/main", &second, &base],
        )
        .spawn()
        .unwrap();
        let one = one.wait_with_output().unwrap();
        let two = two.wait_with_output().unwrap();
        assert_eq!(
            usize::from(one.status.success()) + usize::from(two.status.success()),
            1,
            "one: {}\ntwo: {}",
            String::from_utf8_lossy(&one.stderr),
            String::from_utf8_lossy(&two.stderr)
        );
        let winner = text(git(cwd, &["rev-parse", "refs/heads/main"]));
        assert!(winner == first || winner == second);
        let reflog = text(git(
            cwd,
            &["reflog", "show", "--format=%gs", "refs/heads/main", "-1"],
        ));
        assert_eq!(reflog, "gitr: guarded reference update");
        git(cwd, &["fsck", "--strict"]);
    }
}

#[test]
fn competing_creators_of_same_target_have_exactly_one_winner() {
    let dir = fixture("sha256");
    let cwd = dir.path();
    fs::write(cwd.join("file"), b"base\n").unwrap();
    let base = commit(cwd, "base");
    let zero = "0".repeat(base.len());
    let args = ["update-ref", "refs/gitr/shared", &base, &zero];
    let first = command(env!("CARGO_BIN_EXE_gitr"), cwd, &args)
        .spawn()
        .unwrap();
    let second = command(env!("CARGO_BIN_EXE_gitr"), cwd, &args)
        .spawn()
        .unwrap();
    let first = first.wait_with_output().unwrap();
    let second = second.wait_with_output().unwrap();
    assert_eq!(
        usize::from(first.status.success()) + usize::from(second.status.success()),
        1
    );
    let reflog = fs::read(cwd.join(".git/logs/refs/gitr/shared")).unwrap();
    assert_eq!(reflog.iter().filter(|byte| **byte == b'\n').count(), 1);
    assert!(!native_output(cwd, &args).status.success());
    assert_eq!(
        fs::read(cwd.join(".git/logs/refs/gitr/shared")).unwrap(),
        reflog
    );
    git(cwd, &["fsck", "--strict"]);
}

#[test]
fn status_handles_intent_to_add_and_recreated_staged_deletion() {
    let dir = fixture("sha1");
    let cwd = dir.path();
    fs::write(cwd.join("deleted"), b"old\n").unwrap();
    commit(cwd, "base");
    git(cwd, &["rm", "deleted"]);
    fs::write(cwd.join("deleted"), b"recreated\n").unwrap();
    fs::write(cwd.join("intent"), b"intent\n").unwrap();
    git(cwd, &["add", "--intent-to-add", "intent"]);
    compare(cwd, &["status", "--porcelain", "--no-renames", "-z"]);
}

#[test]
fn repository_directory_queries_match_git() {
    for bare in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut args = vec!["init", "-q", "-b", "main"];
        if bare {
            args.push("--bare");
        }
        git(dir.path(), &args);
        compare(dir.path(), &["rev-parse", "--git-dir"]);
        compare(dir.path(), &["rev-parse", "--git-common-dir"]);
        if !bare {
            compare(&dir.path().join(".git"), &["rev-parse", "--git-dir"]);
            compare(
                &dir.path().join(".git"),
                &["rev-parse", "--is-inside-work-tree"],
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn status_distinguishes_symlink_type_changes_and_execute_bits() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = fixture("sha1");
    let cwd = dir.path();
    fs::write(cwd.join("mode"), b"executable mode fixture\n").unwrap();
    fs::write(cwd.join("type"), b"file\n").unwrap();
    commit(cwd, "base");
    fs::set_permissions(cwd.join("mode"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::remove_file(cwd.join("type")).unwrap();
    symlink("mode", cwd.join("type")).unwrap();
    compare(cwd, &["status", "--porcelain", "--no-renames", "-z"]);
    git(cwd, &["add", "mode", "type"]);
    compare(cwd, &["status", "--porcelain", "--no-renames", "-z"]);
    compare(cwd, &["cat-file", "-p", "HEAD^{tree}"]);
}

#[test]
fn ls_tree_matches_git_without_helpers_and_rejects_corruption() {
    for format in ["sha1", "sha256"] {
        let tmp = fixture(format);
        let cwd = tmp.path();
        fs::create_dir(cwd.join("nested")).unwrap();
        fs::write(cwd.join("nested/file"), b"source").unwrap();
        fs::write(cwd.join("spaces name"), b"binary\0content").unwrap();
        let head = commit(cwd, "tree listing");
        git(cwd, &["update-index", "--add", "--cacheinfo", &format!("160000,{head},gitlink")]);
        let blob = text(git(cwd, &["rev-parse", "HEAD:nested/file"]));
        git(cwd, &["update-index", "--add", "--cacheinfo", &format!("120000,{blob},link")]);
        git(cwd, &["update-index", "--add", "--cacheinfo", &format!("100644,{blob},tab\tname")]);
        git(cwd, &["commit", "-qm", "raw paths and modes"]);
        git(cwd, &["config", "filter.hostile.process", "must-never-execute"]);
        for flags in [vec![], vec!["-r"], vec!["-r", "-z"], vec!["--name-only"], vec!["-r", "--name-only", "-z"]] {
            let mut args = vec!["ls-tree"];
            args.extend(flags);
            args.push("HEAD");
            assert_eq!(gitr(cwd, &args), git(cwd, &args));
            let mut isolated = command(env!("CARGO_BIN_EXE_gitr"), cwd, &args);
            isolated.env("PATH", "");
            let output = execute(isolated, None);
            assert_eq!(success(output), git(cwd, &args));
        }
        assert!(!native_output(cwd, &["ls-tree", "HEAD:missing"]).status.success());
        let blob = text(git(cwd, &["rev-parse", "HEAD:nested/file"]));
        assert!(!native_output(cwd, &["ls-tree", &blob]).status.success());
        let tree = text(git(cwd, &["rev-parse", "HEAD^{tree}"]));
        let object = cwd.join(".git/objects").join(&tree[..2]).join(&tree[2..]);
        fs::remove_file(&object).unwrap();
        fs::write(object, b"corrupt compressed object").unwrap();
        assert!(!native_output(cwd, &["ls-tree", "-r", "HEAD"]).status.success());
    }
}
