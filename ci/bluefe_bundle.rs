//! Hosted build orchestration; application qualification uses the produced compiler only.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BLUEFE: &str = "7005dc02c9fb6674380eade443022bbc04215276";
const LUSTR: &str = "6339fbb104d6adda4be7e6be7dc637a6a9e5586b";
const STAGE0: &str = "1c3a6506966e21845b31de0b075d2527660db132";
const HOST: &str = "x86_64-unknown-linux-gnu";
const TARGET: &str = "wasm32-unknown-unknown";

fn path_env(key: &str) -> Result<PathBuf> {
    let path = PathBuf::from(env::var_os(key).ok_or_else(|| format!("missing {key}"))?);
    if !path.is_absolute() {
        return Err(format!("{key} must be absolute").into());
    }
    Ok(path)
}

fn run(command: &mut Command) -> Result<()> {
    println!("Running {command:?}");
    if !command.status()?.success() {
        return Err("child process failed".into());
    }
    Ok(())
}

fn output(command: &mut Command) -> Result<String> {
    let result = command.output()?;
    if !result.status.success() {
        return Err(format!("command {command:?} failed: {}", String::from_utf8_lossy(&result.stderr)).into());
    }
    Ok(String::from_utf8(result.stdout)?)
}

fn checked_source(path: &Path, expected: &str) -> Result<()> {
    let head = output(Command::new("git").arg("-C").arg(path).args(["rev-parse", "HEAD"]))?;
    if head.trim() != expected {
        return Err(format!("wrong source revision for {}", path.display()).into());
    }
    run(Command::new("git").arg("-C").arg(path).args(["diff", "--exit-code", "HEAD", "--"]))
}

fn checksum(path: &Path) -> Result<String> {
    let text = output(Command::new("sha256sum").arg("--binary").arg(path))?;
    let digest = text.split_whitespace().next().ok_or("missing SHA-256")?;
    if digest.len() != 64 || !digest.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid SHA-256 output".into());
    }
    Ok(digest.to_owned())
}

fn files(root: &Path, current: &Path, result: &mut BTreeMap<String, PathBuf>) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            files(root, &path, result)?;
        } else if kind.is_file() {
            let relative = path.strip_prefix(root)?.to_str().ok_or("non-UTF8 bundle path")?;
            if relative == "SHA256SUMS" {
                continue;
            }
            if relative.contains(['\n', '\r', '\\']) {
                return Err(format!("unsupported bundle path {relative:?}").into());
            }
            result.insert(relative.to_owned(), path);
        } else {
            return Err(format!("bundle must contain only directories and regular files: {}", path.display()).into());
        }
    }
    Ok(())
}

fn inventory(root: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut result = BTreeMap::new();
    files(root, root, &mut result)?;
    Ok(result)
}

