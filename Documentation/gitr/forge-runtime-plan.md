# Gitr: independent Rust Git, repository database, forge and actions

User requirement recorded 2026-11-08. Work stays on GitHub's default `master`.
This is an extension of Gitr, not an OmnidiaDB deployment and not permission to
abandon native Git command parity. Keep local/offline operation, Git/GitHub
interoperability, game assets, low startup cost and bounded resource use.

## Required end state

One installable Rust product supplies native Git operations, Git repositories,
large-asset storage, a forge (issues, patches/PRs, reviews, protected refs, release
and package metadata), durable CI/CD/action workers, and code intelligence.
Stateless Rust API and MCP adapters share a typed authorization-aware service.
Durable state lives in the repository database, not in an MCP session, web process
or worker's memory. Separate optional profiles must keep a local CLI lightweight.

Search must meet or exceed Codebase Memory on measured, pinned workloads, not
just have vectors. Require structural symbol/reference/call/data-flow graphs,
relational joins, source-grounded results with exact commit/blob/span identity,
incremental changes/deletion/rename handling, dependency/impact/test selection,
local learned semantic retrieval, bounded results and explicit coverage. Preserve
raw Git path identity. Approximate or unresolved edges cannot authorize merges.

## Authority and execution boundaries

Git object bytes/IDs and wire contracts remain unchanged. A future database-native
Git backend must expose the same objects and implement ref CAS transactions. It
must not make ordinary Git and a hidden database independent masters of refs.
A server mode can answer queries and perform sparse checkout-free operations;
local mode must not require the server, a vector model or CI daemon to use Git.

Code, immutable inputs, workflow definitions, claims, artifacts and receipts are
persisted in the database. Compile and run guest code OUTSIDE write transactions.
Claim with a monotonically increasing fencing generation; commit a result only
while that claim is current and unexpired. Publish artifacts and receipt in one
durable transaction. A dead worker can be retried; repeated execution is possible,
so do not advertise exactly-once execution or arbitrary deployment side effects.
External effects eventually require an idempotent outbox and receiver-side keys.

No guest gets ambient filesystem, shell, network, environment, time, credentials,
Git hooks, deployment secrets or raw database access. Broader WASI/component and
native-build jobs require explicit capability grants and separately isolated
workers. GitHub JavaScript/Docker/shell actions are not Wasm modules: workflow
syntax compatibility and executable action compatibility are separate contracts.
Never execute those actions inside the database process by pretending otherwise.

Durable forge/action state must not be erased when rebuilding derived code search.
The initial action state is `.git/gitr/state.redb`; the existing search catalog is
rebuildable. This is one logical repository service with distinct durability
classes, not yet a complete single-file database-native Git object backend.

## First executable increment

`actions.rs` is an integration candidate until its qualification workflow passes
and its exact tested integration blobs are committed. It uses upstream Wasmi
2.0.0 and the existing redb 4.5.0. It stores bounded modules and committed source
inputs, validates a narrow byte ABI, executes without a write lock, fences claims,
and publishes a successful output atomically with the terminal job record.
A Rust guest checks source whitespace without a filesystem checkout. CLI and a
one-request JSON service exercise the same implementation. JSON control is NOT
an MCP or HTTP server and is local-trust only.

OmnidiaDB's stored-code/resource-budget/transaction-receipt approach was reviewed.
Gitr has its own smaller ABI, execution policy and job state machine. No private
OmnidiaDB source is copied into this public fork and no Omnidia runtime, cluster,
OS, proprietary package or network service is required. This avoids importing
unrelated appliance responsibilities and exposing private implementation work.

## Upstream decisions and evidence

Reviewed primary sources on 2026-11-08; capabilities below are upstream claims or
documented designs, NOT Gitr qualification. Inspect/pin each component and license
before importing source. Existing Gitr GPL-2.0-only terms are not silently changed.

