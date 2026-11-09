//! Hosted native qualification through the verified produced BlueFe/Lustr bundle.
//! This dependency-free coordinator may be compiled by stock Rust. It never
//! invokes that compiler or its Cargo to build Gitr or an application dependency.
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
use std::time::Instant;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BLUEFE: &str = "7005dc02c9fb6674380eade443022bbc04215276";
const LUSTR: &str = "6339fbb104d6adda4be7e6be7dc637a6a9e5586b";
const ARCHIVE: &str = "gitr-produced-bluefe.tar.gz";
const DIGEST: &str = "gitr-produced-bluefe.tar.gz.sha256";

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
            download: path_env("GITR_BLUEFE_DOWNLOAD")?,
            logs: path_env("GITR_NATIVE_LOGS")?,
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

fn checked_archive_members(archive: &Path, logs: &Path) -> Result<()> {
    let names = output(Command::new("tar")
        .args(["--list", "--gzip", "--quoting-style=literal", "--file"]).arg(archive))?;
    let mut seen = BTreeSet::new();
    for name in names.lines() {
        if name.is_empty() || name.chars().any(char::is_control) || name.contains('\\') {
            return Err("unsupported archive member name".into());
        }
        let path = Path::new(name);
        if path.is_absolute() || path.components().any(|part| {
            matches!(part, Component::ParentDir | Component::RootDir | Component::Prefix(_))
        }) {
            return Err(format!("unsafe archive member: {name:?}").into());
        }
        let normalized: PathBuf = path.components()
            .filter(|part| *part != Component::CurDir).collect();
        if !seen.insert(normalized) {
            return Err(format!("duplicate archive member: {name:?}").into());
        }
    }
    if seen.is_empty() {
        return Err("empty compiler archive".into());
    }
    let details = output(Command::new("tar")
        .args(["--list", "--verbose", "--gzip", "--quoting-style=literal", "--file"])
        .arg(archive))?;
    if details.lines().count() != seen.len()
        || details.lines().any(|line| !matches!(line.as_bytes().first().copied(), Some(b'-' | b'd')))
    {
        return Err("compiler archive may contain only regular files and directories".into());
    }
    fs::write(logs.join("archive-members.txt"), details)?;
    Ok(())
}

fn prepare(context: &Context, verifier: &Path) -> Result<()> {
    let run = env::var("GITR_BLUEFE_RUN_ID")?.parse::<u64>()?;
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
        env::var("GITHUB_SHA")?, env::var("GITR_BLUEFE_ARTIFACT_ID")?,
    );
    fs::write(context.logs.join("bundle-verified.txt"), proof)?;
    fs::write(context.logs.join("reference-git-version.txt"),
        output(Command::new("git").arg("--version"))?)?;
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
        require_passed(context, &["test-root", "test-vendor", "verify-import", "policy", "audit"])?;
    }
    let root = context.bundle.to_str().ok_or("non-UTF8 compiler path")?;
    if root.chars().any(char::is_whitespace) {
        return Err("compiler path must not contain whitespace in RUSTDOCFLAGS".into());
    }
    let mut paths = vec![context.bundle.join("bin")];
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let mut command = Command::new(context.bundle.join("bin/lustr"));
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
            command.args(["test", "--manifest-path", "Cargo.toml", "--locked", "--all-features", "--all-targets"]);
        }
        "test-vendor" => {
            command.args(["test", "--manifest-path", "vendor/grit/Cargo.toml", "--locked",
                "--workspace", "--all-features", "--all-targets"]);
        }
        "verify-import" => {
            command.args(["run", "--example", "verify_import", "--locked"]);
        }
        "policy" | "audit" => {
            command.args(["run", "--manifest-path", "xtask/Cargo.toml", "--locked", "--", phase]);
        }
        "build-release" => {
            command.args(["build", "--manifest-path", "Cargo.toml", "--locked",
                "--all-features", "--release", "--bin", "gitr"])
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
    require_passed(context, &["test-root", "test-vendor", "verify-import", "policy", "audit", "build-release"])?;
    let helper = context.target.join("gitr-benchmark-native");
    logged(context, "compile-benchmark-helper", Command::new(context.bundle.join("bin/rustc"))
        .current_dir(&context.workspace)
        .arg("--sysroot").arg(&context.bundle)
        .args(["--edition=2024", "-C", "opt-level=3", "-C", "debuginfo=0",
            "ci/benchmark_native.rs", "-o"])
        .arg(&helper))?;
    let executable = context.target.join("release/gitr");
    let reference = path_env("GITR_REFERENCE_GIT")?;
    if !reference.is_file() {
        return Err("missing absolute reference Git executable".into());
    }
    fs::write(context.logs.join("benchmark-build.txt"), format!(
        "engine=rustc\nbluefe={BLUEFE}\nlustr={LUSTR}\nprofile=release\nopt-level=3\nlto=false\ncodegen-units=16\ndebug=0\nfeatures=all\nrepository-default-release-lto=thin\ngitr-sha256={}\nreference-git-sha256={}\n",
        checksum(&executable)?, checksum(&reference)?,
    ))?;
    logged(context, "benchmark-native", Command::new(helper)
        .arg("--gitr").arg(executable)
        .arg("--git").arg(reference)
        .arg("--compiler-root").arg(&context.bundle)
        .arg("--fixture").arg(context.workspace.join(".gitr-ci/native-benchmark-fixture"))
        .arg("--output").arg(context.logs.join("benchmarks"))
        .env("CARGO_PROFILE_RELEASE_OPT_LEVEL", "3")
        .env("CARGO_PROFILE_RELEASE_LTO", "false")
        .env("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "16")
        .env("CARGO_PROFILE_RELEASE_DEBUG", "0"))
}

fn run() -> Result<()> {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    let phase = arguments.first().and_then(|value| value.to_str()).ok_or(
        "usage: qualify-native prepare /absolute/bluefe-helper | build-root | test-root | test-vendor | verify-import | policy | audit | build-release | benchmark")?;
    let context = Context::load()?;
    let start = Instant::now();
    let result = match phase {
        "prepare" if arguments.len() == 2 => prepare(&context, Path::new(&arguments[1])),
        "build-root" | "test-root" | "test-vendor" | "verify-import" | "policy" | "audit"
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
