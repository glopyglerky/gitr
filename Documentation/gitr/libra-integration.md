# Libra and git-internal integration review

Reviewed on 2026-11-08. This is a source review, not an upstream build, benchmark, or claim that these features are implemented in gitr. No upstream implementation was copied and no dependency was added for this review.

## Recommendation

Libra is a useful source for native push orchestration, interoperability tests, and versioned workflow metadata. It can reduce implementation work. It is not a drop-in Git client backend for gitr: its command layer owns a different repository layout and database, and some low-level paths would regress gitr's existing byte-path and guarded-reference behavior.

Keep Gitoxide as the authority for ordinary `.git` objects, index files, references, discovery, and linked worktrees. Extract or adapt selected MIT-licensed algorithms and tests behind explicit storage and transport interfaces. Preserve the upstream copyright and license notices if code is adopted.

## Pinned sources

| Project | Version inspected | Commit |
| --- | --- | --- |
| [Libra](https://github.com/libra-tools/libra) | `0.30.162` | [`f7d27a268b13df353d1e556f0c4e3b108679e5b8`](https://github.com/libra-tools/libra/tree/f7d27a268b13df353d1e556f0c4e3b108679e5b8) |
| [git-internal](https://github.com/libra-tools/git-internal) | `0.10.4` | [`841b59b62feb932ea219dc046db77ba33f121a28`](https://github.com/libra-tools/git-internal/tree/841b59b62feb932ea219dc046db77ba33f121a28) |

The downloaded `git-internal 0.10.4` registry archive's `.cargo_vcs_info.json` identifies the same commit. Libra pins that crate exactly with `=0.10.4`. Both repositories declare MIT licenses: [Libra license](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/LICENSE), [git-internal license](https://github.com/libra-tools/git-internal/blob/841b59b62feb932ea219dc046db77ba33f121a28/LICENSE).

## What is actually reusable

| Candidate | Verified API or code boundary | Proposed use in gitr |
| --- | --- | --- |
| [Libra `src/git_protocol.rs`](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/src/git_protocol.rs) | `PktLineError`, `pkt_frame_payload_len`, `read_pkt_line`, and packet framing | Reuse malformed-packet cases and parser behavior. Prefer the existing Gitoxide packet-line implementation when it already meets the same contract. |
| [Libra `src/command/push.rs`](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/src/command/push.rs) | Capability gates, refspec/lease planning, and `validate_receive_pack_response` are separable helpers; `run_push` binds them to Libra state | Extract pure planning and response-validation logic, replacing `Branch`, `Head`, `ConfigKv`, reflog, vault, LFS, and automation dependencies with gitr interfaces. |
| [Libra HTTPS client](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/src/internal/protocol/https_client.rs) | `discovery_reference`, `fetch_objects`, `send_pack(Bytes)` | Use its smart-HTTP exchange as a reference. Supply gitr's explicit credential scope and a bounded stream/file body instead of importing its authentication singleton or whole-buffer upload API. |
| [Libra `src/utils/thin_pack.rs`](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/src/utils/thin_pack.rs) | `delta_encode(&[u8], &[u8]) -> Vec<u8>` is self-contained; `ThinPackEntry` and `encode_thin_pack` use git-internal types | A narrow MIT extraction candidate for Git REF_DELTA encoding. Preserve the 64 KiB copy-operation and 127-byte literal rules; retain bounded object sizes and a full-object fallback. Its complete pack output is still a `Vec`, so adapt the output interface for large transfers. |
| [git-internal pack encoder](https://github.com/libra-tools/git-internal/blob/841b59b62feb932ea219dc046db77ba33f121a28/src/internal/pack/encode/mod.rs) | `PackEncoder::new_with_hash_kind`, `encode`, index-output support; entries arrive through Tokio channels | Evaluate as a pack-writing component, not as a complete push client. Benchmark against available Gitoxide pack facilities before taking a second object model and dependency graph. The delta-window path collects entries for sorting. |
| [Libra tree plumbing](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/src/internal/tree_plumbing.rs) and [index flag helpers](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/src/utils/index_ext.rs) | Index-object preflight, tree construction, intent-to-add exclusion, and preservation of skip-worktree flags | Reuse the behavioral contracts and tests for gitr `add`/`write-tree`/`commit`; implement them over `gix::index` and native object writes. |
| [git-internal workflow object schemas](https://github.com/libra-tools/git-internal/blob/841b59b62feb932ea219dc046db77ba33f121a28/src/internal/object/mod.rs) and [Libra history manager](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/src/internal/ai/history.rs) | Immutable snapshots/events, typed relationships, and an append loop that rebuilds a tree/commit after a stale-head CAS | Useful for optional repository event history and rebuildable graph projections. These are workflow schemas, not an automatic embedding engine or distributed database. |

### Why the published protocol crate is not the missing push client

The [`git_internal::protocol::RepositoryAccess`](https://github.com/libra-tools/git-internal/blob/841b59b62feb932ea219dc046db77ba33f121a28/src/protocol/core.rs) trait exposes repository reads/writes and reference updates. `GitProtocol`, `HttpGitHandler`, and `SshGitHandler` implement server-facing discovery, upload-pack and receive-pack handlers. The client-side transport and send-pack workflow live in Libra. Supplying a `RepositoryAccess` implementation alone does not produce `gitr push`.

The [pack protocol convenience layer](https://github.com/libra-tools/git-internal/blob/841b59b62feb932ea219dc046db77ba33f121a28/src/protocol/pack.rs) also collects complete commit/tree/blob vectors before emitting its response stream. A streaming return type is therefore not evidence of bounded memory across the entire operation.

## Compatibility and correctness constraints

**Keep the standard repository boundary.** Libra stores its authoritative refs, HEAD, configuration and other state in SQLite under `.libra`. Its stable embedding API is [`exec`/`exec_async`](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/src/lib.rs), which dispatches Libra commands. Internal modules are explicitly not a stable embedding API; concurrent embedded commands are serialized around process-wide state. Calling this API would adopt Libra's repository semantics rather than add native writes to gitr's `.git` backend.

**Do not import the index writer.** git-internal's [index implementation](https://github.com/libra-tools/git-internal/blob/841b59b62feb932ea219dc046db77ba33f121a28/src/internal/index.rs) accepts v2/v3, converts names to UTF-8 `String`, rejects required unknown extensions, and writes through `File::create`. Its `save` method adds no exclusive index lock or atomic replacement. Reusing this primitive would require adding those guarantees and would lose support for byte paths and newer index formats already available through Gitoxide.

**Do not copy the local Git-target ref updater.** At this pin, Libra's [`local_push_git::apply_refs`](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/src/internal/protocol/local_push_git.rs) writes `<ref>.lock` using `std::fs::write`, then renames it. It does not exclusively create the lock or validate `old_oid` under that lock. Deletion only removes a loose ref; there is no packed-ref rewrite in the implementation despite the module comment. Its Git-directory helper also only checks for a `.git` directory, not a linked-worktree `.git` file. Gitr's existing Gitoxide transactions and discovery should remain the implementation here.

**Rewrite the large-transfer path.** Libra's push command collects objects, accumulates compressed `pack_data` in a `Vec`, appends it to a request buffer, and passes complete `Bytes` to the transport. Its SSH transport invokes an [external SSH executable](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/src/internal/protocol/ssh_client.rs). These are real implementations, but neither proves a bounded-memory, entirely Rust gaming-asset transport.

**Keep GitHub-facing formats standard.** Explicit SHA-1/SHA-256 context must be carried across threads and async tasks. git-internal retains thread-local convenience APIs as well as explicit-kind APIs. Its [hash module](https://github.com/libra-tools/git-internal/blob/841b59b62feb932ea219dc046db77ba33f121a28/src/hash.rs) documents BLAKE3 as a separate, nonstandard repository namespace. Its [object type definitions](https://github.com/libra-tools/git-internal/blob/841b59b62feb932ea219dc046db77ba33f121a28/src/internal/object/types.rs) include custom AI types and a zstd-delta pack type. Do not put those custom types or zstd-delta payloads on ordinary GitHub Git transports. Use ordinary Git blobs for serialized application metadata, as Libra's history layer does.

**Account for native dependencies.** [git-internal's manifest](https://github.com/libra-tools/git-internal/blob/841b59b62feb932ea219dc046db77ba33f121a28/Cargo.toml) includes unconditional `zstd-sys`; disabling its default delta feature does not remove that dependency. [Libra's manifest](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/Cargo.toml) includes vendored OpenSSL, SQLite, ring and GUI dependencies. A direct dependency is not a shortcut to a dependency graph consisting entirely of Rust.

**Use the compatibility ledger, then verify behavior.** Libra labels [`add`, `commit` and `push` as partial](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/COMPATIBILITY.md). Examples: selected-path commits are explicitly rejected, push has no client pre-push hook, and git:// push is rejected. Some comments about thin packs and leases are stale while implementation exists; inspect the executable path and tests rather than treating either comments or the README as a complete contract.

## Database design worth adopting

Libra's useful separation is immutable facts versus mutable projections. Its history manager stores typed JSON as ordinary Git blobs in trees on a separate history branch. Appending writes immutable objects first, then advances the history ref using an expected parent. A stale-parent result rebuilds from the newer tree within a bounded retry loop. Unreachable objects from losing attempts can later be collected. This is a practical recipe for concurrent local metadata appends over gitr's existing guarded reference operations.

The [workflow schemas](https://github.com/libra-tools/git-internal/blob/841b59b62feb932ea219dc046db77ba33f121a28/src/internal/object/mod.rs) distinguish snapshots from subsequent events and keep current selection/query state in projections. Their [integrity hash](https://github.com/libra-tools/git-internal/blob/841b59b62feb932ea219dc046db77ba33f121a28/src/internal/object/integrity.rs) is separate from the repository object-ID algorithm. Gitr can apply the same separation to graph edges, asset manifests and embedding provenance: record the durable source object, schema and model version, then rebuild indexes when those inputs change.

Libra's [database connection layer](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/src/internal/db.rs) uses local SQLite, FULL synchronous durability and a single pooled connection. Its write-transaction helper acquires a write lock before reads to avoid a deferred-transaction upgrade race. This is useful local concurrency engineering. It does not supply replication consensus, multi-host fencing, leader failover or a distributed multiwriter contract. Gitr should not infer HA from the presence of this database.

## Next implementation slice

1. Implement `add`, `write-tree` and `commit-tree` over Gitoxide, followed by `commit` with an expected-parent update and recovery information. Use Libra's staging and tree rules as test input, retaining byte paths and exclusive index publication.
2. Extract push planning and strict remote-status validation. Keep storage, credential policy, capability negotiation and byte transport separate. Start with a self-contained pack sent from a bounded stream or temporary file over smart HTTPS.
3. Adapt Libra's [real Git receive-pack tests](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/tests/command/push_test.rs), including thin-pack completion through both `index-pack --fix-thin` and `unpack-objects`, capability refusal, and push options. Add stale-ref, interrupted-stream, packed-ref and memory-bound tests for the adapted implementation.
4. Add thin packs and SSH only after the basic transport passes those gates. Keep standard Git LFS pointers and basic transfers as the GitHub-facing asset protocol; optional chunk transport needs explicit peer negotiation.

The inspected upstream test sources establish useful test cases, not a test result for this review. Full upstream builds and live GitHub push were not performed.
