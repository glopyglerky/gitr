# Gitr architecture

Status: active implementation and explicit future design. Updated 2026-11-08.

## Destination

Gitr is a Rust Git implementation with reliable large-asset workflows and a
database-centered repository. It must interoperate with Git and GitHub, support
multiple independent writers without lost updates, and make code relationships
and content searchable without repeatedly parsing every file. It should be useful
as a small local binary and grow into a replicated repository service.

Replacing Git's storage alone does not make status, merges, builds or queries
faster. Each operation needs a measured workload. An embedding is an index of
content, not a substitute for the exact content or a proof that code is correct.

## Two interoperability boundaries

The current compatibility mode uses ordinary Git objects, packs, refs and indexes.
Gitr opens existing repositories through Gitoxide. Reference Git can open those
same repositories, and external writers remain visible. Local metadata and search
indexes are derived from immutable commit IDs and can be deleted and rebuilt.

The intended database-native mode can put objects and transactional refs behind a
Rust database service while retaining exact Git object encodings and serving Git
wire protocols. This preserves remote interoperability with GitHub. It does not
automatically preserve direct access by an unmodified local `git` executable,
which expects Git's repository layout and ref backend. That mode needs an explicit
export/materialization adapter or a standard Git ref/object backend. It must never
present two independently writable copies of the same authoritative ref.

Compatibility mode is the safe default while the database-native mode is built.
Do not replace `.git/refs`, `.git/HEAD` or `.git/index` with private tables and then
claim existing Git clients can read them unchanged.

## Storage responsibilities

| Data | Identity and authority | Intended storage |
|---|---|---|
| Git blob, tree, commit, tag | Exact Git encoding and configured Git object hash | Gitoxide object store now; compatible DB object backend later |
| Git branch/tag refs | Expected-old-value compare-and-swap | Native Git ref locks now; replicated transactional ref service later |
| Git LFS asset | Whole-file SHA-256 and byte length | Standard local LFS cache and basic Batch API |
| Local asset chunks | Verified SHA-256 and byte length | Immutable FastCDC chunk cache; full-file reconstruction verifies the LFS OID |
| Repository catalog | Commit ID, object ID, path bytes, schema version | Embedded redb, rebuildable from Git |
| Syntax and resolved symbol graph | Source blob/span plus parser and resolver identity | Derived tables, with explicit coverage and provenance |
| Text/vector indexes | Source identity plus embedding model and preprocessing identity | Derived tables or optional specialized indexes |
| Operation journal | Monotonic operation identity, preconditions and durable effects | Planned transactional journal with recovery |

The graph distinguishes `contains`, `parent`, `defines`, `references`, `calls`,
`imports` and `similar-to`. Exact tree membership and commit parents can be taken
directly from Git. A name match is not a resolved call edge. Similarity is neither
equality nor authorization. Queries must say which relation and revision they use.

## Local database decision

The first database is pinned redb 4.5.0. It is a Rust embedded transactional store,
not a network service. Its `ConcurrencyMode::MultiWriter` allows multiple local
processes to open one database and serializes their write transactions. Readers
use snapshots. This gives concurrent callers a correctness boundary without a
mandatory daemon. The cache budget and source-blob limit must stay explicit.

The implementation publishes complete commit snapshots transactionally. Search
must select the requested commit, not whichever writer indexed most recently.
Blob analysis is reusable by content identity across branches. A failed build of
an index must not expose partial membership as a complete snapshot. Older complete
snapshots remain usable while another revision is being indexed.

This first vector representation is a deterministic lexical feature embedding.
It is automatic, local, reproducible and needs no model download or paid service.
It measures token overlap through feature hashing. It does not understand meaning
like a trained code model and does not establish parity with Codebase Memory's
resolved symbol graph. Exact and approximate search algorithms also have different
memory and latency costs. Do not label a linear scan as HNSW or claim constant-time
semantic search.

Learned embeddings remain part of the design. A future local provider should use
a reviewed Rust inference backend and pin the model weights, tokenizer, license,
dimensions, quantization and preprocessing. Store those identities with every
embedding. Never mix vectors from different models in one similarity calculation.
Index binary assets by metadata until a user chooses an asset encoder. Do not send
private source or assets to a hosted embedding provider by default.

For Rust code relationships, evaluate rust-analyzer's syntax/HIR and incremental
analysis before inventing a parser. For other languages, evaluate upstream
language-server or SCIP data and distinguish exact, inferred and unresolved edges.
Codebase Memory's query design and coverage reporting are useful references, but
its current native backend and grammars are C and are not imported as Rust code.

