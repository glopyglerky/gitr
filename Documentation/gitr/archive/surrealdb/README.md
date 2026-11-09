# Retired SurrealDB migration experiments

Preserved from Gitr commit `cc6e6748ef7716f631e22b16aac424f98a72c147` when the user
selected **redb**. File bytes and package licenses are retained unchanged.

The `rust`, `ci` and `.github/workflows` subdirectories mirror former repository
paths. Their internal commands and relative paths refer to that historical
layout; this is an archive, not a supported runnable package or CI configuration.
No production database or deployment was migrated by moving these source files.

Current policy: [use redb](../../redb-decision.md).
The live redb runtime and read-only action export remain under `rust/gitr` at
the repository root. The archived `actions_remote.rs` was an unactivated
SurrealDB adapter, not the active redb action implementation.

2026-11-09: executable Python/workflow/SurrealQL sources and the two Rust
probe entrypoints embedding SurrealQL were removed under the strict source
directive. Historical manifests, licenses, Rust-only support and static evidence
remain; these experiments cannot be built or run from this archive. Recover
original implementations from Git history at 1e6cfcf4abe7c23b96db6f5d6f36e8fc789d8200.
