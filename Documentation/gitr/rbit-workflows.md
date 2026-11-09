# Rbit workflow import and durable Wasm DAGs

## Implemented; initial execution qualified on Linux

The complete independent `rbit-workflow` crate is vendored at
`rust/vendor/rbit-workflow`, from GitRbit/rbit revision
`39f932a00b7aa35b324e06a9ff2343a57d4bed76`. Its 48 original files include source,
tests and conformance fixtures. The final 55-file tree also retains both upstream
license notices, the unmodified original manifest, per-file provenance, local
patch notes and the tested standalone lockfile. Gitr selects the offered MIT
grant; inherited Git and existing Gitr license declarations are unchanged.

The implementation was activated from the exact passing source, lockfile and
vendor tree emitted by [run 42549777556](https://github.com/glopyglerky/gitr/actions/runs/42549777556),
job `128942519832`, tested base `f33d5572c3bc44b444c58afd5596f917f03c649a`
plus its reviewed preparation script. The script has now been removed: ordinary
builds compile committed sources without generating or patching their code.
The ongoing workflow is read-only and tests Linux, macOS and Windows, including
an actual pre-workflow binary. Configured platforms are not proof of success;
initial evidence below is Linux-only.

## Observed qualification

Lustr `62494e27717a64c8612b538409de27048fa6adc2`, explicit `rustc` engine,
Rust 1.99.1, Ubuntu 24.04, reference Git 2.56.0. Completed 2026-11-09 UTC
(November 8 in America/Denver). [Exact evidence](rbit-workflow-evidence.json).

| Executed check | Result |
|---|---|
| Complete imported Rbit crate | 118 unit tests and one conformance test passed |
| Rbit documentation example | One passed |
| Gitr with workflows and action API | 114 Rust tests passed |
| Gitr default build, planner and HTTP disabled | 108 Rust tests passed |
| Strict Gitr Clippy, enabled features and all targets | Passed with `-D warnings` |
| Backend-policy self-tests | Five passed |
| Selected storage/engine/runtime versions | redb 4.5.0, gix 0.90.0 and Wasmi 2.0.0 preserved |
| Actual Rust-to-Wasm guest build | Passed through Lustr |
| Complete workflow process fixture | Passed for SHA-1 and SHA-256 |

Gitr profile test counts overlap and must not be summed as unique coverage.
Each has one deliberately ignored process-helper entry invoked by parent tests.
New Gitr coverage is twelve storage/scheduler tests plus three planner tests.
The lockfile adds seven packages for the optional planner; it is not unchanged.
[Dependency/license evidence](rbit-workflow-dependencies.json) records the exact
new packages and available notice hashes, not a full-release legal certification.

The process fixture proved committed YAML -> Rbit matrix/dependency plan -> atomic
redb submission -> real compiled Rust/Wasm -> verified artifacts. Four concurrent
Gitr processes retained one execution attempt per successful child. Both direct
CLI and HTTP run requests refused to bypass dependencies. Changing worktree YAML
and source after submission did not change inputs. Real failed checks cancelled
downstream work without executing it. Reopened processes retained receipts;
repeat submission returned the original terminal run. The actual older executable
refused both managed execution and incomplete export before emitting output.
Independent export verification checked source/object hashes, exact stored records,
three runs, nine jobs and nine ownership links, and the framed checksum. Reference
`git fsck --strict` passed. Every Gitr command in the fixture ran with an empty PATH;
reference Git configured and checked the fixture, not Gitr's operations.

## Supported execution profile

An optional `workflows` build feature enables committed YAML planning/submission.
Lifecycle operations and claim gates compile into all new builds. The supported
profile is manual `workflow_dispatch` without inputs; static matrices with Rbit
include/exclude and expression expansion; `needs`; matrix `max-parallel` and
`fail-fast`; one `gitr/wasm@v1` step per job; and `runs-on: gitr-wasmi-v1`.
`gitr/wasm@v1` is a reserved Gitr adapter, not a downloaded GitHub repository.
Its only inputs are an already-installed module SHA-256 and a committed path.

Other events, shell/JavaScript/Docker actions, reusable workflows, arbitrary
conditions, job outputs, environment/secrets, permissions, concurrency groups
and custom execution timeouts are explicitly rejected. The complete Rbit planner
is retained for subsequent expansion, but parsing support is not permission to
execute every upstream feature. Secret runner specifications are not persisted
or materialized. No Rbit forge database or subprocess runner is imported.

The local deadline patch makes an unrepresentable `Deadline::after` duration
expire, never silently become unlimited, and includes an upstream regression.
Planning resolves the workflow commit once and reads all inputs from that commit.
Dirty source and workflow files are not used. Maximums are 256 KiB YAML, 32 tasks,
32 YAML nesting levels, 16,384 YAML nodes, 64 aliases, 1 MiB per source input,
8 MiB aggregate workflow/input bytes, 1 MiB plan, and a two-second cooperative
planning deadline. These are not whole-process RSS or hard CPU limits.
Each guest retains existing Wasmi fuel/memory/host-call/IO limits and a 60-second
publication lease. Expiry fences results; it does not terminate a host process.

## Durable authority and compatibility

One immediate-durability redb transaction publishes the immutable run, source,
all action jobs and ownership links in the existing action database. A reused run
ID with a different plan fails. Every claim checks dependencies and unexpired
matrix slots inside the writer transaction. Existing CLI/API action commands
cannot bypass readiness. Failure atomically cancels dependent work and fail-fast
matrix siblings; cancellation removes leases so old workers cannot publish later.
An expired worker may still consume CPU: logical slot limits are not OS isolation.

Managed jobs use a distinct execution profile. Older binaries refuse to execute
or export them rather than ignore dependency metadata. New builds without the
optional planner still enforce gates and can inspect/run/cancel stored workflows.
Legacy standalone jobs remain unchanged. No existing database is converted or
deleted. HTTP sessions own no durable workflow state. The existing individual
action HTTP routes honor workflow gates; this increment does not add workflow
submission over HTTP, MCP, remote identities or multi-tenant authorization.

The workflow driver drains ready work and returns when work is held by other
processes. It is not a polling daemon. Retrying resumes from durable receipts;
there is no exactly-once guarantee for arbitrary external effects (these guests
have none). Concurrent cancellation or expiry can reject a worker's publication:
inspect the stored state rather than treating an interrupted request as success.

Exports containing workflow state use `gitr-action-export-v2`, retaining exact
plans, ownership, source objects and jobs in one read snapshot and checksum.
Standalone-only exports remain v1. Missing ownership/objects and active workers
fail export. Restore/import, physical power-loss recovery, multi-host failover,
full forge/Actions compatibility and performance gains remain unproven here.

## Use the implementation

With the pinned Lustr coordinator installed, from the Gitr source directory:

```sh
rustup run 1.99.1 lustr --engine rustc build --manifest-path rust/gitr/Cargo.toml --locked --features workflows
# Install a bounded Wasm module first; put its module_sha256 in committed YAML.
gitr -C REPOSITORY actions workflow plan check --path .gitr/workflows/check.yml
gitr -C REPOSITORY actions workflow submit check --path .gitr/workflows/check.yml
gitr -C REPOSITORY actions workflow run check
gitr -C REPOSITORY actions workflow status check
gitr -C REPOSITORY actions workflow cancel check
```

Use the resulting `rust/gitr/target/debug/gitr` executable directly or place it on
PATH. Add `action-api` to the build features to start the separate optional local
HTTP adapter. Lifecycle operations do not require the optional planner feature.

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

The placeholder is deliberately invalid until replaced with the actual lowercase
64-character module digest. Commit the workflow and source before submission.
Reusing a run ID for a different commit or expanded plan fails, even when names
look similar; choose a new run ID for new work.
