//! Hosted native qualification through the verified produced BlueFe/Lustr bundle.
//! This dependency-free coordinator may be compiled by stock Rust. It never
//! invokes that compiler or its Cargo to build Rit or an application dependency.
#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BLUEFE: &str = "7005dc02c9fb6674380eade443022bbc04215276";
const LUSTR: &str = "6339fbb104d6adda4be7e6be7dc637a6a9e5586b";
const ARCHIVE: &str = "rit-produced-bluefe.tar.gz";
const DIGEST: &str = "rit-produced-bluefe.tar.gz.sha256";
const ORACLE_GIT: &str = "e9019fcafe0040228b8631c30f97ae1adb61bcdc";

struct Context {
    workspace: PathBuf,
    bundle: PathBuf,
    download: PathBuf,
    logs: PathBuf,
    target: PathBuf,
}

fn path_env(key: &str) -> Result<PathBuf> {
    let path = PathBuf::from(env::var_os(key).ok_or_else(|| format!("missing {key}"))?);
    if !path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
        return Err(format!("{key} must be an absolute path without parent traversal").into());
    }
    Ok(path)
}

impl Context {
    fn load() -> Result<Self> {
        let context = Self {
            workspace: path_env("GITHUB_WORKSPACE")?,
            bundle: path_env("BLUEFE_ROOT")?,
            download: path_env("RIT_BLUEFE_DOWNLOAD")?,
            logs: path_env("RIT_NATIVE_LOGS")?,
            target: path_env("CARGO_TARGET_DIR")?,
        };
        for path in [&context.bundle, &context.download, &context.logs, &context.target] {
            if path == &context.workspace || !path.starts_with(&context.workspace) {
                return Err("qualification paths must be beneath the checked-out workspace".into());
            }
        }
        fs::create_dir_all(&context.logs)?;
        Ok(context)
    }
}

fn output(command: &mut Command) -> Result<String> {
    let result = command.output()?;
    if !result.status.success() {
        return Err(format!("command {:?} failed: {}", command.get_program(),
            String::from_utf8_lossy(&result.stderr)).into());
    }
    Ok(String::from_utf8(result.stdout)?)
}

fn checksum(path: &Path) -> Result<String> {
    let result = output(Command::new("sha256sum").arg("--binary").arg(path))?;
    let value = result.split_whitespace().next().ok_or("missing SHA-256 result")?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid SHA-256 result".into());
    }
    Ok(value.to_ascii_lowercase())
}

fn tee(mut reader: impl Read, log: Arc<Mutex<File>>, stderr: bool) -> io::Result<()> {
    let mut bytes = [0_u8; 16 * 1024];
    loop {
        let length = reader.read(&mut bytes)?;
        if length == 0 {
            return Ok(());
        }
        log.lock().map_err(|_| io::Error::other("log lock poisoned"))?
            .write_all(&bytes[..length])?;
        if stderr {
            let mut console = io::stderr().lock();
            console.write_all(&bytes[..length])?;
            console.flush()?;
        } else {
            let mut console = io::stdout().lock();
            console.write_all(&bytes[..length])?;
            console.flush()?;
        }
    }
}

fn logged(context: &Context, name: &str, command: &mut Command) -> Result<()> {
    let mut file = File::create(context.logs.join(format!("{name}.log")))?;
    writeln!(file, "program={:?}\narguments={:?}", command.get_program(),
        command.get_args().collect::<Vec<_>>())?;
    println!("Running {name}; full output is retained in {name}.log");
    let log = Arc::new(Mutex::new(file));
    let start = Instant::now();
    let mut child = command.stdin(Stdio::null())
        .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let stdout = child.stdout.take().ok_or("missing child stdout")?;
    let stderr = child.stderr.take().ok_or("missing child stderr")?;
    let out_log = Arc::clone(&log);
    let err_log = Arc::clone(&log);
    let out = thread::spawn(move || tee(stdout, out_log, false));
    let err = thread::spawn(move || tee(stderr, err_log, true));
    let status = child.wait()?;
    out.join().map_err(|_| "stdout logging thread panicked")??;
    err.join().map_err(|_| "stderr logging thread panicked")??;
    writeln!(log.lock().map_err(|_| "log lock poisoned")?,
        "\nexit={status}\nelapsed_seconds={:.3}", start.elapsed().as_secs_f64())?;
    if !status.success() {
        return Err(format!("{name} failed with {status}; see {name}.log").into());
    }
    Ok(())
}

