# History and commit coverage

This is historical native-engine evidence. Current native reproductions must
set `RIT_BACKEND=native`; compatibility execution is not native coverage.


Recorded November 10, 2026 UTC. This work continues from
[`f4b1367a704bcc12ae4dc784a862526a07f9fcfa`](https://github.com/glopyglerky/gitr/commit/f4b1367a704bcc12ae4dc784a862526a07f9fcfa).
That baseline completed the full
[produced-BlueFe qualification run](https://github.com/glopyglerky/gitr/actions/runs/42759226333).
This change fixes two command behaviors and expands measured compatibility.
Full Git parity remains unfinished.

## Commit summaries for oversized files

A commit could be published successfully, then return failure while its summary
tried to decompress an oversized blob. The post-commit shortstat now uses the
existing object-header lookup to classify regular files larger than
`core.bigFileThreshold` as binary. The comparison is strictly greater than the
threshold. Additions, modifications, same-object mode changes, and deletions
retain their distinct statistics.

Malformed, negative, and overflowing threshold values are rejected before
publication, including quiet commits. Command-line configuration takes precedence
over repository configuration. This change does not alter object or reference
publication algorithms.

Explicit text attributes and custom drivers with `binary=false` override size
classification. Their text decision reaches line counting even when the content
contains NUL bytes. Summary rendering reads only the binary/text metadata. It
does not compile hunk-header or word regexes or initialize textconv or external
diff commands. Invalid configuration for those unrelated patch operations cannot
turn a successful commit into a summary failure.

Gitlinks and symlinks keep their existing content handling. Missing, malformed,
or wrong-type metadata falls back to the strict object-read path. The speculative
header probe suppresses only its own diagnostics, so a strict fallback does not
print corruption messages twice. An ordinary corrupt old blob is still read and
rejected before a large replacement can short-circuit classification.

The tests generate an intentionally incomplete object advertising a 64 GiB size.
This checks Git's metadata-only summary behavior without creating that body.
Full `cat-file blob` reads still reject the fixture. Passing its summary test is
not evidence that an incomplete object became valid. The existing loose-header
reader still reads compressed bytes into memory, so this change does not provide
general object streaming or globally bounded memory.

## Parent lists in path-limited history

`rev-list --parents --simplify-merges` could leave a parent edge pointing to an
omitted ancestor that never contained the selected path. Merge simplification
now feeds its recomputed tree-equality flags and parent lists through the
existing parent-rewrite helper. Roots containing the selected path remain in the
result. Explicit range boundaries and shallow boundaries remain intact.

The same code also restores secondary merge-parent edges in `--first-parent`
output while keeping traversal limited to the first parent. Independently visited
secondary branches still receive the normal rewriting behavior. Tests compare
ordinary and reverse order, merges, first-parent traversal, explicit ranges,
shallow histories, and histories without path restrictions.

The new revision target performs 28 exact stdout, stderr, and exit-status
comparisons with Git 2.55.0 across SHA-1 and SHA-256. Both new test targets accept
an absolute `GITR_TEST_BIN`, isolate configuration, and run the candidate with no
usable Git on its PATH. The existing revision-list fixture also supplies explicit
author identity instead of relying on a developer's global configuration.

## Review corrections

Independent standards and behavior reviews caught three details before
publication: compiling an irrelevant patch regex after commit publication,
losing forced-text line counts for NUL content, and duplicate diagnostics from a
speculative object-header read. All three were corrected and covered by
regressions. Reviewers independently reran the regex and forced-text cases and
found no remaining blocker in their targeted rereviews.

Successful commands and configuration errors are compared exactly with Git.
Malformed-object tests additionally preserve existing native diagnostics. Those
native corruption messages are not claimed to match every Git diagnostic.

## Executed local qualification

Local application checks used pinned Lustr revision
`6339fbb104d6adda4be7e6be7dc637a6a9e5586b`, its explicit `rustc` route, and
Rust 1.99.1. Reference Git is 2.55.0 at
`e9019fcafe0040228b8631c30f97ae1adb61bcdc`.

| Check | Result |
| --- | --- |
| Root all-feature/all-target tests | Five integration tests passed; all targets built |
| Selected CLI tests | 99 passed, including nine new commit-summary tests and three new revision-parent tests |
| Actual product executable | Both new targets passed against the rebuilt root `gitr`; earlier archive and revision-selector targets also used it |
| Changed engine libraries | 190 diff/merge tests and 85 revision tests passed |
| Qualification coordinator | Eight tests passed with the stock orchestration compiler |
| Import verification | 678 files verified, including 32 adaptations and nine locally added files |
| Composition policy and source audit | Passed |

Produced-BlueFe qualification remains a separate hosted gate. Its selection now
includes the diff/merge library, both new CLI targets, and the existing commit
and revision-list suites. Local passes do not substitute for that gate.

## Upstream script results

Ten pinned upstream scripts were executed against both the actual product and
reference Git, in both object formats. Every run completed its TAP plan. Counts
below are per object format; SHA-1 and SHA-256 have the same result counts.

| Script | Gitr passed / failed / skipped | Git passed / failed / skipped | Exact test-result match |
| --- | --- | --- | --- |
| `t1500-rev-parse` | 81 / 0 / 1 | 82 / 0 / 0 | No |
| `t4012-diff-binary` | 13 / 0 / 0 | 13 / 0 / 0 | Yes |
| `t4052-stat-output` | 91 / 0 / 0 | 91 / 0 / 0 | Yes |
| `t5000-tar-tree` | 82 / 2 / 6 | 84 / 0 / 6 | No |
| `t5001-archive-attr` | 44 / 0 / 0 | 44 / 0 / 0 | Yes |
| `t5002-archive-attr-pattern` | 19 / 0 / 0 | 19 / 0 / 0 | Yes |
| `t5003-archive-zip` | 78 / 0 / 0 | 78 / 0 / 0 | Yes |
| `t5004-archive-corner-cases` | 12 / 0 / 2 | 12 / 0 / 2 | Yes |
| `t6012-rev-list-simplify` | 42 / 0 / 0 | 42 / 0 / 0 | Yes |
| `t6016-rev-list-graph-simplify-history` | 8 / 4 / 0 | 12 / 0 / 0 | No |

The tar script improved from four failures to two in each format. Cells 82 and
85 now pass because their commit summaries complete. Cells 83 and 84 still need
streaming archive generation for the huge-object fixture. Every other recorded
test-result line is unchanged from the frozen baseline. Seven scripts match
their oracle result exactly in both formats; that is a measured selection, not
a whole-suite percentage or an increase in the number of fully matching scripts.

The four existing graph-history failures remain in cells 5, 8, 9, and 12. The
revision-parse script retains its compatibility-object-mode skip difference.
The two matching skips in `t5004` are expensive tests of archives and files
larger than 4 GB. They remain unmeasured. No prerequisite was advertised or test
disabled to improve these counts.

The pinned Git suite contains 1,046 numbered scripts; Sley's 891-script manifest
is a curated selection. Neither the ten scripts here nor the donor's reported
557 exact matches establishes Gitr's full conformance.

## Evidence and reproduction

[`history-and-commit-evidence`](history-and-commit-evidence) contains local test
logs, baseline and final result records, raw TAP, failure details, and source and
executable fingerprints. The earlier three-script baseline's raw logs remain in
[`coverage-evidence`](coverage-evidence). The intermediate header-probe review
record documents a defect caught during review; final CLI logs contain the
corrected regression result.

Use the pinned Lustr coordinator, with `SLEY_TEST_GIT` and `GITR_REFERENCE_GIT`
set to the absolute external Git 2.55.0 executable:

```sh
export RIT_BACKEND=native
lustr --engine rustc test --locked --all-features --all-targets --no-fail-fast
GITR_TEST_BIN=/absolute/path/to/gitr lustr --engine rustc test \
  --manifest-path vendor/sley/Cargo.toml --locked -p sley-cli \
  --test archive --test archive_remote --test upload_archive \
  --test filter_branch_unavailable --test rev_parse \
  --test rev_parse_pseudo_names --test commit --test commit_large_blob \
  --test rev_list --test rev_list_path_parents --no-fail-fast
lustr --engine rustc test --manifest-path vendor/sley/Cargo.toml --locked \
  -p sley-rev -p sley-diff-merge --lib --no-fail-fast
lustr --engine rustc run --example verify_import --locked
lustr --engine rustc run --manifest-path xtask/Cargo.toml --locked -- policy
lustr --engine rustc run --manifest-path xtask/Cargo.toml --locked -- audit
```

The [external-script procedure](git-coverage-continuation.md#reproduction)
applies to the ten scripts listed above. Exact executed arguments and environment
values are retained in the result records.

Sley remains pinned at `a71654f50de24b3c750f0df5f1829db7342491f6`.
Original source identities and license declarations are retained in
[`sley-import.json`](sley-import.json). Dependency versions and lockfiles are
unchanged. Native filter-branch, shell-i18n, broader transport and filename
qualification, Windows/macOS execution, and full-suite conformance remain open.
