//! Retained isolated-gix authoring and redb action qualification. The default
//! Git CLI is tested independently in native_porcelain.rs. Reference Git is a
//! fixture/oracle only; authoring uses an empty PATH to catch C Git fallback.
use gitr_test_support::{guest, Guest};
use std::{
    ffi::OsStr,
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
use tempfile::TempDir;

fn command(program: impl AsRef<OsStr>, cwd: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(program.as_ref());
    if program.as_ref() == OsStr::new(env!("CARGO_BIN_EXE_gitr")) {
        cmd.arg("--isolated-git");
    }
    cmd.current_dir(cwd)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", cwd.join("absent-global-config"))
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_AUTHOR_NAME")
        .env_remove("GIT_AUTHOR_EMAIL")
        .env_remove("GIT_AUTHOR_DATE")
        .env_remove("GIT_COMMITTER_NAME")
        .env_remove("GIT_COMMITTER_EMAIL")
        .env_remove("GIT_COMMITTER_DATE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}
fn execute(mut command: Command, input: &[u8]) -> Output {
    let mut child = command.spawn().expect("start fixture command");
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}
fn success(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn git(cwd: &Path, args: &[&str], input: &[u8]) -> Vec<u8> {
    success(execute(command("git", cwd, args), input))
}
fn native(cwd: &Path, args: &[&str], input: &[u8]) -> Vec<u8> {
    success(execute(
        command(env!("CARGO_BIN_EXE_gitr"), cwd, args),
        input,
    ))
}
fn reject(cwd: &Path, args: &[&str], input: &[u8]) {
    let output = execute(command(env!("CARGO_BIN_EXE_gitr"), cwd, args), input);
    assert!(
        !output.status.success(),
        "unexpected native success for {args:?}"
    );
    assert!(
        output.stdout.is_empty(),
        "failure emitted an authoritative object ID"
    );
}
fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap().trim_end().to_owned()
}
fn fixture(format: &str) -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    native(
        tmp.path(),
        &["init", "-b", "main", "--object-format", format],
        b"",
    );
    git(tmp.path(), &["config", "user.name", "Native History"], b"");
    git(
        tmp.path(),
        &["config", "user.email", "history@example.invalid"],
        b"",
    );
    git(tmp.path(), &["config", "core.autocrlf", "false"], b"");
    tmp
}
fn blob(root: &Path, bytes: &[u8]) -> String {
    text(native(root, &["hash-object", "-w", "--stdin"], bytes))
}
fn empty_tree(root: &Path) -> String {
    text(native(root, &["mktree", "-z"], b""))
}

#[test]
fn empty_tree_matches_reference_for_sha1_and_sha256() {
    for format in ["sha1", "sha256"] {
        let tmp = fixture(format);
        assert_eq!(
            native(tmp.path(), &["mktree", "-z"], b""),
            git(tmp.path(), &["mktree", "-z"], b"")
        );
        let head = fs::read(tmp.path().join(".git/HEAD")).unwrap();
        assert_eq!(
            native(tmp.path(), &["write-tree"], b""),
            git(tmp.path(), &["mktree"], b"")
        );
        assert!(!tmp.path().join(".git/index").exists());
        assert_eq!(fs::read(tmp.path().join(".git/HEAD")).unwrap(), head);
    }
}

#[test]
fn tree_order_modes_and_non_utf8_names_match_reference() {
    for format in ["sha1", "sha256"] {
        let tmp = fixture(format);
        let b = blob(tmp.path(), b"hello\0world\xff");
        let t = empty_tree(tmp.path());
        let mut records = format!("040000 tree {t}\ta\0").into_bytes();
        for (mode, name) in [
            ("100644", b"a.c".as_slice()),
            ("100755", b"script"),
            ("120000", b"link"),
            ("100644", b"raw\xff"),
            ("100644", b"line\nname"),
            ("100644", b"tab\tname"),
        ] {
            records.extend_from_slice(format!("{mode} blob {b}\t").as_bytes());
            records.extend_from_slice(name);
            records.push(0);
        }
        assert_eq!(
            native(tmp.path(), &["mktree", "-z"], &records),
            git(tmp.path(), &["mktree", "-z"], &records)
        );
    }
}

#[test]
fn absent_submodule_commit_is_a_valid_gitlink() {
    for format in ["sha1", "sha256"] {
        let tmp = fixture(format);
        let missing = "1".repeat(if format == "sha1" { 40 } else { 64 });
        let records = format!("160000 commit {missing}\tmodule\n");
        assert_eq!(
            native(tmp.path(), &["mktree"], records.as_bytes()),
            git(tmp.path(), &["mktree"], records.as_bytes())
        );
    }
}

#[test]
fn invalid_mktree_records_fail_without_moving_refs() {
    let tmp = fixture("sha1");
    let b = blob(tmp.path(), b"file");
    let t = empty_tree(tmp.path());
    let before_head = fs::read(tmp.path().join(".git/HEAD")).unwrap();
    let missing = "1".repeat(40);
    let null = "0".repeat(40);
    let cases = [
        format!("100644 blob {b}\tx"),
        format!("100644 blob {b}\tx\n100644 blob {b}\tx\n"),
        format!("100644 blob {b}\t.git\n"),
        format!("100644 blob {b}\t../x\n"),
        format!("100644 blob {b}\t\"quoted\"\n"),
        format!("100644 tree {b}\tx\n"),
        format!("040000 tree {b}\tx\n"),
        format!("100644 blob {t}\tx\n"),
        format!("100644 blob {missing}\tx\n"),
        format!("100644 blob {null}\tx\n"),
        format!("100644 blob {}\tx\n", "1".repeat(64)),
        format!("100600 blob {b}\tx\n"),
        format!("800644 blob {b}\tx\n"),
        format!("100644 blob {b}\t\n"),
    ];
    for case in cases {
        reject(tmp.path(), &["mktree"], case.as_bytes());
        assert_eq!(fs::read(tmp.path().join(".git/HEAD")).unwrap(), before_head);
        assert!(!tmp.path().join(".git/refs/heads/main").exists());
    }
}

#[test]
fn write_tree_matches_staged_nested_entries_without_touching_index_or_worktree() {
    for format in ["sha1", "sha256"] {
        let tmp = fixture(format);
        fs::create_dir(tmp.path().join("a")).unwrap();
        fs::write(tmp.path().join("a/source.rs"), b"committed input\n").unwrap();
        fs::write(tmp.path().join("a.c"), b"same-prefix file\n").unwrap();
        git(tmp.path(), &["add", "--", "a", "a.c"], b"");
        let before = fs::read(tmp.path().join(".git/index")).unwrap();
        fs::write(tmp.path().join("a/source.rs"), b"dirty worktree bytes\n").unwrap();
        let tree = native(tmp.path(), &["write-tree"], b"");
        assert_eq!(fs::read(tmp.path().join(".git/index")).unwrap(), before);
        assert_eq!(tree, git(tmp.path(), &["write-tree"], b""));
        let object = format!("{}:a/source.rs", text(tree));
        assert_eq!(
            git(tmp.path(), &["cat-file", "blob", &object], b""),
            b"committed input\n"
        );
        assert_eq!(
            fs::read(tmp.path().join("a/source.rs")).unwrap(),
            b"dirty worktree bytes\n"
        );
    }
}

#[test]
fn write_tree_omits_intent_to_add_entries() {
    let tmp = fixture("sha1");
    fs::write(tmp.path().join("later.txt"), b"not staged").unwrap();
    git(tmp.path(), &["add", "-N", "later.txt"], b"");
    let before = fs::read(tmp.path().join(".git/index")).unwrap();
    let result = native(tmp.path(), &["write-tree"], b"");
    assert_eq!(fs::read(tmp.path().join(".git/index")).unwrap(), before);
    assert_eq!(result, git(tmp.path(), &["write-tree"], b""));
}

#[test]
fn write_tree_rejects_unmerged_stages_and_corrupt_index() {
    let tmp = fixture("sha1");
    let b = blob(tmp.path(), b"conflict");
    let input = format!("100644 {b} 1\tfile\n100644 {b} 2\tfile\n");
    git(
        tmp.path(),
        &["update-index", "--index-info"],
        input.as_bytes(),
    );
    let before = fs::read(tmp.path().join(".git/index")).unwrap();
    reject(tmp.path(), &["write-tree"], b"");
    assert_eq!(fs::read(tmp.path().join(".git/index")).unwrap(), before);
    let mut corrupt = before;
    corrupt[0] ^= 1;
    fs::write(tmp.path().join(".git/index"), &corrupt).unwrap();
    reject(tmp.path(), &["write-tree"], b"");
    assert_eq!(fs::read(tmp.path().join(".git/index")).unwrap(), corrupt);
}

fn reference_commit_with_same_identity(
    root: &Path,
    actual: &str,
    args: &[&str],
    input: &[u8],
) -> String {
    let raw = git(root, &["cat-file", "commit", actual], b"");
    let mut cmd = command("git", root, args);
    for (header, prefix) in [
        (b"author ".as_slice(), "GIT_AUTHOR"),
        (b"committer ".as_slice(), "GIT_COMMITTER"),
    ] {
        let line = raw
            .split(|b| *b == b'\n')
            .find_map(|s| s.strip_prefix(header))
            .unwrap();
        let open = line.iter().position(|b| *b == b'<').unwrap();
        let close = line.iter().position(|b| *b == b'>').unwrap();
        assert_eq!(&line[..open - 1], b"Native History");
        assert_eq!(&line[open + 1..close], b"history@example.invalid");
        cmd.env(format!("{prefix}_NAME"), "Native History")
            .env(format!("{prefix}_EMAIL"), "history@example.invalid")
            .env(
                format!("{prefix}_DATE"),
                std::str::from_utf8(&line[close + 2..]).unwrap(),
            );
    }
    text(success(execute(cmd, input)))
}

#[test]
fn commit_tree_messages_match_git_object_bytes_with_identical_signatures() {
    for format in ["sha1", "sha256"] {
        let tmp = fixture(format);
        let t = empty_tree(tmp.path());
        for parts in [
            vec![""],
            vec!["one", "two"],
            vec!["one\n\n"],
            vec!["a  \n\n#tag"],
            vec!["", "second"],
            vec!["one", ""],
        ] {
            let mut args = vec!["commit-tree", &t];
            for part in parts {
                args.extend(["-m", part]);
            }
            let actual = text(native(tmp.path(), &args, b""));
            assert_eq!(
                actual,
                reference_commit_with_same_identity(tmp.path(), &actual, &args, b"")
            );
        }
        for input in [b"raw no newline".as_slice(), b"raw\n\n", b""] {
            let args = ["commit-tree", &t];
            let actual = text(native(tmp.path(), &args, input));
            assert_eq!(
                actual,
                reference_commit_with_same_identity(tmp.path(), &actual, &args, input)
            );
        }
        fs::write(tmp.path().join("message"), b"from file, no newline").unwrap();
        let args = ["commit-tree", &t, "-F", "message"];
        let actual = text(native(tmp.path(), &args, b""));
        assert_eq!(
            actual,
            reference_commit_with_same_identity(tmp.path(), &actual, &args, b"")
        );
        assert!(!tmp.path().join(".git/refs/heads/main").exists());
    }
}

#[test]
fn commit_tree_preserves_parent_order_and_deduplicates_repeated_parents() {
    for format in ["sha1", "sha256"] {
        let tmp = fixture(format);
        let t = empty_tree(tmp.path());
        let first = text(native(tmp.path(), &["commit-tree", &t, "-m", "first"], b""));
        let second = text(native(
            tmp.path(),
            &["commit-tree", &t, "-m", "second"],
            b"",
        ));
        let args = [
            "commit-tree",
            &t,
            "-p",
            &second,
            "-p",
            &first,
            "-p",
            &second,
            "-m",
            "merge",
        ];
        let actual = text(native(tmp.path(), &args, b""));
        assert_eq!(
            actual,
            reference_commit_with_same_identity(tmp.path(), &actual, &args, b"")
        );
        let raw = git(tmp.path(), &["cat-file", "commit", &actual], b"");
        let parents: Vec<_> = raw
            .split(|b| *b == b'\n')
            .filter_map(|s| s.strip_prefix(b"parent "))
            .collect();
        assert_eq!(parents, vec![second.as_bytes(), first.as_bytes()]);
    }
}

#[test]
fn commit_tree_rejects_invalid_objects_encoding_and_identity() {
    let tmp = fixture("sha1");
    let b = blob(tmp.path(), b"not a tree");
    let t = empty_tree(tmp.path());
    reject(tmp.path(), &["commit-tree", &b, "-m", "no"], b"");
    reject(tmp.path(), &["commit-tree", &t, "-p", &b, "-m", "no"], b"");
    reject(tmp.path(), &["commit-tree", &t], b"bad\0message");
    reject(tmp.path(), &["commit-tree", &t], b"bad\xffmessage");
    git(
        tmp.path(),
        &["config", "i18n.commitEncoding", "ISO-8859-1"],
        b"",
    );
    reject(tmp.path(), &["commit-tree", &t, "-m", "no"], b"");
    git(
        tmp.path(),
        &["config", "--unset", "i18n.commitEncoding"],
        b"",
    );
    git(tmp.path(), &["config", "commit.gpgSign", "true"], b"");
    reject(
        tmp.path(),
        &["commit-tree", &t, "-m", "no unsigned fallback"],
        b"",
    );
    git(tmp.path(), &["config", "--unset", "commit.gpgSign"], b"");
    git(tmp.path(), &["config", "--unset", "user.email"], b"");
    reject(tmp.path(), &["commit-tree", &t, "-m", "no"], b"");
    assert!(!tmp.path().join(".git/refs/heads/main").exists());
}

#[test]
fn completely_native_checkout_free_authoring_and_guarded_publication() {
    for format in ["sha1", "sha256"] {
        let tmp = fixture(format);
        let no_programs = tmp.path().join("empty-path");
        fs::create_dir(&no_programs).unwrap();
        let run = |args: &[&str], input: &[u8]| {
            let mut cmd = command(env!("CARGO_BIN_EXE_gitr"), tmp.path(), args);
            cmd.env("PATH", &no_programs);
            success(execute(cmd, input))
        };
        let b = text(run(&["hash-object", "-w", "--stdin"], b"fn main() {}\n"));
        let record = format!("100644 blob {b}\tmain.rs\0");
        let t = text(run(&["mktree", "-z"], record.as_bytes()));
        let commit = text(run(&["commit-tree", &t, "-m", "native root"], b""));
        assert!(!tmp.path().join(".git/refs/heads/main").exists());
        let zeros = "0".repeat(if format == "sha1" { 40 } else { 64 });
        run(&["update-ref", "refs/heads/main", &commit, &zeros], b"");
        assert_eq!(text(run(&["rev-parse", "HEAD"], b"")), commit);
        assert_eq!(
            git(tmp.path(), &["show", "HEAD:main.rs"], b""),
            b"fn main() {}\n"
        );
        let next = text(run(&["commit-tree", &t, "-p", &commit, "-m", "next"], b""));
        run(&["update-ref", "refs/heads/main", &next, &commit], b"");
        reject(
            tmp.path(),
            &["update-ref", "refs/heads/main", &commit, &commit],
            b"",
        );
        assert_eq!(text(git(tmp.path(), &["rev-parse", "HEAD"], b"")), next);
        git(tmp.path(), &["fsck", "--strict"], b"");
        assert!(!tmp.path().join(".git/index").exists());
        assert!(!tmp.path().join("main.rs").exists());
    }
}

#[test]
fn native_authored_history_indexes_and_executes_from_redb_without_checkout_or_git_subprocess() {
    for format in ["sha1", "sha256"] {
        let tmp = fixture(format);
        let no_programs = tmp.path().join("empty-path");
        fs::create_dir(&no_programs).unwrap();
        let run = |args: &[&str], input: &[u8]| {
            let mut cmd = command(env!("CARGO_BIN_EXE_gitr"), tmp.path(), args);
            cmd.env("PATH", &no_programs);
            success(execute(cmd, input))
        };
        let source = b"fn main() {}\n";
        let b = text(run(&["hash-object", "-w", "--stdin"], source));
        let t = text(run(
            &["mktree", "-z"],
            format!("100644 blob {b}\tsource.rs\0").as_bytes(),
        ));
        let c = text(run(&["commit-tree", &t, "-m", "native source policy"], b""));
        let zeros = "0".repeat(if format == "sha1" { 40 } else { 64 });
        run(&["update-ref", "refs/heads/main", &c, &zeros], b"");
        run(&["db", "index", "HEAD"], b"");
        run(
            &[
                "db",
                "search",
                "main",
                "--revision",
                "HEAD",
                "--limit",
                "10",
            ],
            b"",
        );
        let module = guest(Guest::Echo);
        fs::write(tmp.path().join("echo.wasm"), module).unwrap();
        let installed: serde_json::Value =
            serde_json::from_slice(&run(&["actions", "install", "echo.wasm"], b"")).unwrap();
        let module = installed["module_sha256"].as_str().unwrap();
        let submitted: serde_json::Value = serde_json::from_slice(&run(
            &[
                "actions",
                "submit",
                "source-policy",
                module,
                "--revision",
                "HEAD",
                "--path",
                "source.rs",
            ],
            b"",
        ))
        .unwrap();
        assert_eq!(submitted["spec"]["commit_oid"], c);
        assert_eq!(submitted["spec"]["blob_oid"], b);
        let receipt: serde_json::Value =
            serde_json::from_slice(&run(&["actions", "run", "source-policy"], b"")).unwrap();
        assert_eq!(receipt["status"], "passed");
        assert_eq!(receipt["attempt"], 1);
        assert_eq!(run(&["actions", "artifact", "source-policy"], b""), source);
        assert!(!tmp.path().join("source.rs").exists());
        assert!(!tmp.path().join(".git/index").exists());
        assert!(tmp.path().join(".git/gitr/catalog.redb").is_file());
        assert!(tmp.path().join(".git/gitr/state.redb").is_file());
        git(tmp.path(), &["fsck", "--strict"], b"");
    }
}