| Upstream | Useful part | Boundary / decision |
|---|---|---|
| [Anthropic Scion](https://www.anthropic.com/research/redesigning-source-control) | Checkout-free API operations, incremental changesets, content-addressed payloads, separating immutable creation from ref CAS | Published internal design, not reusable released source. Do not assume its always-online/Spanner infrastructure or its operating scale applies to Gitr. |
| [Meta Sapling/Mononoke](https://github.com/meta/sapling), [architecture](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/1.3-architecture-overview.md) | Stateless request servers, separate read/write paths, typed Rust repository services, blob storage | Production use inside Meta is not plug-and-play external support; upstream explicitly qualifies public support. Audit per-library licenses and dependencies. |
| [EdenFS](https://github.com/facebook/sapling/blob/main/eden/fs/docs/Overview.md) | Lazy materialization and work proportional to touched files | Design reference; not a pure-Rust drop-in filesystem or mandatory local daemon. |
| [Jujutsu](https://github.com/jj-vcs/jj) | Operation history, recoverable changes, conflict and workspace semantics | Study Git-compatible behavior; do not import an entire differently licensed dependency graph without review. |
| [Gitoxide](https://github.com/GitoxideLabs/gitoxide) | Native Git objects/index/refs and protocol components | Continue existing integration; native add/commit/checkout/clone/fetch/push remains a parallel delivery track. |
| [Mega/Libra](https://github.com/gitmono-dev/mega) | Rust forge/monorepo and database-oriented Git client | Valuable prior art; the consulted Mega page reports archival, so identify maintained successors before adopting it as an active platform. |
| [Rbit](https://github.com/GitRbit/rbit) | Single-binary Rust forge, repositories/issues/PRs/reviews/checks | Evaluate individual modules and storage/auth assumptions. Small-project claims are not production reliability evidence. |
| [GitArena](https://github.com/mellowagain/gitarena) | Rust forge web/protocol organization | Evaluate maturity, libgit2/C dependencies and license, not a wholesale replacement core. |
| [Radicle](https://radicle.dev/guides/user) | Repository-owned collaboration metadata and offline replication | Collaboration replication is not consensus for authoritative refs or deployment leases. |
| [Enroute](https://github.com/enroute-sh/enroute) | Rust programmable Git over object storage | Early upstream, design reference; Apache-only source needs a license compatibility decision before incorporating into GPL-2.0-only Gitr. |
| [Obelisk](https://github.com/obeli-sk/obelisk), [docs](https://obeli.sk/docs/latest/) | Durable execution logs, replay, activities, component model | Evaluate workflow/event semantics and Wasmtime-based component profile. Do not pull its entire service into every CLI invocation. |
| [FluentCI Engine](https://github.com/fluentci-io/fluentci-engine) | Rust/Wasm CI plugins and reusable task ecosystems | Host/Nix/tool environments are not an in-database sandbox; subprocess capabilities need separate workers. |
| [Moonlit](https://moonlit.rs/guide/) | Rust release automation, Wasm components, mediated host permissions | Strong component/plugin interface reference; its native subprocess grants must remain explicit and isolated. |
| [Homestar](https://github.com/ipvm-wg/homestar) | Content-addressed tasks, invocation receipts, retry/workflow separation | Evaluate concepts and individual libraries; avoid mandatory IPFS/network stack and audit license compatibility. |
| [Wasmi](https://docs.rs/wasmi/2.0.0/wasmi/) | Lightweight Rust interpreter, fuel and store limits | Immediate narrow core-Wasm execution backend. Guest-memory limits are not process RSS or compilation wall-time limits. |
| [Codebase Memory](https://github.com/DeusData/codebase-memory-mcp) | Graph/code/docs/infra intelligence, MCP tools, impact and test selection | Baseline to reproduce and benchmark; lexical vectors plus tree membership do NOT meet parity. |
| [Official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk) | Standard MCP transports, tool contracts, stateless HTTP support | Planned adapter, not a home-grown protocol claim. Common authorization and typed handlers with the API. |

## Acceptance gates and implementation order

1. Qualify stored Rust/Wasm execution, concurrency, crash/lease recovery, malformed
   guests, limits and receipt integrity on Linux/macOS/Windows using pinned Lustr.
   Distinguish this core from a multi-stage CI workflow engine.
2. Finish the native everyday Git workflow concurrently with a typed repository
   service. Add durable forge entities, CAS revisions, check provenance and
   protected-ref policy bound to the exact evaluated commit. Test permissions,
   stale approvals, rewritten branches, duplicate webhooks and migration/export.
3. Add DAG workflows with immutable definitions, dependency validation, bounded
   ready queues, cancellation, retries, timeouts, content-addressed artifacts,
   cache policy, quotas and explicit secret/capability grants. Evaluate adopting
   Obelisk/Moonlit components rather than reimplementing their entire ecosystems.
4. Implement authenticated stateless HTTP API and official-rmcp adapter over the
   same service. No in-memory sessions may carry write authority or own job state.
   Require repository/tenant scopes, idempotency keys, optimistic revisions,
   cancellation, pagination, body/result limits and audit receipts.
5. Build the measured Codebase Memory parity corpus: pins for source/model/parser,
   ground-truth edges and queries, precision/recall, impact/test-selection quality,
   refresh latency, p50/p95 query latency, cold startup, peak RSS and database size.
   A passing graph/lexical test is not evidence of semantic parity or superiority.
6. Add optional HA with a qualified Rust consensus layer: partition/leader-loss,
   restart, stale worker, quorum object durability, outbox replay, restore and GC
   race tests. redb local MultiWriter alone is not multi-host HA. Preserve a useful
   standalone mode without quorum or cloud dependencies.

The user's end state remains the total package. None of these milestones may be
closed merely by adding documentation, a crate dependency or a scaffold.
