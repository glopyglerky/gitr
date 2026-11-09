#!/usr/bin/env python3
"""Prepare a complete pinned Rbit crate import and reviewed Gitr integration.

Runs only in disposable qualification checkouts. Never updates commits or refs.
The exact tested bytes, including notices and tests, are promoted explicitly.
"""
from pathlib import Path
import hashlib
import json
import shutil
import subprocess
import tomllib

PIN = '39f932a00b7aa35b324e06a9ff2343a57d4bed76'
source = Path('.gitr-ci/rbit-src')
assert subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip() == PIN
upstream = source / 'crates/rbit-workflow'
vendor = Path('rust/vendor/rbit-workflow')
assert not vendor.exists(), 'candidate must start without an existing vendor tree'
originals = []
for path in sorted(upstream.rglob('*')):
    assert not path.is_symlink(), f'unreviewed upstream symlink: {path}'
    if path.is_file():
        originals.append({'path':path.relative_to(upstream).as_posix(), 'source_sha256':hashlib.sha256(path.read_bytes()).hexdigest()})
shutil.copytree(upstream, vendor)
for name in ['LICENSE-MIT', 'LICENSE-APACHE']:
    shutil.copyfile(source / name, vendor / name)
shutil.copyfile(vendor / 'Cargo.toml', vendor / 'Cargo.toml.upstream')
workspace = tomllib.loads((source / 'Cargo.toml').read_text())
assert workspace['workspace']['package']['license'] == 'MIT OR Apache-2.0'
assert workspace['workspace']['package']['version'] == '0.1.5'
# Resolve workspace inheritance without importing the forge's service/runtime graph.
(vendor / 'Cargo.toml').write_text('''[package]
name = "rbit-workflow"
version = "0.1.5"
edition = "2024"
rust-version = "1.94"
license = "MIT OR Apache-2.0"
description = "Vendored complete Rbit workflow planner with Gitr deadline hardening"
repository = "https://github.com/GitRbit/rbit"
publish = false

[workspace]

[dependencies]
regex-automata = "0.4"
serde = { version = "1.0", features = ["derive"] }
serde-saphyr = { version = "2.0", default-features = false, features = ["deserialize"] }
serde_json = "1.0"
zeroize = "1.9"

[dev-dependencies]
proptest = "1"

[lints.rust]
unsafe_code = "forbid"
missing_debug_implementations = "warn"
rust_2018_idioms = { level = "warn", priority = -1 }

[lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
module_name_repetitions = "allow"
missing_errors_doc = "allow"
missing_panics_doc = "allow"
must_use_candidate = "allow"
''')
# Ensure the handwritten inheritance resolution is exactly the inspected closure.
resolved = tomllib.loads((vendor / 'Cargo.toml').read_text())
for section in ['dependencies', 'dev-dependencies']:
    original = tomllib.loads((upstream / 'Cargo.toml').read_text())[section]
    assert set(original) == set(resolved[section])
    for name in original:
        assert original[name] == {'workspace': True}
        assert resolved[section][name] == workspace['workspace']['dependencies'][name], name


def replace_once(text, old, new):
    assert text.count(old) == 1, f'integration source drift: {old[:100]!r}'
    return text.replace(old, new, 1)

path = vendor / 'src/limits.rs'
text = path.read_text()
text = replace_once(text, 'Self(Instant::now().checked_add(budget))', 'let now = Instant::now();\n        // An unrepresentable budget must expire, never become Deadline::NONE.\n        Self(Some(now.checked_add(budget).unwrap_or(now)))')
text = replace_once(text, 'assert!(Deadline::NONE.check().is_ok());', 'assert!(Deadline::NONE.check().is_ok());\n        assert_eq!(Deadline::after(Duration::MAX).check().unwrap_err().kind, ErrorKind::Timeout);')
path.write_text(text)
(vendor / 'GITR-UPSTREAM.json').write_text(json.dumps({
    'schema_version':1, 'repository':'GitRbit/rbit', 'revision':PIN,
    'imported_scope':'complete crates/rbit-workflow source, tests and fixtures',
    'license_expression':'MIT OR Apache-2.0', 'selected_integration_grant':'MIT',
    'retained_root_notices':['LICENSE-MIT','LICENSE-APACHE'],
    'changes':['resolve Cargo workspace inheritance into standalone manifest',
               'make overflowing Deadline::after budgets expire; retain regression in upstream tests'],
    'source_files':originals,
}, indent=2) + '\n')
(vendor / 'GITR-NOTICE.md').write_text('''# Rbit workflow import

Complete `crates/rbit-workflow` from GitRbit/rbit at
`39f932a00b7aa35b324e06a9ff2343a57d4bed76`, including its tests and fixtures.
Original code remains MIT OR Apache-2.0; Gitr uses the offered MIT grant.
Both upstream license texts and the unmodified original crate manifest are retained.
The generated provenance manifest records every original file's SHA-256.

Local changes resolve workspace inheritance and make an overflowing deadline
expire instead of disabling the deadline. The latter includes a regression test.
No Rbit server, SQLite backend, runner subprocess, or secret materialization is
imported into Gitr's execution service. Keep future updates pinned and review the
local patch and full conformance corpus rather than replacing notices or tests.
''')

