# SurrealDB 4.0.0 / SurrealKV 1.2.0 qualification

**Qualified candidate:** GitHub Actions [run 42535967615](https://github.com/glopyglerky/gitr/actions/runs/42535967615)
on exact input commit `ad8e3707000a79d601657d5b40c3d3b6485f2c7e`, Ubuntu 24.04,
Rust 1.99.1 through the pinned Lustr Rustc coordination route.
The successful run emitted immutable formatted-source and lockfile blobs.
The next commit promotes those exact blobs, not regenerated guesses.

- Source blob `7e3aa654de18c35636545cb6f4a49c0b77212b4e`;
  SHA-256 `c6fe755ec098d73a8238b89aba8485d52ddc816c92c7167e65c8b1ab68f6cc5b`.
- Cargo.lock blob `c1d178988554c83452b85d53e6ac1df3d06019e0`;
  SHA-256 `e30cb9de63d2d9a9ac2b4ec9a82007269863a13a7dc7e9f2f0b6c5115dfc2110`.

## Executed result

Exact SurrealDB 4.0.0 with SurrealKV 1.2.0 compiled through Lustr, passed
strict Clippy (`-D warnings`), and passed a disposable-file-store process fixture:

- Eight competing same-record conditional writers yielded exactly one winner;
  a stale conditional writer failed.
- Explicitly aborted writes did not publish an artifact or a receipt; a
  successful artifact and receipt persisted together.
- Defined graph edges, BM25 full-text and HNSW nearest-neighbor fixtures were
  queried and checked against expected concrete results.
- Closing and reopening the datastore preserved the reference and receipt.
- A simultaneous second datastore owner was rejected.
- A write acknowledged before forced process termination remained present
  after recovery and reopening.

The earlier failing run exposed the upstream SurrealDB 4.0.0 Rust SDK's
Tokio-worker stack overflow, also discussed in
[upstream issue 7669](https://github.com/surrealdb/surrealdb/issues/7669).
The candidate now uses the documented 10 MiB Tokio worker stacks. This is
a verified workaround for the checked fixture, not proof that every deep query
or version of SurrealDB is immune to stack exhaustion.

## What this does not prove

This is a **standalone qualification**, not an implemented production Gitr
backend or migrated user data. The currently working local Gitr code and action
store still use redb. SurrealKV's snapshot isolation must be combined with
explicit application-level CAS and cross-record invariant guards. The
qualification did not measure workload latency, startup, RSS, write amplification,
SSD endurance, power-loss recovery, large-asset transfer, full migration,
rollback, protected-branch policy, multi-host partitions, or SurrealDS.

The isolated source uses SurrealDB's actual engine under its upstream license.
SurrealDB's BSL core and Gitr's GPL-2.0-only distribution need review before
a combined installable release. SurrealDS requires separate Enterprise licensing.

## Reproduction

```sh
lustr --engine rustc fmt --manifest-path rust/gitr-surreal-probe/Cargo.toml -- --check
lustr --engine rustc build --manifest-path rust/gitr-surreal-probe/Cargo.toml --locked
lustr --engine rustc clippy --manifest-path rust/gitr-surreal-probe/Cargo.toml --locked -- -D warnings
python3 ci/gitr-surreal-smoke.py
lustr --engine rustc tree --manifest-path rust/gitr-surreal-probe/Cargo.toml --locked -i surrealkv
```

The suite uses a fresh temporary database and may never be pointed at a
production or user repository datastore. The pinned and committed lockfile
keeps subsequent CI runs reproducible without regenerating dependencies.
