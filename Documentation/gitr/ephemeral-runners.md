# Portable ephemeral Wasm workers

## Candidate, not yet active

This implementation is being qualified before activating exact tested files.
It retains redb 4.5.0, Wasmi 2.0.0, Gitoxide 0.90.0 and the pinned Lustr route.
No runtime dependency, Cargo lockfile, object identity or persisted schema change
is proposed. The existing standalone, DAG, CLI and HTTP behavior is preserved.

## Execution boundary

CLI action installation/execution, workflow execution and the action HTTP service
select a trusted absolute path to their own Gitr executable. Each validation or
execution launches one fresh process and one fresh restricted Wasmi instance.
The hidden worker dispatch runs before CLI parsing and repository discovery.
Workers receive only bounded module/input bytes over a length-framed binary pipe;
no repository path, database handle, credentials, job id or publication authority
is part of the protocol. It cannot accept fabricated completion writes to redb.

The parent validates request/response identities, hashes, framing, output budgets
and successful process exit before using any result. Nonzero guest results and
traps discard partial outputs. Only the parent can commit artifacts and receipts,
under the original claim/fencing rules. Integrity/read failures now settle the
current job as failed rather than abandoning it until lease expiry.

The parent clears environment inheritance (except SystemRoot on Windows), uses
an empty private temporary cwd, caps raw module/input/output at 1 MiB each, and
uses a 20-second supervisor deadline. Its poll loop also checks the durable claim;
cancellation, expiry or another attempt terminates the old process. A kill/wait
RAII guard covers error and unwinding paths; bounded pipe threads cannot release
admission early. A separate 30-second worker watchdog covers supervisor loss.
Stdin remains open after the request as a parent-liveness channel: closure or
unexpected extra bytes stops the child. No shell, Docker daemon, external Git or
separately installed runner binary is required.

These controls are not a kernel sandbox, an RSS quota or a real-time deadline.
The native executable and OS account are trusted; the guest still relies on
Wasmi's deny-by-default no-WASI capability boundary. There are no native guest
subprocesses to manage. An OS-killed supervisor can leave an empty temporary
folder; source bytes and artifacts are never written there. A host crash can
leave a redb claim until its existing lease expires; the next supervisor reclaims
with a new generation. This is not exactly-once arbitrary deployment execution.

## Linkable and standalone profiles

The Rust library keeps its embedded interpreter as an explicit host choice via
ActionDb::open. A host chooses process separation using EphemeralRunner and
ActionDb::with_ephemeral_runner, and shares typed control_with_db operations.
The CLI and HTTP adapter choose ephemeral execution without repository-controlled
configuration or silent fallback. A library host must point at a trusted Gitr
worker binary, not blindly treat its own executable as Gitr.

Process startup has a cost; no throughput or latency improvement is asserted.
Keep trusted embedded use available, and benchmark concurrent jobs, small actions,
compiler throughput and repository workloads separately before performance claims.
Whole-system completion still includes native Git transport/porcelain, forge
entities and permissions, standard MCP, graph/semantic search, export/restore,
replication, release packages and fault/performance qualification.

## Reuse and evidence

This is a supervisor adapter over the existing upstream Wasmi implementation,
not a second VM, custom database or rewrite of Rbit. Rust's standard process API
requires explicit kill/wait: dropping a Child alone does not terminate it.
Primary references inspected for this increment:
- https://doc.rust-lang.org/std/process/struct.Child.html
- https://doc.rust-lang.org/std/process/struct.Command.html
- https://docs.rs/wasmi/2.0.0/wasmi/struct.StoreLimits.html
- https://docs.rs/wasmi/2.0.0/wasmi/

The dedicated qualification covers malformed framing, oversized inputs/outputs,
blocked pipe readers, cancellation, unsuccessful child exits, environment/cwd
sanitization, real compiled worker binary, parent-channel loss, fresh VM state,
capability/fuel/memory limits, and existing CLI/HTTP/DAG/crash-recovery fixtures.
Configured tests are not observed passing results. Record actual run and exact
source hashes before promoting this candidate.
