# Portable ephemeral Wasm workers

## Implemented; initial qualification passed on Linux

The CLI, workflow driver and optional HTTP action service now use one fresh Gitr
worker process for each module validation or action execution. Ordinary builds
compile the exact tested sources; no source-generation/preparation step remains.
The library still offers explicit embedded execution or a trusted worker binary.

Activated from successful [run 42558177942](https://github.com/glopyglerky/gitr/actions/runs/42558177942),
job 128968477899, base `0c1fb3ebb6d6ea52b1b5aebcf54a52ee52356b61` plus its
reviewed preparation script and rustfmt. [Exact evidence](ephemeral-runners-evidence.json)
records source hashes and observed scope. No dependency, lockfile, vendor,
persisted schema or object identity was changed by this increment.

## Observed execution

Pinned Lustr `62494e27717a64c8612b538409de27048fa6adc2`, engine `rustc`, Rust 1.99.1,
Ubuntu 24.04, reference Git 2.56.0. Completed 2026-11-09 UTC (November 8 Denver).

| Check | Result |
|---|---|
| Gitr all-features/all-targets Rust tests | 128 passed |
| Gitr default-build Rust tests | 122 passed |
| New supervisor/unit tests | Nine passed |
| New real-worker-binary integration tests | Five passed |
| Strict Clippy all features/targets | Passed, `-D warnings` |
| Backend policy | Five self-tests and redb policy passed |
| Actual compiled Rust guest through native CLI | Passed |
| HTTP interoperability and forced-supervisor recovery | All eight scenario groups passed |
| Rbit workflow through disposable workers | Passed for SHA-1 and SHA-256 |
| Runtime manifests/lockfile/vendor unchanged | Verified by Git diff |

Feature-profile counts overlap. Six ignored helper entries are deliberately
invoked by parent process tests; they are not six unimplemented feature tests.
The workflow fixture uses four concurrent processes and verifies dependency gates
through CLI and HTTP, immutable committed input, older-binary refusal, cancellation,
fail-fast behavior, receipt retry and independent export checksums. The HTTP
fixture kills a native supervisor after a durable claim, restarts the API, waits
for the real 60-second lease, and reclaims with a new generation. Reference
`git fsck --strict` passed. Existing native-history tests still run with empty PATH.

The read-only committed-source workflow covers Linux/macOS/Windows. Its platform
results must be read from the actual run; initial evidence here is Linux-only.
The earlier Rbit integration itself subsequently passed all three platforms in
run 42550544118, before this process-execution increment.

## Execution boundary

CLI installation/execution, workflow execution and HTTP actions select the
trusted absolute path to their own Gitr executable. Each child creates a fresh
restricted Wasmi instance. Hidden worker dispatch runs before CLI parsing and
repository discovery. The child receives bounded module/input bytes over a
length-framed pipe, not a repository path, database handle, job id, credentials,
or publication authority. Only the supervisor can commit artifacts and receipts.

The supervisor verifies request identity, response framing, output length/hash
and successful process exit, then applies the original claim/fencing checks.
Nonzero guest results and traps discard partial outputs. Integrity/read failures
now settle a still-current job as failed rather than abandoning it until expiry.
A cancelled or superseded claim can never publish an old worker result.

Environment inheritance is cleared except SystemRoot on Windows. The child uses
an empty private temporary cwd. Module/input/output each have a 1 MiB raw limit.
A 20-second supervisor deadline and 10 ms poll loop cover blocked pipe IO and
check the durable claim. Cancellation, expiry or a newer attempt stops the child.
A kill/wait guard handles error and unwinding paths before scoped IO threads join.
Stdin remains open after the frame as a parent-liveness channel: closure or
unexpected bytes stops the worker. An independent 30-second worker watchdog
covers parent loss and incomplete requests. No shell, Docker daemon, external Git
or separately installed runner is needed by the standalone Gitr executable.

These are process separation and execution budgets, NOT a kernel sandbox, an RSS
quota or real-time guarantees. The native executable and OS account are trusted;
untrusted guest capabilities still depend on the restricted no-WASI Wasmi host.
There are no guest-created native subprocesses. An OS-killed supervisor can leave
an empty temporary folder; source/artifact bytes are never written there. A host
crash leaves durable claims to their existing recovery lease. Arbitrary external
deployment effects are not exactly-once (these guests have no such capabilities).

## Linkable and standalone profiles

`ActionDb::open` remains available to Rust hosts using the embedded interpreter.
Hosts opt into process separation with `runner::EphemeralRunner` and
`ActionDb::with_ephemeral_runner`, then call the same typed `control_with_db`
operations. They must identify a trusted Gitr executable; an unrelated embedding
application is not automatically a worker. CLI/HTTP choose ephemeral execution
without repository-selected executable paths or silent fallback.

`EphemeralRunner::with_timeout` can tighten the deadline but cannot increase it
past the reviewed 20-second budget. Process startup has overhead: no speedup or
throughput advantage is claimed. Benchmark trusted embedded and process profiles
separately before choosing production capacity or making performance claims.

Existing commands do not change:

```sh
"$BLUEFE_ROOT/bin/lustr" --bluefe-root "$BLUEFE_ROOT" build --manifest-path Cargo.toml --locked --features workflows,action-api
gitr -C REPOSITORY actions install MODULE.wasm
gitr -C REPOSITORY actions run JOB_ID
gitr -C REPOSITORY actions workflow run RUN_ID
```

## Upstream and remaining work

This is a supervisor adapter around upstream Wasmi, not a new VM or replacement
database. Rust's process API requires explicit kill/wait: dropping Child is not
termination. Primary documentation:
- https://doc.rust-lang.org/std/process/struct.Child.html
- https://doc.rust-lang.org/std/process/struct.Command.html
- https://docs.rs/wasmi/2.0.0/wasmi/

Still required: OS-enforced hostile-tenant isolation, aggregate process quotas,
full CI environments/secrets/deployments, native Git transport/porcelain, forge
entities and permissions, standard MCP, resolved code search, restore/import,
distributed replication, packaging and measured performance. Portable process
APIs and a three-platform CI matrix alone do not prove all-device portability.
