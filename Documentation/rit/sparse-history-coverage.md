# Sparse path history and explicit starting tips

Recorded November 10, 2026. This work continues from
[`87649428b17762d3c33b90f543bc733fe19aaa96`](https://github.com/glopyglerky/rit/commit/87649428b17762d3c33b90f543bc733fe19aaa96),
the completed Rit rename. Its separate
[produced-BlueFe qualification](https://github.com/glopyglerky/rit/actions/runs/42771233574)
passed. This change fixes a bounded part of history selection. Full Git parity
remains unfinished.

## Behavior changed

Path-limited `log` and `rev-list` could discard an explicitly selected side
branch when a later merge preserved its first parent's tree for the selected
path. The simplifier reconstructed starting points from maximal DAG tips,
losing the fact that the user had also selected an ancestor tip independently.
The corrected traversal retains those explicit positive starts alongside inferred
starting points. It intersects them with eligible records, so excluded commits
are not reintroduced. Inferred tips remain necessary when filters have removed
a selected head from the records supplied to simplification.

`log --sparse` already reached the revision-option parser, but its value never
reached history simplification. `rev-list` discarded both `--sparse` and
`--dense` before that parser. Both commands now honor these options, with the
last option taking precedence. Sparse mode retains commits reached by the
path-limited walk, including unchanged linear commits and empty roots. It still
follows the selected parent when ordinary history simplification diverts a
merge, and it still runs the separate merge-collapse rules when requested.

Sparse parent output no longer skips commits that are themselves displayed.
First-parent traversal limits which edges are walked; it does not by itself
remove physical secondary parents from output. A real parent diversion can
truncate that list. Full-history, ranges, shallow boundaries, explicit positive
object IDs, and independently selected branches are covered by differential
checks.

Default graph/topological ordering is recomputed after parent diversion.
Full-history, merge simplification, ancestry-path, and date-based ordering retain
the order established before simplification. This distinction matters for tied
or backdated commit timestamps and for merges whose parents are collapsed.

The old simplification entrypoints remain as wrappers. The `fast-export`
callsite initializes the added option to false, preserving its existing
behavior; this change does not qualify sparse fast-export support. No object,
reference, index, or working-tree publication algorithm changes.

## Regression tests and review

Five new tests in
[`history_sparse_tips.rs`](../../vendor/sley/crates/sley-cli/tests/history_sparse_tips.rs)
perform **270 exact comparisons** of status, stdout, and stderr with pinned Git
2.55.0, across SHA-1 and SHA-256. Four additional checks verify failure outcomes
for missing required objects while preserving existing native diagnostic and
exit-status differences. They do not claim exact corruption-error parity.

The new tests select the actual root Rit executable through `RIT_TEST_BIN`,
require an absolute reference binary, isolate configuration, and execute the
candidate with no Git on its PATH. They cover explicit and unselected side tips,
`--all`, raw object IDs, option and revision order, exclusions, sparse/dense
selection, full history, merge simplification, first-parent traversal, graph and
parent output, absent paths, filtered heads, count limits, reverse/date/topological
order, tied/backdated clocks, shallow boundaries, and missing trees and commits.

The first candidate sorted too broadly after simplification. Review caught
24 commands that had matched Git and then changed order, plus six sparse
merge-simplification mismatches. The upstream replay also caught three regressed
`t6012` cases per object format. The final guard restricts the extra sort to
default parent diversion in graph/topological modes. All of those regressions
are corrected. The retained intermediate records describe rejected candidates,
not the final result.

Independent final review used the same frozen root executable as the upstream
replay:

| Review | Result |
| --- | --- |
| Spec matrix | 174/174 exact, up from 66/174 on the baseline; 108 newly passing and zero regressions |
| Standards matrix | 168/216 exact; 48 preexisting mismatches and zero regressions against the baseline |
| Missing-object matrix | 16 checks; all six sparse unused-tree cases match Git, and all eight required-missing-commit cases reject traversal |

The 48 remaining Standards mismatches concern plain `log %P` parent formatting
and equal-clock date ordering. Their baseline and final outputs are retained.
The date-order issue also reproduces without path filtering. These results do
not establish global pretty-format or ordering parity.

Two existing log fixtures also lacked explicit identities for an oracle merge
and an encoded `commit-tree`. Those setup commands now provide identities, so
the existing 17-test log suite runs without a developer's global Git identity.

## Executed local qualification

Application checks used pinned Lustr
`6339fbb104d6adda4be7e6be7dc637a6a9e5586b`, its explicit rustc route, and
Rust 1.99.1. The reference is Git 2.55.0 at
`e9019fcafe0040228b8631c30f97ae1adb61bcdc`.

| Check | Result |
| --- | --- |
| Root all-feature/all-target checks | Five integration tests passed |
| Selected CLI tests | 121 passed, including the five new tests and 17 existing log tests |
| Changed revision engine library | 85 passed |
| Qualification coordinator | Eight passed |
| Import verification | 679 files verified: 36 adapted and ten locally added |
| Composition policy and source audit | Passed |

The CLI selection contains both donor tests and product-directed regressions.
The new target and earlier product-directed targets execute the root Rit binary;
existing donor tests retain their native standalone route. Counts are test
executions, not a full-conformance measure.

The [hosted workflow](https://github.com/glopyglerky/rit/actions/workflows/rit-native.yml)
now includes the existing log target and the new regression target. It records
the separate produced-BlueFe result for the published commit. Local rustc passes
do not substitute for that gate.

## Pinned upstream script results

Four scripts were executed against the frozen baseline, the final actual Rit
binary, and the reference Git in both object formats. Each stage has 16 runs,
and all 32 runs completed their TAP plans. Counts below are per object format;
SHA-1 and SHA-256 agree. No cases were disabled and no prerequisites were changed.

| Script | Baseline Rit pass/fail/skip | Final Rit pass/fail/skip | Reference Git pass/fail/skip |
| --- | --- | --- | --- |
| `t6004-rev-list-path-optim` | 7/0/0 | 7/0/0 | 7/0/0 |
| `t6012-rev-list-simplify` | 42/0/0 | 42/0/0 | 42/0/0 |
| `t6016-rev-list-graph-simplify-history` | 8/4/0 | 10/2/0 | 12/0/0 |
| `t6019-rev-list-ancestry-path` | 11/7/0 | 11/7/0 | 18/0/0 |

Exactly four execution results changed: graph cases 8 and 9 now pass in each
format. Every other test-result line is unchanged. Two of the four scripts match
Git exactly in both formats; the number of fully matching scripts in this
selection did not increase. The change closes two specific graph-history cases.

The remaining graph failures are decoration pruning in case 5 and boundary
ordering in case 12. The ancestry script retains its existing failures for
explicit ancestry-path arguments and criss-cross exclusions. Full-suite
conformance, archive streaming, native filter-branch, broader transport and
filename qualification, and Windows/macOS execution remain open.

## Evidence and reproduction

[`sparse-history-evidence`](sparse-history-evidence) retains baseline and final
TAP, failure details, local logs, independent comparison records, rejected
intermediate review results, executable/source fingerprints, and the exact
changed test-result lines. [The result record](sparse-history-evidence/result.json)
indexes the retained evidence and qualification scope.

Set `RIT_REFERENCE_GIT` and `SLEY_TEST_GIT` to the absolute pinned reference Git.
Use `RIT_TEST_BIN` to select an absolute path to the built root executable:

```sh
lustr --engine rustc test --locked --all-features --all-targets --no-fail-fast
RIT_TEST_BIN=/absolute/path/to/rit lustr --engine rustc test \
  --manifest-path vendor/sley/Cargo.toml --locked -p sley-cli \
  --test history_sparse_tips --test log --test rev_list \
  --test rev_list_path_parents --no-fail-fast
lustr --engine rustc test --manifest-path vendor/sley/Cargo.toml --locked \
  -p sley-rev --lib
lustr --engine rustc run --all-features --locked --example verify_import
lustr --engine rustc run --manifest-path xtask/Cargo.toml --locked -- policy
lustr --engine rustc run --manifest-path xtask/Cargo.toml --locked -- audit
```

The full CLI invocation and external script arguments are recorded in the logs
and JSON. The [external-script procedure](../gitr/git-coverage-continuation.md#reproduction)
uses the built root executable installed under the name `git` for the upstream
runner. Sley remains pinned at
`a71654f50de24b3c750f0df5f1829db7342491f6`; original source identities and
license records are retained in [the import manifest](../gitr/sley-import.json).
Dependency versions and lockfiles are unchanged.
