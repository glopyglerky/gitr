# Gitr development contract

## Product

Gitr is Matt Mayhugh's Rust fork and rewrite of Git. Keep active work on `main`.
Use current upstream Rust implementations where they provide correct, maintainable
behavior. Priorities are Git and GitHub interoperability, ordinary development
friction, large binary assets, game development, and safe concurrent writers.
Build and qualify the Rust port with the pinned Lustr compiler coordinator.

The user also wants a database-centered repository with relational, graph and
vector search, automatic local embeddings, lightweight operation, and high
availability. Preserve this requirement. Treat the checked-in database design
and compatibility matrix as contracts, not evidence that unfinished features work.

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
