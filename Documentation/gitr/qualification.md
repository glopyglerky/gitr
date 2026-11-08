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

The final complete suite passed through `lustr --engine rustc`: **67 passed, zero
failed**, plus one controlled child-process helper described below. Formatting,
Clippy with `-D warnings`, and the optimized release build also passed after
review corrections. [Machine-readable evidence](qualification-evidence.json)
records the tested input hashes, compiler provenance and repository smoke results.

| Suite | Tests | Coverage |
|---|---:|---|
| Native Git | 21 | Git differential checks, packed objects, SHA-1/SHA-256, linked worktrees, byte paths, strict ref compare-and-swap, concurrent writers |
| LFS cache | 9 | Exact pointers, empty content, streaming integrity, corruption, concurrent immutable publication, symlink rejection |
| LFS HTTP and locking | 12 | Real isolated TCP fixtures, credential origin boundaries, redirects, bounded responses, upload/download verification, lock pagination |
| Asset chunks | 12 | Exact hydration, insertion reuse, concurrent import, interrupted input, malformed manifests and corrupt chunks |
| Asset inventory | 3 | Ignore rules, largest-file limits, profiles and symlink handling |
| Git filter interoperability | 1 | Reference Git clean/smudge, commit/checkout, binary and empty files, linked worktrees and `git fsck --strict` |
| Repository database | 9 | Exact commit snapshots, incremental reuse, rollback, SHA-256, byte paths, concurrent processes, killed-writer recovery, pointer coverage, exact token ranking and the observed hash-collision regression |

One ignored database test is a child-process entry point explicitly invoked by
the process tests; it is not an unimplemented feature test.

The real-repository smoke check indexed local snapshot
`93f5c19673fd699fb25311f3f36e983707338218`: 4,893 files, 230 directories and one
submodule. Of those files, 4,842 were eligible for lexical indexing and 51 had
explicit exclusion coverage. The query `atomic publication` returned relevant
Gitr contracts and design documents after the exact-token ranking correction.
The release and debug binaries returned the same search report. A selected-path
graph query produced the commit/tree record and the requested file relation.
These are correctness observations, not a retrieval benchmark or general speedup.
The same indexed tree is preserved in
[`2a0c5ee`](https://github.com/glopyglerky/gitr/commit/2a0c5ee3bbd710f55d512e1168bf85c5bd7006b8);
publication through the GitHub connection gives it different commit metadata.

The [independent review record](review.md) distinguishes corrected implementation
findings from the substantial requirements still on the roadmap.

## Hosted platform checks

The [Rust workflow](https://github.com/glopyglerky/gitr/actions/workflows/gitr-rust.yml)
runs through the same pinned Lustr coordinator on Linux, macOS and Windows.
The first hosted run passed on Linux and exposed two portability assumptions:
Windows needs Git-style path separators without Rust's verbatim-path prefix, and
host filesystems do not all accept arbitrary Git filename bytes. The implementation
now normalizes Windows display paths, while the fixtures construct raw Git objects
and indexes without requiring those filenames to exist on disk. Object/index and
catalog byte-path assertions run on all platforms. The physical non-UTF-8 worktree
portion runs only where the filesystem can represent that name.

Hosted results are recorded separately from the local Linux evidence above; see
the linked workflow for the result associated with each published revision.

## Experimental Cranelift result

**Not qualified for this dependency set.** The pinned backend compiled the package,
but the database test process aborted while serializing vector components:

```text
llvm.x86.sse2.pmulhu.w is not yet supported.
core::core_arch::x86::sse2::_mm_mulhi_epu16
zmij::to_bcd_4x4
zmij::Buffer::format_finite::<f32>
serde_json::ser::CompactFormatter::write_f32
gitr::db::index
```

The failing dependency is `zmij 1.0.23`, used by `serde_json 1.0.154`. A focused
rerun of `sha256_repositories_keep_their_native_object_identity` reproduced the
abort. This is a runtime backend-intrinsic limitation, not a Git SHA-256 identity
mismatch. No compiler flags were used to pretend to run under Miri, disable the
platform ABI, or hide the failing test. The supported route remains Lustr's
explicit `rustc` engine. Native Lust compilation is also not qualified.

The backend points to its [SIMD intrinsic tracking issue](https://github.com/rust-lang/rustc_codegen_cranelift/issues/171).
Requalify the full suite before adopting a backend revision or a dependency
implementation that addresses this instruction.

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
