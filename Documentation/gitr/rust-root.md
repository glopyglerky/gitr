# Rust root migration qualification

The root Cargo package now builds Gitr, with `src/`, `tests/`, the reviewed
`Cargo.lock`, isolated `crates/action-example`, and `vendor/rbit-workflow`.
The default executable is Gitr. There is no root build script linking gitcore,
Git or zlib. redb remains exactly 4.5.0; schemas, data and action authority were
not migrated. The root licenses and GitHub fork relationship are unchanged.

`root-retirement.json` inventories 4,861 retired paths with exact original blob
IDs and modes at bf628ff8c74da4ab03a4a99001edf6780d8d84a0. This includes 642 C
and 346 header files, 1,307 shell files, inherited Perl/Tcl/Python frontends,
Make/Meson/autoconf infrastructure, gitcore, language GUIs and unused upstream
tests/manuals. Source is recoverable with `git show <baseline>:<path>`; no large
copy was retained. An audit of the standalone manifests, includes, runtime
process launches and fixture constructors found no Gitr production/test consumer
of that tree. Tests synthesize their own repositories with installed Git. Git's
former Makefile enumerating its own source was not a reason to keep it active.
Owned Python qualification scripts and passive historical research/archive files
remain. This is not a claim that every repository file or dependency is Rust.

## Adopted native capability

`gitr ls-tree [--name-only] [-r] [-z] [tree-ish]` now uses Gitoxide's revision
resolver, object peeling and fallible `TreeRefIter` decoder. It lists blob,
symlink, subtree and gitlink entries in stored Git order with byte-preserving NUL
output or Git quoting. It never resolves external gitlink repositories or runs
filters/hooks/helpers. Listing is repository-root-relative, including from a
subdirectory. Pathspecs, prefix rewriting, `-t` and `-l` remain unsupported.
There are limits of 100,000 visited entries, 128 tree levels and 16 MiB per tree
or initial tree-ish object. Pack/delta decoding is not an OS memory quota.
Clone/fetch/push, add/porcelain commit, checkout, merge and rebase remain unfinished.

The dependency remains gix **0.90.0**, published from Gitoxide revision
`dd39fb66e3b1551c9fe849043e39b357a9799ca1`; the registry checksum is in Cargo.lock.
No donor source snippets were copied or patched. Upgrades use an explicit
manifest/lock update and rerun differential qualification. Gitoxide's MIT license
alternative fits the unchanged GPL-2.0-only package scope; donor notices remain
in registry packages and the retained complete Rbit import. Rbit's existing
GITR-UPSTREAM.json, notices, tests and patch record moved together without changes.

