# Qualification

This document records checks actually run for the initial Rust implementation.
It does not certify full Git compatibility, distributed HA, semantic code search,
or a performance improvement.

## Compiler provenance

- Lustr coordinator: `62494e27717a64c8612b538409de27048fa6adc2`.
- Default route: `lustr --engine rustc`, Rust `1.99.1`.
- Experimental route: `lustr --engine cranelift`, nightly `2026-11-05`, backend
  `8edf4d040acb77048e04b52a379e93dccae888e6` built with `unwinding`.
- Reference Git: `2.51.1`.
- Local execution platform: Linux, `x86_64-unknown-linux-gnu`.

The full Rust dependency graph is compiled through Lustr's selected Cargo/rustc
route. Native Lust compilation is not claimed. The Cranelift route uses a different
compiler version and is a compatibility check, not a controlled speed comparison.

## Executed tests

The individual suites passed during implementation. The full gates are being
repeated after removing generated build artifacts from a full local filesystem.
Final results will be recorded before publication.

| Suite | Tests | Coverage |
|---|---:|---|
| Native Git | 21 | Git differential checks, packed objects, SHA-1/SHA-256, linked worktrees, byte paths, strict ref compare-and-swap, concurrent writers |
| LFS cache | 9 | Exact pointers, empty content, streaming integrity, corruption, concurrent immutable publication, symlink rejection |
| LFS HTTP and locking | 12 | Real isolated TCP fixtures, credential origin boundaries, redirects, bounded responses, upload/download verification, lock pagination |
| Asset chunks | 12 | Exact hydration, insertion reuse, concurrent import, interrupted input, malformed manifests and corrupt chunks |
| Asset inventory | 3 | Ignore rules, largest-file limits, profiles and symlink handling |
| Git filter interoperability | 1 | Reference Git clean/smudge, commit/checkout, binary and empty files, linked worktrees and `git fsck --strict` |
| Repository database | 7 | Exact commit snapshots, incremental reuse, rollback, SHA-256, byte paths, two writer processes, a killed writer and concurrent readers |

One ignored database test is a child-process entry point explicitly invoked by
the process tests; it is not an unimplemented feature test.

## Reproduction

With the pinned Lustr binary and the selected Rust toolchain on `PATH`:

```sh
lustr --engine rustc fmt --manifest-path rust/gitr/Cargo.toml -- --check
lustr --engine rustc test --manifest-path rust/gitr/Cargo.toml --locked --all-targets
lustr --engine rustc clippy --manifest-path rust/gitr/Cargo.toml --locked --all-targets -- -D warnings
lustr --engine rustc build --manifest-path rust/gitr/Cargo.toml --locked --release
```

For a constrained build volume, set `CARGO_PROFILE_DEV_DEBUG=0`,
`CARGO_PROFILE_TEST_DEBUG=0`, and `CARGO_INCREMENTAL=0`. These reduce compiler
artifacts; they do not change the tests' requested behavior.

## Limits of this evidence

- The cross-platform GitHub Actions workflow is configured for Linux, macOS and
  Windows. Local Linux execution is not evidence that the other runners passed.
- LFS network tests use controlled servers. A real authenticated GitHub LFS
  transfer has not been executed.
- The 8 MiB insertion fixture reuses over half its original bytes and hydrates
  both versions exactly. This is a deterministic correctness check, not a general
  storage, CPU, latency or bandwidth benchmark.
- Process termination tests cover transaction rollback and lock release. They do
  not simulate disk/controller power loss or a network partition.
- The original C Git test suite has not been rerun for this standalone Rust
  package. Its supported command subset is qualified by the differential tests.
