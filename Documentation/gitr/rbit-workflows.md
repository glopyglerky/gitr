# Rbit workflow import and durable Wasm DAGs

## Candidate: activation requires observed execution evidence

This increment imports the complete independent `rbit-workflow` crate and tests
from GitRbit/rbit at `39f932a00b7aa35b324e06a9ff2343a57d4bed76` during qualification.
Both license notices, the original manifest and every source file's SHA-256 are
retained. Gitr selects the offered MIT grant without changing inherited Git's
license. A local patch makes an overflowing deadline expire rather than silently
becoming unlimited. No Rbit forge/server database or subprocess runner is imported.
The exact tested vendor tree, manifests, lockfiles and integration are activated
only after the Lustr workflow passes. Until activation the source is a candidate.

## Execution profile, not a claim of complete GitHub Actions compatibility

An optional `workflows` build feature enables committed YAML planning/submission.
Lifecycle operations and claim gates compile into all new builds. The supported
profile is manual `workflow_dispatch` without inputs; static matrices with Rbit
include/exclude and expression expansion; `needs`; matrix `max-parallel` and
`fail-fast`; one `gitr/wasm@v1` step per job; and `runs-on: gitr-wasmi-v1`.
`gitr/wasm@v1` is a reserved Gitr adapter, not a downloaded GitHub repository.
Its only inputs are an installed module SHA-256 and a committed repository path.

Other events, shell/JS/Docker actions, reusable workflows, arbitrary conditions,
job outputs, environment/secrets, permissions, concurrency groups and custom
execution timeouts are explicitly rejected. The entire Rbit planner is retained
for subsequent expansion, but parsing support is not permission to execute every
upstream feature. Secret runner specifications are not persisted or materialized.

Planning resolves the workflow commit once and reads all inputs from that exact
commit. Dirty source and workflow files are not used. Maximums are 256 KiB YAML,
32 expanded tasks, 32 nesting levels, 16,384 YAML nodes, 64 aliases, 1 MiB per source
input, 8 MiB aggregate workflow/input bytes, 1 MiB plan, and a two-second cooperative
planning deadline. These do not constitute a whole-process RSS or hard CPU limit.
Each guest retains the existing Wasmi fuel/memory/host-call/IO limits and 60-second
publication lease; timeout expiry fences results but is not OS process termination.

## Durable authority and compatibility

One immediate-durability redb transaction publishes the immutable run, source,
all action jobs and ownership links. A reused run ID with a different plan fails.
Every claim checks dependencies and active matrix slots inside the same writer
transaction. Existing CLI/API action commands therefore cannot bypass readiness.
A failed job atomically cancels dependent work and fail-fast matrix siblings;
cancellation removes leases so old workers cannot publish later success.

Managed jobs use a distinct runtime profile. Older binaries refuse to execute
or export them rather than ignoring dependency metadata. New builds without the
optional planner still enforce gates and can inspect/run/cancel stored workflows.
Legacy standalone jobs remain unchanged; no existing database is converted or
deleted. State stays in the existing action database, not an HTTP session.

The workflow driver drains ready work and returns when work is held by other
processes. It is not a polling daemon. Retrying resumes from durable receipts;
there is no exactly-once guarantee for external effects (guests have none).
Concurrent cancellation or expiry can reject a worker's publication: inspect the
stored state rather than treating an interrupted request as successful execution.

Exports with workflow state use `gitr-action-export-v2`, retaining exact plans,
ownership, source bytes and jobs in one read snapshot and checksum. Standalone-only
exports remain v1. Missing ownership/objects and active workers fail export.
There is still no qualified restore/import facility or distributed HA claim.

## Intended CLI after activation

```sh
lustr --engine rustc build --manifest-path rust/gitr/Cargo.toml --locked --features workflows
# Install a bounded Wasm module first; use the returned module_sha256 in YAML.
gitr -C REPOSITORY actions workflow plan check --path .gitr/workflows/check.yml
gitr -C REPOSITORY actions workflow submit check --path .gitr/workflows/check.yml
gitr -C REPOSITORY actions workflow run check
gitr -C REPOSITORY actions workflow status check
gitr -C REPOSITORY actions workflow cancel check
```

```yaml
on: workflow_dispatch
jobs:
  source:
    runs-on: gitr-wasmi-v1
    strategy:
      max-parallel: 1
      matrix:
        file: [src/main.rs, src/lib.rs]
    steps:
      - uses: gitr/wasm@v1
        with:
          module: 'REPLACE_WITH_INSTALLED_MODULE_SHA256'
          path: '${{ matrix.file }}'
  followup:
    needs: source
    runs-on: gitr-wasmi-v1
    steps:
      - uses: gitr/wasm@v1
        with:
          module: 'REPLACE_WITH_INSTALLED_MODULE_SHA256'
          path: src/main.rs
```

## Qualification

The candidate gate runs the imported crate's own tests and documentation examples,
Gitr unit/integration tests in enabled and default profiles, strict diagnostics,
backend policy, and an actual compiled Rust/Wasm workflow in disposable SHA-1 and
SHA-256 repositories. The fixture exercises CLI/API readiness gates, concurrent
processes, previous-binary refusal, immutable inputs, failure/cancellation and
checksummed exports. Configuring these tests is not evidence that they passed.
No benchmark, multi-host failover, full forge or universal Actions claim is made.
