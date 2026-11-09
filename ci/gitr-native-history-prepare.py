#!/usr/bin/env python3
"""Qualify reviewed native authoring against the existing CLI; never update refs."""
import hashlib
from pathlib import Path
import tomllib

root = Path('rust/gitr')
manifest = tomllib.loads((root / 'Cargo.toml').read_text(encoding='utf-8'))
assert manifest['dependencies']['redb'] == '=4.5.0', 'review changed redb pin'
assert manifest['dependencies']['gix']['version'] == '=0.90.0', 'review changed Gitoxide pin'
path = root / 'src/git.rs'
data = path.read_bytes()
blob = hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()
assert blob == '0160b20d4c548cc83179c2db7e01d3c1a5b212c1', 'git.rs changed; rebase/review before qualification'
text = data.decode('utf-8')
for old, new in [
    ('use std::{', '#[path = "git_write.rs"]\nmod write;\n\nuse std::{'),
    ('pub enum Command {\n', 'pub enum Command {\n    #[command(flatten)]\n    Write(write::Command),\n'),
    ('    match command {\n', '    match command {\n        Command::Write(command) => write::run(command, &mut out),\n'),
]:
    assert text.count(old) == 1, f'nonunique integration anchor: {old!r}'
    text = text.replace(old, new, 1)
test = root / 'tests/native_history.rs'
assert not test.exists(), 'do not overwrite concurrent test implementation'
test.write_bytes(Path('ci/candidates/native_history.rs').read_bytes())
path.write_text(text, encoding='utf-8')
print('Prepared native authoring integration without changing dependencies, lockfiles or refs.')
