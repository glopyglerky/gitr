// SPDX-License-Identifier: MIT OR Apache-2.0
//! Product checks for repository-scoped omdb snapshot records. Every candidate
//! process runs with an empty PATH; the absolute reference Git builds fixtures.

use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use serde_json::{Value, json};

struct Fixture {
    temp: tempfile::TempDir,
    repo: PathBuf,
    empty_path: PathBuf,
    oracle: PathBuf,
    format: String,
}

impl Fixture {
    fn new(format: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let empty_path = temp.path().join("empty-path");
        fs::create_dir(&repo).unwrap();
        fs::create_dir(&empty_path).unwrap();
        let oracle = std::env::var_os("RIT_REFERENCE_GIT")
            .or_else(|| std::env::var_os("GITR_REFERENCE_GIT"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/usr/bin/git"));
        assert!(oracle.is_absolute() && oracle.is_file());
        let fixture = Self {
            temp,
            repo,
            empty_path,
            oracle,
            format: format.to_owned(),
        };
        fixture.git(&[
            "init",
            "--initial-branch=main",
            &format!("--object-format={format}"),
        ]);
        fixture
    }

    fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(program);
        command
            .current_dir(&self.repo)
            .env("PATH", &self.empty_path)
            .env("HOME", self.temp.path())
            .env("XDG_CONFIG_HOME", self.temp.path().join("xdg"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.temp.path().join("absent-config"))
            .env("GIT_CONFIG_SYSTEM", self.temp.path().join("absent-system"))
            .env("GIT_CONFIG_COUNT", "0")
            .env("GIT_AUTHOR_NAME", "Catalog Test")
            .env("GIT_AUTHOR_EMAIL", "catalog@example.invalid")
            .env("GIT_COMMITTER_NAME", "Catalog Test")
            .env("GIT_COMMITTER_EMAIL", "catalog@example.invalid")
            .env("GIT_AUTHOR_DATE", "2001-02-03T04:05:06 +0000")
            .env("GIT_COMMITTER_DATE", "2001-02-03T04:05:06 +0000")
            .env("GIT_TERMINAL_PROMPT", "0");
        for name in [
            "GIT_DIR",
            "GIT_COMMON_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG",
            "GIT_REPLACE_REF_BASE",
            "GIT_NO_REPLACE_OBJECTS",
            "GIT_REFERENCE_BACKEND",
        ] {
            command.env_remove(name);
        }
        command
    }

    fn git(&self, args: &[&str]) -> Vec<u8> {
        success(self.command(&self.oracle).args(args).output().unwrap())
    }

    fn git_text(&self, args: &[&str]) -> String {
        String::from_utf8(self.git(args)).unwrap().trim().to_owned()
    }

    fn commit(&self, bytes: &[u8], message: &str) -> String {
        fs::write(self.repo.join("value"), bytes).unwrap();
        self.git(&["add", "value"]);
        self.git(&["commit", "-m", message]);
        self.git_text(&["rev-parse", "HEAD"])
    }

    fn identity(&self, commit: &str) -> Value {
        json!({
            "object_format": self.format,
            "commit_oid": commit,
            "root_tree_oid": self.git_text(&["rev-parse", &format!("{commit}^{{tree}}")]),
        })
    }

    fn db_command(&self, repo: &Path, args: &[&str]) -> Command {
        let mut command = self.command(env!("CARGO_BIN_EXE_rit"));
        command.args(["--rit-db", args[0], "--repo"]).arg(repo);
        command.args(&args[1..]);
        command
    }

    fn db_at(&self, repo: &Path, args: &[&str]) -> Output {
        self.db_command(repo, args).output().unwrap()
    }

    fn db(&self, args: &[&str]) -> Output {
        self.db_at(&self.repo, args)
    }

    fn db_json(&self, args: &[&str]) -> Value {
        decode(self.db(args))
    }

    fn put(&self, name: &str, revision: &str, expected: u64, request: &str) -> Value {
        self.db_json(&[
            "put",
            "--name",
            name,
            "--revision",
            revision,
            "--expect-generation",
            &expected.to_string(),
            "--request-id",
            request,
        ])
    }

    fn delete(&self, name: &str, expected: u64, request: &str) -> Value {
        self.db_json(&[
            "delete",
            "--name",
            name,
            "--expect-generation",
            &expected.to_string(),
            "--request-id",
            request,
        ])
    }

    fn saved_at(&self, repo: &Path, action: &str, name: &str, path: Option<&str>) -> Output {
        let mut command = self.command(env!("CARGO_BIN_EXE_rit"));
        command
            .args(["--rit-snapshot", action, "--repo"])
            .arg(repo)
            .args(["--saved", name]);
        if let Some(path) = path {
            command.args(["--path", path]);
        }
        command.output().unwrap()
    }

    fn saved(&self, action: &str, name: &str, path: Option<&str>) -> Output {
        self.saved_at(&self.repo, action, name, path)
    }

    fn database_path(&self) -> PathBuf {
        self.repo.join(".git/rit/metadata.omdb")
    }

    fn object_path(&self, oid: &str) -> PathBuf {
        self.repo
            .join(".git/objects")
            .join(&oid[..2])
            .join(&oid[2..])
    }
}

fn success(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "status={}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn decode(output: Output) -> Value {
    serde_json::from_slice(&success(output)).unwrap()
}

fn rejected(output: Output) {
    assert_eq!(output.status.code(), Some(128), "{output:?}");
    assert!(output.stdout.is_empty(), "failure emitted data: {output:?}");
    assert!(!output.stderr.is_empty(), "missing diagnostic: {output:?}");
}

#[test]
fn explicit_initialization_and_saved_reads_preserve_immutable_git_identity() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        rejected(f.db(&["get", "--name", "build"]));
        rejected(f.db(&["list"]));
        rejected(f.saved("inspect", "build", None));
        rejected(f.db(&[
            "put",
            "--name",
            "build",
            "--expect-generation",
            "0",
            "--request-id",
            "before-init",
        ]));
        success(
            f.command(env!("CARGO_BIN_EXE_rit"))
                .args(["status", "--porcelain=v1"])
                .output()
                .unwrap(),
        );
        assert!(!f.repo.join(".git/rit").exists());

        // Initialization is independent of having a first commit.
        let initialized = f.db_json(&["init"]);
        assert_eq!(initialized["schema_version"], 1);
        assert_eq!(initialized["object_format"], format);
        assert!(f.database_path().is_file());
        assert_eq!(f.db_json(&["list"])["records"], json!([]));
        rejected(f.db(&["init"]));

        let original = f.commit(b"committed original\0bytes\n", "first");
        let saved = f.put("build", "HEAD", 0, "save-build");
        assert_eq!(saved["request_id"], "save-build");
        assert_eq!(saved["replayed"], false);
        assert_eq!(saved["record"]["generation"], 1);
        assert_eq!(saved["record"]["snapshot"], f.identity(&original));

        let latest = f.commit(b"new committed content\n", "second");
        fs::write(f.repo.join("value"), b"uncommitted worktree bytes\n").unwrap();
        let refs = f.git(&["for-each-ref"]);
        let index = fs::read(f.repo.join(".git/index")).unwrap();
        let db_bytes = fs::read(f.database_path()).unwrap();
        assert_eq!(f.db_json(&["get", "--name", "build"]), saved["record"]);
        assert_eq!(
            decode(f.saved("inspect", "build", None)),
            f.identity(&original)
        );
        assert_eq!(
            success(f.saved("read", "build", Some("value"))),
            b"committed original\0bytes\n"
        );
        let listing = decode(f.saved("list", "build", None));
        assert_eq!(listing["snapshot"], f.identity(&original));
        assert_eq!(listing["entries"].as_array().unwrap().len(), 1);
        let stat = decode(f.saved("stat", "build", Some("value")));
        assert_eq!(stat["snapshot"], f.identity(&original));
        assert_eq!(stat["entry"]["kind"], "blob");
        assert_eq!(fs::read(f.database_path()).unwrap(), db_bytes);
        assert_eq!(f.git_text(&["rev-parse", "HEAD"]), latest);
        assert_eq!(f.git(&["for-each-ref"]), refs);
        assert_eq!(fs::read(f.repo.join(".git/index")).unwrap(), index);
        assert_eq!(
            fs::read(f.repo.join("value")).unwrap(),
            b"uncommitted worktree bytes\n"
        );
    }
}

