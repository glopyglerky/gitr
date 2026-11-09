# Gitr

Matt Mayhugh's independently maintained Git fork, owned at
[glopyglerky/gitr](https://github.com/glopyglerky/gitr), with verified fork parent
[git/git](https://github.com/git/git).

[![Gitr Rust qualification](https://github.com/glopyglerky/gitr/actions/workflows/gitr-rust.yml/badge.svg?branch=master)](https://github.com/glopyglerky/gitr/actions/workflows/gitr-rust.yml)

A Rust Git implementation with verified large-file storage, game-asset chunk
reuse, a transactional repository graph and vector catalog, and bounded Rust/Wasm
actions and dependency workflows against immutable Git snapshots. Active development
lives on the default branch, [`master`](https://github.com/glopyglerky/gitr/tree/master).

**Status: initial implementation. Gitr is not yet a complete Git replacement.**
The new implementation is in [`rust/gitr`](rust/gitr). This fork also retains the
upstream Git source and test corpus, so the entire repository is not all Rust.

## Implemented capabilities

- Native Gitoxide commands for repositories, objects, refs, history, index and
  status. Guarded `update-ref` uses expected-old-value compare-and-swap.
- Native `mktree`, `write-tree` and `commit-tree` for checkout-free history
  authoring. Object creation does not move refs or modify the index/worktree.
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

Learned semantic embeddings, resolved call graphs, distributed HA, native
clone/fetch/push, add/porcelain-commit/checkout and complete merge/rebase support
remain work. `commit-tree` creates a commit object; it is not the full porcelain
`commit` command or a content merge. Local redb multi-process writers serialize
transactions; that is not distributed consensus. The first search model uses
lexical feature hashing, not a trained code model. The bounded Wasm workflow
profile is not a general-purpose build/deployment runner, standard MCP server,
remote multi-tenant service, full forge, or complete GitHub Actions implementation.
Unsupported commands and workflow execution features fail explicitly.

The capabilities below describe checked-in implementation and linked qualification
records. This documentation update does not rerun those builds or certify a release.

## Source map

| Path | Role |
| --- | --- |
| `rust/gitr/src/main.rs` | Gitr CLI entry point |
| `rust/gitr/src/git.rs`, `git_write.rs` | Native Git operations and authoring |
| `rust/gitr/src/lfs.rs`, `remote.rs`, `chunks.rs`, `assets.rs` | LFS and large assets |
| `rust/gitr/src/db.rs`, `actions*.rs` | redb catalog, bounded Wasm jobs and optional API |
| `rust/gitr/Cargo.toml` | Standalone Gitr build, separate workspace |
| `rust/gitr-action-example`, `rust/vendor/rbit-workflow` | Example guest and retained workflow donor |
| `Documentation/gitr` | Gitr decisions, compatibility limits and qualification |
| Root C sources, `builtin`, `t`, `Makefile`, `meson.build` | Inherited Git reference implementation and tests |
| Root `Cargo.toml`, `build.rs`, `src` | Inherited C-linked `gitcore`, not the Gitr binary |

The [fork design and layout exceptions](Documentation/gitr/fork-design.md) explain
why these paths remain and how to maintain this map. The inherited
[Git README](UPSTREAM-README.md) is reference documentation.

## Build with Lustr

Use Rust 1.99.1 and the compiler revision in
[`lustr-toolchain.json`](rust/gitr/lustr-toolchain.json). From this repository root:

```sh
git clone https://github.com/glopyglerky/lustr .gitr-ci/lustr-src
git -C .gitr-ci/lustr-src checkout 62494e27717a64c8612b538409de27048fa6adc2
rustup toolchain install 1.99.1 --profile minimal
cargo +1.99.1 install --path .gitr-ci/lustr-src/tools/cargo-lust --locked
rustup run 1.99.1 lustr --engine rustc build --manifest-path rust/gitr/Cargo.toml --locked --release
rustup run 1.99.1 lustr --engine rustc test --manifest-path rust/gitr/Cargo.toml --locked
```

The executable is `rust/gitr/target/release/gitr`, or `gitr.exe` on Windows.
The explicit `rustc` route uses Lustr's full-Rust Cargo coordinator. Native Lust
does not currently compile arbitrary Gitoxide/redb dependencies. Cranelift needs
Lustr's matching backend and compiler; see [qualification](Documentation/gitr/qualification.md).

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

Build the [example Rust action](rust/gitr-action-example) for
`wasm32-unknown-unknown` using the same pinned toolchain:

```sh
rustup target add --toolchain 1.99.1 wasm32-unknown-unknown
rustup run 1.99.1 lustr --engine rustc build --manifest-path rust/gitr-action-example/Cargo.toml --locked --release --target wasm32-unknown-unknown
WASM_PATH="$(pwd)/rust/gitr-action-example/target/wasm32-unknown-unknown/release/gitr_action_example.wasm"
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
The complete [vendored Rbit workflow crate](rust/vendor/rbit-workflow) supplies
parsing and matrix planning, with retained tests, notices and source provenance.
Gitr keeps durable run/job authority in its existing redb database and executes
only the explicitly supported Wasmi profile. No external database is required.

```sh
rustup run 1.99.1 lustr --engine rustc build --manifest-path rust/gitr/Cargo.toml --locked --features workflows
rust/gitr/target/debug/gitr -C /path/to/repo actions workflow plan checks --path .gitr/workflows/check.yml
rust/gitr/target/debug/gitr -C /path/to/repo actions workflow submit checks --path .gitr/workflows/check.yml
rust/gitr/target/debug/gitr -C /path/to/repo actions workflow run checks
rust/gitr/target/debug/gitr -C /path/to/repo actions workflow status checks
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
rustup run 1.99.1 lustr --engine rustc build --manifest-path rust/gitr/Cargo.toml --locked --features action-api
export GITR_API_TOKEN="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
rust/gitr/target/debug/gitr -C /path/to/repo actions serve --repository my-repository
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

The authored Rust package currently declares GPL-2.0-only. The project owner
has authorized a different license for code they control where lawful to enable
the best product; existing declarations require a scoped provenance review before
changing. See the [license assessment](Documentation/gitr/research/2026-11-08-upstream-survey/licensing-review.md).
Upstream licenses remain intact. Gitoxide and the vendored Rbit workflow crate use
their offered MIT alternatives; both Rbit license notices are retained. No libgit2
or inherited C Git library is linked into this package. TLS currently uses rustls
with ring, whose cryptographic implementation includes native code. See the
catalog before incorporating differently licensed upstream source.

## Toolchain and memory-safety direction

Full native Rust conversion and 100% memory-safe Rust through Lustr/BlueFe are
future goals. Gitr's manifest forbids unsafe code in its own crate, which does
not prove that dependencies or the complete artifact contain no unsafe or native
code. The retained Git reference includes C, shell, Perl, Python and Tcl/Tk.
The current Lustr `rustc` engine coordinates Cargo/rustc; it does not establish
that native Lustr/BlueFe builds those non-Rust components. Working Git behavior
and compatibility remain the basis for any later replacement.
