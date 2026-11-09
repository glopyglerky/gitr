# Independent SurrealDB action store

## Status

An integration candidate is checked in under `rust/gitr-store`, with the native
client in `src/actions_remote.rs`. It is NOT active in the default Gitr
command until the exact tested integration is committed. The dedicated workflow
stages that integration locally, compiles it through pinned Lustr and validates
real processes against SurrealDB 4.0.0 / SurrealKV 1.2.0. A dependency declaration
or test definition is not evidence of a passing run.

## What this increment implements

A separate Rust executable is the sole owner of an opt-in local datastore.
Independent CLI/worker processes use a bounded typed HTTP protocol. Every request
must carry a 256-bit bearer token and the configured repository identity. The
profile binds literal IPv4 loopback only, disables ambient proxies in its client,
does not follow redirects, rejects browser Origin headers and accepts no raw
SQL, database path, namespace or arbitrary host command from a request.

The service persists immutable module/input/artifact bytes and action records.
Submission keys are bound to the exact module, Git snapshot, source blob and
input hash. Jobs use a compare-and-swap record revision and monotonically
increasing attempt number. Claim, cancellation and terminal publication are
short transactions. Successful output and its terminal receipt are committed
atomically; retries return the same terminal receipt only for identical results.
Expired/cancelled/replaced attempts cannot publish a new terminal result.

Gitr resolves the committed source with Gitoxide and executes the existing
capability-denying Wasmi sandbox in the worker process. No Wasm execution or
compilation holds the database write transaction. No guest receives the HTTP
credential, environment, filesystem, network, clock or process capabilities.
The service does not execute hooks or untrusted source.

The storage process contains independently authored MIT code and does not link
Gitr or inherited Git code. SurrealDB retains its own license. This separation is
not a legal opinion about a distributed combined product; the existing BSL/GPL
packaging gate remains open.

## Explicit opt-in, not automatic migration

Planned qualified CLI shape:

```text
GITR_STORE_TOKEN=<generated 64-character lowercase hex secret>
gitr-store NEW_STORAGE_DIRECTORY 127.0.0.1:PORT REPOSITORY_ID
gitr actions remote --endpoint http://127.0.0.1:PORT/v1/action --repository REPOSITORY_ID install policy.wasm
gitr actions remote --endpoint http://127.0.0.1:PORT/v1/action --repository REPOSITORY_ID submit JOB MODULE_SHA256 --revision HEAD --path src/main.rs
gitr actions remote --endpoint http://127.0.0.1:PORT/v1/action --repository REPOSITORY_ID run JOB
```

The token belongs in the environment, not a command argument, checked-in config,
URL or log. The service creates a format/repository identity marker and refuses
to adopt nonempty unmarked directories. Unix directories must be private to the
current account. Existing redb catalogs/action stores and Git refs remain intact.
Normal local commands do not silently switch to this service. Multi-gigabyte LFS
and Git objects are not copied into this bounded action store.

## Scope and remaining gates

This is a **local trusted-OS-user profile**, not remote or multi-tenant security.
The bearer credential grants all operations in one repository. A holder can submit
and complete actions, so a receipt is not proof of an attested honest worker. The
Gitr client verifies snapshot reads; the storage API does not independently prove
that a caller-provided commit/blob relation exists. Do not use these receipts to
authorize a protected remote ref without a separate provenance/policy gate.

Fixed 60-second leases have no renewal. Retry can execute the guest more than
once. Generation fencing and idempotent terminal publication do not provide
exactly-once external deployment effects. The host wall clock is trusted; clock
jump behavior and hostile host users are outside this qualification. Guest fuel
and memory budgets are not OS process resource limits.

Small action payloads are at most 1 MiB and stored as hex for the typed protocol.
This deliberately bounded first version doubles byte representation size; it is
not a high-throughput large-asset transport. There is no quota-aware GC, automated
owner election, system service installer, migration, graph-index migration,
standard MCP transport or SurrealDS/HA in this increment.

Qualification covers real native-client Rust/Wasm execution, dirty-worktree
exclusion, concurrency, idempotency, cancellation, expiry and stale results,
second-owner rejection, forced process restart and persistence, authentication
and input bounds. It does not simulate disk/controller power loss or a distributed
network partition. Startup latency, RSS, write amplification and workload speed
must be measured before asserting this backend is faster than redb.
