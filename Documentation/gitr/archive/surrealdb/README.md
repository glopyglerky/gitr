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
