# Database decision: use redb

Status: selected by the user's latest instruction on 2026-11-08: `use redb`.
This replaces the SurrealDB/SurrealKV/SurrealDS direction, including any earlier
documentation describing redb as a transition backend. The Git/forge/actions,
code-intelligence, multi-writer and HA product requirements are unchanged.

## Current implementation retained

The standalone `rust/gitr` package pins **redb 4.5.0** in its manifest and
lockfile. The catalog in `src/db.rs` and action storage in `src/actions.rs`
already use redb. This selection requires no data conversion.

- `<common-git-dir>/gitr/catalog.redb` contains rebuildable commit snapshots,
  factual Git membership relations and lexical search features.
- `<common-git-dir>/gitr/state.redb` contains durable action modules, inputs,
  job claims, outcomes and small artifacts. Search rebuilds must not erase it.
- The existing action export remains available and read-only. An export is not
  proof of a complete restore facility; qualify restoration separately.
- Git objects/refs and large Git LFS/chunk payloads keep their existing formats
  and locations. No new authoritative ref owner is installed by this change.

These are two physical files with different retention rules, not an assertion
that the full Git repository already lives in a single database. Consolidation,
when justified, requires a verified schema/authority migration and rollback.

## Development boundaries

Use redb as the durable storage substrate for the repository service, forge
records, workflow state and execution receipts. Preserve bounded transactions,
explicit durability, guarded revisions, idempotent operations, cancellation and
monotonic worker fencing. Keep Wasmi execution outside write transactions; only
claiming and publishing results acquire the writer. Do not hold a write
transaction during compilation, network IO, or arbitrary CI work.

Build exact relational indexes and source-grounded graph edges over redb, with
embeddings and retrieval indexes keyed by immutable content, revision, parser
and model identity. Derived indexes may be rebuilt without changing authoritative
records. Selecting redb does not itself implement a SQL/graph query engine,
learned semantic retrieval or Codebase Memory parity. Keep those deliverables
and their measured acceptance criteria in the existing roadmap.

Stateless Rust API/MCP adapters must share a typed, authorized repository
service. No session or worker owns the authoritative state. A future redb service
may coordinate requests and transactions without changing the chosen database;
its authentication and transport need their own implementation and tests.

Local redb concurrency is not multi-host HA. Distributed operation still needs
a Rust replication/consensus layer, durable object publication, fencing and
partition/failover/recovery tests. Each replica must own its local storage; a
shared writable database file is not a replication design. No performance gain
or distributed guarantee is claimed by this backend decision.

## Retired experiments

The SurrealDB probe, candidate service, remote-action adapter, preparation/smoke
scripts and two qualification workflows are moved intact to
[`archive/surrealdb`](archive/surrealdb). Original licenses and source are kept.
They are outside active Rust package and workflow paths. The preparation script
must not patch the production CLI, and even successful historical candidate
artifacts must not be activated. Surrealism and SurrealDS are not dependencies
or planned backend requirements under this decision.

The old [SurrealDB decision URL](surrealdb-decision.md) is retained as an explicit
supersession notice so older links cannot be mistaken for current instructions.
No existing database, Git history, asset, credential or deployment is changed.

## Qualification

The new backend-policy check verifies the exact redb manifest/lockfile pin and
rejects active Surreal-family dependencies or migration entry points. It scans
tracked active Rust manifests and lockfiles, not archived research or generated
build files. This is a regression guard, not proof of storage correctness.

```sh
python ci/gitr-redb-policy.py --self-test
python ci/gitr-redb-policy.py
lustr --engine rustc test --manifest-path rust/gitr/Cargo.toml --locked --all-targets
lustr --engine rustc clippy --manifest-path rust/gitr/Cargo.toml --locked --all-targets -- -D warnings
```

Existing runtime and export code is preserved rather than replaced by the
unqualified SurrealDB candidate. See [action qualification](actions-qualification.md)
for previous executed evidence; new workflow results must be read for their
actual commit before being reported as passing.
