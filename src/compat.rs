// SPDX-License-Identifier: MIT OR Apache-2.0
//! Pre-execution selection of a separately built, pinned upstream Git.
//! No native command is retried through this lane after it starts.

use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub(crate) const SOURCE_RECEIPT: &str = include_str!("../compat/git-source.txt");

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Backend {
    Native,
    Compat,
}

pub(crate) fn backend(value: Option<&OsStr>) -> Result<Backend> {
    match value {
        None => Ok(Backend::Compat),
        Some(value) if value == "auto" || value == "compat" => Ok(Backend::Compat),
        Some(value) if value == "native" => Ok(Backend::Native),
        Some(_) => Err("RIT_BACKEND must be auto, compat, or native".into()),
    }
}

fn same_file(first: &Path, second: &Path) -> bool {
    if first == second {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(first), Ok(second)) = (fs::metadata(first), fs::metadata(second)) {
            return first.dev() == second.dev() && first.ino() == second.ino();
        }
    }
    false
}

fn validate_installation(binary: &Path, current: &Path) -> Result<PathBuf> {
    if !binary.is_absolute() {
        return Err("RIT_GIT_COMPAT must be an absolute path to the built companion Git".into());
    }
    let binary = fs::canonicalize(binary).map_err(|error| {
        format!("Git companion is unavailable at {}: {error}; build ci/build_git_compat.rs or select RIT_BACKEND=native explicitly", binary.display())
    })?;
    if !binary.is_file() || same_file(&binary, current) {
        return Err("Git companion must be a separate upstream executable, not Rit".into());
    }
    let bin = binary.parent().ok_or("Git companion has no bin directory")?;
    if bin.file_name() != Some(OsStr::new("bin"))
        || binary.file_name() != Some(OsStr::new(if cfg!(windows) { "git.exe" } else { "git" }))
    {
        return Err("Git companion must use the installed prefix/bin/git layout".into());
    }
    let prefix = bin.parent().ok_or("Git companion has no install prefix")?;
    let receipt = fs::read_to_string(prefix.join("rit-git-source.txt"))
        .map_err(|_| "Git companion lacks its build receipt; use ci/build_git_compat.rs")?;
    if receipt != SOURCE_RECEIPT {
        return Err("Git companion source receipt does not match this Rit revision".into());
    }
    Ok(binary)
}

fn companion(current: &Path) -> Result<PathBuf> {
    if let Some(path) = env::var_os("RIT_GIT_COMPAT") {
        return validate_installation(Path::new(&path), current);
    }
    let installed = current
        .parent()
        .and_then(Path::parent)
        .map(|prefix| prefix.join("libexec/rit-git/bin").join(if cfg!(windows) { "git.exe" } else { "git" }));
    if let Some(path) = installed
        && path.exists()
    {
        return validate_installation(&path, current);
    }
    let development = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".rit-ci/git-compat/bin")
        .join(if cfg!(windows) { "git.exe" } else { "git" });
    validate_installation(&development, current)
}

fn command(binary: &Path, basename: &str, args: &[OsString]) -> Result<Command> {
    let mut command = if basename == "scalar" {
        let scalar = binary.with_file_name(if cfg!(windows) { "scalar.exe" } else { "scalar" });
        if !scalar.is_file() {
            return Err("the pinned Git installation is missing its scalar companion".into());
        }
        Command::new(scalar)
    } else {
        let mut command = Command::new(binary);
        if let Some(helper) = basename.strip_prefix("git-") {
            command.arg(helper);
        }
        command
    };
    command.args(args);
    Ok(command)
}

pub(crate) fn run(basename: &str, args: &[OsString]) -> Result<ExitCode> {
    let current = fs::canonicalize(env::current_exe()?)?;
    // Reject a copied Rit masquerading as a companion. Unlike a global depth
    // counter, this still permits a configured hook to invoke Rit normally.
    if let Some(dispatched) = env::var_os("RIT_COMPAT_DISPATCH")
        && fs::canonicalize(dispatched).is_ok_and(|path| same_file(&path, &current))
    {
        return Err("refusing recursive Git companion dispatch".into());
    }
    let binary = companion(&current)?;
    let mut command = command(&binary, basename, args)?;
    let program = fs::canonicalize(command.get_program())?;
    if same_file(&program, &current) {
        return Err("refusing recursive Git companion dispatch".into());
    }
    command.env("RIT_COMPAT_DISPATCH", &program);
    if env::var_os("RIT_TRACE_BACKEND").is_some_and(|value| value == "1") {
        eprintln!("rit backend: compat {}", program.display());
    }
    // exec keeps the process's inherited descriptors, environment, cwd and
    // signal behavior. Arguments remain OsString, including raw Unix bytes.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(command.exec().into())
    }
    #[cfg(not(unix))]
    {
        let status = command.status()?;
        // Preserve the platform's complete process exit code, not just a u8.
        std::process::exit(status.code().unwrap_or(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_selection_is_explicit_and_pre_execution() {
        assert_eq!(backend(None).unwrap(), Backend::Compat);
        for value in ["auto", "compat"] {
            assert_eq!(backend(Some(OsStr::new(value))).unwrap(), Backend::Compat);
        }
        assert_eq!(backend(Some(OsStr::new("native"))).unwrap(), Backend::Native);
        assert!(backend(Some(OsStr::new(""))).is_err());
        assert!(backend(Some(OsStr::new("fallback-after-error"))).is_err());
    }

    #[test]
    fn helper_dispatch_preserves_argument_order() {
        let args = [OsString::from("--strict"), OsString::from("a repository")];
        let command = command(Path::new("/prefix/bin/git"), "git-upload-pack", &args).unwrap();
        assert_eq!(command.get_args().collect::<Vec<_>>(), [OsStr::new("upload-pack"), OsStr::new("--strict"), OsStr::new("a repository")]);
    }

    #[cfg(unix)]
    #[test]
    fn helper_dispatch_keeps_raw_argument_bytes() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let args = [OsString::from_vec(b"path-\xff".to_vec())];
        let command = command(Path::new("/prefix/bin/git"), "rit", &args).unwrap();
        assert_eq!(command.get_args().next().unwrap().as_bytes(), b"path-\xff");
    }
}