fn build() -> Result<()> {
    let source_checkout = path_env("BLUEFE_SOURCE")?;
    let lustr_checkout = path_env("LUSTR_SOURCE")?;
    // Keep Cargo's workspace discovery outside the application's workspace.
    // Only these fresh checkouts on the ephemeral hosted runner are relocated.
    let source = path_env("RUNNER_TEMP")?.join("rit-bluefe-source");
    let lustr = path_env("RUNNER_TEMP")?.join("rit-lustr-source");
    if source.exists() || lustr.exists() {
        return Err("refusing to replace an existing compiler checkout".into());
    }
    fs::rename(&source_checkout, &source)?;
    fs::rename(&lustr_checkout, &lustr)?;
    let build = path_env("BLUEFE_BUILD_DIR")?;
    let lustr_build = path_env("LUSTR_BUILD_DIR")?;
    let root = path_env("BLUEFE_ROOT")?;
    if root.exists() {
        return Err("refusing to overwrite an existing compiler bundle".into());
    }
    checked_source(&source, BLUEFE)?;
    checked_source(&lustr, LUSTR)?;
    let metadata = fs::read_to_string(source.join("src/stage0"))?;
    if !metadata.lines().any(|line| line == format!("compiler_git_commit_hash={STAGE0}")) {
        return Err("unexpected stage0 compiler pin".into());
    }
    fs::create_dir_all(&build)?;
    let build_str = build.to_str().ok_or("non-UTF8 build path")?;
    if build_str.contains(['"', '\\', '\n', '\r']) {
        return Err("unsupported build path".into());
    }
    let config = format!(
        "change-id = \"ignore\"\n[build]\nbuild-dir = \"{build_str}\"\nbuild = \"{HOST}\"\nhost = [\"{HOST}\"]\ntarget = [\"{HOST}\", \"{TARGET}\"]\nsubmodules = true\ndocs = false\nextended = false\njobs = 2\n[rust]\ndownload-rustc = false\nchannel = \"dev\"\nomit-git-hash = false\ndebuginfo-level = 0\nincremental = false\nlto = \"off\"\nlld = true\n[llvm]\ndownload-ci-llvm = true\nassertions = false\n"
    );
    let config_path = build.join("rit-bootstrap.toml");
    fs::write(&config_path, &config)?;
    run(Command::new(source.join("x"))
        .current_dir(&source)
        // BlueFe is a fork build, not Rust's bors CI merge pipeline. The
        // explicit bootstrap override selects the upstream LLVM artifact
        // matching the submodule instead of incorrectly treating HEAD^ as it.
        .args(["build", "--ci", "false", "--config"])
        .arg(&config_path)
        .args(["--stage", "1", "--target", &format!("{HOST},{TARGET}"), "compiler/rustc", "library", "src/tools/rustdoc"])
        .env_remove("CARGO_TARGET_DIR"))?;
    if !build.join(HOST).join("ci-llvm/bin/llvm-config").is_file() {
        return Err("expected downloaded upstream LLVM was not produced".into());
    }
    let stage0 = build.join(HOST).join("stage0");
    let stage1 = build.join(HOST).join("stage1");
    let bootstrap_version = output(Command::new(stage0.join("bin/rustc")).arg("-vV"))?;
    if !bootstrap_version.lines().any(|line| line == format!("commit-hash: {STAGE0}")) {
        return Err("bootstrap compiler identity mismatch".into());
    }
    run(Command::new(stage0.join("bin/cargo"))
        .current_dir(&lustr)
        .args(["build", "--manifest-path"])
        .arg(lustr.join("crates/lustr/Cargo.toml"))
        .args(["--bin", "lustr", "--release", "--locked", "--offline"])
        .env("CARGO_TARGET_DIR", &lustr_build)
        .env("RUSTC", stage1.join("bin/rustc"))
        .env("RUSTDOC", stage1.join("bin/rustdoc"))
        .env("CARGO_ENCODED_RUSTFLAGS", format!("--sysroot\x1f{}", stage1.display())))?;
    fs::create_dir_all(&root)?;
    run(Command::new("cp").args(["-aL", "--"]).arg(stage1.join(".")).arg(&root))?;
    // Bootstrap exposes its source checkout through this optional sysroot link.
    // It is not needed to compile applications and must not bloat the portable
    // runtime bundle with compiler test fixtures or checkout metadata.
    let rustc_sources = root.join("lib/rustlib/rustc-src");
    if rustc_sources.is_dir() { fs::remove_dir_all(rustc_sources)?; }
    let notices = root.join("source-notices");
    fs::create_dir_all(&notices)?;
    for name in ["LICENSE-MIT", "LICENSE-APACHE", "COPYRIGHT"] {
        if source.join(name).is_file() { fs::copy(source.join(name), notices.join(format!("bluefe-{name}")))?; }
    }
    for name in ["LICENSE", "LICENSE-MIT", "LICENSE-APACHE"] {
        if lustr.join(name).is_file() { fs::copy(lustr.join(name), notices.join(format!("lustr-{name}")))?; }
    }
    fs::copy(stage0.join("bin/cargo"), root.join("bin/cargo"))?;
    fs::copy(lustr_build.join("release/lustr"), root.join("bin/lustr"))?;
    fs::write(root.join("bootstrap.toml"), config)?;
    let provenance = format!(
        "bluefe={BLUEFE}\nlustr={LUSTR}\nstage0={STAGE0}\nhost={HOST}\ntarget={TARGET}\ncargo=upstream-stage0\n"
    );
    fs::write(root.join("source-provenance.txt"), provenance)?;
    let version = output(Command::new(root.join("bin/rustc")).arg("-vV"))?;
    fs::write(root.join("rustc-version.txt"), version)?;
    let version = output(Command::new(root.join("bin/cargo")).arg("-Vv"))?;
    fs::write(root.join("cargo-version.txt"), version)?;
    let mut hashes = String::new();
    for (relative, path) in inventory(&root)? {
        hashes.push_str(&format!("{}  {relative}\n", checksum(&path)?));
    }
    fs::write(root.join("SHA256SUMS"), hashes)?;
    verify()
}

