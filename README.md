# Rit

Rit is a Git-compatible implementation being ported to Rust. It assembles
upstream Rust code, keeps connected upstream Git available for remaining
compatibility gaps, and replaces that implementation incrementally through
Lustr. Active development is on [`master`](https://github.com/glopyglerky/rit/tree/master).

The native engine is [Sley](https://github.com/HeddleCo/sley), pinned at
[`a71654f50de24b3c750f0df5f1829db7342491f6`](https://github.com/HeddleCo/sley/tree/a71654f50de24b3c750f0df5f1829db7342491f6).
Its engine crates, Git frontend, and Rust tests live under `vendor/sley`.
The connected Git source is pinned at
[`e9019fcafe0040228b8631c30f97ae1adb61bcdc`](https://github.com/git/git/tree/e9019fcafe0040228b8631c30f97ae1adb61bcdc)
under `compat/git`.

**Complete native Rust Git parity is unfinished.** The default route currently
uses the verified upstream Git compatibility executable. `RIT_BACKEND=native`
selects Sley for native development and qualification. The backend is selected
before execution; a failed native command is never retried with Git.

## Build and run

Use the pinned [Lustr coordinator](https://github.com/glopyglerky/lustr) with
Rust 1.99.1 or the produced BlueFe bundle recorded in `lustr-toolchain.json`.
The current native Lust subset does not compile the complete dependency graph.

```sh
lustr build --locked
RIT_BACKEND=native lustr test --locked --all-features --lib --test git_workflows --test grep_patterns

RIT_BACKEND=native target/debug/rit -C /path/to/repository status --porcelain=v1
RIT_BACKEND=native target/debug/rit -C /path/to/repository log --oneline -n 10
```

The same entrypoint builds as `rit` and `git`, with a `scalar` companion. Keep
the generated `git` executable separate from an installed Git while native
qualification remains incomplete.

To build the connected Git installation, initialize its pinned source and run
the dependency-free build coordinator with the pinned Rust compiler:

```sh
git submodule update --init compat/git
rustup run 1.99.1 rustc --edition=2024 ci/build_git_compat.rs -o /tmp/rit-build-git
/tmp/rit-build-git --prefix "$PWD/.rit-ci/git-compat"

target/debug/rit -C /path/to/repository status --porcelain=v1
RIT_GIT_COMPAT="$PWD/.rit-ci/git-compat/bin/git" RIT_BACKEND=native lustr test --locked --all-features --all-targets
```

This build needs upstream Git's C build tools and HTTP, TLS, compression, and
PCRE2 development libraries. The coordinator verifies the source revision and
records the installation's source identity. The launcher accepts only an
installation with the matching receipt. For a custom installation, set
`RIT_GIT_COMPAT` to its absolute `bin/git` path. An installed Rit looks for its
companion at `../libexec/rit-git/bin/git`; a source build also checks
`.rit-ci/git-compat/bin/git`.

| Selection | Behavior |
| --- | --- |
| `RIT_BACKEND=native` | Execute the Rust Sley engine |
| `RIT_BACKEND=compat` | Execute the verified connected Git installation |
| `RIT_BACKEND=auto` or unset | Use compatibility execution until native command semantics are admitted |

There is no automatic PATH lookup for a substitute Git. A missing or invalid
compatibility installation fails before the requested operation starts.

## Native coverage and replacement work

Native workflow tests exercise initialization, staging, commits, branches,
merges, rebases, stashes, worktrees, local clone/fetch/push/pull, and checked ref
updates in SHA-1 and SHA-256 repositories. They also cover selected conflict,
raw-filename, history, and archive cases. These fixtures establish their tested
operations, not every flag or error case of the commands.

Sley's reported conformance results and Rit's qualification results must remain
separate. The full upstream Git test suite, authenticated transport, platform
matrix, and performance qualification remain required work.

Replace each compatibility path only when the native implementation preserves
its command-line contract and repository effects. Keep exact upstream source
identities, failing cases, source notices, and Lustr compiler-route evidence.
Do not turn source import size or a passing smoke test into a parity claim.

`RIT_REFERENCE_GIT` selects an absolute reference Git for root tests.
`RIT_TEST_BIN` selects Rit for donor regression tests. Legacy `GITR_` equivalents
remain accepted when the corresponding `RIT_` setting is absent. Selected donor
conformance tests require `SLEY_TEST_GIT` to point to Git 2.55.x. Native CI selects
`RIT_BACKEND=native` so its results cannot be supplied by compatibility execution.

## Source map

| Path | Purpose |
| --- | --- |
| `src/lib.rs`, `src/main.rs`, `src/bin` | Rit, Git, and Scalar entrypoints |
| `src/compat.rs` | Backend selection and verified Git process execution |
| `vendor/sley/crates` | Imported Rust Git engine, frontend, and tests |
| `compat/git` | Pinned upstream Git source connection |
| `ci/build_git_compat.rs` | Verify and build the connected compatibility installation |
| `Documentation/gitr/sley-import.json` | Imported file identities, hashes, exclusions, and adaptations |
| `tests/git_workflows.rs` | Native product workflows with Git absent from the candidate PATH |
| `examples/verify_import.rs` | Verify the complete imported Sley inventory |
| `ci/qualify_native.rs`, `xtask` | Compiler qualification, source composition, and inventory checks |

## Licensing

Rit-controlled original code is MIT OR Apache-2.0. Sley retains Apache-2.0.
Connected and inherited Git material retains its applicable upstream licenses,
including the terms in `COPYING`. Preserve donor notices and corresponding
source when distributing compatibility components. The project license does
not override a donor's terms.

The Rust engine retains small shell compatibility adapters and generated
fixtures, and dependencies include native cryptographic code and unsafe Rust.
Compatibility mode executes upstream Git. This is not an exclusively Rust or
wholly memory-safe distribution. See [LICENSE](LICENSE), the
[license map](Documentation/gitr/licensing.md), and
[Sley's notice](vendor/sley/NOTICE.md).
