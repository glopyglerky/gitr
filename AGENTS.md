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

## Preferred database target

Use SurrealDB 4.0.0 with SurrealKV for local/embedded storage; evaluate SurrealDS
as a separate, optional licensed distributed backend, not a replication layer
added to the same SurrealKV directory. See `Documentation/gitr/surrealdb-decision.md`.
The current working redb catalog/action implementation is transition evidence,
not a decision to reject the user's preferred stack. Never destroy or silently
convert existing data. Qualify storage contracts, migration and rollback first.

SurrealKV has one datastore owner with many concurrent transactions/client clones.
Multiple CLI/worker/API/MCP processes must use the owner service rather than each
opening the same directory. Preserve durable compare-and-swap, fencing and
idempotency, including cross-record invariants under snapshot isolation. Enforce
explicit durability and do not advertise SurrealKV as multi-host HA.

Surrealism is the upstream Rust/Wasm extension candidate for trusted bounded
in-database functions. Long or untrusted CI execution stays outside write
transactions. Keep large streaming Git/LFS/chunk payloads out of ordinary rows
unless workload measurements justify a different physical layout. Preserve exact
Git identities, source-grounded search and local/offline operation.

Core BSL licensing and inherited GPL-2.0-only distribution require review; SDK
licensing does not relicense the engine. Do not silently relicense inherited code.
SurrealDS needs a vendor-issued Enterprise license. Offline license files exist
when offline use is included; online key validation is not universal. Do not
purchase/configure licenses or claim DS deployment/HA qualification without evidence.

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

## Repository layout

- `rust/gitr` is the standalone Rust implementation. Its manifest does not link
  upstream Git's C `gitcore` library.
- The root currently retains upstream Git and its test corpus for compatibility
  work. Do not describe this inherited source tree as an all-Rust repository.
- `Documentation/gitr` records decisions, upstream reuse, qualification evidence,
  and remaining implementation work.

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
