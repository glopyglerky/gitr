# Native Git import qualification (2026-11-10)

[PR #1](https://github.com/glopyglerky/gitr/pull/1) merged the historical Rust Git
source import into `master` at
`5f4aea7467fd9c05d815c4bf1399d9e932f91267` after
[hosted qualification run 42734454475](https://github.com/glopyglerky/gitr/actions/runs/42734454475)
passed. The selected Linux build, tests and four output-verified benchmarks are
qualified here. **Full Git parity and a general speed advantage remain unproven.**

## Exact source and compiler

| Identity | Revision |
| --- | --- |
| PR candidate | `72a7a71eb1d942284c65fce29daf7f1eacac4d2c` |
| GitHub CI checkout | `9a4495b10fd19c833179a26d58c64f997426e19a` |
| Final merge into master | `5f4aea7467fd9c05d815c4bf1399d9e932f91267` |
| Shared source tree | `932e504f41c8efe7fa908a53d79f6a494ae539e6` |
| Produced BlueFe compiler | `7005dc02c9fb6674380eade443022bbc04215276` |
| Lustr coordinator | `6339fbb104d6adda4be7e6be7dc637a6a9e5586b` |

The candidate, synthetic test merge and final merge have identical trees. CI
checked out the synthetic merge; it did not literally check out the final merge
commit. This evidence and its documentation were added afterward and do not
change application, test, dependency or qualification-helper source.

Application compilation used pinned Lustr with the produced BlueFe compiler.
The benchmark helper used that produced compiler directly. Stock Rust compiled
only the dependency-free CI coordinators. The runner was hosted Ubuntu 24.04,
Linux x86-64, with four available execution threads. Compiler versions, binary checksums, hardware information and
profile settings are retained in [the evidence directory](native-git-evidence/).

The imported source is the complete Rust CLI, library and test-support package
family from [historical Grit](https://github.com/gitbutlerapp/grit/tree/e197103087b1110db2d629c4c269ef9258508169):
`grit-git`, `grit-lib` and `grit-test-support`. The source verifier checked **450
original records, 54 modified originals, 21 additions and one supplementary
notice**. The imported directory contains 448 Rust files and 439,251 Rust lines.
Other historical workspace packages, including HTTP and protocol services, are
outside this import. The CLI/root remain GPL-2.0-only; library and test-support
MIT notices are retained. See the [source and license record](rust-git-import.md).

## Executed checks

| Check | Result |
| --- | --- |
| Root build, all features and targets | Passed |
| Root tests | 146 passed, 0 failed; 13 targets |
| Imported library/CLI workspace tests | 705 passed, 0 failed; 56 targets |
| Controlled subprocess fixture entries | 6 root and 1 donor entries marked ignored, invoked by parent tests |
| Archive-validator tests / produced-compiler smoke test | 8 / 1 passed |
| Imported source inventory and SHA-256 verification | Passed |
| redb 4.5.0 storage policy | Passed |
| Rust-only source audit | Passed |
| Optimized all-feature release build | Passed |
| Four output-verified benchmark workloads | Passed |

The **851 passing application/donor tests** are actual results from this run.
They are not the historical upstream shell-suite headline, and tests proving
that unsupported operations are rejected do not establish implementation parity.
The seven controlled fixture entries are identified separately from that total.

The final correction commits fix namespaced packed-reference conflict detection,
compare raw HEAD under the same lock used for publication, and strengthen the
imported receiver's packet framing, checksum/version, delta-offset and policy
checks. Regressions cover valid pack-to-redb ingestion and stale ref rejection.
The unreadable-file fixture now captures metadata after chmod, preserving the
ctime/racy-timestamp content checks. Three suites now isolate inherited Git
configuration while retaining explicit local/configuration fixtures and the
production inspection guard.

The standalone donor retains its historical native local SHA-256 push path when
both repositories use SHA-256. This is conformance coverage for that path; it
neither enables product push nor qualifies the checked receiver for SHA-256.

## Observed performance

Reference Git was **2.56.0**. Each engine received one warmup and seven measured
invocations per workload, with alternating order and a warm shared OS cache.
Every invocation had to succeed and match reference stdout and stderr exactly.
Timings include process startup and output capture. Medians were independently
recalculated from the retained samples.

| Workload | Git median ms | Gitr median ms | Gitr / Git |
| --- | ---: | ---: | ---: |
| Hash a 32 MiB blob | 71.984000 | 22.954420 | 0.318882 |
| Status, 1,000 tracked files | 1.821752 | 4.812936 | 2.641927 |
| Log, 200 commits | 3.167139 | 9.220970 | 2.911451 |
| Recursive tree listing, 1,000 files | 1.173465 | 2.668416 | 2.273963 |

A ratio above one means Gitr was slower. Hashing took 31.9% of Git's elapsed time
in this run. Status, log and tree listing took 2.27–2.91 times as long.

The fixture has 1,000 tracked files, 20 modifications, 10 untracked files and 200
linear commits sharing one tree. It uses SHA-1 and ordinary loose Git objects;
**redb performance and peak memory were not measured**. The CI release profile
uses optimization level 3, LTO disabled, 16 codegen units, no debug information
and all root features. The repository's default release profile still uses thin
LTO. Shared-runner contention and Git's packaged build configuration limit the
comparison. These four read workloads establish neither universal speed nor
write, transport, packed-object or cold-cache performance.

Raw [samples](native-git-evidence/benchmarks/samples.csv),
[results](native-git-evidence/benchmarks/results.json),
[commands](native-git-evidence/benchmarks/commands.json),
[fixture](native-git-evidence/benchmarks/fixture.json) and
[metadata](native-git-evidence/benchmarks/metadata.json) are retained.

## Remaining acceptance work

The product enables core staging, commits, simple branches/tags, ordinary local
branch/detached checkout, object plumbing and inspection. Default routes for
clone/fetch/push/pull, merge/rebase and other unconverted history/maintenance
writers remain disabled pending their checked publication migration. Imported
source and standalone donor tests are not enabled-product parity.

The next work is migrating those writers to checked ref transactions, qualifying
transport, and completing redb authority for refs/indexes/pack ingestion/GC and
coordinated backup. Existing object storage remains opt-in and keeps durable
standard loose-object mirrors. Non-UTF-8 CLI paths, all Git options/error modes,
authenticated GitHub roundtrips, complete bounded-memory object reads and
Windows/macOS execution remain acceptance gaps. Local redb serialization does
not provide distributed HA. The Rust source import also retains transitive
native TLS/OS dependency boundaries.

The [compatibility matrix](compatibility.md) and
[import record](rust-git-import.md) define these limits. The
[machine-readable qualification record](native-git-evidence/qualification.json)
links the tested tree, artifact digest, source hashes, counts and measurements.
Raw test/build/check logs and individual benchmark samples are retained here.
The [artifact manifest](native-git-evidence/artifact-manifest.json) records the
retained workflow files' digests and the sizes/hashes of verified output bytes. The two
large archive-member listings and raw benchmark output files were not copied;
they remain in the original workflow artifact. No executable archive is added.