path = Path('rust/gitr/Cargo.toml')
text = path.read_text()
assert 'redb = "=4.5.0"' in text and 'version = "=0.90.0"' in text
text = replace_once(text, 'default = []', 'default = []\nworkflows = ["dep:rbit-workflow"]')
text = replace_once(text, '[dependencies]', '[dependencies]\nrbit-workflow = { path = "../vendor/rbit-workflow", optional = true }')
path.write_text(text)

path = Path('rust/gitr/src/actions.rs')
text = path.read_text()
text = replace_once(text, 'pub enum Command {', '''pub enum Command {
    /// Durable multi-job Wasm workflows; planning requires the workflows feature.
    Workflow {
        #[command(subcommand)]
        command: workflow::Command,
    },''')
text = replace_once(text, 'pub fn run(command: Command) -> Result<()> {\n    let repo = crate::git::discover()?;', '''pub fn run(command: Command) -> Result<()> {
    let repo = crate::git::discover()?;
    if let Command::Workflow { command } = command {
        return workflow::run(&repo, command);
    }''')
text = replace_once(text, '        Command::Control => unreachable!', '        Command::Workflow { .. } => unreachable!("handled above"),\n        Command::Control => unreachable!')
text = replace_once(text, 'job.spec.profile == PROFILE,', 'job.spec.profile == PROFILE || job.spec.profile == workflow::MANAGED_PROFILE,')
text = replace_once(text, '        job.attempt = job', '''        if !workflow::claim_allowed(&tx, &job, now)? {
            tx.commit()?;
            return Ok(None);
        }
        job.attempt = job''')
text = replace_once(text, '            write_job(&tx, &job)?;\n            tx.commit()?;', '            write_job(&tx, &job)?;\n            workflow::after_change(&tx, &job)?;\n            tx.commit()?;')
text = replace_once(text, '        // Artifact and terminal receipt become visible together, or neither does.\n        write_job(&tx, &job)?;', '        // Artifact, receipt and workflow propagation become visible together.\n        write_job(&tx, &job)?;\n        workflow::after_change(&tx, &job)?;')
text += '\n#[path = "actions_workflow.rs"]\npub mod workflow;\n'
path.write_text(text)

path = Path('rust/gitr/src/actions_export.rs')
text = path.read_text()
for function in ['encode_hex', 'line', 'mix_record']:
    text = replace_once(text, f'fn {function}(', f'pub(super) fn {function}(')
text = replace_once(text, '        let objects = tx.open_table(OBJECTS)?;', '        let workflow_export = workflow::export_preflight(&tx)?;\n        let objects = tx.open_table(OBJECTS)?;')
text = replace_once(text, 'job.spec.profile == PROFILE,', 'job.spec.profile == PROFILE || job.spec.profile == workflow::MANAGED_PROFILE,')
text = replace_once(text, '            for reference in [', '            workflow::verify_export_job(&tx, &job)?;\n            for reference in [')
text = replace_once(text, '&json!({"kind":"header","format":"gitr-action-export-v1","schema":1}),', '&if workflow_export { json!({"kind":"header","format":"gitr-action-export-v2","schema":1,"workflow_schema":1}) } else { json!({"kind":"header","format":"gitr-action-export-v1","schema":1}) },')
text = replace_once(text, '''        line(
            out,
            &json!({
                "kind":"end", "objects":object_count, "jobs":job_count,
                "sha256":encode_hex(&hash.finalize())
            }),
        )?;''', '''        let workflow_count = workflow::export_records(&tx, out, &mut hash)?;
        let mut footer = json!({"kind":"end","objects":object_count,"jobs":job_count,"sha256":encode_hex(&hash.finalize())});
        if workflow_export { footer["workflows"] = workflow_count.into(); }
        line(out, &footer)?;''')
path.write_text(text)
print(f'Prepared complete Rbit import: {len(originals)} upstream files; preserved redb/Gix/Wasmi pins.')
