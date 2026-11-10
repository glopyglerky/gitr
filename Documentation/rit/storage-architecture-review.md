# Rit storage architecture assessment

Reviewed November 10, 2026 against Rit
[`3ef9ba3`](https://github.com/glopyglerky/rit/commit/3ef9ba33aabc0df2bd5f5ffbc08a3cd1ada4d3d7).
This is a research assessment and proposed later architecture. It does not
activate a database, replace the selected redb backend, or change the priority
of full native Git compatibility.

The subsequent [snapshot reader implementation](snapshot-reader.md) delivers
the first local source-read interface over the existing Git object store.
The database, writable overlay, retention, and worker layers discussed here
remain proposed work.

## Recommendation

Build a Git-aware repository service from existing storage and execution
components. Keep standard Git object and pack storage initially, use redb for
transactional application metadata, and add rebuildable indexes for queries
that Git does not provide. Evaluate alternative engines with representative
workloads before replacing a selected component. Do not start a custom LSM
implementation to combine attractive features from several databases.

Separating immutable payloads, mutable metadata, derived indexes, and execution
is useful. Putting them in one physical database file, address space, or failure
domain is not required for a self-contained installation. A single distributable
Rit executable can run separate service and worker processes.

## What exists today

The active root manifest links the pinned Sley engine and frontend. The former
redb catalog, object-store, LFS, Wasm actions, and forge implementations are
preserved in the inactive
[database-first archive](../gitr/archive/database-first/README.md). Its reviewed
redb 4.5.0 selection remains the future database decision. Those archived tests
and reports do not describe the current executable's capabilities.

The current [development contract](../../AGENTS.md) and [README](../../README.md)
take precedence over older architecture documents that describe Gitoxide or
database features as active. They require Git identities, checked reference
publication, durability, worker fencing, and independently verifiable exports.
They also keep Rit independently installable and require actual replication and
consensus for multi-host HA.

Sley already provides `ObjectReader` and `ObjectWriter` interfaces, pack-reuse
hints, shallow boundaries, and promised-object handling in
[`sley-odb`](../../vendor/sley/crates/sley-odb/src/lib.rs). These are useful
integration points. They are not a completed remote storage adapter; the current
object read interface materializes an encoded object, so bounded streaming
needs additional design and qualification.

## Corrections to the proposal

Git is a specialized versioned object store. Using commits as the transaction
protocol for arbitrary application records can be a poor fit, but that does not
make database-backed Git storage inherently unsuitable. Cargo's sparse-index
RFC identifies a more specific problem: clients downloaded a complete Git index
to read a small fraction of its records. Its solution changes distribution to
selective HTTP reads while allowing the index content to remain unchanged.[1]

Several stronger claims need qualification:

| Claim | Correction and consequence for Rit |
| --- | --- |
| Two writers necessarily cause a non-fast-forward failure. | Competing branch histories create a logical conflict; independent refs are different. Git already supports expected-old-value checks and reference transactions. A database must preserve rejection and merge semantics rather than overwrite a competing writer.[2] |
| Git has no indexes or fast point lookup. | Pack indexes locate objects by ID, and commit graphs store parent relationships and traversal metadata. Git lacks general application queries, not all indexing. Keep specialized Git algorithms for ancestry.[3][4] |
| Full blobs make storage grow exponentially. | Retaining `n` distinct full versions of approximately `S` bytes costs roughly `n × S` before compression and deduplication. Compression, delta encoding, and separating keys from values solve different problems. |
| A compression option replaces Git packs. | Git packs combine compression, deltas, indexed lookup, and transfer representations. LZ4 or zstd alone does not preserve those benefits. Compare storage and transfer costs before changing representations.[3] |
| Schema changes require rewriting history and recloning. | New data can use a new schema in a new commit. Readers must understand historical versions or migrate them explicitly. Rewriting all old commits is an application choice. |
| Case-insensitive filesystems make the object database inherently corrupt. | Canonical paths and materialized worktrees have different rules. Git documents filesystem-specific handling through `core.ignoreCase`. Rit must preserve path bytes and handle checkout collisions explicitly.[5] |

Git IDs must continue to identify the canonical type, size, and object body.
Changing compression or physical placement must not change that identity.[6]
Internal keys should include the object format: SHA-1 is 20 raw bytes, and
SHA-256 is 32, with hexadecimal representations twice those lengths. An object
hash is also not authorization to read that object. Repository membership and
tenant access remain separate checks.

## Database candidates and decision rule

The following are candidates for specific responsibilities, not measured winners
for Rit. Selecting an LSM tree instead of a B-tree does not establish better
throughput, lower tail latency, or simpler recovery for this workload.

| Engine | Relevant properties | Proposed Rit role |
| --- | --- | --- |
| redb 4.5.0 | Embedded transactional storage. Its `MultiWriter` mode supports multiple local processes while serializing active write transactions. That is not simultaneous unrestricted writes or distributed consensus.[7] | Retain as the initial choice for forge records, operation receipts, job state, and transactional indexes. Keep transactions short and large payloads separate. |
| Fjall | Rust LSM engine with optional key-value separation and compression including LZ4. Its database is owned by one process. Persistence settings, compaction, blob reclamation, and transaction mode affect behavior.[8] | Benchmark if observed ingestion or metadata workloads exceed the selected design's limits. Blob separation can reduce some compaction copying, but does not eliminate write amplification or garbage collection. |
| SlateDB | Object-storage-backed LSM with a fenced single writer and multiple readers. Shallow clones share immutable SST data and checkpoints. Durability depends on the documented write options and durable-completion boundary.[9][10][11] | Consider for a later object-storage deployment profile. Its clones are writable database forks from a checkpoint; they do not supply Git branch/merge semantics or ready-to-run working trees. |

For Fjall, distinguish flushing user-space buffers from syncing durable storage.
For SlateDB, distinguish a write accepted into memory from an operation completed
with `await_durable`. These differences belong in the service's acknowledgment
contract and its crash tests.[8][11]

Fjall documents a concrete separation tradeoff: reducing the size threshold can
reduce compaction copying while lowering read performance. Blob reclamation
still balances space, writes, and maintenance cost.[18] This needs measurement,
not a blanket claim that large values make a B-tree unusable.

SlateDB is Apache-2.0 licensed.[19] That makes it eligible for evaluation; it
does not establish its workload fit. RocksDB's integrated BlobDB and TiKV's
Titan are separate implementations, not two names for the same project.[20][21]
Their designs are useful references, but neither justifies adding a C++ storage
dependency to Rit now.

Do not put a local filesystem database over an invented object-storage-backed
disk and assume its correctness carries over. The adapter would have to provide
the exact locking, ordering, atomic replacement, and durability behavior the
engine requires. Use an engine's supported storage interface or a separately
qualified adapter.

The existing exclusion of Surreal-family backends remains unchanged. The
published SurrealDB license is Business Source License 1.1 with a database-service
restriction, rather than an interchangeable permissive dependency.[22] This
assessment does not authorize a new license decision or database migration.

## Proposed storage responsibilities

| Data | Authority and intended handling |
| --- | --- |
| Git blobs, trees, commits, tags | Exact Git bytes and IDs. Keep ordinary loose objects and packs first; later add immutable backing storage behind qualified readers and writers. |
| Branches, tags, symbolic refs, reflogs | One publication authority with expected-old-value checks and recovery. Preserve Git's externally visible behavior. |
| Issues, permissions, workflow state, receipts | Versioned application records in bounded transactions. These records do not need a Git commit for every update. |
| CI logs and large artifacts | Bounded streams or immutable segments with size, integrity, retention, and ownership metadata. Avoid a writer transaction per emitted line. |
| Search, path, symbol, and relationship indexes | Rebuildable and addressed by commit/object identity plus parser or schema version. Similarity never substitutes for object identity or merge rules. |

Standard repository compatibility and database-native hosting are separate
contracts. Unmodified local Git expects the documented repository and ref
layouts.[12] A service can retain Git wire compatibility while using private
storage, but direct local Git access then needs a compatible backend or explicit
materialization/export. Never expose two independently writable copies of the
same authoritative ref.

## Publication, recovery, and garbage collection

Immutable content simplifies sharing but does not make publication automatically
atomic. The required invariant is that an acknowledged ref or artifact receipt
never points at required content that can disappear after the promised class of
failure.

A future storage service should implement this sequence:

1. Receive objects into a staging area with bounded memory and input limits.
2. Verify object identities and required connectivity, accounting explicitly for
   shallow and promisor semantics.
3. Make payloads durable to the selected guarantee before publishing references.
   A local WAL alone cannot promise survival of losing that machine.
4. Publish checked ref changes and the relevant operation state under the owning
   transaction or publication protocol.
5. Reconcile interrupted publication and reclaim unreferenced staged data only
   after readers, snapshots, and active jobs can no longer need it.

Git documents the same object-visibility requirement for receive quarantine:
refs must not expose quarantined objects that other repository readers cannot
access.[24]

There is an important boundary in the initial design: on-disk Git refs and redb
records cannot participate in one ordinary redb transaction. Use a recoverable
operation journal, transactional outbox, idempotent effects, and reconciliation
with a defined ordering. Do not describe this as cross-store ACID. A later
database-native ref owner could place its own metadata in one transaction, but
external object persistence would still require a publication protocol.

Garbage collection must include branch and tag roots, retained reflogs,
snapshots, active job leases, shallow/partial-clone rules, and retained artifacts.
Shared objects must remain reachable for every entitled repository. Deleting
one tenant's ref must not collect another tenant's live data. Backup qualification
must restore both authoritative metadata and required payloads, then verify
integrity and replay/recovery behavior.

## Wasm execution and lazy source access

The useful design is a lazy, immutable source snapshot pinned to a commit, with
a writable overlay owned by the job. A worker reads only requested files and
can reuse a shared cache. Outputs and scratch files stay in the overlay until
the supervisor validates and publishes them. This can avoid a full checkout or
network clone for suitable jobs; its value must be measured.

It is not generally zero-copy execution. Ordinary Wasmtime memory APIs read
and write guest linear memory, and source bytes may also require decompression,
delta reconstruction, validation, and copying.[13] Fast instantiation techniques
reduce particular runtime costs, not compilation, dependency acquisition, tests,
or artifact publication.[14] No microsecond startup or near-zero CI duration is
claimed here.

Workers need narrow filesystem/network capabilities, resource limits,
deadlines, cancellation, and limits on host operations. Runtime memory limits
alone do not bound all host allocations or external work.[15][16] Fuel and epoch
interruption do not time out blocked host calls; host operations need their own
deadlines and cancellation.[23] Keep workers
in separate processes even when distributed as the same executable. Provide
immutable inputs and scoped handles. Grant credentials only for explicitly
authorized job operations; a job must not inherit unrestricted storage or ref
authority. A fenced supervisor publishes job results after validating the
current attempt.

The filesystem view must also preserve executable modes, symlink behavior, raw
path bytes, submodule boundaries, and configured checkout transformations where
supported. Reading raw blob bytes alone is not full checkout compatibility.

Never run compilation or a job inside a write transaction. Claim the work in a
short transaction, execute outside it, and publish with a checked receipt.
Arbitrary existing native programs, operating-system services, and container
workloads need a native execution profile. A WASI target is not an automatic
compatibility layer for them; Rust's WASI platform documentation itself lists
target-specific limitations.[17]

## Evidence required before changing storage

The next implementation priority remains a complete Rit-specific Git
conformance baseline. Database work follows that compatibility work.

When storage integration starts, compare the standard pack-backed baseline,
redb metadata plus separate payloads, and any proposed alternative using the
same repositories and operations. Include many small files, large changing
binaries, deep histories, concurrent pushes to the same and different refs,
metadata transactions, and CI log/artifact streams. Measure latency distributions,
throughput, memory, transferred bytes, on-disk size, bytes rewritten, recovery
time, and restore correctness under equal durability settings. Include cold and
warm caches and sustained runs long enough to expose compaction and reclamation
costs.

Inject crashes between payload persistence and ref publication, stale writers,
partial uploads, corrupted/missing objects, worker retries, and interrupted
garbage collection. Accept a new backend only after it preserves Git behavior
and demonstrates a worthwhile gain on the workload it is intended to improve.

## Primary sources

[1]: https://rust-lang.github.io/rfcs/2789-sparse-index.html
[2]: https://git-scm.com/docs/git-update-ref
[3]: https://git-scm.com/docs/gitformat-pack
[4]: https://git-scm.com/docs/gitformat-commit-graph
[5]: https://git-scm.com/docs/git-config#Documentation/git-config.txt-coreignoreCase
[6]: https://git-scm.com/book/en/v2/Git-Internals-Git-Objects
[7]: https://github.com/cberner/redb/releases/tag/v4.5.0
[8]: https://github.com/fjall-rs/fjall
[9]: https://slatedb.io/docs/get-started/introduction/
[10]: https://slatedb.io/docs/design/clones/
[11]: https://slatedb.io/docs/design/writes/
[12]: https://git-scm.com/docs/gitrepository-layout
[13]: https://docs.wasmtime.dev/api/wasmtime/struct.Memory.html
[14]: https://docs.wasmtime.dev/examples-fast-instantiation.html
[15]: https://docs.wasmtime.dev/security.html
[16]: https://docs.wasmtime.dev/api/wasmtime/trait.ResourceLimiter.html
[17]: https://doc.rust-lang.org/rustc/platform-support/wasm32-wasip1.html
[18]: https://docs.rs/fjall/latest/fjall/struct.KvSeparationOptions.html
[19]: https://github.com/slatedb/slatedb/blob/main/LICENSE
[20]: https://github.com/facebook/rocksdb/wiki/BlobDB
[21]: https://github.com/tikv/titan
[22]: https://github.com/surrealdb/surrealdb/blob/main/LICENSE
[23]: https://docs.wasmtime.dev/api/wasmtime/struct.Config.html
[24]: https://git-scm.com/docs/git-receive-pack#_quarantine_environment

- [Cargo sparse-index RFC][1]
- [Git reference transactions][2], [pack format][3], [commit graphs][4],
  [filesystem case handling][5], [object identity][6], and [repository layout][12]
- [redb 4.5.0 release][7] and [Fjall documentation][8]
- [Fjall separation tradeoffs][18], [BlobDB][20], and [Titan][21]
- [SlateDB introduction][9], [clones][10], and [writes and durability][11]
- [SlateDB license][19] and [SurrealDB license][22]
- [Wasmtime memory][13], [instantiation][14], [security][15], and
  [resource limits][16], plus [host-call interruption boundaries][23]
- [Rust WASI target support][17]
- [Git receive quarantine][24]
