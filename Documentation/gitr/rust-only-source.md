# Strict Rust-only repository source

The 2026-11-09 directive supersedes permission to retain foreign development or
compatibility programs. All checked-in implementation, tests and operational,
build and qualification helpers are Rust, including retained archive support.
Licenses, notices, prose, Cargo manifests/locks, evidence and declarative data
remain. This policy does not mean dependencies or generated machine code are
entirely Rust or memory safe. GitHub fork metadata and licensing are unchanged.

## Source map and removals

- `src/` and `tests/`: Gitr runtime and Rust tests, including differential tests
  against the separately installed reference Git. Production never falls back
  to that executable. No Git implementation is vendored as an oracle.
- `crates/action-example`: Rust source-policy Wasm guest.
- `crates/test-support`: typed `wasm-encoder` constructors replacing every
  executable WAT string in Rust tests. This constructs valid/hostile binary
  modules with Rust APIs; no text interpreter or foreign program payload is used.
  Fixtures cover echo, fresh globals, partial-output failure/trap, looping,
  memory growth, buffer bounds, host/output budgets, invalid imports, start
  functions, signatures and memory size. Test assertions remain in their owners.
- `xtask`: Rust storage guard and qualification coordinator. The guard parses
  TOML, checks all tracked active manifests/locks, rejects Surreal dependencies
  including aliases/optional/target/workspace/build/dev/transitive dependencies,
  rejects retired active entrypoints, and requires matching nonoptional exact
  redb 4.5.0. It does not open or migrate repository databases.
- `vendor/rbit-workflow`: pinned Rust donor and coherent parser tests. YAML/JSON
  golden inputs and strings describing shell steps are inert parser data; no
  shell executor is imported. Donor source hashes, MIT/Apache notices and the
  prior workspace/deadline divergence remain unchanged.

Removed eleven Python helpers (eight active, three archived), three workflows
(the catalog job and two archived qualification jobs), the archived SurrealQL
schema, and both retired Rust probe entrypoints executing embedded SurrealQL.
The schema/probe experiments are no longer buildable; retained manifests,
licenses and Rust support are historical records. Previous code is available
in Git history at `1e6cfcf4abe7c23b96db6f5d6f36e8fc789d8200`. Original Git source
belongs in upstream `git/git`, not in a documentation archive here.

Redb policy and compiler qualification orchestration were migrated. Python
preparation, action/API/workflow/forge smoke drivers, benchmark statistics and
catalog merge/validation were retired rather than represented as Rust passes.
Their end-to-end orchestration, HTTP concurrency probes, old-worker comparison,
benchmark reproducibility and catalog regeneration are capability gaps. Existing
Rust durability, fencing, authorization, workflow and worker tests remain; they
do not establish parity with every removed smoke driver. Static prior logs are
historical evidence only. No forge/HA/search feature was added or claimed.

## Rust entrypoints

With the reviewed absolute `BLUEFE_ROOT` bundle, invoke its `bin/lustr` with
`--bluefe-root` and one of these argument vectors:

- `test --manifest-path xtask/Cargo.toml --locked`
- `run --manifest-path xtask/Cargo.toml --locked -- policy`
- `run --manifest-path xtask/Cargo.toml --locked -- qualify`
- `run --manifest-path xtask/Cargo.toml --locked -- audit`

`qualify` verifies the produced compiler commit then invokes Lustr directly for
root builds, feature-on/off tests, doctests, Wasm example and donor/guard tests.
It sets exact RUSTC/RUSTDOC/sysroot and limits jobs to two. CI contains only
single declarative Rust command invocations, with no inline foreign programs.
It requires a provisioned Linux runner; no automatic compiler installation or
observed hosted CI result is claimed. macOS/Windows are unexecuted and their
previous speculative dispatch choices were removed.

## External dependency boundary

Production dependency selections remain unchanged: redb 4.5.0, Wasmi 2.0.0,
gix 0.90.0, Rust zlib-rs and Rust sha1dc. TLS remains ureq/rustls with ring
0.17.14 native C/assembly. Certificate/hostname/crypto verification was not
weakened or replaced. No already-qualified pure-Rust TLS backend was available
in this pinned selection; a provider substitution needs separate compatibility
qualification. OS bindings, upstream unsafe Rust, libc/allocator, compiler
runtime, native linker and the external reference Git remain external native
components. On other targets platform import libraries also apply. The exact
prior feature/build-script inventory is in `rust-root-evidence`; this cleanup
changes only test dependencies from wat/wast/bumpalo to the existing encoder
and adds an isolated TOML-based xtask, not the production dependency graph.

## Observed qualification and audit

Fresh Linux qualification used Lustr 6339fbb104d6adda4be7e6be7dc637a6a9e5586b,
produced rustc/rustdoc 7005dc02c9fb6674380eade443022bbc04215276,
explicit compiler paths/sysroot and at most two jobs. Root all-feature tests:
129 passed, six deliberately ignored subprocess fixtures; feature-off tests:
123 passed with the same six ignored fixtures. Doctest invocation passed with
zero cases. Five Rust guard tests, actual repository policy, root locked
all-feature metadata/build and the release wasm32 action example build passed.
The initial root build failed because a new test-only import preceded module
docs and escaped a test module; that failure was corrected before acceptance.
No full compiler rebuild, dependency update, fresh donor test run, release
native optimization build, macOS/Windows execution or hosted CI run is claimed.
The qualification dispatcher compiled; its aggregate `qualify` command was
reviewed, while its constituent checks above were executed individually.

The audit command inventories every tracked path, rejects unknown extensions,
executable modes, symlinks/submodules and foreign shebangs. Manual review also
covered embedded Wasm/SQL programs, process launches, includes and workflow
bodies. Extension classification alone is not proof of source language. Remaining
foreign-looking workflow strings are non-executed parser data. No Linguist
attributes were added. `rust-only-audit.json` records the complete final path
classification; private build logs retain actual compiler failures and passes.
Linux runtime linkage observed libc, libm, libgcc_s and the system loader;
this does not reveal statically linked ring native code.

Owned generated-output cleanup acquired all three native Cargo build locks
after confirming no active descriptors/processes or mounts in the target scope.
Disposable compiler targets and temporary editing/orchestration scripts were
deleted, not moved. Current native/xtask/Wasm deliverables and small evidence
remain outside the removed target; the previous owner’s rollback is preserved
read-only. Observed shared-filesystem available space increased by 975294464
bytes; concurrent worker activity can affect that delta.