#[test]
fn receipts_replay_original_results_and_tombstones_prevent_stale_recreation() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        let first = f.commit(b"first\n", "first");
        let second = f.commit(b"second\n", "second");
        f.db_json(&["init"]);
        let created = f.put("build", &first, 0, "create");
        let updated = f.put("build", &second, 1, "update");
        assert_eq!(updated["record"]["generation"], 2);
        let replay = f.put("build", &first, 0, "create");
        assert_eq!(replay["replayed"], true);
        assert_eq!(replay["record"], created["record"]);
        assert_eq!(f.db_json(&["get", "--name", "build"]), updated["record"]);

        for args in [
            vec![
                "put",
                "--name",
                "build",
                "--revision",
                &second,
                "--expect-generation",
                "0",
                "--request-id",
                "create",
            ],
            vec![
                "put",
                "--name",
                "other",
                "--revision",
                &first,
                "--expect-generation",
                "0",
                "--request-id",
                "create",
            ],
            vec![
                "put",
                "--name",
                "build",
                "--revision",
                &first,
                "--expect-generation",
                "2",
                "--request-id",
                "create",
            ],
            vec![
                "delete",
                "--name",
                "build",
                "--expect-generation",
                "0",
                "--request-id",
                "create",
            ],
            vec![
                "delete",
                "--name",
                "build",
                "--expect-generation",
                "1",
                "--request-id",
                "stale-delete",
            ],
        ] {
            rejected(f.db(&args));
        }
        let deleted = f.delete("build", 2, "delete");
        assert_eq!(deleted["record"]["generation"], 3);
        assert!(deleted["record"]["snapshot"].is_null());
        assert_eq!(f.db_json(&["get", "--name", "build"]), deleted["record"]);
        rejected(f.saved("inspect", "build", None));
        assert_eq!(
            f.put("build", &second, 1, "update")["record"],
            updated["record"]
        );

        for expected in ["0", "1", "2"] {
            rejected(f.db(&[
                "put",
                "--name",
                "build",
                "--revision",
                &first,
                "--expect-generation",
                expected,
                "--request-id",
                "stale-recreate",
            ]));
        }
        let recreated = f.put("build", &first, 3, "recreate");
        assert_eq!(recreated["record"]["generation"], 4);
        let replay_delete = f.delete("build", 2, "delete");
        assert_eq!(replay_delete["replayed"], true);
        assert_eq!(replay_delete["record"], deleted["record"]);
        assert_eq!(f.db_json(&["get", "--name", "build"]), recreated["record"]);
        assert_eq!(
            f.put("other", &second, 0, "other")["record"]["generation"],
            5
        );
        assert_eq!(
            f.put("build", &second, 4, "last")["record"]["generation"],
            6
        );
    }
}

