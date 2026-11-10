//! Resolve dependency locks without compiling application code. Qualification
//! runs separately with the pinned produced BlueFe compiler and these locks.
#![forbid(unsafe_code)]
use std::{env, error::Error, fs, path::PathBuf, process::{Command, ExitCode}};
type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn run(command: &mut Command) -> Result<()> {
    if !command.status()?.success() { return Err("dependency-lock command failed".into()); }
    Ok(())
}

fn refresh() -> Result<()> {
    let branch = env::var("GITHUB_REF_NAME")?;
    if branch != "gitr/rust-upstream-import" {
        return Err("automatic lock commits are restricted to the import branch".into());
    }
    let root = PathBuf::from(env::var("GITHUB_WORKSPACE")?);
    let out = root.join(".gitr-ci/lock-metadata");
    fs::create_dir_all(&out)?;
    for (name, manifest) in [("gitr", "Cargo.toml"), ("grit", "vendor/grit/Cargo.toml")] {
        // Metadata resolves existing locked versions plus new path dependencies;
        // it does not invoke rustc or compile Gitr with a fallback compiler.
        let file = fs::File::create(out.join(format!("{name}.json")))?;
        run(Command::new("cargo").args(["metadata", "--format-version", "1", "--manifest-path", manifest]).stdout(file))?;
    }
    run(Command::new("git").args(["add", "--", "Cargo.lock", "vendor/grit/Cargo.lock"]))?;
    let status = Command::new("git").args(["diff", "--cached", "--quiet", "--exit-code"]).status()?;
    if status.success() { println!("Dependency locks are already current"); return Ok(()); }
    if status.code() != Some(1) { return Err("could not inspect lock changes".into()); }
    run(Command::new("git").args(["-c", "user.name=Gitr qualification", "-c", "user.email=actions@users.noreply.github.com", "commit", "-m", "Lock the imported Rust Git dependency closure"]))?;
    // A normal push rejects concurrent branch movement. Never force another
    // writer's changes away; the caller can re-run against the new branch head.
    run(Command::new("git").args(["push", "origin", &format!("HEAD:refs/heads/{branch}")]))?;
    Ok(())
}

fn main() -> ExitCode {
    match refresh() { Ok(()) => ExitCode::SUCCESS, Err(error) => { eprintln!("{error}"); ExitCode::FAILURE } }
}
