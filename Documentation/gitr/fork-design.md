# Gitr fork design and divergence

## Ownership and baseline

Gitr is Matt Mayhugh's project at [glopyglerky/gitr](https://github.com/glopyglerky/gitr).
GitHub's repository API verified `fork: true` and both parent and source as
[git/git](https://github.com/git/git) on 2026-11-08. This audit used default
`master` at `60616187139a8638e121f586c63fef91431ef340`. That identifies the fork
snapshot inspected, not an assertion that it matches today's upstream head.

The active implementation is the standalone `rust/gitr` package. Its own
workspace boundary and manifest select Gitoxide, redb 4.5.0 and Wasmi, with
optional workflow/API features. It does not link the inherited root `gitcore`.
The retained reference implementation still builds Git from C and other languages.

## Deliberate layout exceptions

| Retained path or name | Concrete contract and reason |
| --- | --- |
| Root C files, `builtin/`, `compat/`, `t/` | `Makefile` and `meson.build` enumerate these paths. The reference executable and shell test corpus are needed for Git compatibility work; moving them requires a separate reference audit and qualified build. |
| Root `Cargo.toml`, `build.rs`, `src/` | The inherited `gitcore` static library uses build-script links to `git` and `z`. It must not be mistaken for the separate Gitr package or moved independently of Git's build rules. |
| `rust/gitr`, example guest and vendored workflow crate | Gitr workflows pass explicit manifest paths, and the optional workflow dependency uses `../vendor/rbit-workflow`. A cosmetic move would break these contracts. |
| `Documentation/`, `INSTALL`, `COPYING`, `LGPL-2.1` | Reference manuals, installation instructions and license scopes remain applicable to inherited Git. Gitr decisions live under `Documentation/gitr`. |
| Git object, ref and LFS names/protocols | Existing repositories and Git/GitHub clients depend on exact object identities, wire formats and compare-and-swap publication. Branding cannot change those formats. |
| Donor module names and notices | Retained coherent source and tests need traceable origin and update paths; preserve their licenses. |

The root README now leads with Gitr's owned entry points and source map. The
duplicated original Git presentation was extracted to `UPSTREAM-README.md`,
keeping its root-relative links valid. No executable source path moved.

## Current support and future migration

The manifest and CLI establish implemented command entry points, not complete
Git parity. Read `compatibility.md`, `qualification.md` and the feature-specific
records before selecting a command. Network clone/fetch/push, full porcelain
authoring and distributed HA remain unfinished in the audited README.

The pinned `lustr-toolchain.json` selects the `rustc` engine. This is Cargo/rustc
coordination, not proof of native Lust compilation or BlueFe compilation of C,
Perl, shell, Python or Tcl/Tk. Full native Rust conversion and 100% memory-safe
Rust are future acceptance goals. The crate's `unsafe_code = "forbid"` does not
cover transitive native crypto or inherited Git. Preserve working behavior and
qualify replacements before retiring it.

## Maintenance and verification

Update this record and the root source map when a build entry point, donor,
protocol boundary or retained path changes. Record exact reference consumers and
focused qualification before a physical move; retain required notices and rollback.
The baseline commit above preserves the prior presentation and source.

This change is documentation only. Checks cover relative links, manifest and
workflow path references, preservation of the extracted upstream text and a
Git-tree diff limited to these Markdown files. No product build or runtime
acceptance was executed for this change. Existing qualification records retain
their original scope.