#[test]
fn moving_reference_retry_requires_the_original_immutable_target() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        let first = f.commit(b"first\n", "first");
        f.db_json(&["init"]);
        let original = f.put("build", "HEAD", 0, "resolve-once");
        f.commit(b"later\n", "later");
        rejected(f.db(&[
            "put",
            "--name",
            "build",
            "--revision",
            "HEAD",
            "--expect-generation",
            "0",
            "--request-id",
            "resolve-once",
        ]));
        let replay = f.put("build", &first, 0, "resolve-once");
        assert_eq!(replay["replayed"], true);
        assert_eq!(replay["record"], original["record"]);
        assert_eq!(f.db_json(&["get", "--name", "build"]), original["record"]);
    }
}

#[test]
fn linked_worktrees_share_catalog_while_bare_clones_have_separate_metadata() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        let main = f.commit(b"main source\n", "main");
        f.db_json(&["init"]);
        let saved = f.put("main", &main, 0, "main-save");
        let linked = f.temp.path().join("linked");
        success(
            f.command(&f.oracle)
                .args(["worktree", "add", "--detach"])
                .arg(&linked)
                .arg(&main)
                .output()
                .unwrap(),
        );
        assert_eq!(
            decode(f.db_at(&linked, &["get", "--name", "main"])),
            saved["record"]
        );
        rejected(f.db_at(&linked, &["init"]));
        fs::write(linked.join("value"), b"linked source\n").unwrap();
        success(
            f.command(&f.oracle)
                .current_dir(&linked)
                .args(["add", "value"])
                .output()
                .unwrap(),
        );
        success(
            f.command(&f.oracle)
                .current_dir(&linked)
                .args(["commit", "-m", "linked"])
                .output()
                .unwrap(),
        );
        let linked_saved = decode(f.db_at(
            &linked,
            &[
                "put",
                "--name",
                "linked",
                "--expect-generation",
                "0",
                "--request-id",
                "linked-save",
            ],
        ));
        assert_ne!(linked_saved["record"]["snapshot"]["commit_oid"], main);
        assert_eq!(linked_saved["record"]["generation"], 2);
        assert_eq!(
            f.db_json(&["get", "--name", "linked"]),
            linked_saved["record"]
        );
        assert_eq!(
            success(f.saved("read", "linked", Some("value"))),
            b"linked source\n"
        );
        assert_eq!(
            success(f.saved_at(&linked, "read", "main", Some("value"))),
            b"main source\n"
        );
        assert_eq!(f.git_text(&["rev-parse", "HEAD"]), main);
        assert!(!f.repo.join(".git/worktrees/linked/rit").exists());

        let bare = f.temp.path().join("bare.git");
        success(
            f.command(&f.oracle)
                .args(["clone", "--bare"])
                .arg(&f.repo)
                .arg(&bare)
                .output()
                .unwrap(),
        );
        assert!(!bare.join("rit").exists());
        rejected(f.db_at(&bare, &["get", "--name", "main"]));
        decode(f.db_at(&bare, &["init"]));
        assert!(bare.join("rit/metadata.omdb").is_file());
        let bare_saved = decode(f.db_at(
            &bare,
            &[
                "put",
                "--name",
                "bare",
                "--expect-generation",
                "0",
                "--request-id",
                "bare-save",
            ],
        ));
        assert_eq!(bare_saved["record"]["generation"], 1);
        assert_eq!(bare_saved["record"]["snapshot"], f.identity(&main));
        assert_eq!(
            success(f.saved_at(&bare, "read", "bare", Some("value"))),
            b"main source\n"
        );
        rejected(f.db(&["get", "--name", "bare"]));
        assert_eq!(f.db_json(&["list"])["records"].as_array().unwrap().len(), 2);
    }
}

