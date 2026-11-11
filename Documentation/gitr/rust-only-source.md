# Rust implementation and connected Git source

Rit preserves working Git behavior while replacing its implementation with Rust.
The imported Sley engine and frontend are in `vendor/sley`; original wrappers,
tests and build coordinators are Rust. The exact upstream Git source remains
connected at `compat/git` until qualified Rust implementations cover its behavior.

The launcher selects its backend before an operation starts. Current default
execution uses the separately built upstream Git installation. Set
`RIT_BACKEND=native` to execute and qualify Sley. Native errors are returned
directly. No failed native mutation is retried through another engine.

The connected Git build retains upstream source, helpers, build files and
notices together. It is an explicit compatibility component. Its execution must
never be reported as native Rust coverage. The native Sley import still excludes
the inherited shell `filter-branch` and `git-sh-i18n` bodies with unresolved
source-origin declarations; upstream Git's own implementations remain available
through the compatibility component under upstream's notices.

Native Git behavior also includes caller-configured hooks, filters, editors,
credential helpers and aliases. The donor contains small shell dispatch adapters
and generated shell test fixtures. Native dependencies include cryptographic
code and unsafe Rust. File-extension counts cannot prove a wholly memory-safe
or exclusively Rust artifact.

`xtask policy` checks the native dependency composition against reviewed source
and tooling locks. `xtask audit` permits only the recorded Git source submodule
and classifies the remaining tracked files. Import verification checks every
Sley source path and SHA-256 against `sley-import.json`.

See [the replacement plan](../rit/port-plan.md) and
[compatibility build instructions](../../compat/README.md).
