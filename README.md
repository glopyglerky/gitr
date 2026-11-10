# Gitr

Independently maintained Git fork, owned at
[glopyglerky/gitr](https://github.com/glopyglerky/gitr), with verified fork parent
[git/git](https://github.com/git/git).

[![Native Git qualification with produced BlueFe](https://github.com/glopyglerky/gitr/actions/workflows/gitr-native.yml/badge.svg?branch=master)](https://github.com/glopyglerky/gitr/actions/workflows/gitr-native.yml)

A Rust Git implementation with verified large-file storage, game-asset chunk
reuse, a transactional repository graph and vector catalog, and bounded Rust/Wasm
actions and dependency workflows against immutable Git snapshots. Active development
lives on the default branch, [`master`](https://github.com/glopyglerky/gitr/tree/master).

**Status: the historical Grit Rust CLI, library and test support are imported;
100% Git parity and a general speed advantage are not established.**
The default executable builds the imported Rust CLI together with Gitr's redb
storage, catalog and actions. Source coverage and enabled product operations are
different: reference-changing commands remain disabled until their publication
paths meet Gitr's expected-old-value transaction contract. See the
[import decision, exact source lineage and capability boundary](Documentation/gitr/rust-git-import.md).
The [executed native qualification](Documentation/gitr/native-git-qualification.md)
passed 146 root tests and 705 imported tests through produced BlueFe/Lustr. Its
four-workload release benchmark found faster blob hashing and slower status, log
and tree listing than reference Git; exact samples and limits are retained.

The inherited C Git build,
production sources and unused upstream shell corpus have been retired from the
working tree; their exact recovery point remains in Git history. The GitHub fork
relationship is unchanged. Transitive native TLS code remains (see dependency limits).

## Implemented capabilities

- In-process Rust Git commands from the vendored Grit engine, including staging,
  commits, simple branches/tags, ordinary local branch checkout/switch, detached
  checkout, object plumbing and repository inspection. Reference publication
  checks the expected value under its lock; checkout also locks the index before
  changing worktree files.
- The earlier isolated Gitoxide profile remains available with `--isolated-git`.
  The default `update-ref` route retains that profile's strict compare-and-swap.
- An opt-in redb 4.5 object store, with exact SHA-1/SHA-256 identities, atomic
  chunked writes and durable standard Git loose-object mirrors. Raw blob hashing
  and writing use bounded-memory streaming.
- Native `mktree`, `write-tree` and `commit-tree` for checkout-free history
  authoring. These do not move refs or change the worktree; native `write-tree`
  may update the index's cache-tree metadata.
- Standard Git LFS clean/smudge filters, verified immutable cache, explicit HTTPS
  basic transfers and the file-lock API.
- FastCDC local chunk reuse across large asset revisions, with verified hydration
  into the standard Git LFS cache.
- Large-asset inventory and candidate Unreal/Unity `.gitattributes` rules.
- redb snapshots keyed by exact Git commits, factual Git graph queries, and
  automatically generated lexical vectors with exact token matching for local search.
- Content-addressed Wasm action modules, immutable committed-source inputs,
  idempotent job submissions, durable fenced worker claims, and atomic
  artifact/receipt publication in the local database.
- Optional committed-YAML workflow planning using the complete vendored Rbit
  workflow crate, with durable dependencies, matrix limits, failure propagation
  and cancellation enforced by the same redb worker claims.
- An optional authenticated local HTTP action API over the same redb database
  and execution methods as the CLI; no external database server or data migration.
- Builds and tests through a pinned version of [Lustr](https://github.com/glopyglerky/lustr).

Learned semantic embeddings, resolved call graphs and distributed HA remain work.
The clone/fetch/push, advanced checkout, merge/rebase and other history-writing algorithms
are now checked in; their default Gitr command routes remain disabled pending
the remaining reference-transaction migration and qualification. `commit-tree`
creates a commit object without moving refs. Local redb multi-process writers serialize
transactions; that is not distributed consensus. The first search model uses
lexical feature hashing, not a trained code model. The bounded Wasm workflow
profile is not a general-purpose build/deployment runner, standard MCP server,
remote multi-tenant service, full forge, or complete GitHub Actions implementation.
Unsupported commands and workflow execution features fail explicitly.

The capabilities below describe checked-in implementation. The root migration
was built and tested through produced BlueFe; see its linked qualification record.
These checks do not certify complete Git parity or a public release.

## Source map

| Path | Role |
| --- | --- |
| `src/main.rs` | Gitr CLI entry point |
| `src/native_cli.rs` | Default native dispatcher and isolated-profile selection |
| `vendor/grit/grit-git`, `vendor/grit/grit-lib`, `vendor/grit/grit-test-support` | Imported Rust Git CLI, engine and test support |
| `vendor/grit/upstream-source.json`, `vendor/grit/NOTICE.md` | Exact donor revision, original hashes and license/provenance notices |
| `src/git.rs`, `git_write.rs` | Isolated Gitoxide operations and authoring |
| `src/lfs.rs`, `remote.rs`, `chunks.rs`, `assets.rs` | LFS and large assets |
| `src/db.rs`, `actions*.rs` | redb catalog, bounded Wasm jobs and optional API |
| `Cargo.toml` | Default Gitr executable build |
| `crates/action-example`, `vendor/rbit-workflow` | Example guest and retained workflow donor |
| `Documentation/gitr` | Gitr decisions, compatibility limits and qualification |
| `tests` | Self-contained differential fixtures using installed reference Git |
| `Documentation/gitr/root-retirement.json` | Exact retired paths and recovery blob IDs |

The [fork design and source retirement](Documentation/gitr/fork-design.md) explain
the current boundaries and how to maintain this map. The inherited
[Git README](UPSTREAM-README.md) is historical text for the retired tree.

## Build with Lustr

Use the reviewed produced BlueFe bundle described in
[`lustr-toolchain.json`](lustr-toolchain.json). `BLUEFE_ROOT` is an absolute path
to a platform-matched bundle containing `bin/lustr`, `bin/cargo`, produced
`bin/rustc`, produced `bin/rustdoc`, and native/Wasm standard libraries:

```sh
"$BLUEFE_ROOT/bin/lustr" --bluefe-root "$BLUEFE_ROOT" build --locked --release
"$BLUEFE_ROOT/bin/lustr" --bluefe-root "$BLUEFE_ROOT" test --locked --all-features --all-targets
```

The default executable is `target/release/gitr` (`gitr.exe` on Windows).
See [root migration qualification](Documentation/gitr/rust-root.md) for the
portable bundle contract, actual Linux results, source retirement and limits.
Native Lust does not compile this dependency graph. No matching Clippy/Cranelift
is available; those routes are not accepted. macOS/Windows remain supported code
paths requiring matching bundles and execution before acceptance.

## Try it

With the built executable on `PATH`:

```sh
gitr -C /path/to/repo doctor
gitr -C /path/to/repo log --oneline -n 10
gitr -C /path/to/repo status --porcelain
gitr -C /path/to/repo assets scan --threshold-bytes 52428800
gitr assets attributes unreal

gitr -C /path/to/repo db index HEAD
gitr -C /path/to/repo db search "asset upload" --revision HEAD --limit 10
gitr -C /path/to/repo db graph --revision HEAD --path src/main.rs

gitr -C /path/to/repo assets import Content/World.uasset
# Use the returned full-file OID and size to materialize the standard LFS cache:
gitr -C /path/to/repo assets hydrate OID SIZE
```

For the database-backed object path, explicitly enable it in a repository:

```sh
gitr -C /path/to/repo config gitr.redbObjects true
gitr -C /path/to/repo hash-object --no-filters -w path/to/blob
```

Objects are committed to `<common-git-dir>/gitr/objects.redb` before a standard
loose-object mirror is acknowledged. Existing packs are readable but are not
automatically imported. Refs/indexes remain in Git's format, and database pruning
and pack ingestion still need integration. See the [storage boundaries](Documentation/gitr/rust-git-import.md#redb-object-storage)
before treating the database as a replacement for the whole repository.

`db index` reads committed objects, not uncommitted worktree files. Search requires
that exact indexed commit and reports excluded content. The graph currently
describes repository membership, not language-resolved calls. Commands emit JSON
or JSON Lines where a structured report is useful.

For native authoring, `hash-object` supplies blob IDs, `mktree` constructs trees
(or `write-tree` reads staged content), and `commit-tree` writes a commit. Publish
separately with an expected-old-value `update-ref`. The
[native authoring guide and executed evidence](Documentation/gitr/native-history.md)
show a SHA-1/SHA-256 workflow that continues through redb indexing and Wasm source
execution without a Git subprocess or source checkout. No native GitHub network
roundtrip is claimed by that local qualification.

The large-file layer preserves whole-file SHA-256 Git LFS pointers. GitHub still
receives whole LFS objects. Chunk transfer requires a separately negotiated server
protocol. To use explicit standard LFS transfers:

```sh
gitr -C /path/to/repo lfs import Content/World.uasset
gitr -C /path/to/repo lfs upload OID SIZE --endpoint https://github.com/OWNER/REPO.git/info/lfs
gitr -C /path/to/repo lfs fetch OID SIZE --endpoint https://github.com/OWNER/REPO.git/info/lfs
```

Authenticated requests read `GITR_LFS_AUTHORIZATION` from the environment. Do not
put a token in source files or command arguments. Endpoint discovery, credential
helpers, automatic pre-push traversal and enforcement of asset locks are not yet
wired into a complete native push workflow.

## Database: redb

**Gitr uses redb.** The current reviewed pin is **4.5.0**. The repository catalog
and action state already use it; redb is the selected backend, not a temporary
fallback. The earlier SurrealDB/SurrealKV/SurrealDS migration direction is retired.
No existing repository data is converted or deleted.

The rebuildable catalog and durable action state retain their separate files in
`<common-git-dir>/gitr/`. Relational/code-graph queries, learned vector retrieval,
forge services and distributed replication remain Gitr implementation work over
this storage substrate. Selecting redb does not make those capabilities complete
or turn local writer coordination into multi-host HA.

See the [redb decision](Documentation/gitr/redb-decision.md). Earlier database
experiments are preserved in a [non-active archive](Documentation/gitr/archive/surrealdb),
outside the active Rust packages and GitHub workflows. A
[backend-policy check](.github/workflows/gitr-redb-policy.yml) rejects their
reintroduction into the active dependency manifests or migration paths.

## Run a database-resident Rust/Wasm action

Build the [example Rust action](crates/action-example) for
`wasm32-unknown-unknown` using the same pinned toolchain:

```sh
# Bundle must include wasm32-unknown-unknown std.
"$BLUEFE_ROOT/bin/lustr" --bluefe-root "$BLUEFE_ROOT" build --manifest-path crates/action-example/Cargo.toml --locked --release --target wasm32-unknown-unknown
WASM_PATH="$(pwd)/crates/action-example/target/wasm32-unknown-unknown/release/gitr_action_example.wasm"
gitr -C /path/to/repo actions install "$WASM_PATH"
# The install response prints module_sha256:
gitr -C /path/to/repo actions submit whitespace-policy MODULE_SHA256 --revision HEAD --path src/main.rs
gitr -C /path/to/repo actions run whitespace-policy
gitr -C /path/to/repo actions status whitespace-policy
gitr -C /path/to/repo actions artifact whitespace-policy
# Export includes source and action module bytes: store the backup privately.
umask 077
gitr -C /path/to/repo actions export > /secure/backups/gitr-actions.jsonl
```

The export verifies action object hashes and their receipts; it refuses snapshots
with running workers. It is a read-only export, not an authority cutover or
proof of a fully qualified restore facility, and does not change stored records.

Action inputs are the bytes in the named **committed Git revision**, not dirty
worktree contents. A failed policy test has no downloadable successful artifact.
The local `actions control` JSON command is not a standards-compliant MCP/HTTP
transport and assumes OS-level authorization. In-process Wasm fuel and guest
memory limits do not impose a hard whole-process memory or wall-time budget.

See [action execution evidence](Documentation/gitr/actions-qualification.md)
for the measured tests and remaining restrictions.

## Durable dependency and matrix workflows

Build with `--features workflows` to enable planning/submission of committed YAML.
The complete [vendored Rbit workflow crate](vendor/rbit-workflow) supplies
parsing and matrix planning, with retained tests, notices and source provenance.
Gitr keeps durable run/job authority in its existing redb database and executes
only the explicitly supported Wasmi profile. No external database is required.

```sh
"$BLUEFE_ROOT/bin/lustr" --bluefe-root "$BLUEFE_ROOT" build --manifest-path Cargo.toml --locked --features workflows
target/debug/gitr -C /path/to/repo actions workflow plan checks --path .gitr/workflows/check.yml
target/debug/gitr -C /path/to/repo actions workflow submit checks --path .gitr/workflows/check.yml
target/debug/gitr -C /path/to/repo actions workflow run checks
target/debug/gitr -C /path/to/repo actions workflow status checks
```

The supported execution profile is manual `workflow_dispatch`, static matrices,
`needs`, `max-parallel`, `fail-fast`, and one installed `gitr/wasm@v1` module per
job using `runs-on: gitr-wasmi-v1`. See the [YAML example, bounds and executed
evidence](Documentation/gitr/rbit-workflows.md). Shell/JavaScript/Docker actions,
secret environments, arbitrary conditions and deployment capabilities are rejected,
not silently emulated. The driver returns when other workers hold remaining work;
it is not a background scheduler daemon.

Dependency and cancellation gates remain active even without the optional planner
feature. Direct CLI/HTTP action execution cannot bypass them. Older executables
refuse managed workflow jobs instead of ignoring the new ownership metadata.
Workflow exports use v2 to include exact plans and ownership; standalone-only
exports remain v1. Existing databases and standalone actions are preserved.

## Optional local action API

Build with `--features action-api` to add `gitr actions serve`. The default CLI
build does not enable this feature and ordinary commands never start a listener.
The API uses the same action records as native CLI processes, not a replacement
datastore or in-memory session state. Combine `--features workflows,action-api`
for workflow planning and the local individual-action HTTP surface; no workflow
submission endpoint or MCP transport is claimed.

```sh
"$BLUEFE_ROOT/bin/lustr" --bluefe-root "$BLUEFE_ROOT" build --manifest-path Cargo.toml --locked --features action-api
# Set GITR_API_TOKEN to a securely generated secret supplied by your secret manager.
target/debug/gitr -C /path/to/repo actions serve --repository my-repository
```

Startup prints the selected loopback endpoint. Every request requires the bearer
token and matching repository header. The service supports installation,
submission, status, cancellation, execution and artifact retrieval. It rejects
non-loopback listeners, browser Origin headers, oversized requests, raw queries
and caller-fabricated worker completion records. See the
[protocol, executed Linux qualification and limitations](Documentation/gitr/action-api.md).
This is a trusted-local-user profile, not an Internet-facing forge or standard MCP
transport. Platform success must be read from its actual CI run, not inferred from
the configured Linux/macOS/Windows matrix.

## Design and evidence

- [Compatibility matrix and exact limits](Documentation/gitr/compatibility.md)
- [Historical Grit import, redb integration and qualification](Documentation/gitr/rust-git-import.md)
- [Executed qualification](Documentation/gitr/qualification.md)
- [Native tree/commit authoring and end-to-end redb actions](Documentation/gitr/native-history.md)
- [Rbit workflows, transaction gates and execution evidence](Documentation/gitr/rbit-workflows.md)
- [Selected redb backend](Documentation/gitr/redb-decision.md)
- [Repository database, multi-writer and HA design](Documentation/gitr/architecture.md)
- [Git, Lore and asset upstream catalog](Documentation/gitr/upstreams.md)
- [Broad Rust upstream, fork and wholesale reuse survey](Documentation/gitr/research/2026-11-08-upstream-survey/README.md)
- [Rust database and retrieval comparison](Documentation/gitr/database-research.md)
- [Closest similar projects: Libra, Git-vdb, GitQL and Codebase Memory](Documentation/gitr/similar-projects.md)
- [Libra component review and native push integration plan](Documentation/gitr/libra-integration.md)
- [Next implementation work and acceptance gates](Documentation/gitr/roadmap.md)

## Licensing

Gitr-controlled contributions in the [reviewed source scope](Documentation/gitr/license-scope.json)
are available under **MIT OR Apache-2.0**. The project uses permissive terms for
its own code so future implementations can adopt the compatible upstream code
they need. See [LICENSE](LICENSE) and the [licensing and import policy](Documentation/gitr/licensing.md).

The **current combined executable remains subject to GPL-2.0-only** because it
links the historical Grit CLI. The new grant does not relicense that CLI or make
Apache-only code compatible with the unchanged combined build. Third-party
imports retain their original terms and notices. Grit's library/test support are
MIT; Gitoxide and Rbit can use their offered MIT alternatives in this build.
Both Rbit license texts and all Grit notices remain intact. A future composition
can use a different compatible product license after resolving inherited terms.

No libgit2 or inherited C Git library is linked into this package. TLS uses
rustls with ring, whose cryptographic implementation includes native code.

## Toolchain and memory-safety direction

Full native Rust conversion and 100% memory-safe Rust through Lustr/BlueFe are
future goals. Gitr's manifest forbids unsafe code in its own crate, which does
not prove that dependencies or the complete artifact contain no unsafe or native
code. The donor uses some platform FFI and transitive cryptographic dependencies
include native code. The former C Git reference tree is retained in Git history.
The current Lustr `rustc` engine coordinates Cargo/rustc; it does not establish
that native Lustr/BlueFe builds those non-Rust components. Working Git behavior
and compatibility remain the basis for any later replacement.

The [Rust-only source contract](Documentation/gitr/rust-only-source.md) covers
implementation, tests and qualification helpers, including archived programs.
It records retired helper gaps and external native dependencies separately.
