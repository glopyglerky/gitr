# Grit source imported into Gitr

Source: [gitbutlerapp/grit](https://github.com/gitbutlerapp/grit), revision
[`e197103087b1110db2d629c4c269ef9258508169`](https://github.com/gitbutlerapp/grit/tree/e197103087b1110db2d629c4c269ef9258508169).
This is the revision immediately preceding removal of upstream's `grit-git`
compatibility CLI. Newer upstream releases intentionally target a different CLI.

The import started with the complete Rust sources and Rust tests of `grit-lib`,
`grit-git`, and `grit-test-support`, their embedded help text and manifests.
The local changes and removed execution paths are recorded below. The
upstream Git reference tree and executable shell helpers are not included.
`upstream-source.json` records each imported path and the original SHA-256, before
local changes. Root qualification records describe the resulting build and tests.

The original `LICENSE` states the upstream split: `grit-lib` is MIT licensed,
`grit-git` is GPL-2.0-only, and Git-derived reference material is GPLv2. The
`grit-test-support` manifest declares MIT. `COPYING` retains the upstream Git GPLv2
license text. `LICENSE-MIT` supplements the historic split notice with the full
upstream MIT notice at `f615c13628313bd63f25d157d55da24a79ad9d68`, for the
already MIT-declared library and test-support only. Gitr's existing GPL-2.0-only declaration remains compatible with
this composition; this import does not relicense inherited code. Original
copyright notices and code comments remain in the source files.

## Integration changes

- The `grit-git` implementation is also a Rust library. Gitr calls its process
  entry function directly; it does not launch an external Git executable.
- Native subprocesses run the current Gitr executable. Pack generation and HTTP
  submodule cloning no longer select the system C Git executable.
- `filter-branch` fails explicitly until a native implementation exists. Its
  upstream implementation ran the system Git shell script.
- The build no longer installs `git-sh-setup` or `git-sh-i18n` shell programs.
  The shell source interface `sh-setup` is explicitly unsupported.
- The donor self-updater is disabled. It previously downloaded and ran a remote
  shell installer that could replace Gitr with a different upstream product.
- Built-in Vimdiff merge layouts are unsupported because their implementation
  generated executable Vimscript. The library retains declarative layout data;
  `mergetool.<tool>.cmd` still runs an explicit user-supplied tool command with
  `BASE`, `LOCAL`, `REMOTE` and `MERGED` paths in its environment.
- Diff hunk-context regex validation and matching use the Rust regex engine.
  Unsupported regex syntax fails explicitly instead of delegating to `grep`.
- Timestamp changes and executable existence checks use Rust filesystem APIs.
  Pack hooks launch their configured executable directly and receive the current
  native executable as the delegated pack command. Multi-argument submodule
  commands also launch directly. Browser shell adapters evaluate the configured
  command without an extra fixed `eval` program.
- Archive subprocesses no longer fall back to a `git` executable on PATH or use
  the donor's `GUST_BIN` override. The bisect test-fixture environment shim that
  derived `p` from a file named `hello` was removed.
- Read commands reject effective executable filter, textconv, external diff,
  fsmonitor, signature verifier and pager configuration. They do not execute those helpers while
  inspecting a repository. Worktree-writing commands retain Git's explicit
  hook and filter interfaces.
- Rust subprocess fixture programs replace embedded shell fixtures in tests.
  They are compiled by the selected qualification compiler through `RUSTC`.
- The nested workspace contains only the imported package family. Unrelated
  modern-client, web-serving and upstream maintenance packages are omitted.

`src/main.rs` in `grit-git` still owns process state and termination. Its public
`main_entry` function is for a single-threaded executable entry point, not a
reentrant application API. Applications should use `grit-lib` for typed APIs.

## Compatibility claims

The donor's historical test-suite headline is not a qualification result for
this import. More than 140 command handlers and their source tests are retained,
but passing Gitr's executed differential tests is required for every reported
capability. The removed C Git fallbacks, shell-only interfaces, non-UTF-8 CLI
arguments, built-in Vimdiff layouts, self-update and any unexecuted upstream
scenarios remain explicit parity gaps.
No 100% parity or universal speed claim follows from importing these sources.

For an update, compare upstream source hashes at a pinned revision, inspect the
license split and command coverage, reapply the documented local patches, then
repeat Gitr's source, dependency, differential and performance qualification.
Do not replace this snapshot with current upstream `main` without first checking
that the full compatibility CLI is still available.