[Gitoxide's status](https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md)
identifies mature object/ref/tree plumbing but incomplete higher-level workflows.
The exact pinned registry source was inspected before using its APIs. The
[git2-rs project](https://github.com/rust-lang/git2-rs) binds C libgit2 and does
not meet this replacement scope. [Jujutsu's library manifest](https://github.com/jj-vcs/jj/blob/main/lib/Cargo.toml)
was also inspected: importing another application's state/backend is unnecessary
for listing standard Git trees. Existing Sapling/Jujutsu surveys remain navigation,
not imported dependencies or measured contenders. Gitoxide wins integration fit
for this narrow scope, not a universal speed ranking or full porcelain maturity.

## Compiler and portable invocation

`lustr-toolchain.json` records Lustr 6339fbb104d6adda4be7e6be7dc637a6a9e5586b
and the produced BlueFe rustc/rustdoc at
7005dc02c9fb6674380eade443022bbc04215276 (1.102.0-dev, LLVM 23.1.5).
The bundle's Cargo is unmodified stage0 1.101.0-nightly (31d3246e0), not produced
Cargo. BlueFe source main cf66538177236a167307b07fa8f862f03a524919 has the same
compiler/library/rustdoc tree as the produced revision; no compiler was rebuilt
or modified here. Host-specific paths are deliberately absent from this contract.

Obtain or build the platform-matched produced compiler/standard libraries from
[BlueFe](https://github.com/glopyglerky/bluefe) at the produced revision using its
Rust bootstrap instructions; build the Lustr coordinator from the exact pin in
[its source](https://github.com/glopyglerky/lustr). Assemble `bin/lustr`,
`bin/cargo`, `bin/rustc`, `bin/rustdoc` and matching `lib/rustlib` together.
The bundle must include native and wasm32-unknown-unknown standard libraries.
Verify `bin/rustc -vV` and the bundle receipt against the pin before use.
Then set `BLUEFE_ROOT` to that bundle's absolute location:

```sh
"$BLUEFE_ROOT/bin/lustr" --bluefe-root "$BLUEFE_ROOT" build --locked
"$BLUEFE_ROOT/bin/lustr" --bluefe-root "$BLUEFE_ROOT" test --locked --all-features --all-targets
"$BLUEFE_ROOT/bin/lustr" --bluefe-root "$BLUEFE_ROOT" test --locked --no-default-features --all-targets
"$BLUEFE_ROOT/bin/lustr" --bluefe-root "$BLUEFE_ROOT" test --locked --doc --all-features
"$BLUEFE_ROOT/bin/lustr" --bluefe-root "$BLUEFE_ROOT" build --manifest-path crates/action-example/Cargo.toml --locked --release --target wasm32-unknown-unknown
python3 ci/gitr-redb-policy.py
```

Lustr's `--bluefe-root` selects that bundle's Cargo/rustc/rustdoc without a
stock fallback. Native Lust cannot compile arbitrary dependencies. Matching
Clippy/Cranelift and rustfmt were absent, so no strict lint/format acceptance is
claimed. The replaced inherited/stock-toolchain build workflows no longer run.
The new dispatch workflow requires a provisioned matching bundle and validates
the produced compiler revision. Linux/NAS execution is observed; macOS/Windows
paths remain in source and dispatch but were not executed or provisioned here.
This intentionally gives up automatic hosted stock-compiler checks until those
produced bundles are available, rather than labeling them BlueFe acceptance.

## Observed checks and limits

Evidence is under `rust-root-evidence/`. All-feature root tests passed **129**
checks, with six controlled child-process helpers marked ignored and invoked by
parent tests. Feature-off tests passed **123** checks with the same six helpers.
The suite includes existing stale/concurrent ref writers, corrupt assets,
interrupted IO, worker fencing and authentication boundaries. New ls-tree checks
compare SHA-1/SHA-256, recursion, NUL/raw tab names, symlink/gitlink modes,
name-only output, missing/wrong-kind revisions and corrupt objects with reference
Git. Running with an empty PATH verifies no Git subprocess is required.
A first fixture failed when overwriting a read-only loose object; it now removes
that disposable fixture file first. The first failure log is preserved.

The moved Wasm example built and real action/HTTP/workflow smoke checks passed,
including committed-source isolation, cancellation and restart behavior. One
initial workflow smoke ran after a feature-off build replaced the shared binary;
rebuilding with all features and rerunning passed. This was a qualification
ordering error, not an accepted feature-off workflow result. The final build
ordering is sequential. Historical qualification documents keep their original
compiler/evidence claims and link here for the current route.

The inspected closure uses Rust Gitoxide, zlib-rs and sha1dc 0.1.5 (Rust, not the
similarly named historical C submodule). ureq/rustls still select ring 0.17.14
with native C/assembly. OS bindings and upstream unsafe Rust remain. Selected
Git features do not include curl, OpenSSL or libgit2. Runtime ldd is insufficient
to rule out statically linked native crypto. No crypto swap was attempted without
equivalent compatibility evidence. Gitr-owned production modules forbid unsafe;
that does not establish whole-stack memory safety.

`ci/gitr-root-benchmark.py` records a fixed tree, binary hashes, commands, 24
balanced trials (first three excluded), stdout hashes, medians and seeded paired
bootstrap bounds. The baseline is the pre-move binary built with the same
produced compiler, default features and dev profile. The after binary also uses
default/dev. Stock Git is optimized. These startup-heavy warm-cache measurements
show substantial remaining Gitr overhead; they do not establish a speedup.
Memory uses separate /proc VmHWM samples and reports lower bounds; wait4 values
include inherited launcher high-water and must not rank memory. Compilation logs
record real elapsed build times separately; dependency-cache and feature-set
differences preclude a compiler-speed comparison. Release runtime measurements,
network operations, large-history workloads and exact peak-memory bounds remain
unqualified. No fastest/best universal claim follows from this migration.

The unchanged complete Rbit workflow donor at 39f932a00b7aa35b324e06a9ff2343a57d4bed76
also passed its independent 118 unit tests, one integration test and one doctest
through the same produced compiler after relocation.

Generated-output cleanup checked real open descriptors and acquired native Cargo
build locks, then deleted this worker's native, Wasm and donor build targets.
Available space increased by 1,365,381,120 bytes during cleanup; see cleanup.json
for exact before/after values and shared-filesystem attribution limits. Current
release/all-feature binaries and Wasm, one pre-move rollback binary, source and
bounded evidence remain. The shared toolchain and Cargo cache were not modified
or deleted by cleanup. No service, installed program, user data or HDD allocation
was changed.
