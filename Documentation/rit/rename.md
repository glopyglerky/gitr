# Project rename to Rit

Recorded November 10, 2026 UTC. The repository owner renamed
`glopyglerky/gitr` to [`glopyglerky/rit`](https://github.com/glopyglerky/rit).
The code update continues from
[`790fd9908610c5c0f509afe3aabd97b2cd9bc291`](https://github.com/glopyglerky/rit/commit/790fd9908610c5c0f509afe3aabd97b2cd9bc291).

## Current names

| Item | Previous name | Current name |
| --- | --- | --- |
| Project | Gitr | Rit |
| GitHub repository | `glopyglerky/gitr` | `glopyglerky/rit` |
| Root Cargo package and Rust library | `gitr` | `rit` |
| Primary executable | `gitr` | `rit` |
| Git compatibility executables | `git`, `scalar` | `git`, `scalar` |
| Root-test reference Git setting | `GITR_REFERENCE_GIT` | `RIT_REFERENCE_GIT` |
| Selected donor-test product setting | `GITR_TEST_BIN` | `RIT_TEST_BIN` |
| CI work directory | `.gitr-ci` | `.rit-ci` |
| Workflow files | `.github/workflows/gitr-*.yml` | `.github/workflows/rit-*.yml` |

Build and run with the pinned Lustr coordinator:

```sh
lustr build --locked
target/debug/rit -C /path/to/repository status --porcelain=v1
```

The engine remains the pinned Sley workspace. Git-compatible command names,
repository formats, and upstream project identities retain their existing names.
This change adds no Git compatibility coverage.

## Existing local and CI settings

The new `RIT_REFERENCE_GIT` and `RIT_TEST_BIN` environment variables take
precedence. Their `GITR_` equivalents remain accepted when the new setting is
absent. The primary executable is now `rit`; build and invocation examples use
that name.

Manual self-hosted workflows prefer the repository variables `RIT_BLUEFE_ROOT`
and `RIT_REFERENCE_GIT`, with fallback to their previously configured `GITR_`
equivalents. They retain the `gitr-bluefe` runner label because runner
registration is external to the repository. Hosted qualification uses the new
`RIT_` names. Both `.gitr-ci` and `.rit-ci` build directories are ignored. Their
`reference-git-source` checkouts are excluded from the root Cargo workspace.

## Historical records

`Documentation/gitr` retains earlier reports, source archives, and test evidence.
Their command lines, package names, paths, source hashes, and recorded results
identify the revisions that were actually tested. Renaming those records would
obscure their provenance. Active build instructions and the project contribution
license use Rit; prior copyright notices and donor licenses retain their text.

The source import manifest records current hashes for any adapted donor test
entrypoints changed by this rename. Original donor paths and hashes still identify
the pinned imported source.

## Validation

Local validation used pinned Lustr `6339fbb104d6adda4be7e6be7dc637a6a9e5586b`
through its explicit rustc route, Rust 1.99.1, and the Git 2.55.0 reference.

| Check | Result |
| --- | --- |
| Root integration tests against the renamed `rit` executable | 5 passed |
| Selected CLI regression tests | 99 passed |
| Helper materialization and i18n tests | 11 passed |
| CI coordinator tests | 8 passed |
| Composition policy tests | 6 passed |
| Imported source inventory | 678 files verified, 32 adapted and 9 locally added |
| Cargo metadata | Root package and default executable `rit`; binaries `rit`, `git`, `scalar` |
| Entry point checks | `rit` and `git` version output, `scalar list`, and Rit-branded filter-branch refusal |

The CLI run set `RIT_TEST_BIN` to the root `rit` executable and deliberately set
`GITR_TEST_BIN` to a nonexistent path. Root-reference precedence and both legacy
aliases were also checked with existing tests. These repeated alias checks add
no new compatibility coverage. [Local evidence](rename-evidence/result.json)
records the commands, counts, and hashes.

The renamed [hosted qualification workflow](https://github.com/glopyglerky/rit/actions/workflows/rit-native.yml)
records the separate produced-BlueFe result for this commit. Local rustc results
are not a substitute for that gate. The workflow tries the new compiler cache,
then only the exact legacy cache verified on the previous commit. It verifies
the restored compiler before saving it under the new name; a cache miss uses
the normal pinned compiler build.
