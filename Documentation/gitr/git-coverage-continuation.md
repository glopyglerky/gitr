# Git coverage continuation

This is historical native-engine evidence. Current native reproductions must
set `RIT_BACKEND=native`; compatibility execution is not native coverage.


Recorded November 9, 2026, America/Denver. This change continues from
[`4d23776e19faaf161db66feb1768e64233a4620b`](https://github.com/glopyglerky/gitr/commit/4d23776e19faaf161db66feb1768e64233a4620b).
It completes two bounded command changes and repairs the hosted qualification
setup. Full Git coverage remains unfinished.

## Native remote archive client

`gitr archive --remote` now uses the imported transport and protocol code for
SSH and `git://` requests. It resolves configured remote names and `insteadOf`
URLs, applies transport policy, and forwards archive options in order. Output
filename extensions select a format when no explicit format was supplied.
Listing formats, prefixes, pathspecs, and explicit service commands use the same
request/response protocol.

The client validates the initial ACK and its flush before copying sideband data
to the output writer. Progress and fatal responses remain separate from archive
bytes. A missing final flush, malformed packet, server error, failed writer, or
nonzero SSH/proxy exit returns failure. The implementation retains one sideband
frame at a time instead of collecting the entire archive. Archive subprocess
stderr is drained concurrently, with at most 64 KiB retained for diagnostics.
Archive services own isolated Unix process groups. Failed exchanges stop their
group before waiting. On Linux and macOS, completed services are observed with
`waitid` without reaping first, so remaining descendants can be terminated while
the leader's PID is still reserved. Reaping then preserves the service's actual
exit status. This prevents a descendant holding stderr open from hanging either
a failed or completed archive operation. Other platforms retain direct-child
completion waits pending qualification.

An explicit local `--exec` now runs the selected service and checks its response
and status. The former implementation merely probed that command and generated
an archive regardless of the probe's result. Default local archives continue
through the Rust archive engine. Configured commands, the SSH executable, and
test shims remain explicit compatibility boundaries; none is a silent fallback
to an installed Git implementation.

The new tests exercise an actual Git upload-archive service through a local SSH
shim, Gitr's native service, and a Git daemon over pipes through
`GIT_PROXY_COMMAND`. They include both object formats, output format selection,
configured remotes and commands, transport refusal, unreachable refs, truncated
responses, malformed peers, and nonzero termination after a valid payload.
Candidate PATH contains a Git trap that must remain unused. The tests can select
the actual root executable using an absolute `GITR_TEST_BIN`.

These fixtures qualify service-command and wire interoperability. They do not
qualify SSH authentication, a live TCP connection, Windows/macOS execution, or
all inherited SSH command parsing behavior. Non-UTF-8 remote archive arguments
remain explicitly unsupported and are rejected before transport startup.

## Revision-selector names

Reference selectors such as `rev-parse --all`, `--branches`, `--tags`,
`--remotes`, and `--glob` previously printed object IDs even when the caller
requested symbolic names. The command now renders selector output according to
`--symbolic`, `--symbolic-full-name`, and strict/loose `--abbrev-ref` modes. It
reuses Sley's existing unambiguous-ref shortening implementation.

The change preserves ordinary object-ID output, option order, exclusion state,
namespace-specific short names, collision diagnostics, and symbolic-ref
resolution. Effective global/repository config and working-state refs participate
in ambiguity checks. Valid symbolic aliases participate in selector expansion.
Missing, cyclic, and malformed targets do not discard valid results. Differential fixtures compare
stdout, stderr, and exit status with the pinned Git oracle in SHA-1 and SHA-256
repositories, with no executable on the candidate's PATH.

This repairs one cause of the recovered native filter-branch failures. It does
not activate that larger rewrite. The current filter-branch refusal and the
excluded shell-i18n library remain unchanged.

## Hosted qualification repair

The baseline [hosted run](https://github.com/glopyglerky/gitr/actions/runs/42754137566)
completed with failure. Its compiler job passed, but the native job stopped
before application compilation because the external Git oracle could not find
`curl/curl.h`.

The workflow now installs the oracle's build dependencies explicitly, keeps HTTP
enabled, enables PCRE2 to match Sley's oracle manifest, and builds the external
test helpers. The native gate includes the changed revision and remote libraries
and the archive/revision CLI tests. New CLI fixtures run against the actual root
`gitr` binary in hosted qualification. The produced BlueFe and Lustr source pins
are unchanged.

The [first continuation run](https://github.com/glopyglerky/gitr/actions/runs/42758073780)
passed the compiler job and progressed past the missing HTTP headers, then
stopped while compiling Git's own Rust support library. Its nested checkout was
being treated as a member of Gitr's Cargo workspace. The root workspace now
explicitly excludes that external checkout. The oracle build also overrides the
job's application `CARGO_TARGET_DIR` with its own `target` directory, matching
the static-library location expected by Git's Makefile, and selects Rust 1.99.1.
HTTP, PCRE2, and Git's Rust support remain enabled.

The repaired `build-oracle` phase passed in a fresh local checkout nested at the
same relative path, with an inherited application target directory reproducing
the hosted environment. Git's source remained unchanged, its Rust static library
was created at the Makefile's expected path, and the application target was
unused. The build log and installed-oracle identity are retained in
[`oracle-isolation-build.log`](coverage-evidence/oracle-isolation-build.log) and
[`oracle-isolation-provenance.txt`](coverage-evidence/oracle-isolation-provenance.txt).
The five root tests, eight coordinator tests, import verification, policy, and
source audit also passed after this repair.

## What the coverage denominator means

The pinned Git 2.55.0 source at
[`e9019fcafe0040228b8631c30f97ae1adb61bcdc`](https://github.com/git/git/tree/e9019fcafe0040228b8631c30f97ae1adb61bcdc/t)
contains 1,046 numbered `tNNNN-*.sh` test scripts. Sley's
[`upstream-manifest.tsv`](../../vendor/sley/crates/sley-testkit/upstream-manifest.tsv)
includes 891 and excludes 155. Its reported 557 exact matches concern that
curated selection. Neither those donor results nor this change establishes
Gitr's whole-suite coverage.

This continuation executes three relevant upstream scripts with both the
reference Git and the actual Gitr binary in both object formats. It retains
each raw TAP result, exit status, and skip differences. A zero exit status alone
does not count as exact parity when the oracle executes a cell that Gitr skips.
The complete run records and source fingerprints are in
[`coverage-evidence`](coverage-evidence).

## Executed local results

| Check | Observed result |
| --- | --- |
| Root all-feature/all-target tests | Five integration tests passed; all binary/example targets built |
| Selected CLI tests | 53 passed, including 13 new archive tests and five new revision-selector tests |
| Actual root executable | All 18 new CLI tests use `GITR_TEST_BIN`; the revision tests contain 272 exact oracle comparisons |
| Selected engine libraries | 1,165 passed; four existing filesystem-monitor tests failed because this environment denies Unix sockets |
| Source inventory verification | Passed for all 676 files, including 26 adaptations and seven locally added files |
| Qualification coordinator | Eight tests passed with the stock orchestration compiler |

The four filesystem-monitor tests remain enabled in hosted CI. Raw test logs
retain their failures. No dependency version or test was removed to obtain a
pass.

| External Git script, each run in SHA-1 and SHA-256 | Gitr | Reference Git | Exact result |
| --- | --- | --- | --- |
| `t1500-rev-parse.sh` | 81 passed, one skipped | 82 passed | No, skip difference |
| `t5000-tar-tree.sh` | 80 passed, four failed, six skipped | 84 passed, six skipped | No, four failures |
| `t5003-archive-zip.sh` | 78 passed | 78 passed | Yes |

Every selected script's test-result vector is unchanged from the baseline. The
new regressions qualify behavior that those scripts did not exercise; this is
not a claim that the full-suite script count increased.

The baseline archive script already fails four cells around its huge loose
object fixture. The revision script already skips its compatibility-object-mode
cell when running Gitr while the oracle executes it. Those cases are retained
as unfinished coverage, without disabling or relabeling their tests.

## Reproduction

Use the pinned Lustr coordinator. Local development evidence uses its explicit
`rustc` engine with Rust 1.99.1; this is separate from hosted produced-BlueFe
qualification. `GITR_REFERENCE_GIT` and `SLEY_TEST_GIT` point to the absolute
external Git 2.55.0 executable.

```sh
export RIT_BACKEND=native
lustr --engine rustc test --locked --all-features --all-targets --no-fail-fast
GITR_TEST_BIN=/absolute/path/to/gitr lustr --engine rustc test \
  --manifest-path vendor/sley/Cargo.toml --locked -p sley-cli \
  --test archive --test archive_remote --test upload_archive \
  --test filter_branch_unavailable --test rev_parse \
  --test rev_parse_pseudo_names --no-fail-fast
lustr --engine rustc run --example verify_import --locked
lustr --engine rustc run --manifest-path xtask/Cargo.toml --locked -- policy
lustr --engine rustc run --manifest-path xtask/Cargo.toml --locked -- audit
```

For external scripts, use the separately built pinned Git source as the working
directory's parent and set `GIT_TEST_INSTALLED` to an isolated directory containing
the candidate `git` and `scalar` binaries. Run each script from the oracle's `t`
directory with `--no-bin-wrappers --root=/fresh/test/directory --verbose-log`.
Set `TEST_OUTPUT_DIRECTORY` to a separate fresh directory for each script/lane,
`GIT_TEST_DEFAULT_HASH` to `sha1` or `sha256`, `GIT_TEST_EXT_CHAIN_LINT=0`, and
`GIT_BUILD_DIR`/`GIT_SRC_DIR` to the external source. Repeat with the oracle's
installed binary directory. Exact commands and environment values are recorded
with the results.

## Source boundaries

The donor remains pinned at Sley
`a71654f50de24b3c750f0df5f1829db7342491f6`. The archive transport work reuses the
existing SSH/Git connections, URL resolution, transport policy, and packet
codecs. Independently authored additions and adaptations are identified in
[`sley-import.json`](sley-import.json), with current hashes and original donor
identities preserved. No excluded shell engine or C Git implementation was
reintroduced by that change.

The archive cleanup code uses Rustix's safe process API. `sley-remote` now names
the existing locked Rustix 1.1.5 dependency directly on Unix with its `process`
feature. Both lockfiles preserve all dependency versions and checksums.