Source: [redb 4.5.0 release and concurrency contracts](https://github.com/cberner/redb/releases/tag/v4.5.0).
This version's multi-process locking cannot be mixed with the experimental
multi-process mode in redb 4.3/4.4. Upgrades need an exclusive maintenance boundary.

## Distributed high availability

Local redb concurrency is not distributed consensus. The intended service uses a
replicated state machine for authoritative refs, permissions, lock ownership and
operation decisions. OpenRaft is a candidate to evaluate, not a dependency already
integrated. Multiple clients may propose updates, but one ordered authority per
repository or shard decides conflicting mutations.

Publication sequence:

1. Receive immutable Git objects or asset chunks into an uncommitted staging area.
2. Verify types, lengths and hashes; establish the configured durable replication
   policy for every newly reachable object.
3. Submit a transaction containing repository identity, authorization epoch,
   expected old refs, new refs, lock preconditions and an idempotency key.
4. Commit that transaction through the current consensus leader and quorum.
5. Acknowledge the new authoritative revision only after durable commit. Update
   derived indexes asynchronously with an explicit indexed revision watermark.

Two writers updating different branches can succeed. Two writers replacing the
same old branch head cannot both silently succeed. The loser receives the actual
current revision and can merge/rebase or retry with a fresh precondition. A retry
after a lost acknowledgement returns the prior idempotent result rather than
duplicating the operation. Old leaders must be fenced by epoch/term at the write
boundary. A minority partition cannot publish new authoritative refs.

Offline Git commits remain possible. They are local history until a remote accepts
publication; they do not pretend to be quorum-committed remote changes. Asset locks
need owner identity, fencing, expiration policy and explicit offline behavior.
Standard LFS servers may offer fewer guarantees, so capabilities must be reported.

Do not put a shared redb file on a network filesystem and call that a cluster.
Replicas own their own databases and exchange authenticated protocol messages.
Test leader loss, delayed messages, duplicate requests, stale terms, partial object
replication, restarts and partition healing before claiming HA.

## Large files and game development

GitHub mode retains standard LFS pointers and whole-object HTTPS transfers. The
local FastCDC cache learns boundaries from content, so an insertion need not shift
every subsequent chunk. It can reuse identical chunks across revisions and paths.
Hydration reconstructs the exact full-file bytes and verifies the LFS hash before
publishing a standard cache object. A chunk manifest is never substituted for a
standard LFS pointer in Git history.

A future optional Rust asset service may negotiate chunk transfers, range reads,
resumption, shared LAN caches and content authorization. GitHub does not gain those
server features merely because a client has a chunk cache. An interoperability
endpoint must still reconstruct whole objects for ordinary Git LFS clients.

Unreal `.uasset` and `.umap` files are candidates for exclusive-edit asset policy.
Unity's Force Text scenes/prefabs and their `.meta` files need text-aware handling.
Profiles print candidate rules for review; they do not rewrite history or silently
ignore plugins and production assets. Generated outputs and caches must be
excluded deliberately. Watchers, sparse hydration and editor integrations remain
separate tasks with their own correctness tests.

Chunking is not guaranteed to help already-compressed or encrypted files. Measure
local bytes written, network bytes, CPU, peak memory and edit locality. Reusing a
chunk must avoid rewriting its bytes, which also matters for SSD wear. Garbage
collection needs reachability from active manifests and a grace period for staged
writers. No destructive GC is enabled in this initial cache.

Sources: [Lore design](https://epicgames.github.io/lore/explanation/system-design/),
[Git LFS specification](https://github.com/git-lfs/git-lfs/blob/main/docs/spec.md),
[Git LFS Batch API](https://github.com/git-lfs/git-lfs/blob/main/docs/api/batch.md).

## Safety and resource limits

Read-only operations isolate repository configuration and do not execute filters,
hooks, textconv, fsmonitor or credential helpers. Unsupported configurations fail
explicitly. Network credentials come from a selected environment variable, stay
out of diagnostics, and are not forwarded to another origin. TLS uses rustls;
its selected ring cryptographic backend includes native code. The authored Gitr
code forbids unsafe Rust, but that does not make every transitive crate or system
library pure Rust.

Large payloads stream with bounded buffers. Every metadata parser has limits and
checked arithmetic. Storage publishes atomically and never overwrites a different
object under the same identity. Repository storage is owned by the trusted local
user; defending against a malicious process with that same user's filesystem
authority requires a stronger sandbox and is outside the current cache contract.

No performance or reliability claim is inferred from the word Rust. The
[qualification record](qualification.md) and [compatibility matrix](compatibility.md)
describe what the checked-in code actually demonstrated.
