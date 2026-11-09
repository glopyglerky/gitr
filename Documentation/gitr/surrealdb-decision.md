# Superseded: the database is redb

The user's later instruction on 2026-11-08 is **use redb**. Follow the current
[redb database decision](redb-decision.md) and root `AGENTS.md`.

SurrealDB, SurrealKV, SurrealDS and Surrealism are no longer the selected backend
or migration direction. The current redb catalog and action state remain in
place. No user data is converted or removed.

The previous decision and experimental code are retained in
[`archive/surrealdb`](archive/surrealdb). Those files are historical material,
not active build targets or instructions to resume the retired migration.
