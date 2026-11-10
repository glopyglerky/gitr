# Rit

Rit assembles an upstream Rust implementation of Git before adding database,
forge, or performance extensions. Active development is on
[`master`](https://github.com/glopyglerky/rit/tree/master). The project was renamed
from Gitr to Rit; see the [rename record](Documentation/rit/rename.md) for the
current executable and CI names. Historical reports and archived source remain
under `Documentation/gitr` with the names used when they were recorded.

**The default executable now uses the Sley engine and Git-compatible frontend.
Full Git parity is not complete or certified.**

The import pins [HeddleCo/sley](https://github.com/HeddleCo/sley) at
[`a71654f50de24b3c750f0df5f1829db7342491f6`](https://github.com/HeddleCo/sley/tree/a71654f50de24b3c750f0df5f1829db7342491f6),
workspace version 0.13.0. Its coherent engine crates, command implementation,
and Rust tests are kept under `vendor/sley`. Rit calls that implementation in
process. It does not silently delegate unsupported commands to installed C Git.

## What this integration establishes

The local qualification exercises the actual `rit` binary with Git absent from
its `PATH`. It covers initialization, staging, commits, branches, merge, rebase,
stash, worktrees, local clone/fetch/push/pull, and checked reference updates in
SHA-1 and SHA-256 repositories. It also checks merge-conflict abort and raw Unix
filename staging. A separately invoked Git 2.55.0 checks repository integrity,
refs, index entries, and worktree state. These fixtures establish the operations
they exercise, not every flag or error case of those commands.

Local patches reject stale reftable transactions under the publication lock,
release ref locks on failure, and prevent full repack from pruning around missing
required objects. They also preserve raw filename bytes during staging and add
a native `upload-archive` service. See the [import and qualification record](Documentation/gitr/sley-integration.md)
for exact checks, limitations, compiler routes, and source hashes.

The [coverage continuation](Documentation/gitr/git-coverage-continuation.md)
adds native remote archive requests over SSH and Git transport, checks explicit
service failures, and fixes symbolic reference-selector output. It records
command-level qualification and the remaining archive, object-format, transport,
and platform gaps.

The [history and commit coverage record](Documentation/gitr/history-and-commit-coverage.md)
adds oversized-file commit summaries, threshold validation, and corrected parent
lists for path-limited revision history. It retains results from ten upstream
scripts in both object formats, including the remaining failures and skips.

The [sparse history coverage record](Documentation/rit/sparse-history-coverage.md)
adds explicit starting-tip preservation and sparse path-limited history in `log`
and `rev-list`. The selected upstream graph script now passes 10 of 12 cases in
both object formats; decoration and boundary-order differences remain recorded.

The [ancestry-path coverage record](Documentation/rit/ancestry-path-coverage.md)
adds explicit ancestry anchors, correct criss-cross selection, and export-mark
connectivity. The selected ancestry script now passes all 18 cases in both
object formats. The report retains the neighboring script results, independent
reviews, and remaining option-combination gaps.

The upstream README reports an exact conformance floor of **557 of 891 selected
Git 2.55.0 test scripts**. That is upstream evidence, not Rit's measured result.
The pinned Git source contains **1,046 numbered test scripts**. Sley's manifest
excludes 155 from its 891-script selection, so even passing that entire selection
would not establish full Git coverage. We do not turn a passing assertion count
into a claim of complete command semantics. The full upstream conformance suite,
network authentication, platform matrix, and performance certification remain
required work.

## Build and run

Use the pinned [Lustr coordinator](https://github.com/glopyglerky/lustr) with
Rust 1.99.1 or a matching produced BlueFe bundle. The current native Lust subset
does not compile this complete dependency graph.

```sh
lustr build --locked
lustr test --locked --all-features --all-targets

target/debug/rit -C /path/to/repository status --porcelain=v1
target/debug/rit -C /path/to/repository log --oneline -n 10
target/debug/rit clone /path/to/source /path/to/clone
```

The same entrypoint builds as `rit` and `git`, with a `scalar` companion.
No command above replaces the Git installed on your computer. Keep the generated
`git` binary separate while qualification is incomplete.

`RIT_REFERENCE_GIT` selects an absolute reference Git path for root tests.
`RIT_TEST_BIN` selects the actual Rit executable for selected donor regression
tests. The legacy `GITR_REFERENCE_GIT` and `GITR_TEST_BIN` settings are accepted
when the corresponding `RIT_` setting is absent. Selected donor conformance
tests require `SLEY_TEST_GIT` to point to Git 2.55.x.
The native qualification workflow builds the exact Git 2.55.0 oracle outside
tracked source and uses the pinned produced BlueFe bundle through Lustr.
Local Lustr/rustc checks and hosted BlueFe checks are recorded separately.

## Read a committed source snapshot

The explicit Rit extension reads a committed tree directly from the existing
Git object store, without materializing a checkout:

```sh
rit --rit-snapshot inspect --repo /path/to/repository --revision HEAD
rit --rit-snapshot list --repo /path/to/repository --revision HEAD --path src
rit --rit-snapshot read --repo /path/to/repository --revision HEAD --path src/main.rs
```

Use the commit ID returned by `inspect` for later CLI calls that must keep the
same revision. The public `rit::snapshot::Snapshot` Rust API resolves once and
keeps that identity across reads. Metadata commands return JSON; `read` returns
verified raw blob bytes. `--path-hex` preserves arbitrary Git path bytes.

This implements the source reader described in the
[snapshot implementation record](Documentation/rit/snapshot-reader.md).
It is a local read interface, with no retention lease, writable overlay, mounted
filesystem, or active database. Symlinks return target bytes, checkout filters
are not applied, and unavailable objects fail. See the record for size limits
and the four ambient storage variables that must be unset.

## Delivery order

1. Establish native Git behavior and interoperability against a pinned oracle.
2. Close the remaining conformance differences, including remote and platform
   qualification. Unsupported behavior stays explicit.
3. Integrate redb behind stable Git contracts and measure any speed changes.

The [storage architecture assessment](Documentation/rit/storage-architecture-review.md)
reviews redb, Fjall, SlateDB, immutable payload storage, and lazy CI snapshots.
It retains redb as the future metadata choice and defines the measurements and
durability requirements for later integration. It does not activate a new backend.

The former redb catalog, object-store, LFS, Wasm action, and forge prototypes,
including their tests and manifests, are preserved under
[`Documentation/gitr/archive/database-first`](Documentation/gitr/archive/database-first).
The old Grit and Rbit donors remain available for selective reuse. They are
outside the new executable's dependency graph. No existing repository database
is opened, converted, or deleted by this source migration. The old `db`,
`assets`, `lfs`, `actions`, and `doctor` extensions are not active commands in
this Git-first frontend. Redb remains the selected future database.

## Source map

| Path | Purpose |
| --- | --- |
| `src/lib.rs`, `src/main.rs`, `src/bin` | Thin Rit, Git, and Scalar entrypoints |
| `vendor/sley/crates` | Imported Git engine, frontend, and Rust tests |
| `vendor/sley/NOTICE.md`, `Documentation/gitr/sley-import.json` | Upstream identity, file origins, exclusions, and local patches |
| `tests/git_workflows.rs` | Product workflows with no Git executable on candidate PATH |
| `examples/verify_import.rs` | Verify the complete imported file inventory and hashes |
| `ci/qualify_native.rs`, `xtask` | Rust build, test, provenance, and policy coordinators |
| `Documentation/gitr/archive/database-first` | Deferred prototype with its original source and tests |

The inherited C Git production tree remains in Git history. It is not restored
as the product implementation or bundled as a fallback.

## Licensing and non-Rust boundaries

Rit-controlled code is available under MIT OR Apache-2.0. Sley declares
Apache-2.0. Historical Grit's GPL-2.0-only CLI is retained as source but is no
longer linked into the default executable. Donor licenses are not rewritten.
See [LICENSE](LICENSE) and the [license map](Documentation/gitr/licensing.md).

The Git engine is Rust. Some compatibility behavior still requires shell:
small installed helper adapters, user-requested hooks and filters, and
generated conformance fixtures. The imported shell `filter-branch` engine and
`git-sh-i18n` library were excluded after source-origin review found substantial
overlap with Git scripts whose notices were absent. `filter-branch` currently
fails explicitly before modifying a repository; the shell i18n library is not
materialized. These exceptions are documented in the
[source contract](Documentation/gitr/rust-only-source.md). TLS dependencies
also include native cryptographic code, and some dependencies contain unsafe
Rust. This is not a claim that the complete artifact is exclusively Rust or
memory safe.
