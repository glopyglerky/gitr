use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

fn main() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let target = std::fs::read_to_string(executable.with_extension("target"))?;
    Err(Command::new(target).stderr(Stdio::null()).exec())
}
