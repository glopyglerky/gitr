# Database decision: SurrealDB 4.0.0, SurrealKV and optional SurrealDS

Decision date: 2026-11-08. User-selected preferred target; migration is gated by
qualification, packaging/licensing review and measured Gitr workloads. This
supersedes earlier suggestions that redb must be the final repository database.
Existing Git objects, redb catalogs and action state are not converted or deleted.

## The correct composition

- **Local/embedded:** SurrealDB `4.0.0` over SurrealKV. The v4.0.0 source manifest
  currently specifies SurrealKV `1.2.0`; documentation's “1.0” wording describes
  its stable-format generation, not an instruction to downgrade that dependency.
- **Optional multi-node:** SurrealDB Enterprise over **SurrealDS**, a different
  distributed backend. It is not replication switched on inside SurrealKV and
  is not a third layer underneath a local SurrealKV directory.
- **Wasm extension candidate:** Surrealism, already upstream in SurrealDB, for
  trusted short database functions. Keep untrusted or long CI/build/deployment
  tasks outside database transactions, with durable jobs and fenced publication.

Primary sources: [4.0 releases](https://surrealdb.com/releases/4.0),
[v4.0.0 source manifest](https://github.com/surrealdb/surrealdb/blob/v4.0.0/Cargo.toml),
[storage architecture](https://surrealdb.com/docs/learn/data-models/architecture),
[SurrealDS announcement](https://surrealdb.com/blog/introducing-surrealds-fault-tolerant-distributed-storage).

## One datastore owner, many clients and transactions

SurrealKV permits concurrent transactions but only one open datastore owner.
Multiple processes or independent embedded connections must NOT open the same
folder. A small Gitr storage service owns the embedded client once; internal
handlers clone that client, while CLI/API/MCP/worker processes use the service's
scoped protocol. A standalone command may own the datastore exclusively only
when the service is not running. The service election/lock/IPC/restart protocol
must be implemented before replacing redb's current multi-process access model.

Stateless API/MCP means no session owns durable jobs, authorization state or
workflow state. It does not mean the database has no state. Authenticate and
scope every request; do not expose an administrative arbitrary-SurrealQL endpoint
as Gitr's MCP interface. Parameterize queries and check statement errors as well
as transport errors. Use durable operation keys and optimistic revisions.

Sources: [embedded ownership](https://surrealdb.com/docs/reference/rust/embedding),
[SDK error handling](https://surrealdb.com/docs/reference/rust/concepts/error-handling).

## Authority, isolation and durability

Git SHA-1/SHA-256 IDs remain exact identities; database record IDs, vectors and
embedding similarity never replace them. During coexistence, ordinary Git owns
its refs. A future database-native authority needs an explicit Git projection and
exclusive authority transition, not silent dual writers.

SurrealKV transactions use snapshot isolation and detect write conflicts.
Serializable isolation is available on SurrealMX/SurrealDS, not SurrealKV; do not
request it on KV and silently continue on weaker semantics. Enforce same-record
ref CAS and job fencing. Multi-record policy decisions need an explicit common
invariant/version record or tested FOR UPDATE reads to prevent write skew.
Retry only typed conflicts, rerun the whole transaction, and bound attempts.

Set `sync=every` explicitly for authoritative state. Persist immutable object or
artifact data before publishing its durable reference. Commit a terminal receipt
and its small artifact reference atomically. Do not treat a query response before
COMMIT as an acknowledged operation. Process-kill tests are not disk/controller
power-loss tests. HA additionally needs remote object durability, partition,
leader-loss, fencing, replay, backup/restore and recovery qualification.

Sources: [transactions](https://surrealdb.com/docs/learn/querying/concepts-and-guides/transactions),
[BEGIN isolation](https://surrealdb.com/docs/reference/query-language/statements/begin),
[file-backed durability](https://surrealdb.com/docs/running/file-backed).

## Gitr data model and cost boundaries

Use the database for repository metadata, immutable snapshot membership,
revisioned refs/policies, issues/PRs/reviews/checks, symbols and source-grounded
edges, model-versioned embeddings, workflows/jobs/leases, receipts and artifact
manifests. Keep large Git packs, LFS objects and asset chunks in a streaming
content-addressed payload store unless benchmarks show a better layout. A unified
repository/database API does not require decoding multi-gigabyte assets into rows.

Graph plus vector plus full-text queries can share one database, but they do not
supply parsers, resolved symbol edges, embeddings or Codebase Memory parity.
Record exact commit/blob/span/parser/model/preprocessing identities; separate
inferred and resolved edges. Filter repository/tenant/revision before retrieval.
Measure recall, stale/deleted results, impact queries and latency/memory before
claiming parity or superiority. The qualification fixture's vectors are synthetic.

Benchmark cold CLI startup, hot queries, small repo size, concurrent CAS, graph
and vector indexing, binary assets, write amplification, compaction/GC, backup
cost and p50/p95 latency. Keep search regeneration separate from durable forge and
job records. Do not version ephemeral heartbeats forever; retention is explicit.

## Surrealism is directly relevant, but not a CI scheduler

SurrealDB 4 includes Rust/WASI Preview 2 component extensions with fresh instances,
transaction-scoped database handles and resource/capability policies. The feature
still requires the experimental `surrealism` capability. Evaluate pinned, signed,
digest-verified extensions for small trusted transformations and queries. Do not
load arbitrary PR-supplied components into a privileged database transaction.
The existing narrow Wasmi action candidate remains an isolated execution reference
while its state store is abstracted; it is not Surrealism-compatible by assertion.
Neither extension runtime supplies GitHub Actions JS/Docker/shell compatibility,
a complete workflow scheduler, secret broker or safe deployment side effects.

Sources: [extension architecture](https://surrealdb.com/docs/learn/extensions/guides/extension-architecture),
[DEFINE EXTENSION](https://surrealdb.com/docs/reference/query-language/statements/define/extension).

## Licensing and operational independence gates

The SurrealDB core is BSL 1.1/source-available, not presently unrestricted
OSI-open-source code. The SDK's license does not relicense the embedded engine.
The repository retains GPL-2.0-only source, so do not silently link and distribute
a combined binary or assume all inherited source can be relicensed. Review the
actual distribution with qualified counsel/upstream; a separately packaged
process with a narrow protocol is an engineering option, not a legal conclusion.
New standalone qualification code is separately MIT-licensed and links no Gitr or
inherited Git code. This is not permission to relicense existing source.

SurrealDS requires a SurrealDB-issued Enterprise license. The current documentation
requires online startup validation and periodic checks at api.keygen.sh; new
nodes cannot start fully offline, and prolonged license-service unavailability
can take running nodes out of service. Keep this an explicitly opted-in licensed
HA profile, never a required dependency for offline/local Gitr. No license has
been purchased or configured. Do not claim SurrealDS HA has been executed.

Sources: [licensing FAQ](https://surrealdb.com/license),
[v4.0.0 license](https://github.com/surrealdb/surrealdb/blob/v4.0.0/LICENSE),
[Enterprise license lifecycle](https://surrealdb.com/docs/manage/enterprise/product/licensing).

## Executable qualification and migration gates

`rust/gitr-surreal-probe` is an isolated SDK/storage qualification program, not
Gitr's active backend. Its workflow uses the pinned Lustr coordinator. It checks
competing same-record CAS, typed conflict handling, rollback, graph/full-text/HNSW
queries, persistence after close, process ownership and acknowledged writes after
forced process termination. A fresh temporary datastore is used, never user data.
A passing probe is only that bounded evidence, not performance, full Git parity,
production schema/authorization, multi-node HA or migration completion.

Next: qualify actual Gitr action transactions and ref receipts against both
backends; introduce the one-owner service; add schema migration journals and
export/import verification; rebuild derived indexes; copy durable records into a
new location; compare IDs/counts/hashes; switch with an explicit authority marker.
Preserve originals and a tested rollback path. Never rely on automatic in-place
storage conversion as the migration/backup strategy.