fn archive_path(name: &str) -> Result<PathBuf> {
    if name.is_empty() || name.chars().any(char::is_control) || name.contains('\\') {
        return Err(format!("unsupported archive path: {name:?}").into());
    }
    let path = Path::new(name);
    if path.is_absolute() || path.components().any(|part| {
        matches!(part, Component::ParentDir | Component::RootDir | Component::Prefix(_))
    }) {
        return Err(format!("unsafe archive path: {name:?}").into());
    }
    Ok(path.components().filter(|part| *part != Component::CurDir).collect())
}

fn verbose_member(line: &str) -> Result<&str> {
    // GNU tar with numeric owners has five metadata fields: mode, uid/gid,
    // size, date, time. Remove exactly the final separating space so leading
    // spaces in the member name remain data rather than being trimmed away.
    let mut rest = line;
    for field in 0..5 {
        rest = rest.trim_start_matches([' ', '\t']);
        let end = rest.find([' ', '\t']).ok_or("invalid verbose archive record")?;
        rest = &rest[end..];
        if field == 4 {
            return rest.strip_prefix(' ').ok_or_else(|| "invalid archive name separator".into());
        }
    }
    Err("invalid verbose archive record".into())
}

fn validate_archive_listing(names: &str, details: &str) -> Result<()> {
    if names.chars().chain(details.chars()).any(|character| character.is_control() && character != '\n') {
        return Err("control characters are forbidden in archive listings".into());
    }
    let names: Vec<_> = names.lines().collect();
    let details: Vec<_> = details.lines().collect();
    if names.is_empty() || names.len() != details.len() {
        return Err("empty or inconsistent compiler archive listing".into());
    }
    let mut seen = BTreeSet::new();
    let mut regular_files = BTreeSet::new();
    for (name, detail) in names.into_iter().zip(details) {
        let normalized = archive_path(name)?;
        if !seen.insert(normalized.clone()) {
            return Err(format!("duplicate archive member: {name:?}").into());
        }
        let member = verbose_member(detail)?;
        match detail.as_bytes().first().copied() {
            Some(b'-' | b'd') => {
                if member != name {
                    return Err(format!("archive member order or name mismatch: {name:?}").into());
                }
                if detail.starts_with('-') {
                    if normalized.as_os_str().is_empty() {
                        return Err("archive root must be a directory".into());
                    }
                    regular_files.insert(normalized);
                }
            }
            Some(b'h') => {
                // Only a hardlink record interprets this suffix. The separate
                // name listing disambiguates names containing " link to ".
                let target = member.strip_prefix(name)
                    .and_then(|rest| rest.strip_prefix(" link to "))
                    .ok_or_else(|| format!("invalid hardlink record for {name:?}"))?;
                let target = archive_path(target)?;
                if normalized.as_os_str().is_empty() || !regular_files.contains(&target) {
                    return Err(format!("hardlink {name:?} must target an earlier regular file").into());
                }
                // A subsequent hardlink may safely target this validated one.
                regular_files.insert(normalized);
            }
            _ => return Err(format!("symlink or special archive member is forbidden: {name:?}").into()),
        }
    }
    Ok(())
}

fn checked_archive_members(archive: &Path, logs: &Path) -> Result<()> {
    let names = output(Command::new("tar")
        .env("LC_ALL", "C")
        .args(["--list", "--gzip", "--quoting-style=literal", "--file"]).arg(archive))?;
    fs::write(logs.join("archive-member-names.txt"), &names)?;
    let details = output(Command::new("tar")
        .env("LC_ALL", "C")
        .args(["--list", "--verbose", "--numeric-owner", "--gzip", "--quoting-style=literal", "--file"])
        .arg(archive))?;
    // Persist both listings before validation, including on rejected archives.
    fs::write(logs.join("archive-members.txt"), &details)?;
    validate_archive_listing(&names, &details)
}

