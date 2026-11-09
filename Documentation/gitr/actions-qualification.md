# Database-resident action qualification

Current root layout and produced BlueFe qualification: [rust-root.md](rust-root.md).
Compiler versions and measurements below retain their historical scope.


The standalone `gitr` executable now includes `actions install`, `submit`, `run`,
`status`, `cancel`, `artifact` and `control`. The code and dependency lockfile were
activated from the exact immutable blobs emitted by successful
[run 42533179289](https://github.com/glopyglerky/gitr/actions/runs/42533179289),
job `128891666765`, using input commit
`4f6e840cd8b3d8e145d1446b238031d182606961` plus the reviewed integration script.
The preparation script is removed: ordinary builds compile the committed source
without patching it. The ongoing workflow has read-only permissions and tests
the real Rust guest on Linux, macOS and Windows.

## Observed Linux evidence, 2026-11-08

Lustr coordinator `62494e27717a64c8612b538409de27048fa6adc2`, engine `rustc`, Rust
1.99.1; Ubuntu 24.04 runner; reference Git 2.56.0.

- 12 action unit tests passed, zero failures.
- Existing Git/LFS/chunk/catalog tests: 67 passed, zero failures. One ignored
  helper is explicitly invoked by the existing child-process tests.
- Clippy `--all-targets -- -D warnings` passed after correcting SHA-256 encoding
  for sha2 0.11 and propagating all redb durability configuration failures.
- The actual Rust guest compiled through Lustr to `wasm32-unknown-unknown`.
- The end-to-end CLI fixture installed that module, stored an immutable Git blob
  input, ran it, retrieved the artifact and obtained the identical receipt on
  retry. Dirty worktree bytes were excluded. A committed whitespace violation
  returned a failed receipt, exit status 1 and no published artifact.
- Reference `git fsck --strict` passed in the fixture.

Guest module SHA-256:
`73162c366e26f4cccb00168c66fdd0b75dac40aedf67106b88f268ef021b1a42`.
The passing report was `{"bytes":13,"lines":1,"trailing_whitespace":false}`.
This is functional execution evidence, not a speed benchmark.

## Selected storage: redb

The working action implementation uses redb, which is also the user's selected
backend. The [redb decision](redb-decision.md) supersedes the earlier SurrealDB
migration proposal. Retain claim fencing, idempotency, short publication
transactions and atomic artifact/receipt semantics in this implementation.
Existing redb files and the newer read-only `actions export` functionality are
preserved; no database conversion is part of the backend-policy change.

The historical test counts above describe the recorded run, not a new execution
of every later change. Retired database candidates and their workflows are
archived and cannot qualify a backend switch. Read the actual workflow result
for each new commit before attributing a passing runtime check to it.

## Exact scope

This is a bounded core-Wasm action primitive, not a complete forge, DAG workflow
scheduler, GitHub Actions replacement, deployment engine or multi-host HA
implementation. The local JSON control command is not MCP/HTTP and assumes the
invoking OS user is authorized for the repository. Cross-platform results belong
to their actual workflow run; Linux success is not proof of them.

Claims expire after 60 seconds; execution does not renew them. An expired worker
cannot publish, but may still consume CPU until it stops. Fuel and guest-memory
limits are not compilation wall-time or whole-process RSS guarantees. Production
multi-tenant workers need OS isolation, quotas and hard timeouts in addition to
Wasm limits. Process recovery is lease-based and permits repeated execution;
external side effects are not covered by an exactly-once guarantee.

## Reproduce

With the pinned Lustr coordinator installed and Rust 1.99.1 selected:

```sh
lustr --engine rustc test --manifest-path Cargo.toml --locked --all-targets
lustr --engine rustc clippy --manifest-path Cargo.toml --locked --all-targets -- -D warnings
# Use a reviewed bundle including wasm32-unknown-unknown std.
lustr --engine rustc build --manifest-path crates/action-example/Cargo.toml --locked --release --target wasm32-unknown-unknown
python ci/gitr-actions-smoke.py
```

The smoke test uses a disposable repository; reference Git is used only to build
and validate the fixture. The action's snapshot read and guest execution use Rust.
