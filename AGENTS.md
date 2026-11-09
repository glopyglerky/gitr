# Gitr development contract

## Product

Gitr is Matt Mayhugh's Rust fork and rewrite of Git. Keep active work on the GitHub default branch, `master`. Commit implemented, tested changes directly to `master` when safe; never leave validated work stranded on `main`.
Use current upstream Rust implementations where they provide correct, maintainable
behavior. Priorities are Git and GitHub interoperability, ordinary development
friction, large binary assets, game development, and safe concurrent writers.
Build and qualify the Rust port with the pinned Lustr compiler coordinator.

The user also wants a database-centered repository with relational, graph and
vector search, automatic local embeddings, lightweight operation, and high
availability. Preserve this requirement. Treat the checked-in database design
and compatibility matrix as contracts, not evidence that unfinished features work.

## Database: redb

The user's latest instruction is **use redb**. It supersedes the earlier
SurrealDB/SurrealKV/SurrealDS selection. redb is Gitr's selected database, not a
fallback or a temporary migration source. Retain the reviewed redb 4.5.0 pin and
lockfile unless a separately tested dependency update changes them. See
`Documentation/gitr/redb-decision.md`.

Keep the existing redb repository catalog and durable action database. Preserve
all data, verified exports, transaction rollback, compare-and-swap, idempotency,
worker fencing and atomic artifact/receipt publication. Do not convert or delete
existing databases, impose a replacement service, or activate SurrealDB candidate
artifacts. Retired experiments and their licenses are preserved under
`Documentation/gitr/archive/surrealdb` for historical reference only.

Build relational, graph and vector indexing as Gitr capabilities over redb with
rebuildable, revision-addressed indexes. redb alone does not provide semantic
embeddings, a query language or resolved code relationships. Preserve exact Git
object identities and source provenance. Large Git/LFS/chunk payloads remain
bounded-memory streaming data unless measurements justify a different layout.

Retain the Rust/Wasmi action runtime. Store code, inputs, jobs and receipts in
redb; execute untrusted guests outside write transactions. Do not introduce
Surrealism, SurrealDS or a vendor database license as a dependency of Gitr.
Multi-host HA remains a separate Rust replication/consensus implementation and
qualification requirement; local cooperating redb writers are not HA. Never
share a writable redb file between machines as a substitute for replication.

## Integrated forge, actions and control

The required product is one self-contained Rust Git implementation, repository
service, coding forge, CI/CD/action worker and code-intelligence system. It must
eventually replace the useful Git/Gitea/Forgejo capabilities and match or exceed
Codebase Memory against reproducible source-grounded qualification. A lexical
vector catalog is not resolved code intelligence or evidence of that parity.
See `Documentation/gitr/forge-runtime-plan.md` for the complete acceptance gates.

Use suitable upstream Rust/Wasm workflow and forge components. Study Anthropic
Scion, Meta Sapling/Mononoke/EdenFS and other established alternatives, while
preserving local/offline Git compatibility rather than importing their unique
infrastructure assumptions. Learn from OmnidiaDB but keep Gitr independently
installable, licensed, optimized and free of mandatory Omnidia services. Do not
copy private/proprietary source into this public repository without an explicit
publication and license decision.

Durable code, action inputs, claims, results and forge entities belong in the
repository database. Compile and execute untrusted Wasm outside database write
transactions. Keep guest capabilities explicit and deny ambient host authority.
Publish artifacts and their receipt atomically; fence expired/cancelled workers.
Execution retries are not an exactly-once guarantee for external deployment
side effects. A local writer lock is not distributed consensus or HA.

Stateless Rust API and official MCP adapters must share typed, repository-scoped
authorization and operation services. Sessions and workers must not own durable
state or write authority. A local JSON control command is not an authenticated
HTTP API or a standards-compliant MCP transport. GitHub workflow syntax support
is not automatically compatibility with JavaScript, Docker or shell actions.

