# Connected Git compatibility source

`git/` is the `git/git` source submodule at
`e9019fcafe0040228b8631c30f97ae1adb61bcdc`, Git 2.55.0. Its source, notices,
helpers and build system remain together. Upstream Git's own `COPYING`,
`LGPL-2.1` and file notices apply to that source and its separate executable.
Rit's licenses do not replace them.

The default `RIT_BACKEND=auto` selects this separate Git installation. Every
command currently uses it because full native command semantics have not been
qualified. `RIT_BACKEND=compat` selects it explicitly. `RIT_BACKEND=native`
selects the imported Sley Rust engine for port development and native tests.
`RIT_TRACE_BACKEND=1` reports the selected backend on stderr when requested.
Native failures are returned directly, with no attempt to run Git afterward.

## Build and install

From the Rit root on a supported Unix build host:

```sh
git submodule update --init compat/git
mkdir -p target
rustc --edition=2024 ci/build_git_compat.rs -o target/rit-build-git-compat
target/rit-build-git-compat
lustr build --locked
RIT_GIT_COMPAT="$PWD/.rit-ci/git-compat/bin/git" target/debug/rit --version
RIT_GIT_COMPAT="$PWD/.rit-ci/git-compat/bin/git" lustr test --locked --test compat
RIT_BACKEND=native target/debug/rit --version
```

Use the recorded orchestration Rust version from the CI workflow when compiling
the builder. The builder verifies the source commit and rejects tracked changes
and unrecorded `config.mak` overrides. It runs upstream `make install` with HTTP
and PCRE2 support, and verifies the installed version. Build dependencies include
make, a C compiler, curl/OpenSSL, expat, zlib and PCRE2 development libraries,
gettext, Tcl/Tk, plus any compiler required by this upstream Git revision. Perl,
shell and graphical helpers keep their upstream runtime requirements. The
compatibility build retains the upstream Tcl/Tk tools and translations. Other
platform build profiles remain unqualified.

`--source ABSOLUTE_PATH` reuses an already checked out copy of the exact source.
`--prefix ABSOLUTE_PATH` selects the separate Git install prefix. For an installed
Rit at `/prefix/bin/rit`, use `/prefix/libexec/rit-git` as the Git prefix. The
entire Git installation must travel with Rit, including helpers and templates.
Source distributions must include the connected source and its notices.

At runtime, `RIT_GIT_COMPAT` can select that installation's absolute `bin/git`
path. An explicit invalid path fails; Rit never searches `PATH` for a replacement.
Without the variable, Rit checks the installed companion location and then the
build-tree `.rit-ci/git-compat` location. A matching `rit-git-source.txt` receipt
is required. The receipt records the builder's source selection; it is not a
signature or a claim that the installed files cannot be modified afterward.

On Unix, Rit replaces its process with Git, preserving argument bytes, cwd,
stdio and signal behavior. `scalar` uses the companion Scalar executable and
`git-*` helper names become Git subcommands. Configured Git hooks, filters and
external tools retain Git's ordinary execution behavior. Rit rejects recursive
companion selection, including a second Rit copy posing as the companion.

## Replacing the compatibility lane

Keep the native Rust engine, tests and upstream source inventory in the public
project. Use pinned Lustr to compile and qualify each replacement. Admit a native
command into default routing only after its options, error behavior, object
formats, configured helpers and supported platforms have evidence against the
connected Git revision. A smoke test or a command-name match is insufficient.
Keep the compatibility source until every required operation has a qualified
Rust replacement. Do not classify a companion execution as native Rust coverage.
