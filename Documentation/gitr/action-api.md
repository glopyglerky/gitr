# Local redb action API

## Implemented and qualified on Linux

The optional `action-api` feature is activated in the normal Gitr source and
lockfile. It exposes `gitr actions serve` over the existing ActionDb and Wasmi
runtime. No replacement datastore, storage conversion or migration is introduced.
The default build does not enable the HTTP server feature.

The exact source and lockfile were promoted from successful
[run 42540077250](https://github.com/glopyglerky/gitr/actions/runs/42540077250),
job `128912821801`, input commit
`69a16ea1506d8efa20dff0e559b5716dc95743bd` plus its reviewed preparation script.
Qualification completed on 2026-11-09 UTC (November 8 in America/Denver).
The preparation script is now removed. Ordinary builds compile committed code;
CI no longer rewrites the implementation and has read-only repository permissions.
[Machine-readable evidence](action-api-evidence.json) records exact tested blobs.

## Executed evidence

Lustr coordinator `62494e27717a64c8612b538409de27048fa6adc2`, explicit `rustc`
engine, Rust 1.99.1, Ubuntu 24.04, reference Git 2.56.0.

| Check | Observed result |
|---|---|
| API-enabled Rust unit/integration tests | 84 passed, zero failed |
| Default-feature-disabled Rust unit/integration tests | 81 passed, zero failed |
| Strict Clippy, API-enabled and all targets | Passed with `-D warnings` |
| Backend policy self-tests | 5 passed |
| Backend dependency/path policy | redb 4.5.0; no active Surreal-family dependency or migration path |
| Actual Rust guest compiled through Lustr | Passed for `wasm32-unknown-unknown` |
| Native CLI and HTTP integration/crash fixture | All eight scenario groups passed |

Each Rust suite includes one ignored helper entry that is explicitly invoked by
the database process tests. It is not an unimplemented feature test.

The end-to-end fixture proved that jobs created by the native CLI before server
startup remained visible over HTTP; HTTP-created jobs and verified artifacts
were readable by a separate native CLI while the API was running. The actual
Rust/Wasm source policy read committed bytes, not dirty worktree contents. Four
concurrent requests produced one execution attempt. Cancellation, changed-input
idempotency conflicts and a failing guest produced no successful artifact.

The test forcibly killed a native worker after its claim was persisted, killed
and restarted the API process, confirmed acknowledged receipts and exact artifact
bytes survived, waited for the real 60-second lease to expire, and reclaimed the
job with the next attempt generation. The loop guest then exhausted its fuel and
persisted a failed receipt with no artifact. `git fsck --strict` still passed.
A separate unit test proved that cancelling the HTTP waiter does not release a
running blocking worker's admission permit.

The ongoing workflow runs on Linux, macOS and Windows. The initial evidence above
is Linux-only; a configured platform job is not evidence of that platform passing.
These are bounded functional/recovery checks, not performance benchmarks, physical
power-loss simulation or multi-host partition/failover qualification.

## Start the optional service

From the Gitr source root, with the pinned Lustr coordinator installed:

```sh
"$BLUEFE_ROOT/bin/lustr" --bluefe-root "$BLUEFE_ROOT" build --manifest-path Cargo.toml --locked --features action-api
export GITR_API_TOKEN="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
target/debug/gitr -C /path/to/repository actions serve --repository my-repository
```

The default listener is `127.0.0.1:0`, which selects an available local port.
The startup JSON prints the endpoint and repository scope, never the token.
An explicit loopback port can be provided with `--listen 127.0.0.1:PORT`.
The token belongs in the environment, not source control, URLs or command-line
arguments. Keep the operating-system account and repository directory trusted.

Send JSON to `POST /v1/actions` with `Content-Type: application/json`,
`Authorization: Bearer <token>`, and `x-gitr-repository: my-repository`.
For example, the request body to read a job is:

```json
{"operation":"status","job":"whitespace-policy"}
```

Supported tagged operations are `install` (`wasm_hex`), `submit` (`job`, `module`,
`revision`, `path`), `status`, `cancel`, `run`, and `artifact` (each with `job`).
The same existing `actions install/submit/run/status/cancel/artifact` CLI commands
remain usable against the same redb file. No separate database server is required.

## Authority and resource boundaries

The API and native CLI use `.git/gitr/state.redb` (the common Git directory for
linked worktrees) through redb's cooperating local-process mode. All job, claim,
and result state remains in redb; HTTP sessions own none of it. The server binds
literal IPv4 loopback only, checks the generated 256-bit bearer secret and exact
configured repository label on every operation, and rejects browser Origin
headers. This is a local trusted-OS-user profile, not Internet-facing TLS,
per-user permissions, separate worker credentials, attestation or multi-tenant
authorization. It is not an MCP protocol implementation or multi-host HA.

A submission names a revision and repository path; Gitr resolves the committed
blob itself. Clients cannot supply fabricated commit/blob relations or terminal
worker receipts. There is no raw query, arbitrary filesystem/database path,
shell operation, caller-selected clock or guest network/filesystem/environment
capability. The original action sandbox and reference checks remain authoritative.

Compilation and execution run outside redb write transactions on bounded
blocking workers. Eight permits bound simultaneous admitted body readers and
work items. A disconnected waiter does not release its running worker's permit.
Body reads have a ten-second timeout and bounded length. Guest fuel, memory,
host-call and output budgets remain unchanged; they are not a whole-process RSS
or hard compilation-time limit, and do not replace OS isolation for hostile tenants.

Module/artifact bytes use lowercase hex with a 1 MiB raw limit. This small-action
protocol is not a large-asset transport. Operation failures return redacted 422
`operation_failed`; the adapter does not guess storage error types from private
message text. Malformed requests are 400, authentication 401, scope 403, content
type 415, body limit/timeout 413, and busy admission 503. A completed run request
returns its job record: HTTP 200 alone does not mean the guest passed.

The fixed 60-second leases have no renewal; recovery may repeat execution. There
is no exactly-once deployment side-effect guarantee. A repository API token grants
all these operations, including module installation, so a receipt alone is not a
protected-ref authorization decision without a separate trusted policy/module
binding. Forge entities, workflow DAGs, remote identity/permissions, MCP transport,
semantic/code-graph parity and distributed replication remain separate work.
