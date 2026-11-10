# Rust source and Git compatibility assets

The Sley integration follows the owner's current delivery order: working Git
behavior first, with upstream Rust reused wherever possible, then database and
speed improvements. It replaces the previous blanket removal rule with an
explicitly bounded compatibility exception. Removing working upstream behavior
solely to obtain a Rust-only file count does not establish Git parity.

## Active implementation

`src/` builds the `rit`, `git`, and `scalar` entrypoints into the Rust Sley engine
and CLI under `vendor/sley/crates`. Build, import-verification, and qualification
coordinators
are Rust. The C Git implementation is not restored or linked. Reference Git is
an external test oracle and never a hidden production fallback.

The former redb/forge root is preserved under `archive/database-first` with its
original source, tests, manifest, and lock. Its prior source audit and test counts
remain historical evidence, not evidence about the new executable.

## Explicit compatibility exceptions

| Source | Retained foreign execution | Reason |
| --- | --- | --- |
| `vendor/sley/crates/sley-i18n/src/lib.rs` | Five small Git helper shell adapters | Installed-layout compatibility; each adapter invokes this Rust engine |
| Donor Rust tests and testkit | Generated hooks, filters, editors, transport probes, and other shell fixtures | Test Git's execution contracts; these are not production Git fallback engines |
| Configured Git hooks, aliases, filters, credential helpers, signing tools, and editors | Caller-selected external commands | Existing Git extension contracts; availability depends on the user's environment |

The source manifest records files containing shell markers, but that scan is
not a full language or execution audit. Shell strings are not mislabeled as
native Rust implementation. No standalone upstream shell/Python development
scripts or C Git source are checked in by this import. The complete external
Git test harness remains separate work.

The recovered native filter-branch rewrite was not activated: its focused tests
still exposed behavior differences. Source review then found substantial
literal overlap between the donor's embedded shell filter-branch and shell i18n
library and Git's original scripts, with the original attribution headers
missing. The donor workspace's Apache declaration does not resolve those
specific origins. Both embedded programs were excluded. The filter-branch route
returns an error before mutation; `git-sh-i18n` is not materialized. Rust gettext
and envsubst implementation and the five independent dispatch adapters remain.
This documents a concrete compatibility gap rather than silently relabeling
inherited source. The original source identities remain in the import manifest;
this is a targeted provenance finding, not a broader legal conclusion about Sley.

## Native dependency boundary

The active dependency lock includes Rust Git engines, rustls/ureq, ring native
cryptographic code, platform bindings, and memory-mapped access. Some upstream
Rust permits unsafe operations. The compiler, linker, C runtime, loader, and
OS are external native components. An exclusively Rust or whole-artifact
memory-safety claim would be inaccurate.

## Inventory and validation

`xtask audit` classifies tracked source and data, including `.tsv` evidence,
and rejects unreviewed executable modes or file extensions. This is a file
inventory; the compatibility table above is necessary to interpret Rust source
containers containing shell programs. `xtask policy` checks the active dependency
composition, excludes historical Grit's GPL-only CLI and retired database
backends, and preserves the future redb pin in the archived prototype.

The [Sley integration record](sley-integration.md) identifies actual tests,
compiler routes, source hashes, and remaining conformance work.