fn prepare(context: &Context, verifier: &Path) -> Result<()> {
    let run = env::var("RIT_BLUEFE_RUN_ID")?.parse::<u64>()?;
    if run == 0 {
        return Err("pin the successful compiler artifact run before qualification".into());
    }
    if !verifier.is_absolute() || !verifier.is_file() {
        return Err("prepare requires the absolute compiled bluefe_bundle.rs helper path".into());
    }
    let archive = context.download.join(ARCHIVE);
    let sidecar = fs::read_to_string(context.download.join(DIGEST))?;
    let lines: Vec<_> = sidecar.lines().collect();
    if lines.len() != 1 {
        return Err("expected one archive digest record".into());
    }
    let (expected, name) = lines[0].split_once("  ").ok_or("invalid archive digest record")?;
    if name != ARCHIVE || expected.len() != 64
        || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("invalid compiler archive identity or digest".into());
    }
    let actual = checksum(&archive)?;
    if actual != expected.to_ascii_lowercase() {
        return Err("compiler archive SHA-256 mismatch".into());
    }
    checked_archive_members(&archive, &context.logs)?;
    fs::create_dir_all(context.bundle.parent().ok_or("bundle has no parent")?)?;
    // Never merge archive contents with an old or cached compiler directory.
    fs::create_dir(&context.bundle)?;
    logged(context, "bundle-extract", Command::new("tar")
        .args(["--extract", "--gzip", "--no-same-owner", "--no-same-permissions",
            "--no-overwrite-dir", "--file"])
        .arg(&archive).arg("--directory").arg(&context.bundle))?;
    logged(context, "bundle-verify", Command::new(verifier).arg("verify"))?;
    let proof = format!(
        "bluefe={BLUEFE}\nlustr={LUSTR}\nengine=rustc\nartifact_run={run}\narchive_sha256={actual}\nmanifest_sha256={}\nbundle={}\napplication_commit={}\nartifact_id={}\n",
        checksum(&context.bundle.join("SHA256SUMS"))?, context.bundle.display(),
        env::var("GITHUB_SHA")?, env::var("RIT_BLUEFE_ARTIFACT_ID")?,
    );
    fs::write(context.logs.join("bundle-verified.txt"), proof)?;
    fs::write(context.logs.join("reference-git-version.txt"),
        output(Command::new(path_env("RIT_REFERENCE_GIT")?).arg("--version"))?)?;
    for name in ["source-provenance.txt", "rustc-version.txt", "cargo-version.txt"] {
        fs::copy(context.bundle.join(name), context.logs.join(name))?;
    }
    Ok(())
}

fn require_verified_bundle(context: &Context) -> Result<()> {
    let proof = fs::read_to_string(context.logs.join("bundle-verified.txt"))?;
    for expected in [format!("bluefe={BLUEFE}"), format!("lustr={LUSTR}"),
        format!("bundle={}", context.bundle.display()),
        format!("application_commit={}", env::var("GITHUB_SHA")?),
        format!("manifest_sha256={}", checksum(&context.bundle.join("SHA256SUMS"))?)]
    {
        if !proof.lines().any(|line| line == expected) {
            return Err("compiler bundle has not passed preparation or changed afterward".into());
        }
    }
    Ok(())
}

fn build_oracle(context: &Context) -> Result<()> {
    // The C Git source is a pinned external test oracle. It is never linked
    // into Rit or copied into the committed product source.
    let source = context.workspace.join(".rit-ci/reference-git-source");
    let prefix = context.workspace.join(".rit-ci/reference-git");
    let executable = prefix.join("bin/git");
    if output(Command::new("git").arg("-C").arg(&source).args(["rev-parse", "HEAD"]))?.trim()
        != ORACLE_GIT
    {
        return Err("reference Git checkout is not the pinned v2.55.0 commit".into());
    }
    for name in ["RIT_REFERENCE_GIT", "SLEY_TEST_GIT"] {
        if path_env(name)? != executable {
            return Err(format!("{name} must select the installed pinned test oracle").into());
        }
    }
    let prefix_argument = format!("prefix={}", prefix.display());
    logged(context, "build-oracle-install", Command::new("make").arg("-C").arg(&source)
        // Git's Makefile links target/release/libgitcore.a relative to its
        // own source. Do not leak Rit's application target into this build.
        .env("CARGO_TARGET_DIR", source.join("target"))
        .env("RUSTUP_TOOLCHAIN", "1.99.1")
        .args(["-j2", "USE_LIBPCRE2=1", "NO_TCLTK=YesPlease", "NO_GETTEXT=YesPlease",
            "install", "t/helper/test-tool", "t/helper/test-fake-ssh"])
        .arg(prefix_argument))?;
    let version = output(Command::new(&executable).arg("--version"))?;
    if version.trim() != "git version 2.55.0" {
        return Err("installed test oracle is not Git 2.55.0".into());
    }
    fs::write(context.logs.join("reference-git-version.txt"), version)?;
    fs::write(context.logs.join("reference-git-provenance.txt"), format!(
        "repository=https://github.com/git/git\nrevision={ORACLE_GIT}\nversion=2.55.0\nbinary_sha256={}\nsource={}\nexecutable={}\n",
        checksum(&executable)?, source.display(), executable.display(),
    ))?;
    Ok(())
}