fn verify() -> Result<()> {
    let root = path_env("BLUEFE_ROOT")?;
    let mut expected = BTreeMap::new();
    for line in fs::read_to_string(root.join("SHA256SUMS"))?.lines() {
        let (digest, path) = line.split_once("  ").ok_or("invalid bundle digest record")?;
        if expected.insert(path.to_owned(), digest.to_owned()).is_some() {
            return Err("duplicate bundle digest record".into());
        }
    }
    let actual = inventory(&root)?;
    if actual.len() != expected.len() {
        return Err("bundle inventory size mismatch".into());
    }
    for (relative, path) in actual {
        if expected.get(&relative) != Some(&checksum(&path)?) {
            return Err(format!("bundle integrity mismatch: {relative}").into());
        }
    }
    let provenance = fs::read_to_string(root.join("source-provenance.txt"))?;
    for field in [format!("bluefe={BLUEFE}"), format!("lustr={LUSTR}"), format!("stage0={STAGE0}")] {
        if !provenance.lines().any(|line| line == field) {
            return Err("bundle source identity mismatch".into());
        }
    }
    let rustc = root.join("bin/rustc");
    let version = output(Command::new(&rustc).arg("-vV"))?;
    if !version.lines().any(|line| line == format!("commit-hash: {BLUEFE}")) {
        return Err("wrong produced compiler commit".into());
    }
    let reported = output(Command::new(&rustc).args(["--print", "sysroot"]))?;
    if Path::new(reported.trim()).canonicalize()? != root.canonicalize()? {
        return Err("compiler uses a sysroot outside the assembled bundle".into());
    }
    for target in [HOST, TARGET] {
        let libs = root.join("lib/rustlib").join(target).join("lib");
        if !fs::read_dir(libs)?.any(|entry| entry.is_ok_and(|e| e.file_name().to_string_lossy().starts_with("libstd-"))) {
            return Err(format!("missing standard library for {target}").into());
        }
    }
    for tool in ["rustc", "rustdoc", "cargo", "lustr"] {
        let links = output(Command::new("ldd").arg(root.join("bin").join(tool)))?;
        if links.contains("not found") {
            return Err(format!("missing runtime dependency for {tool}").into());
        }
        for key in ["BLUEFE_BUILD_DIR", "LUSTR_BUILD_DIR", "BLUEFE_SOURCE", "LUSTR_SOURCE"] {
            if links.contains(path_env(key)?.to_str().ok_or("non-UTF8 build path")?) {
                return Err(format!("{tool} depends on a build-directory runtime library").into());
            }
        }
        run(Command::new(root.join("bin").join(tool)).arg("--version"))?;
    }
    let scratch = path_env("RUNNER_TEMP")?.join("rit-bluefe-smoke");
    fs::create_dir_all(&scratch)?;
    let source = scratch.join("smoke.rs");
    fs::write(&source, "fn main() { assert_eq!(std::hint::black_box(2) + 2, 4); }\n#[test] fn arithmetic() { main(); }\n")?;
    let native = scratch.join("smoke-native");
    run(Command::new(&rustc).arg(&source).args(["--edition=2024", "-o"]).arg(&native))?;
    run(&mut Command::new(&native))?;
    let tests = scratch.join("smoke-tests");
    run(Command::new(&rustc).arg(&source).args(["--edition=2024", "--test", "-o"]).arg(&tests))?;
    run(&mut Command::new(&tests))?;
    run(Command::new(&rustc).arg(&source).args(["--edition=2024", "--target", TARGET, "-o"]).arg(scratch.join("smoke.wasm")))?;
    println!("Produced BlueFe bundle verified for native execution, libtest and wasm linking");
    Ok(())
}

fn archive() -> Result<()> {
    let root = path_env("BLUEFE_ROOT")?;
    let archive = path_env("RUNNER_TEMP")?.join("rit-produced-bluefe.tar.gz");
    run(Command::new("tar").args(["--sort=name", "-C"]).arg(&root).arg("-czf").arg(&archive).arg("."))?;
    fs::write(archive.with_extension("gz.sha256"), format!("{}  rit-produced-bluefe.tar.gz\n", checksum(&archive)?))?;
    Ok(())
}

fn main() -> ExitCode {
    let result = match env::args().nth(1).as_deref() {
        Some("build") => build(),
        Some("verify") => verify(),
        Some("archive") => archive(),
        _ => Err("usage: bluefe-bundle build|verify|archive".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => { eprintln!("{error}"); ExitCode::FAILURE }
    }
}
