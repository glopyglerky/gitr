#!/usr/bin/env python3
"""Prepare a candidate integration without updating any branch or remote ref."""
from pathlib import Path

root = Path("rust/gitr")
manifest = root / "Cargo.toml"
text = manifest.read_text()
if "wasmi =" not in text:
    text = text.replace('[lints.rust]', '[dev-dependencies]\nwat = "=1.228.0"\n\n[lints.rust]')
    text = text.replace('url = "2.5"', 'url = "2.5"\nwasmi = { version = "=2.0.0", default-features = false, features = ["stable", "std", "validate", "portable-dispatch", "extra-checks"] }')
    manifest.write_text(text)
lib = root / "src/lib.rs"
text = lib.read_text()
if "pub mod actions;" not in text:
    assert "pub mod assets;" in text
    lib.write_text(text.replace("pub mod assets;", "pub mod actions;\npub mod assets;"))
main = root / "src/main.rs"
text = main.read_text()
if "Actions {" not in text:
    text = text.replace('use gitr::{assets,', 'use gitr::{actions, assets,')
    text = text.replace('    Doctor,', '    Doctor,\n    /// Database-resident bounded Wasm actions and local JSON control.\n    Actions {\n        #[command(subcommand)]\n        command: actions::Command,\n    },')
    text = text.replace('        Command::Git(command) =>', '        Command::Actions { command } => actions::run(command),\n        Command::Git(command) =>')
    main.write_text(text)
source = root / "src/actions.rs"
text = source.read_text().replace("ReadableDatabase, TableDefinition", "ReadableDatabase, ReadableTable, TableDefinition")
text = text.replace('format!("{:x}", Sha256::digest(bytes))', 'Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()')
text = text.replace('tx.set_durability(Durability::Immediate);', 'tx.set_durability(Durability::Immediate)?;')
source.write_text(text)
tests = root / "src/actions_tests.rs"
text = tests.read_text().replace("filter_map(|t| t.join().ok())", "map(|t| t.join().unwrap())")
tests.write_text(text)
