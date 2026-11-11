// SPDX-License-Identifier: MIT OR Apache-2.0
//! Build the connected upstream Git source as Rit's separate compatibility lane.
//! The compiler used for this dependency-free build coordinator does not select
//! the compiler used to build Rit. Build Rit itself through pinned Lustr.
#![forbid(unsafe_code)]

use std::{
    env, fs,
    path::{Component, Path, PathBuf},
    process::{Command, ExitCode},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const REVISION: &str = "e9019fcafe0040228b8631c30f97ae1adb61bcdc";
const VERSION: &str = "git version 2.55.0";
const RECEIPT: &str = include_str!("../compat/git-source.txt");

fn output(command: &mut Command) -> Result<String> {
    let result = command.output()?;
    if !result.status.success() {
        return Err(format!("{} failed: {}", command.get_program().to_string_lossy(), String::from_utf8_lossy(&result.stderr)).into());
    }
    Ok(String::from_utf8(result.stdout)?)
}

fn absolute(path: PathBuf) -> Result<PathBuf> {
    if !path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
        return Err("source and prefix must be absolute paths without parent traversal".into());
    }
    Ok(path)
}

fn build(source: &Path, prefix: &Path) -> Result<()> {
    let source = fs::canonicalize(source).map_err(|error| format!(
        "Git source is unavailable: {error}; run git submodule update --init compat/git"
    ))?;
    if output(Command::new("git").arg("-C").arg(&source).args(["rev-parse", "HEAD"]))?.trim() != REVISION {
        return Err("Git source does not match the recorded compatibility revision".into());
    }
    if !output(Command::new("git").arg("-C").arg(&source).args(["status", "--porcelain", "--untracked-files=no"]))?.is_empty() {
        return Err("Git source has tracked modifications; record a new qualified source pin first".into());
    }
    for name in ["config.mak", "config.mak.autogen"] {
        if source.join(name).exists() {
            return Err(format!("unrecorded Git build override: {name}").into());
        }
    }
    fs::create_dir_all(prefix)?;
    let prefix = fs::canonicalize(prefix)?;
    if prefix == source || prefix.starts_with(&source) || prefix.parent().is_none() {
        return Err("Git install prefix must be a dedicated directory outside its source".into());
    }
    let receipt = prefix.join("rit-git-source.txt");
    if receipt.exists() {
        fs::remove_file(&receipt)?;
    }
    // Keep the full companion helper/template installation. An isolated git
    // binary is insufficient for Git commands that use scripts or helpers.
    let status = Command::new("make")
        .arg("-C").arg(&source)
        .args(["-j2", "USE_LIBPCRE2=1", "install"])
        .arg(format!("prefix={}", prefix.display()))
        .env("CARGO_TARGET_DIR", source.join("target"))
        .status()?;
    if !status.success() {
        return Err("upstream Git compatibility build failed; no receipt published".into());
    }
    let binary = prefix.join("bin").join(if cfg!(windows) { "git.exe" } else { "git" });
    if output(Command::new(&binary).arg("--version"))?.trim() != VERSION {
        return Err("installed Git version does not match the connected source".into());
    }
    let scalar = prefix.join("bin").join(if cfg!(windows) { "scalar.exe" } else { "scalar" });
    if !scalar.is_file() {
        return Err("upstream Git install is missing scalar".into());
    }
    fs::write(&receipt, RECEIPT)?;
    println!("Git compatibility source: {REVISION}");
    println!("RIT_GIT_COMPAT={}", binary.display());
    Ok(())
}

fn main() -> ExitCode {
    let result = (|| -> Result<()> {
        let root = env::current_dir()?;
        let mut source = root.join("compat/git");
        let mut prefix = root.join(".rit-ci/git-compat");
        let mut args = env::args_os().skip(1);
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--source") => source = PathBuf::from(args.next().ok_or("missing --source value")?),
                Some("--prefix") => prefix = PathBuf::from(args.next().ok_or("missing --prefix value")?),
                _ => return Err("usage: rit-build-git-compat [--source ABSOLUTE_PATH] [--prefix ABSOLUTE_PATH]".into()),
            }
        }
        build(&absolute(source)?, &absolute(prefix)?)
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
