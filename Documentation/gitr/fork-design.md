# Gitr fork design and divergence

> Historical record for the Grit/database-first implementation. The active
> Sley composition and its current limits are in [sley-integration.md](sley-integration.md).


The default `master` now builds the root Rust Gitr package. `src`, `tests`,
`Cargo.toml` and `Cargo.lock` are owned Gitr; `crates/action-example` is the
isolated Wasm guest; `vendor/rbit-workflow` retains the complete reviewed donor.
The GitHub parent remains git/git; removing working-tree files does not detach a fork.

The inherited C-linked gitcore manifest/build script and all C Git build paths,
production code, language frontends and unused shell test corpus were retired
together. No Gitr package uses them. Differential tests generate fixtures using
an installed reference Git, which is test infrastructure, not a runtime fallback.
Every removed path and blob/mode is recorded in `root-retirement.json`; recover
original text using `git show bf628ff8c74da4ab03a4a99001edf6780d8d84a0:<path>`.
All source/history and root COPYING/LGPL notices remain recoverable. The root
migration did not change license grants. The later [licensing decision](licensing.md)
adds MIT OR Apache-2.0 for reviewed owner-controlled contributions while
preserving inherited terms. The old Git README is historical text; its links
refer to the old tree. No entire retired source tree was copied elsewhere.

The migration and current compiler evidence are in [rust-root.md](rust-root.md).
Historical qualification JSON, research source references and archived experiments
retain their original paths and do not describe the current build layout.

The [strict Rust-only source contract](rust-only-source.md) now also removes
foreign build/test/qualification programs and archived executable probes.
Database and forge design gates remain requirements, not delivered features.
