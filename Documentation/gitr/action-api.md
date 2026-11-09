# Local redb action API

## Candidate status

`actions_api.rs` is a staged optional integration until the Lustr qualification
passes and its exact tested source/lockfile are activated. It reuses Gitr's
existing ActionDb and Wasmi runtime. No Surreal-family dependency, replacement
database process, storage conversion or migration is introduced.

## Contract

The `action-api` feature adds an explicitly started `gitr actions serve`
subcommand. Ordinary commands do not bind a socket or require a service. The API
and native CLI operate on the same existing `.git/gitr/state.redb` using redb's
cooperating local-process mode. All job/claim/result state remains in redb; an HTTP
session carries none of it. This is not multi-host HA or an MCP transport.

The server binds literal IPv4 loopback only, requires a generated 256-bit bearer
secret from `GITR_API_TOKEN` and a matching `x-gitr-repository` header on every
operation, and rejects browser Origin headers. This is a local trusted-OS-user
profile. It does not supply Internet-facing TLS, per-user permissions, separate
worker credentials, attestation or multi-tenant authorization.

`POST /v1/actions` accepts bounded tagged JSON operations: install, submit,
status, cancel, run and artifact. A submission names a revision and repository
path; Gitr itself resolves the exact committed blob. Clients cannot submit fake
commit/blob provenance or a fabricated completion receipt. There is no raw SQL,
arbitrary filesystem/database path, shell invocation or client-supplied claim
clock. Existing source-reading and Wasm capability restrictions remain in force.

Compilation and execution happen outside redb write transactions on bounded
blocking workers. Eight admission permits limit simultaneous body readers and
work items. A disconnected HTTP waiter does not release a running worker's
permit. Body reads have a ten-second timeout and bounded length. Existing guest
fuel, memory, host-call and output budgets remain unchanged; these are not a
whole-process RSS guarantee or an OS sandbox.

The module and artifact byte protocol uses lowercase hex and a 1 MiB raw limit.
This is for small source-policy actions, not a new large-asset transport. Missing
jobs, stale submissions and other operation errors currently return a redacted
422 `operation_failed`; the adapter does not classify errors by matching private
storage error strings. Malformed requests are 400, authentication 401, scope 403,
content type 415, body limit/timeout 413, and busy admission 503. A successful run
request returns its durable job record; callers must inspect its status rather
than assume HTTP 200 means the guest passed.

The 60-second lease policy has no renewal; crash recovery can repeat execution.
No exactly-once deployment side-effect guarantee is made. Repository API tokens
grant all supported actions for the configured repository. Do not use an action
receipt alone to authorize protected refs without a trusted policy/module binding.

## Qualification scope

The dedicated workflow builds the actual Rust guest through pinned Lustr. It
checks pre-existing native jobs, bidirectional CLI/API visibility, binary artifacts,
committed versus dirty inputs, duplicate and concurrent requests, cancellation,
a failing guest, authentication/scope/body boundaries, forced API restart and
reclaiming a native worker's durable claim after that worker is forcibly killed.
The disconnected-waiter admission test covers blocking work that outlives its
request. No user repository, real token or paid service is used by these tests.

A configured test is not a passing result. Record observed run and exact commit
before promoting this candidate; retain existing redb data and backend policy.
