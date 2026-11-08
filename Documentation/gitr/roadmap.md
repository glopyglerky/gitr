# Implementation map

Keep work on `main`. Each entry needs executable evidence before being marked
complete. The user's goal is the full Git-compatible Rust system, including game
assets and the repository database, not an indefinitely growing research catalog.

## Working initial implementation

- Native Gitoxide object/ref/index inspection and guarded reference publication.
- Verified standard LFS filters, local storage, explicit basic HTTPS transfers,
  and file-lock API client.
- Local FastCDC chunk deduplication and verified reconstruction into Git LFS.
- Engine-aware candidate attributes and metadata-only large-asset inventory.
- redb commit snapshots, automatic lexical feature vectors and factual Git graph.
- Pinned Lustr build/test workflow and differential/reference Git qualification.

Consult [qualification](qualification.md) for executed results and
[compatibility](compatibility.md) for actual command limits.

## Next deliverable: a native everyday Git workflow

Reuse reviewed Gitoxide components and evaluate the MIT Libra/git-internal code
identified in [similar projects](similar-projects.md) before writing missing
protocols from scratch. Add transactional native `add`, `commit`, `checkout`,
`clone`, `fetch` and `push`, including LFS-aware status and attributes. Preserve
index conflict stages, executable bits, symlinks, byte paths, linked worktrees,
autocrlf/encoding filters and sparse/promisor semantics deliberately.

Acceptance: an isolated repository can clone from GitHub, commit source plus an
LFS asset, push to a test branch, and be cloned/read by ordinary Git and Git LFS.
All native operations use no C Git subprocess. A standard client and gitr take
turns writing compatible refs/index state. Credential failures, rejected pushes,
unmerged indexes and interrupted checkouts cannot lose acknowledged work.

## Next deliverable: safe changes and large-asset workflow

Add recoverable worktree/index/ref operation journals and preview/undo for local
changes, informed by jj and git-branchless. Implement stack navigation/restacking,
conflict recovery and tested merge/rebase behavior. Undo must not clobber changes
made by another writer since the recorded operation.

For assets, add long-running filter-process, ref-aware upload traversal,
pre-publication lock verification, bounded parallel transfer, authenticated endpoint
discovery, resumable transfer, selected hydration and shared local cache quotas.
Integrate the native chunk store without storing both representations needlessly.
Add safe manifest/chunk GC with leases and grace periods. Only then build the
optional chunk-aware asset service using Lore/LFSX patterns.

Acceptance: a representative Unreal/Unity repository can clone selectively, edit
and lock binary assets, switch branches, recover interruption, and publish with
ordinary Git clients retaining exact bytes. Measure transfer volume and SSD writes
on repeated edits and on compressed assets, where dedup may not help.

## Next deliverable: resolved code relationships and learned vectors

Keep the existing lexical fallback available. Add reviewed local model providers
with pinned weights, tokenizers and model licenses. Key embeddings by content,
span, model and preprocessing identity. Evaluate GitQL data-provider integration
for relational queries, rust-analyzer for Rust symbol/call resolution, and SCIP or
language-server sources for other languages. Separate inferred and unresolved
edges from verified relationships.

Index incrementally by changed blob IDs; handle deletions and branch moves without
stale results. Add a worktree overlay with a separate identity if live edits are
searched. Add ANN only after exact search exceeds a measured latency/memory target.

Acceptance: benchmark a source-grounded corpus against Codebase Memory on the same
revisions. Publish accuracy, missed relationships, unsupported languages, freshness,
p50/p95 latency, peak memory and rebuild costs. Do not use similarity to resolve
merges or authorize writes. No unrequested hosted model or paid fallback.

## Next deliverable: database-native repository and HA

Implement a Git-compatible object backend and ref transaction service with explicit
local-Git materialization rules. Ordinary Git and a private database cannot both
independently own the same refs. Use durable compare-and-swap publication and
idempotent operations; integrate an evaluated Rust consensus implementation for
multi-host authority. Couple object durability to metadata commit before ack.

Acceptance: three replicas survive leader termination and loss of one replica;
two conflicting writers cannot both publish from one expected old head; minority
partitions reject authoritative writes; retries return the original committed
result. Test fencing, lock expiry, snapshot restore, garbage collection races and
GitHub synchronization conflicts. A local multi-process redb test does not satisfy
this gate.

## Performance and releases

Create a reproducible benchmark runner only for implemented operations, with
reference Git/Git LFS baselines and cold/warm cache control. Measure small-source,
large-monorepo and binary-asset workloads separately. Compare Lustr LLVM and its
qualified Cranelift route for build time independently of Gitr runtime speed.
Publish signed/checksummed installable packages after supported-platform CI passes;
do not label a development build a production Git replacement.
