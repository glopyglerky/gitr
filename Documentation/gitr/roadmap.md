# Implementation map

Keep work on GitHub's default branch, `master`. Commit incremental validated
improvements there; record incomplete requirements honestly. Each entry needs
executable evidence before being marked complete. The user's goal is the full
Git-compatible Rust system, including game assets and the repository database,
not an indefinitely growing research catalog.

## Working initial implementation

- Native Gitoxide object/ref/index inspection and guarded reference publication.
- Native `mktree`, read-only `write-tree` and `commit-tree`: checkout-free object
  authoring and guarded publication, tested against Git in SHA-1 and SHA-256.
- Verified standard LFS filters, local storage, explicit basic HTTPS transfers,
  and file-lock API client.
- Local FastCDC chunk deduplication and verified reconstruction into Git LFS.
- Engine-aware candidate attributes and metadata-only large-asset inventory.
- redb commit snapshots, automatic lexical feature vectors and factual Git graph.
- Stored Wasmi actions with immutable committed input, idempotent jobs, fenced
  local claims, atomic artifact/receipt publication and verified read-only export.
- Complete vendored Rbit workflow planner with a bounded Wasm execution adapter:
  immutable committed YAML, durable DAG dependencies, matrix limits/fail-fast,
  atomic run/job/ownership publication and cancellation fencing. Claim gates are
  shared by CLI/API and default builds; old workers refuse managed jobs. Exports
  retain workflow ownership. This is not general shell/JS/Docker Actions support.
- Optional authenticated local HTTP action API sharing the existing redb state
  with CLI processes; restart and crashed-worker recovery qualified on Linux.
- Pinned Lustr build/test workflow and differential/reference Git qualification.

Consult [qualification](qualification.md) for executed results and
[compatibility](compatibility.md) for actual command limits. The
[native authoring evidence](native-history.md) includes a tested route from a
native-created commit through redb indexing to verified Wasm output, without a
Git subprocess or source checkout in that route. The
[Rbit workflow evidence](rbit-workflows.md) adds actual committed-YAML matrix/DAG
execution, concurrency and exports. Neither closes complete porcelain/network
Git parity or full CI/CD. The [forge/runtime contract](forge-runtime-plan.md)
retains the integrated-product acceptance gates.

## Next deliverable: a native everyday Git workflow

Build on the implemented immutable authoring primitives rather than recreating
them. Reuse reviewed Gitoxide components and evaluate the MIT Libra/git-internal
code identified in [similar projects](similar-projects.md) before writing missing
protocols from scratch. Add transactional native `add`, porcelain `commit`,
`checkout`, `clone`, `fetch` and `push`, including LFS-aware status and attributes.
Preserve index conflict stages, executable bits, symlinks, byte paths, linked
worktrees, autocrlf/encoding filters and sparse/promisor semantics deliberately.

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

## Next deliverable: forge and general-purpose workflow execution

Build on the complete Rbit import and qualified Wasm DAG, not a second scheduler.
Add revisioned forge entities/reviews/checks and protected-ref policy bound to the
exact commit/module evaluation. Add official stateless MCP and authenticated API
adapters over shared typed services; the current action HTTP endpoint is local
trusted-user control and has no workflow submission route or remote user roles.

Extend workflow triggers, conditions, outputs, multiple steps and reusable calls
with explicit compatibility tests. Broader native builds, shell/JS/Docker actions,
network access and secret use require separately isolated capability-controlled
workers, durable outbox/idempotency contracts for external effects, lease renewal
and hard resource limits. Import appropriate upstream modules/tests rather than
silently treating these as Wasmi capabilities. Preserve mandatory readiness and
ownership checks even when a binary lacks the optional planning dependency.

Implement verified export restoration into a new empty store with provenance,
rollback and format-version checks. A checksummed export alone is not a proven
restore facility. Upgrade and downgrade tests must prevent older binaries from
executing jobs or exporting snapshots while ignoring newer authority metadata.

Acceptance: a scoped user submits a change; trusted policy schedules its exact
commit, executes dependencies under the permitted worker profile, records checks,
and authorizes publication only for that evaluated revision. Cancellation, failed
jobs, stale approvals and duplicate requests cannot publish success or leak secrets.

## Next deliverable: resolved code relationships and learned vectors

Keep the existing lexical fallback available. Add reviewed local model providers
with pinned weights, tokenizers and model licenses. Key embeddings by content,
span, model and preprocessing identity. Evaluate GitQL data-provider integration,
Syntext indexing, Basemind maintained graph crates/rules, rust-analyzer for Rust
resolution, and SCIP/language-server sources. Separate inferred and unresolved
edges from verified relationships; an empty/failed parse is not proof of no uses.

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
