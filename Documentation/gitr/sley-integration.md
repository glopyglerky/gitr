# Sley integration and executed qualification

This is historical native-engine evidence. Current native reproductions must
set `RIT_BACKEND=native`; compatibility execution is not native coverage.


Recorded November 9, 2026, America/Denver. Baseline Gitr commit:
[`fe8e48a59e07c630b59f4a38c2c406b13f58c28b`](https://github.com/glopyglerky/gitr/commit/fe8e48a59e07c630b59f4a38c2c406b13f58c28b).

## Completed scope

At the recorded baseline, the Gitr executable calls the imported Sley implementation in
process. The same Rust entrypoint builds as `gitr` and `git`; `scalar` has its
corresponding entrypoint. This completes an upstream import and working product
connection, not full Git compatibility.

Donor: [HeddleCo/sley](https://github.com/HeddleCo/sley), exact revision
[`a71654f50de24b3c750f0df5f1829db7342491f6`](https://github.com/HeddleCo/sley/tree/a71654f50de24b3c750f0df5f1829db7342491f6),
workspace version 0.13.0, 36 crates. The pinned source is preserved in its
upstream crate layout with original hashes, adaptations, exclusions, and notices
in [sley-import.json](sley-import.json) and [the donor notice](../../vendor/sley/NOTICE.md).

## Recovered fixes that are included

| Area | Change and reason |
| --- | --- |
| Reftable publication | Expected-value and directory/file checks execute while holding the list publication lock. A competing update cannot pass the old check and overwrite the winner. |
| Loose-reference failures | Release individual and packed-reference locks when packed-ref parsing or the prepared transaction hook fails. |
| Full repack | Refuse missing required descendants before pruning. Missing initial roots and explicitly promised objects retain their distinct treatment. |
| Unix staging paths | Preserve raw bytes through argv decoding, normalized pathspecs, worktree scanning, index entries, and `add`. Valid private-use Unicode remains distinct from invalid byte names. |
| Grep pattern files | Preserve file/stdin bytes independently of the internal argv escape representation; a real NUL followed by private-use Unicode must not turn into byte FF. |
| Upload archive | Implement bounded native request handling, repository/ref restrictions, archive-format policy, ACK/flush/sideband framing, and a native dispatch adapter. |
| ZIP dates | Clamp timestamps to the DOS date range instead of overflowing. |

Reftable tests exercise stale updates, checked deletes, create-only writes and
name conflicts across SHA-1/SHA-256 with combined logs on/off. This does not
establish atomic transactions across multiple worktree reftable stacks or fix
all reference-transaction hook differences.

The archive tests compare tar, ZIP, and decompressed gzip content. Different
valid gzip encoders need not emit identical compressed streams. The change
qualifies the native server and local archive behavior; unfinished SSH/git
remote archive client additions were not imported.

## Source-origin corrections

Review found substantial literal overlap between two embedded donor shell
programs and Git's `git-filter-branch.sh` and `git-sh-i18n.sh`. The donor copies
omit original attribution headers. The Apache workspace declaration does not
resolve those specific inherited origins. This is a targeted provenance finding,
not a broader legal conclusion about the donor.

Those embedded programs are excluded from the active source. `filter-branch`
returns an explicit error before mutation. `git-sh-i18n` is not materialized,
and stale copies are rejected/removed from the newly versioned helper directory.
Five short native-dispatch adapters remain, along with Rust gettext/envsubst
and generated donor hook/filter fixtures. The [source contract](rust-only-source.md)
records these compatibility exceptions and native dependency boundaries.

The recovered native filter-branch rewrite remains outside this delivery.
Its focused run passed eight tests and failed three involving `--all` ref
selection and subdirectory-parent rewriting. We did not replace those failures
with skips or count the unfinished rewrite as working source coverage.

## Executed local checks

All application checks use Lustr
`6339fbb104d6adda4be7e6be7dc637a6a9e5586b`, explicitly selecting its `rustc`
engine. The local compiler is stock rustc 1.99.1,
`58cf8f9c35b355199727627d07ae34a4e2846d67`, x86-64 Linux, LLVM 23.1.1.
The reference is Git 2.55.0, source
`e9019fcafe0040228b8631c30f97ae1adb61bcdc`, invoked explicitly. This is not
an executed local produced-BlueFe or native-Lust qualification.

| Check | Observed result |
| --- | --- |
| Root all-feature/all-target build and tests | Passed; five integration tests plus all binary/example test targets compiled |
| Root Git workflows | Four tests passed, including both object formats, native local clone/fetch/push/pull, merge/rebase/stash/worktrees, conflict abort, and byte-path staging |
| Root grep differential regression | One test passed with six file/stdin byte-pattern scenarios |
| Core/object/index/refs/ODB/pack/protocol/pathspec libraries | 721 tests passed |
| Worktree library | 152 passed; four filesystem-monitor daemon tests failed because this environment denies Unix socket creation |
| New upload-archive integration | Seven tests passed, including a real Git client using the generated native service helper |
| Explicit filter-branch refusal | One test passed; HEAD, index, tracked contents unchanged and no shell/Git helper executed |
| Existing archive integration | Seven tests passed |
| Archive library | Three tests passed |
| Native-helper/i18n library | Eleven tests passed after excluding the inherited shell library |
| Source/storage policy tests | Six tests passed |
| Coordinator archive-validation tests | Eight tests passed |

The four worktree failures are `daemon_exits_when_its_repository_is_deleted`,
`daemon_lifecycle_uses_a_live_endpoint`,
`long_git_directory_uses_a_short_address_without_changing_the_endpoint`, and
`stale_endpoint_is_replaced_before_serving`. They report `Operation not
permitted`. An independent AF_UNIX stream-socket probe fails at socket creation
with the same restriction. The tests remain enabled in hosted CI; this record
does not label them passing.

Root workflows remove Git from the candidate PATH. Reference Git executes only
by an explicit absolute path and checks repository integrity, index entries,
refs, and status. The new grep test also compares exact candidate and oracle
outputs. This demonstrates no installed-Git fallback for the exercised workflows,
not a complete process-execution audit of every donor command.

Raw logs, source fingerprints, and the structured result are retained in
[`sley-evidence`](sley-evidence). The import verifier checks the entire imported
file set and every installed SHA-256. It refuses missing, extra, changed, or
unrecorded source files and unresolved historical CLI/database linkage in the
active lock.

## Hosted gate

The automatic native workflow retains the pinned produced BlueFe compiler
`7005dc02c9fb6674380eade443022bbc04215276` and Lustr coordinator. It builds an
external pinned Git oracle, then runs root and selected donor tests, source
provenance verification, policy checks, a release build, and output-checked
measurements. No full upstream Git conformance run is inferred from those steps.
Configured CI is not recorded as a pass before a completed run is observed.

The source importer excludes standalone donor development scripts and the two
Rust targets whose purpose is to test those omitted scripts and upstream CI.
Those exclusions are individually listed in the manifest. The complete upstream
Git harness must be restored as an external pinned runner before release-level
parity can be claimed; a blanket `cargo test --workspace` is not the qualification
command for this adapted testkit.

## Coverage that remains open

Upstream's [README at the pinned revision](https://github.com/HeddleCo/sley/blob/a71654f50de24b3c750f0df5f1829db7342491f6/README.md)
reports an exact floor of 557 out of 891 Git 2.55.0 scripts after follow-up fixes.
Its integrated run reports 30,103 of 32,357 assertions. Those are donor-reported
results, not a Gitr run. Assertion count does not measure completeness, and at
least 334 scripts lack demonstrated exact parity. Some older donor parity docs
are stale, so executable evidence and explicit source findings take precedence.

The 891-script denominator is a curated selection. The pinned Git 2.55.0 tree
contains 1,046 numbered test scripts; Sley's `upstream-manifest.tsv` excludes
155. Those exclusions remain part of Gitr's full-coverage goal, not satisfied
or waived requirements.

Remaining work includes the excluded filter-branch and shell-i18n contracts,
dumb HTTP fetch, full clone/fetch/push transport and credential qualification,
raw-byte handling beyond tested staging/grep paths, all upstream conformance
scripts, newer Git behavior, Windows/macOS execution, and release-grade
performance measurements. Do not claim a general speed advantage from this
import. Database changes follow the Git compatibility gate.