fn require_passed(context: &Context, phases: &[&str]) -> Result<()> {
    let records = fs::read_to_string(context.logs.join("phases.tsv"))?;
    for phase in phases {
        let status = records.lines().filter_map(|line| {
            let mut fields = line.split('\t');
            (fields.next() == Some(*phase)).then(|| fields.next()).flatten()
        }).next_back();
        if status != Some("PASS") {
            return Err(format!("{phase} must pass before release measurement").into());
        }
    }
    Ok(())
}

fn application(context: &Context, phase: &str) -> Result<()> {
    require_verified_bundle(context)?;
    if phase == "build-release" {
        require_passed(context, &["build-oracle", "test-root", "test-vendor", "test-vendor-cli", "verify-import", "policy", "audit"])?;
    }
    let root = context.bundle.to_str().ok_or("non-UTF8 compiler path")?;
    if root.chars().any(char::is_whitespace) {
        return Err("compiler path must not contain whitespace in RUSTDOCFLAGS".into());
    }
    let mut paths = vec![context.bundle.join("bin"), context.workspace.join(".rit-ci/reference-git/bin")];
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let mut command = Command::new(context.bundle.join("bin/lustr"));
    // A fresh fixture parent prevents donor PID/counter temp names from being
    // reused after an interrupted run or across separate test phases.
    let temporary = context.workspace.join(".rit-ci/native-temp").join(format!("{phase}-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()));
    fs::create_dir_all(&temporary)?;
    command.current_dir(&context.workspace)
        .args(["--engine", "rustc", "--bluefe-root"]).arg(&context.bundle)
        .env("PATH", env::join_paths(paths)?)
        .env("CARGO", context.bundle.join("bin/cargo"))
        .env("LUST_CARGO", context.bundle.join("bin/cargo"))
        .env("RUSTC", context.bundle.join("bin/rustc"))
        .env("RUSTDOC", context.bundle.join("bin/rustdoc"))
        .env("CARGO_ENCODED_RUSTFLAGS", format!("--sysroot\x1f{root}"))
        .env("RUSTDOCFLAGS", format!("--sysroot {root}"))
        .env("CARGO_TARGET_DIR", &context.target)
        .env("CARGO_BUILD_JOBS", "2")
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
        .env("CARGO_PROFILE_TEST_DEBUG", "0")
        .env("CARGO_INCREMENTAL", "0")
        .env("RUST_TEST_THREADS", "2")
        .env("CARGO_TERM_COLOR", "never")
        .env("RUST_BACKTRACE", "1");
    command.env("TMPDIR", temporary);
    for key in ["RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC_WRAPPER",
        "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER", "CARGO_ENCODED_RUSTDOCFLAGS",
        "RUSTFLAGS", "RUSTC_BOOTSTRAP", "LUST_CRANELIFT_BACKEND"]
    {
        command.env_remove(key);
    }
    match phase {
        "build-root" => {
            command.args(["build", "--manifest-path", "Cargo.toml", "--locked", "--all-features", "--all-targets"]);
        }
        "test-root" => {
            command.args(["test", "--manifest-path", "Cargo.toml", "--locked", "--all-features", "--all-targets", "--no-fail-fast"]);
        }
        "test-vendor" => {
            // Self-contained engine library tests. The complete upstream Git
            // suite needs its separately pinned, externally built oracle and
            // runner; passing this selection is not a claim of full parity.
            command.args(["test", "--manifest-path", "vendor/sley/Cargo.toml", "--locked",
                "--lib", "--no-fail-fast", "-p", "sley-core", "-p", "sley-object",
                "-p", "sley-index", "-p", "sley-refs", "-p", "sley-odb",
                "-p", "sley-pack", "-p", "sley-protocol", "-p", "sley-worktree",
                "-p", "sley-pathspec", "-p", "sley-i18n", "-p", "sley-archive",
                "-p", "sley-rev", "-p", "sley-remote", "-p", "sley-diff-merge"]);
        }
        "test-vendor-cli" => {
            require_passed(context, &["build-oracle"])?;
            command.args(["test", "--manifest-path", "vendor/sley/Cargo.toml", "--locked",
                "-p", "sley-cli", "--test", "archive", "--test", "archive_remote",
                "--test", "upload_archive", "--test", "filter_branch_unavailable",
                "--test", "rev_parse", "--test", "rev_parse_pseudo_names",
                "--test", "commit", "--test", "commit_large_blob",
                "--test", "rev_list", "--test", "rev_list_path_parents",
                "--test", "log", "--test", "history_sparse_tips",
                "--test", "history_ancestry_paths", "--test", "fast_export",
                "--no-fail-fast"])
                // New command fixtures can exercise the actual root executable
                // while retaining their standalone donor-workspace test route.
                .env("RIT_TEST_BIN", context.target.join("debug/rit"));
        }
        "verify-import" => {
            command.args(["run", "--example", "verify_import", "--locked"]);
        }
        "policy" | "audit" => {
            command.args(["run", "--manifest-path", "xtask/Cargo.toml", "--locked", "--", phase]);
        }
        "build-release" => {
            command.args(["build", "--manifest-path", "Cargo.toml", "--locked",
                "--all-features", "--release", "--bin", "rit"])
                .env("CARGO_PROFILE_RELEASE_OPT_LEVEL", "3")
                .env("CARGO_PROFILE_RELEASE_LTO", "false")
                .env("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "16")
                .env("CARGO_PROFILE_RELEASE_DEBUG", "0");
        }
        _ => return Err("unknown application qualification phase".into()),
    }
    logged(context, phase, &mut command)
}

fn benchmark(context: &Context) -> Result<()> {
    require_verified_bundle(context)?;
    require_passed(context, &["build-oracle", "test-root", "test-vendor", "test-vendor-cli", "verify-import", "policy", "audit", "build-release"])?;
    let helper = context.target.join("rit-benchmark-native");
    logged(context, "compile-benchmark-helper", Command::new(context.bundle.join("bin/rustc"))
        .current_dir(&context.workspace)
        .arg("--sysroot").arg(&context.bundle)
        .args(["--edition=2024", "-C", "opt-level=3", "-C", "debuginfo=0",
            "ci/benchmark_native.rs", "-o"])
        .arg(&helper))?;
    let executable = context.target.join("release/rit");
    let reference = path_env("RIT_REFERENCE_GIT")?;
    if !reference.is_file() {
        return Err("missing absolute reference Git executable".into());
    }
    fs::write(context.logs.join("benchmark-build.txt"), format!(
        "engine=rustc\nbluefe={BLUEFE}\nlustr={LUSTR}\nprofile=release\nopt-level=3\nlto=false\ncodegen-units=16\ndebug=0\nfeatures=all\nrepository-default-release-lto=thin\nrit-sha256={}\nreference-git-sha256={}\n",
        checksum(&executable)?, checksum(&reference)?,
    ))?;
    logged(context, "benchmark-native", Command::new(helper)
        .arg("--rit").arg(executable)
        .arg("--git").arg(reference)
        .arg("--compiler-root").arg(&context.bundle)
        .arg("--fixture").arg(context.workspace.join(".rit-ci/native-benchmark-fixture"))
        .arg("--output").arg(context.logs.join("benchmarks"))
        .env("CARGO_PROFILE_RELEASE_OPT_LEVEL", "3")
        .env("CARGO_PROFILE_RELEASE_LTO", "false")
        .env("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "16")
        .env("CARGO_PROFILE_RELEASE_DEBUG", "0"))
}

fn run() -> Result<()> {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    let phase = arguments.first().and_then(|value| value.to_str()).ok_or(
        "usage: qualify-native prepare /absolute/bluefe-helper | build-oracle | build-root | test-root | test-vendor | test-vendor-cli | verify-import | policy | audit | build-release | benchmark")?;
    let context = Context::load()?;
    let start = Instant::now();
    let result = match phase {
        "prepare" if arguments.len() == 2 => prepare(&context, Path::new(&arguments[1])),
        "build-oracle" if arguments.len() == 1 => build_oracle(&context),
        "build-root" | "test-root" | "test-vendor" | "test-vendor-cli" | "verify-import" | "policy" | "audit"
            | "build-release" if arguments.len() == 1 => application(&context, phase),
        "benchmark" if arguments.len() == 1 => benchmark(&context),
        _ => Err("invalid qualification phase or argument count".into()),
    };
    let outcome = if result.is_ok() { "PASS" } else { "FAIL" };
    let record = format!("{phase}\t{outcome}\t{:.3}s\n", start.elapsed().as_secs_f64());
    OpenOptions::new().create(true).append(true).open(context.logs.join("phases.tsv"))?
        .write_all(record.as_bytes())?;
    if let Err(error) = &result {
        OpenOptions::new().create(true).append(true).open(context.logs.join("failures.log"))?
            .write_all(format!("{phase}: {error}\n").as_bytes())?;
    }
    if let Some(path) = env::var_os("GITHUB_STEP_SUMMARY") {
        OpenOptions::new().create(true).append(true).open(path)?
            .write_all(format!("- `{phase}`: **{outcome}**, {:.1} seconds.\n", start.elapsed().as_secs_f64()).as_bytes())?;
    }
    result
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listings(entries: &[(char, &str, Option<&str>)]) -> (String, String) {
        let mut names = String::new();
        let mut details = String::new();
        for (kind, name, target) in entries {
            names.push_str(name);
            names.push('\n');
            details.push_str(&format!("{kind}rwxr-xr-x 1001/1001 0 2026-11-09 08:00 {name}"));
            if let Some(target) = target {
                details.push_str(if *kind == 'h' { " link to " } else { " -> " });
                details.push_str(target);
            }
            details.push('\n');
        }
        (names, details)
    }

    #[test]
    fn allows_internal_hardlinks_and_validated_hardlink_chains() {
        let (names, details) = listings(&[
            ('d', "./", None),
            ('d', "./lib/", None),
            ('-', "./lib/has link to marker.so", None),
            ('h', "./lib/first alias.so", Some("./lib/has link to marker.so")),
            ('h', "./lib/second.so", Some("lib/first alias.so")),
        ]);
        assert!(validate_archive_listing(&names, &details).is_ok());
    }

    #[test]
    fn rejects_hardlinks_with_escaping_or_control_character_targets() {
        for target in ["/outside", "../outside", "lib/../../outside", "./lib/../original",
            "C:\\outside", "./lib/original\t", "./lib/original\r", "./lib/original\n"]
        {
            let (names, details) = listings(&[
                ('-', "./lib/original", None),
                ('h', "./lib/alias", Some(target)),
            ]);
            assert!(validate_archive_listing(&names, &details).is_err(), "accepted {target:?}");
        }
    }

    #[test]
    fn rejects_missing_future_and_self_hardlink_targets() {
        for entries in [
            vec![('h', "./alias", Some("./missing"))],
            vec![('h', "./alias", Some("./future")), ('-', "./future", None)],
            vec![('h', "./alias", Some("./alias"))],
        ] {
            let (names, details) = listings(&entries);
            assert!(validate_archive_listing(&names, &details).is_err());
        }
    }

    #[test]
    fn rejects_symlinks_and_special_files() {
        for kind in ['l', 'b', 'c', 'p', 's'] {
            let (names, details) = listings(&[
                ('-', "./original", None),
                (kind, "./alias", Some("./original")),
            ]);
            assert!(validate_archive_listing(&names, &details).is_err());
        }
    }

    #[test]
    fn rejects_hardlink_targets_that_are_directories() {
        let (names, details) = listings(&[
            ('d', "./directory/", None),
            ('h', "./alias", Some("./directory")),
        ]);
        assert!(validate_archive_listing(&names, &details).is_err());
    }

    #[test]
    fn rejects_duplicate_normalized_member_paths() {
        let (names, details) = listings(&[
            ('-', "./lib/original", None),
            ('h', "lib//original", Some("./lib/original")),
        ]);
        assert!(validate_archive_listing(&names, &details).is_err());
    }

    #[test]
    fn pairs_names_and_verbose_records_in_order() {
        let (names, _) = listings(&[('-', "./first", None), ('-', "./second", None)]);
        let (_, details) = listings(&[('-', "./second", None), ('-', "./first", None)]);
        assert!(validate_archive_listing(&names, &details).is_err());
    }

    #[test]
    fn preserves_leading_spaces_in_member_names() {
        let (names, details) = listings(&[
            ('-', " leading space", None),
            ('h', "another file", Some(" leading space")),
        ]);
        assert!(validate_archive_listing(&names, &details).is_ok());
    }
}