#[test]
fn catalog_lists_are_bounded_and_input_errors_do_not_mutate_records() {
    let f = Fixture::new("sha256");
    let commit = f.commit(b"source\n", "source");
    f.db_json(&["init"]);
    for name in ["a", "b", "z"] {
        f.put(name, &commit, 0, &format!("create-{name}"));
    }
    f.delete("b", 2, "delete-b");
    let first = f.db_json(&["list", "--limit", "2"]);
    let rows = first["records"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["name"], "a");
    assert_eq!(rows[1]["name"], "b");
    assert!(rows[1]["snapshot"].is_null());
    assert_eq!(first["next_after"], "b");
    let rest = f.db_json(&["list", "--after", "b", "--limit", "2"]);
    assert_eq!(rest["records"].as_array().unwrap().len(), 1);
    assert_eq!(rest["records"][0]["name"], "z");
    assert!(rest["next_after"].is_null());
    assert_eq!(f.db_json(&["list", "--after", "z"])["records"], json!([]));

    let before = f.db_json(&["list"]);
    let too_long = "a".repeat(129);
    for name in [
        "",
        ".",
        "..",
        "../escape",
        "_prefix",
        "with/slash",
        "white space",
        "caf\u{e9}",
        &too_long,
    ] {
        rejected(f.db(&["get", "--name", name]));
        rejected(f.db(&[
            "put",
            "--name",
            name,
            "--revision",
            &commit,
            "--expect-generation",
            "0",
            "--request-id",
            "invalid-name",
        ]));
    }
    for request in ["", "../request", "bad request", &too_long] {
        rejected(f.db(&[
            "put",
            "--name",
            "new",
            "--revision",
            &commit,
            "--expect-generation",
            "0",
            "--request-id",
            request,
        ]));
    }
    for generation in ["", "-1", "+1", "1x", "18446744073709551616"] {
        rejected(f.db(&[
            "put",
            "--name",
            "new",
            "--revision",
            &commit,
            "--expect-generation",
            generation,
            "--request-id",
            "invalid-generation",
        ]));
    }
    for args in [
        vec!["put", "--name", "new", "--expect-generation", "0"],
        vec!["get", "--name", "a", "--name", "a"],
        vec!["get", "--name", "a", "--revision", "HEAD"],
        vec!["init", "--name", "a"],
        vec![
            "delete",
            "--name",
            "a",
            "--expect-generation",
            "1",
            "--request-id",
            "wrong-options",
            "--revision",
            "HEAD",
        ],
        vec!["list", "--limit", "0"],
        vec!["list", "--limit", "1001"],
        vec!["list", "--after", "a", "--after", "b"],
    ] {
        rejected(f.db(&args));
    }
    rejected(
        f.command(env!("CARGO_BIN_EXE_rit"))
            .args([
                "--rit-snapshot",
                "inspect",
                "--saved",
                "a",
                "--revision",
                "HEAD",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(f.db_json(&["list"]), before);
    let largest = "a".repeat(128);
    assert_eq!(
        f.put(&largest, &commit, 0, &largest)["record"]["generation"],
        5
    );
}

#[test]
fn preexisting_empty_foreign_and_nonregular_database_paths_are_not_adopted() {
    for bytes in [b"".as_slice(), b"unrelated application data\n"] {
        let f = Fixture::new("sha1");
        f.commit(b"source\n", "source");
        fs::create_dir(f.repo.join(".git/rit")).unwrap();
        fs::write(f.database_path(), bytes).unwrap();
        rejected(f.db(&["init"]));
        rejected(f.db(&["get", "--name", "build"]));
        rejected(f.db(&[
            "put",
            "--name",
            "build",
            "--expect-generation",
            "0",
            "--request-id",
            "attempt",
        ]));
        assert_eq!(fs::read(f.database_path()).unwrap(), bytes);
    }

    let f = Fixture::new("sha1");
    f.commit(b"source\n", "source");
    fs::create_dir(f.repo.join(".git/rit")).unwrap();
    {
        let db = omdb::Database::create(f.database_path()).unwrap();
        let write = db.begin_write().unwrap();
        {
            let mut table = write
                .open_table(omdb::TableDefinition::<&str, &str>::new("unrelated"))
                .unwrap();
            table.insert("sentinel", "keep").unwrap();
        }
        write.commit().unwrap();
    }
    let foreign = fs::read(f.database_path()).unwrap();
    rejected(f.db(&["init"]));
    rejected(f.db(&["get", "--name", "build"]));
    rejected(f.db(&[
        "put",
        "--name",
        "build",
        "--expect-generation",
        "0",
        "--request-id",
        "foreign-write",
    ]));
    assert_eq!(fs::read(f.database_path()).unwrap(), foreign);

    let f = Fixture::new("sha1");
    fs::create_dir_all(f.database_path()).unwrap();
    rejected(f.db(&["init"]));
    rejected(f.db(&["list"]));
    assert!(f.database_path().is_dir());
    let f = Fixture::new("sha1");
    fs::write(f.repo.join(".git/rit"), b"keep directory-name file").unwrap();
    rejected(f.db(&["init"]));
    rejected(f.db(&["list"]));
    assert_eq!(
        fs::read(f.repo.join(".git/rit")).unwrap(),
        b"keep directory-name file"
    );
}

#[cfg(unix)]
#[test]
fn symlink_catalog_paths_do_not_redirect_creation_or_reading() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new("sha1");
    let outside = f.temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, f.repo.join(".git/rit")).unwrap();
    rejected(f.db(&["init"]));
    rejected(f.db(&["list"]));
    assert!(!outside.join("metadata.omdb").exists());

    let f = Fixture::new("sha1");
    fs::create_dir(f.repo.join(".git/rit")).unwrap();
    let outside = f.temp.path().join("outside.omdb");
    fs::write(&outside, b"must remain unchanged").unwrap();
    symlink(&outside, f.database_path()).unwrap();
    rejected(f.db(&["init"]));
    rejected(f.db(&["list"]));
    assert_eq!(fs::read(outside).unwrap(), b"must remain unchanged");
}

#[test]
fn saved_records_fail_when_git_objects_are_missing_and_failed_put_has_no_receipt() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        let commit = f.commit(b"available once\n", "source");
        let blob = f.git_text(&["rev-parse", "HEAD:value"]);
        f.db_json(&["init"]);
        let saved = f.put("source", &commit, 0, "save-source");
        fs::remove_file(f.object_path(&blob)).unwrap();
        assert_eq!(
            decode(f.saved("inspect", "source", None)),
            saved["record"]["snapshot"]
        );
        rejected(f.saved("read", "source", Some("value")));
        assert_eq!(f.db_json(&["get", "--name", "source"]), saved["record"]);
        let commit_bytes = fs::read(f.object_path(&commit)).unwrap();
        fs::remove_file(f.object_path(&commit)).unwrap();
        rejected(f.saved("inspect", "source", None));
        let receipt = f.put("source", &commit, 0, "save-source");
        assert_eq!(receipt["replayed"], true);
        assert_eq!(receipt["record"], saved["record"]);
        rejected(f.db(&[
            "put",
            "--name",
            "later",
            "--revision",
            &commit,
            "--expect-generation",
            "0",
            "--request-id",
            "retry-after-restore",
        ]));
        rejected(f.db(&["get", "--name", "later"]));
        assert_eq!(f.db_json(&["get", "--name", "source"]), saved["record"]);
        fs::write(f.object_path(&commit), commit_bytes).unwrap();
        let restored = f.put("later", &commit, 0, "retry-after-restore");
        assert_eq!(restored["replayed"], false);
        assert_eq!(restored["record"]["generation"], 2);
    }
}

