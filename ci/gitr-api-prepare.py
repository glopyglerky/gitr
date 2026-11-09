#!/usr/bin/env python3
"""Stage the optional redb action API for exact qualification, never update refs."""
from pathlib import Path

path = Path('rust/gitr/Cargo.toml')
text = path.read_text()
if '[features]' not in text:
    text = text.replace('[dependencies]', '[features]\ndefault = []\naction-api = ["dep:axum", "dep:tokio", "dep:subtle"]\n\n[dependencies]')
    text = text.replace('[dev-dependencies]', 'axum = { version = "=0.8.9", default-features = false, features = ["http1", "json", "tokio"], optional = true }\ntokio = { version = "=1.54.1", features = ["rt-multi-thread", "net", "sync", "time"], optional = true }\nsubtle = { version = "=2.6.1", optional = true }\n\n[dev-dependencies]')
    path.write_text(text)
else:
    assert 'action-api = ' in text

path = Path('rust/gitr/src/actions.rs')
text = path.read_text()
if 'mod http;' not in text:
    text += '\n#[cfg(feature = "action-api")]\n#[path = "actions_api.rs"]\nmod http;\n'
    needle = 'pub enum Command {'
    assert text.count(needle) == 1
    text = text.replace(needle, needle + '''
    /// Opt-in authenticated local HTTP API over the same redb action database.
    #[cfg(feature = "action-api")]
    Serve {
        #[arg(long, default_value = "127.0.0.1:0")]
        listen: std::net::SocketAddr,
        #[arg(long)]
        repository: String,
    },''')
    needle = 'pub fn run(command: Command) -> Result<()> {\n    let repo = crate::git::discover()?;'
    assert text.count(needle) == 1
    text = text.replace(needle, needle + '''
    #[cfg(feature = "action-api")]
    if let Command::Serve { listen, repository } = command {
        return http::serve(&repo, listen, &repository);
    }''')
    needle = '        Command::Control => unreachable!'
    assert text.count(needle) == 1
    text = text.replace(needle, '''        #[cfg(feature = "action-api")]
        Command::Serve { .. } => unreachable!("handled above"),
''' + needle)
    path.write_text(text)
