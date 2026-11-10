# Ancestry-path selection and export connectivity

Recorded November 10, 2026. This change continues from
[`b40277b3c7cea2a607640df70d1fb9de28340a25`](https://github.com/glopyglerky/rit/commit/b40277b3c7cea2a607640df70d1fb9de28340a25).
The pinned upstream ancestry-path script now passes all 18 cases in both SHA-1
and SHA-256. Full Git parity remains unfinished.

## Behavior changed

`log`, `rev-list`, and `fast-export` now accept explicit
`--ancestry-path=<commit>` anchors. Repeated options combine their selections.
Bare `--ancestry-path` uses the excluded revision tips as anchors. The supported
commands use a new ancestry-aware setup entrypoint; other consumers keep the
existing generic parser and their previous unsupported-option behavior.

Selection follows the eligible revision walk. It computes ancestors and
descendants separately, then combines them, so a shared ancestor does not pull
in unrelated cousins. An anchor outside the walk cannot introduce new history
or bridge an excluded intermediate commit. First-parent and shallow boundaries
remain boundaries. Message and author filters can hide output without erasing
the graph nodes needed to establish ancestry.

This implements the bounded selection contract described by the pinned
[Git revision options](https://github.com/git/git/blob/e9019fcafe0040228b8631c30f97ae1adb61bcdc/Documentation/rev-list-options.adoc).
Two observed Git 2.55.0 details are preserved explicitly:

- An unpeeled annotated-tag anchor selects the tagged commit and descendants.
  A commit ID or explicitly peeled tag also selects ancestors. A typed anchor
  retains both the original object ID and its peeled commit ID.
- Packed queries using `--use-bitmap-index` can bypass ancestry limiting in the
  reference. The existing bitmap path remains available; forcing the ordinary
  ancestry filter there would change previously matching results. Loose and
  packed cases are compared separately. This is not a claim that ancestry
  limiting applies uniformly to bitmap queries.

The three limited-output/metadata shortcuts in `log` and the metadata shortcut
in `rev-list` now avoid bypassing active ancestry filtering. In particular,
plain and count-only `rev-list` requests no longer include unrelated commits in
criss-cross histories. Ancestry selection also runs before bisection chooses its
result.

For ancestry-filtered `rev-list --children`, child edges come from the ordered
records before ancestry and path pruning. A child hidden from displayed
history can therefore remain in its displayed parent's child list. Later
`--reverse` changes displayed commit order without reversing those child lists.
The snapshot is used only when ancestry limiting is active.

`fast-export` computes ancestry before removing commits already present in an
imported marks file. Marks suppress emission; they do not sever the graph.
Exact stream comparisons cover marked intermediate commits, repeated anchors,
tag anchors, and both object formats.

Direct positive `--no-walk` requests in `log` and `rev-list` retain their tip
selection, including bare ancestry requests without excluded bottom commits.
Invalid explicit anchors still reject. Broader no-walk combinations are listed
under remaining limitations below.

## Regression tests and review

The new
[`history_ancestry_paths.rs`](../../vendor/sley/crates/sley-cli/tests/history_ancestry_paths.rs)
target contains ten tests. Its final run performs **522 exact comparisons** of
status, stdout, and stderr against pinned Git, across both object formats.
It also performs **two export-retention outcome checks** and **four child-list
equivalence checks** for existing no-walk limitations. Those six checks do not
claim exact Git parity.

The target selects the actual root executable through `RIT_TEST_BIN`, requires
an absolute reference executable, isolates Git configuration, and removes Git
from the candidate's PATH. It covers anchor unions and order, raw and tag object
IDs, exclusions, range boundaries, count/limit/order options, filters, paths,
commit graphs and bitmaps, invalid and noncommit anchors, shallow history,
export marks, no-walk selection, and child edges.

The final test source was also run against the original frozen baseline: eight
feature tests fail and the two containment tests pass. The two containment tests
were separately shown to fail against rejected candidate v2 before its guards
were corrected.

### Rejected intermediate behavior

Review caught an unsafe interaction in v2. A positive-only
`fast-export --no-walk --ancestry-path` request could emit no commits and reset
the requested reference to the zero object ID, despite exiting successfully.
The baseline's generic fast-export no-walk behavior already differed from Git,
so a simple exact-match count did not reveal this deterioration.

The final guard applies ancestry limiting only when walk mode is selected
or the request contains negative revisions. It restores the baseline bare no-walk export
stream byte-for-byte, retains the selected tip, and prevents the new zero-ref
reset. A matching guard prevents inactive ancestry processing from changing
existing no-walk child-list order. These guards contain the regression; they
do not implement general fast-export no-walk support.

Earlier review also corrected lost annotated-tag identity and missing child
edges. The retained v1 and v2 records are explicitly intermediate evidence.
Only v3, with SHA-256
`5925c272cb97352c55a2f1828f91a3ac62709473a067ab6ea41a0ab9dd3c6749`,
is the final qualified root executable.

### Independent final results

| Review | Final result |
| --- | --- |
| Spec matrix | 206/208 exact, up from 16/208; 190 newly passing and zero regressions |
| Minimal annotated-tag matrix | 60/60 exact across three commands and both formats |
| Standards comparisons | 50/64 exact, 14 retained compatibility mismatches, zero previously exact cases becoming failures |

The two remaining main Spec cases are existing no-revision `rev-list` usage
text and exit-status differences. The Standards comparisons include 32 exact
imported-mark exports, eight exact bisection results, eight exact bare no-walk
log/rev-list controls, and two exact walking child-list controls. Its remaining
cases concern six unsupported generic clients and eight broader no-walk export
or child-list combinations. The corrected bare export and four inactive
child-list controls are restored to their baseline bytes. Two explicit
no-walk export cases still differ from Git after becoming accepted arguments.

A separate 42-case child-output probe was executed on v2, not replayed on v3.
It is retained only as `children-intermediate-v2.json`. It identifies existing
general child-list, grep/path, and `--children --simplify-merges` differences.
It is not counted as final v3 execution evidence.

## Executed local qualification

Application checks used pinned Lustr
`6339fbb104d6adda4be7e6be7dc637a6a9e5586b`, its explicit rustc route, and Rust
1.99.1. The reference is Git 2.55.0 at
`e9019fcafe0040228b8631c30f97ae1adb61bcdc`.

| Check | Result |
| --- | --- |
| Root all-feature/all-target checks | Five integration tests passed |
| Selected CLI tests | 136 passed, including ten new tests and five existing fast-export tests |
| Changed revision engine library | 85 passed |
| Qualification coordinator | Eight passed |
| Import verification | 680 files verified: 36 adapted and 11 locally added |
| Composition policy and source audit | Passed |

The selected CLI set contains both donor tests and product-directed tests.
The new target and existing product-directed targets execute the frozen root
Rit binary; other donor tests retain their standalone route. The revision
library's final source fingerprint is unchanged from its recorded 85-test run.
No dependency versions or lockfiles changed.

The [hosted workflow](https://github.com/glopyglerky/rit/actions/workflows/rit-native.yml)
now includes the new target and the existing fast-export target. It records
separate produced-BlueFe qualification for the published commit. Local rustc
results do not replace that gate.

## Pinned upstream results

Five scripts were executed against the frozen baseline, the final actual root
Rit executable, and the reference Git. Both stages contain 20 executions across
SHA-1 and SHA-256; all 40 TAP plans completed. Counts below are per object
format, and both formats agree. No tests were disabled or prerequisites changed.

| Script | Baseline Rit pass/fail/skip | Final Rit pass/fail/skip | Reference Git pass/fail/skip |
| --- | --- | --- | --- |
| `t6004-rev-list-path-optim` | 7/0/0 | 7/0/0 | 7/0/0 |
| `t6012-rev-list-simplify` | 42/0/0 | 42/0/0 | 42/0/0 |
| `t6016-rev-list-graph-simplify-history` | 10/2/0 | 10/2/0 | 12/0/0 |
| `t6019-rev-list-ancestry-path` | 11/7/0 | 18/0/0 | 18/0/0 |
| `t6111-rev-list-treesame` | 65/0/0 | 65/0/0 | 65/0/0 |

Exactly 14 execution results improve: ancestry cases 4-8, 17, and 18 in each
format. Every other result line is unchanged. Final Rit totals are **284 passed,
four failed, zero skipped**, compared with **288 passed** for the reference.
Four of these five scripts now match exactly in both formats, up from three.
This is a measured five-script selection, not a whole-suite coverage estimate.

## Remaining limitations

The selected graph script still fails decoration pruning and boundary ordering.
Existing differences also remain in generic negative-range/no-walk handling,
fast-export no-walk traversal, general child-list formatting and filtering,
some anchored log-grep patterns, no-revision usage diagnostics, and export
parent formatting when the original first parent is filtered. Some newly
accepted option combinations encounter those preexisting implementation gaps;
explicit acceptance does not establish every combination's parity.

Full-suite conformance, archive streaming, native filter-branch, broader
transport/authentication and filename qualification, and Windows/macOS
execution remain open. Database and forge integration still follow native Git
compatibility.

## Evidence and reproduction

[`ancestry-path-evidence`](ancestry-path-evidence) retains baseline/final TAP,
failure details, local test logs, independent matrices, rejected-candidate
records, and executable/source fingerprints.
The [result record](ancestry-path-evidence/result.json) indexes the scope and
raw evidence. Original Sley object identities and licenses remain recorded in
the [import manifest](../gitr/sley-import.json), still pinned to
`a71654f50de24b3c750f0df5f1829db7342491f6`.

Set `RIT_REFERENCE_GIT` and `SLEY_TEST_GIT` to an absolute pinned Git executable.
Use `RIT_TEST_BIN` to select the built root Rit executable:

```sh
lustr --engine rustc test --locked --all-features --all-targets --no-fail-fast
RIT_TEST_BIN=/absolute/path/to/rit lustr --engine rustc test \
  --manifest-path vendor/sley/Cargo.toml --locked -p sley-cli \
  --test history_ancestry_paths --test history_sparse_tips --test fast_export \
  --test log --test rev_list --test rev_list_path_parents --no-fail-fast
lustr --engine rustc test --manifest-path vendor/sley/Cargo.toml --locked \
  -p sley-rev --lib
lustr --engine rustc run --all-features --locked --example verify_import
lustr --engine rustc run --manifest-path xtask/Cargo.toml --locked -- policy
lustr --engine rustc run --manifest-path xtask/Cargo.toml --locked -- audit
```

The full selected CLI invocation is in `cli-tests.log`. Upstream commands,
environment settings, complete TAP plans, and result lines are in
`upstream-baseline.json` and `upstream-final.json`. Follow the existing
[external-script procedure](../gitr/git-coverage-continuation.md#reproduction)
with the root executable installed under the name `git` for the upstream runner.