Native everyday Git completion remains a delivery track alongside the forge;
do not replace implementation with an indefinitely growing research catalog.

## Licensing and upstream reuse

The user authorizes choosing a different license for Gitr-controlled code where
lawful when that enables the best product. Do not exclude Apache, GPL or AGPL
donors solely because the current Rust manifest declares GPL-2.0-only. Assess
their code quality and integration fit, then select a compatible composition.

Complete source copies, whole-crate/package-family imports and whole application
forks are explicitly permitted when upstream terms and source provenance allow
them. Prefer retaining coherent upstream modules, tests, fuzz fixtures and notices
over unnecessary reimplementation or fragile snippet extraction. Record the exact
upstream revision, imported paths/hashes, selected license alternatives, build
closure, local patches, qualification and update method. Gitr still owns its ref,
job and storage authority: a large donor import must not introduce competing
masters or discard the selected redb/Wasmi/Git compatibility contracts.

Preserve the inherited Git GPLv2-only scope and all donor notices and obligations.
Before changing existing license declarations, record file-level source origin,
copyright/relicensing authority and the resolved build/release composition.
Commit authorship or a separate Rust directory alone does not establish those
rights. For new independently authored reusable Rust components, MIT OR
Apache-2.0 is a practical default; selected donor terms may require another scope.
The broad donor catalog and scoped license assessment are in
`Documentation/gitr/research/2026-11-08-upstream-survey`.
The expanded import-sized recommendations, source findings and reproducible
combined catalog are linked from `Documentation/gitr/upstream-reuse-expanded.md`.
A dependency allowlist or source-only survey is not legal approval or executed
runtime qualification; preserve that distinction in reports and release gates.

## Repository layout

- Root `src`, `Cargo.toml` and `Cargo.lock` build the standalone Rust Gitr executable.
- `crates/action-example` and `vendor/rbit-workflow` are isolated support packages.
- Inherited C Git production/build/test sources were retired; recover them from
  `bf628ff8c74da4ab03a4a99001edf6780d8d84a0`. Reference tests use installed Git.
- `Documentation/gitr` records decisions, upstream reuse, qualification evidence,
  and remaining implementation work. Its `archive` is not an active build target.

## Correctness

- Standard Git object bytes, object IDs, refs, index/worktree semantics and Git
  wire protocols are compatibility contracts. Derived embeddings never identify
  or replace Git objects and never decide history or merge correctness.
- A writable ref requires compare-and-swap against an expected old value. No
  silent last-writer-wins publication. Distributed authoritative refs additionally
  need durable quorum replication and fencing before acknowledgement.
- Blob and LFS caches use bounded-memory streaming, content verification and
  atomic publication. Never acknowledge a pointer to partial local data.
- LFS wire pointers and transfers must remain compatible with ordinary Git LFS.
  Do not claim a client-side chunk cache changes GitHub's transfer protocol.
- Read-only repository inspection must not execute repository-provided filters,
  hooks, textconv commands or credential helpers.
- Keep credentials out of logs, source files and command-line arguments. Do not
  forward endpoint authorization to a different origin or follow redirects with it.
- Unimplemented commands fail explicitly. Never silently fall back to C Git and
  describe that execution as native Rust.
- Preserve upstream licenses and provenance. A Rust wrapper over a C library is
  not a Rust replacement. Record native transitive dependencies honestly.

## Qualification

Use Lustr for build and test. Record the selected engine. The `rustc` route uses
Cargo/rustc; native Lust does not currently compile arbitrary Rust dependencies.
Cranelift qualification must use Lustr's pinned backend and matching compiler.

Differential tests compare supported operations with reference Git. Test hostile
and malformed objects, stale writes, concurrent publication, corrupt asset data,
interrupted IO and authentication boundaries where the change touches them.
Record actual executed checks and distinguish CI configuration from observed CI
results. Performance claims need a reproducible workload and measured results.
Run `python ci/gitr-redb-policy.py` to check the active backend dependency policy.
