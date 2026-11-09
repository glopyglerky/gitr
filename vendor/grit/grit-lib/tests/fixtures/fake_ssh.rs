use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

// Decode only the single argument quoting emitted by SshTransport. No command
// language is evaluated; only the two named Git transport services may run.
fn decode_argument(value: &str) -> io::Result<String> {
    let mut quoted = false;
    let mut chars = value.chars();
    let mut result = String::new();
    while let Some(ch) = chars.next() {
        match (quoted, ch) {
            (_, '\'') => quoted = !quoted,
            (false, '\\') => result.push(chars.next().ok_or_else(|| io::Error::other("dangling escape"))?),
            (false, _) => return Err(io::Error::other("unquoted remote path")),
            (true, _) => result.push(ch),
        }
    }
    if quoted {
        return Err(io::Error::other("unclosed quote"));
    }
    Ok(result)
}

fn main() -> io::Result<()> {
    let remote = std::env::args().last().ok_or_else(|| io::Error::other("missing remote command"))?;
    let (service, path) = remote.split_once(' ').ok_or_else(|| io::Error::other("missing remote path"))?;
    let command = match service {
        "git-upload-pack" => "upload-pack",
        "git-receive-pack" => "receive-pack",
        _ => return Err(io::Error::other("unsupported fixture service")),
    };
    let mut child = Command::new("git");
    child.arg(command).arg(decode_argument(path)?);
    if command == "receive-pack" {
        child.stderr(Stdio::null());
    }
    Err(child.exec())
}