#[test]
fn separate_process_writers_cannot_both_publish_the_same_expected_generation() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::new(format);
        let first = f.commit(b"first\n", "first");
        let second = f.commit(b"second\n", "second");
        f.db_json(&["init"]);
        f.put("race", &first, 0, "initial");
        let mut children = Vec::new();
        for (target, request) in [(&first, "writer-one"), (&second, "writer-two")] {
            let child = f
                .db_command(
                    &f.repo,
                    &[
                        "put",
                        "--name",
                        "race",
                        "--revision",
                        target,
                        "--expect-generation",
                        "1",
                        "--request-id",
                        request,
                    ],
                )
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            children.push(child);
        }
        let outputs: Vec<_> = children
            .into_iter()
            .map(|child| child.wait_with_output().unwrap())
            .collect();
        assert_eq!(
            outputs
                .iter()
                .filter(|output| output.status.success())
                .count(),
            1,
            "{outputs:?}"
        );
        let winner: Value = serde_json::from_slice(
            &outputs
                .iter()
                .find(|output| output.status.success())
                .unwrap()
                .stdout,
        )
        .unwrap();
        let loser = outputs
            .iter()
            .find(|output| !output.status.success())
            .unwrap();
        assert!(matches!(loser.status.code(), Some(75 | 128)), "{loser:?}");
        assert!(loser.stdout.is_empty());
        assert_eq!(winner["record"]["generation"], 2);
        assert_eq!(f.db_json(&["get", "--name", "race"]), winner["record"]);
        rejected(f.db(&[
            "put",
            "--name",
            "race",
            "--revision",
            &first,
            "--expect-generation",
            "1",
            "--request-id",
            "stale-after-race",
        ]));
        assert_eq!(f.db_json(&["get", "--name", "race"]), winner["record"]);
    }
}

#[test]
fn catalog_extension_keeps_git_dispatch_and_rejects_ambient_storage_redirects() {
    let f = Fixture::new("sha1");
    f.commit(b"source\n", "source");
    f.git(&["config", "alias.db", "status --porcelain=v1"]);
    assert!(
        success(
            f.command(env!("CARGO_BIN_EXE_rit"))
                .arg("db")
                .output()
                .unwrap()
        )
        .is_empty()
    );
    let ordinary_git = f
        .command(env!("CARGO_BIN_EXE_git"))
        .args(["--rit-db", "init"])
        .output()
        .unwrap();
    assert!(!ordinary_git.status.success());
    for variable in [
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_REFERENCE_BACKEND",
    ] {
        rejected(
            f.db_command(&f.repo, &["init"])
                .env(variable, "outside")
                .output()
                .unwrap(),
        );
        assert!(!f.repo.join(".git/rit").exists());
    }
    let ignored = f.temp.path().join("absent-repository");
    decode(
        f.db_command(&f.repo, &["init"])
            .env("GIT_DIR", &ignored)
            .env("GIT_WORK_TREE", &ignored)
            .output()
            .unwrap(),
    );
    assert!(f.database_path().is_file());
    assert!(!ignored.exists());
}
