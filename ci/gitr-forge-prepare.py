#!/usr/bin/env python3
"""Vendor pinned complete Rbit domain crate and stage reviewed forge adapters."""
from pathlib import Path
import hashlib
import json
import shutil
import subprocess

PIN = '39f932a00b7aa35b324e06a9ff2343a57d4bed76'
origin = Path('.gitr-ci/rbit-src')
assert subprocess.check_output(['git','-C',str(origin),'rev-parse','HEAD'],text=True).strip() == PIN
source = origin/'crates/rbit-core'
target = Path('rust/vendor/rbit-core')
assert not target.exists(), 'refuse to overwrite an existing donor import'
original = []
for path in sorted(source.rglob('*')):
    assert not path.is_symlink(), str(path)
    if path.is_file():
        original.append({'path':path.relative_to(source).as_posix(),'sha256':hashlib.sha256(path.read_bytes()).hexdigest()})
shutil.copytree(source,target)
shutil.copyfile(target/'Cargo.toml',target/'UPSTREAM-Cargo.toml')
for name in ['LICENSE-MIT','LICENSE-APACHE']:
    shutil.copyfile(origin/name,target/name)
(target/'Cargo.toml').write_text('''[package]
name = "rbit-core"
version = "0.1.5"
edition = "2024"
rust-version = "1.94"
license = "MIT OR Apache-2.0"
publish = false
repository = "https://github.com/GitRbit/rbit"
description = "Complete pinned Rbit domain model, used through its MIT grant"

[workspace]

[dependencies]
serde = { version = "1.0", features = ["derive"] }
thiserror = "2.0"
time = { version = "0.3", features = ["formatting", "macros"] }
pulldown-cmark = { version = "0.13", default-features = false, features = ["html"] }

[dev-dependencies]
proptest = "1"
serde_json = "1.0"

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
(target/'.gitignore').write_text('/target/\n')
(target/'UPSTREAM.json').write_text(json.dumps({'schema_version':1,'repository':'GitRbit/rbit','revision':PIN,
    'source_path':'crates/rbit-core','selected_license':'MIT','retained_notices':['LICENSE-MIT','LICENSE-APACHE'],
    'scope':'entire original crate including tests; no upstream Rust changes',
    'patches':['resolve workspace-inherited Cargo fields/dependencies/lints in standalone manifest; publish=false'],
    'original_files':original},indent=2)+'\n')
(target/'GITR-IMPORT.md').write_text('''# Rbit core import

Complete crates/rbit-core at 39f932a00b7aa35b324e06a9ff2343a57d4bed76.
Both upstream notices and original manifest are preserved. Gitr selects MIT.
All original Rust sources/tests are retained unchanged. Only Cargo inheritance
is materialized into a standalone package. UPSTREAM.json records original hashes.

This import supplies domain validation and pure check aggregation, not Rbit's
SQLite storage, web server, authentication, permissions enforcement or Git daemon.
Gitr adapters retain redb transactions, exact-source receipts and ref authority.
Update by comparing the whole pinned crate and its tests, refreshing provenance,
then rerunning donor and Gitr qualification before accepting a new revision.
''')

def replace(name, old, new):
    path = Path(name)
    text = path.read_text()
    assert text.count(old) == 1, (name,text.count(old),old[:80])
    path.write_text(text.replace(old,new))

replace('rust/gitr/Cargo.toml','[features]\n','[features]\nforge = ["dep:rbit-core"]\n')
replace('rust/gitr/Cargo.toml','[dependencies]\n','[dependencies]\nrbit-core = { path = "../vendor/rbit-core", optional = true }\n')
p = Path('rust/gitr/src/actions.rs')
p.write_text(p.read_text()+'\n#[cfg(feature = "forge")]\n#[path = "actions_forge.rs"]\npub mod forge;\n')
p = Path('rust/gitr/src/lib.rs')
p.write_text(p.read_text()+'\n#[cfg(feature = "forge")]\npub use actions::forge;\n')
replace('rust/gitr/src/main.rs','enum Command {','''enum Command {
    /// Local forge records, exact-revision reviews and existing redb action checks.
    #[cfg(feature = "forge")]
    Forge {
        #[command(subcommand)]
        command: gitr::forge::Command,
    },''')
replace('rust/gitr/src/main.rs','        Command::Actions { command } => actions::run(command),','''        Command::Actions { command } => actions::run(command),
        #[cfg(feature = "forge")]
        Command::Forge { command } => gitr::forge::run(command),''')
replace('rust/gitr/src/actions_api.rs','async fn handle(State(api): State<Api>, request: HttpRequest<Body>) -> Response {','''enum EndpointOperation {
    Action(Operation),
    #[cfg(feature = "forge")]
    Forge(forge::Request),
}
impl EndpointOperation {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Action(op) => op.validate(),
            #[cfg(feature = "forge")]
            Self::Forge(op) => op.validate(),
        }
    }
    fn perform(self, api: &Api) -> Result<Value> {
        match self {
            Self::Action(op) => api.perform(op),
            #[cfg(feature = "forge")]
            Self::Forge(op) => api.db.forge_control(&crate::git::discover_at(&api.discovery_root)?,op),
        }
    }
}
async fn handle(State(api): State<Api>, request: HttpRequest<Body>) -> Response {
    handle_endpoint(api,request,false).await
}
#[cfg(feature = "forge")]
async fn handle_forge(State(api): State<Api>, request: HttpRequest<Body>) -> Response {
    handle_endpoint(api,request,true).await
}
async fn handle_endpoint(api: Api, request: HttpRequest<Body>, is_forge: bool) -> Response {''')
replace('rust/gitr/src/actions_api.rs','''    let operation: Operation = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };''','''    let operation = if is_forge {
        #[cfg(feature = "forge")]
        {
            match serde_json::from_slice(&bytes) {
                Ok(value) => EndpointOperation::Forge(value),
                Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_request"),
            }
        }
        #[cfg(not(feature = "forge"))]
        return error(StatusCode::NOT_FOUND,"not_found");
    } else {
        match serde_json::from_slice(&bytes) {
            Ok(value) => EndpointOperation::Action(value),
            Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_request"),
        }
    };''')
replace('rust/gitr/src/actions_api.rs','match offload(permit, move || api.perform(operation)).await {','match offload(permit, move || operation.perform(&api)).await {')
replace('rust/gitr/src/actions_api.rs','''            axum::serve(listener, Router::new().route("/v1/actions", post(handle)).with_state(api)).await?;''','''            let router = Router::new().route("/v1/actions", post(handle));
            #[cfg(feature = "forge")]
            let router = router.route("/v1/forge",post(handle_forge));
            axum::serve(listener, router.with_state(api)).await?;''')
print('Prepared complete Rbit domain import:',len(original),'original files. Existing redb/Gix/Wasmi pins preserved.')
