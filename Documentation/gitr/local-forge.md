# Repository-local redb forge

## Candidate status

This increment is staged until the exact implementation, complete Rbit domain
import and dependency lockfile pass qualification. It is not yet activated by the
normal Gitr build. `forge` will be an optional build feature alongside `workflows`
and `action-api`; normal Git operations still require no server or forge daemon.

## Implemented candidate scope

The complete pure `rbit-core` crate supplies title/body validation, issue/review
state types and required-check aggregation. Preserve the full upstream crate,
tests, both license notices, original Cargo manifest and per-file source hashes.
Select its MIT grant; do not change inherited Git or existing Gitr declarations.
The Rbit SQLite storage, web application and authentication stack are NOT imported.

Gitr stores issues/change proposals, revision-bound reviews, check-job links,
comment/edit timelines and idempotent command receipts in the existing
`<common-git-dir>/gitr/state.redb`. A mutation publishes its record, event, numeric
counter and original retry result in one immediate-durability transaction. Edits,
state changes, reviews, retargets and check attachments require an expected revision.
Comments append without overwriting one another. Reusing a key with changed request
content fails. Retrying the same key returns its original receipt even after the
referenced Git branch moves or newer edits/comments are committed.

A change proposal contains exact verified Git commit IDs, not mutable branch
names as authority. Retargeting invalidates reviews and check links. A check names
an existing Gitr action job; callers cannot submit fabricated successful check
receipts. The adapter verifies the exact head, module digest, committed input path,
blob and input hash; readiness also verifies persisted module/input/artifact bytes.
Queued/missing checks are pending, failed/cancelled checks fail, and corrupt or
unverifiable records report error. The Wasm checks use the qualified disposable
worker path; durable results remain in redb.

Readiness is explicitly LOCAL ADVISORY and `authorizes_merge` is always false.
There is one local-operator authority, not authenticated human identities or
independent approvals. A bearer token grants that repository's local authority.
No merge/ref update, protected-branch enforcement, review quorum, remote hosting,
Git daemon, UI or full Gitea/Forgejo replacement is claimed. Native Git refs still
have their existing authority; no hidden database master competes with them.

## Interfaces and bounded operation

Planned active commands: `gitr forge control` accepts one tagged JSON request on
stdin; show/list/timeline/check are convenience reads; export emits forge-only
JSON Lines. `gitr actions serve` with both forge and action-api features exposes
`POST /v1/forge` alongside `/v1/actions`, using the SAME bearer/repository/origin
checks, body timeout, admission permits and redacted error handling. No raw query,
caller-selected database path, fake actor or client-supplied check state is accepted.
The typed service is linkable through `gitr::forge` and `ActionDb::forge_control`.

Requests are capped at 128 KiB, persisted records at 256 KiB, title/body sizes by
Rbit's domain limits, required checks at sixteen, and pages at eight records.
List/timeline use exclusive numeric cursors. Timeline content is raw text/data;
a future UI must escape it, not treat it as trusted HTML. Secret values must not
be placed into issue text or command bodies.

Forge export takes one read snapshot, includes thread/event/operation records and
the numeric counter, and frames exact record bytes in a final SHA-256 digest.
It explicitly excludes Git history and action objects. It is NOT restore/import,
a coordinated combined forge+action backup or a migration/failover checkpoint.
Existing actions export remains action-scoped. There is no destructive conversion.

## Qualification gates

Test all imported Rbit core source and documentation examples through pinned Lustr.
Gitr tests cover optimistic concurrency, concurrent comments, exact receipt replay,
rollback after partial writes, stale reviews, invalid identities and shared action
storage. The real SHA-1/SHA-256 process fixture exercises CLI/HTTP, real Rust/Wasm
checks, credential/scope/request limits, branch movement, stale/wrong checks,
failed guest results, linked-worktree sharing, forced API restart, independent
export verification and reference Git fsck. Gitr fixture commands have empty PATH.
Source qualification also verifies every unmodified upstream source hash and the
selected redb/Gix/Wasmi pins. Configured tests are not passing evidence.

Next completion tracks remain ordinary native Git transport/porcelain and LFS
integration, real identities/roles and protected-ref merge transactions, forge UI,
MCP, replication/HA, portable release packages and workload benchmarks. No speedup,
multi-host safety or entire-product completion follows from adding forge tables.
