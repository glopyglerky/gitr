# Compatibility and limits

This is an initial working Rust implementation, not complete Git command parity.
Unimplemented commands fail instead of silently invoking the C Git executable.
The root Git source remains a reference implementation and test corpus.

## Native Git commands

| Command | Implemented behavior | Explicit differences and limits |
|---|---|---|
| `init` | Worktree/bare repositories, SHA-1/SHA-256, selected initial branch | Defaults to `main`; isolated configuration |
| `hash-object` | Exact-byte hashing and optional loose-object writes; blob/tree/commit/tag validation | Filename inputs require `--no-filters`; stdin is raw. Structured nonblob input is capped at 16 MiB. Input is snapshotted to disk to obtain a stable length. |
| `cat-file -t/-s` | Type/size from headers, loose and packed objects | Does not materialize the object payload |
| `cat-file -p` | Binary blobs, commits/tags, correct tree pretty printing | Payload capped at 64 MiB until streaming packed extraction is implemented |
| `rev-parse` | One revision; Git/common directory; worktree root; bare/worktree/object-format queries | Not the full Git argument language or shell quoting frontend |
| `show-ref` | Sorted references, annotated tag IDs, optional HEAD/heads/tags | Subset of flags |
| `log` | Full OID and subject; `--oneline`, `-n`, `--first-parent` | Compact Gitr format, not Git's full default log formatting |
| `ls-files` | Cached repository-relative paths, conflict stages, `-s`, `-z` | Sparse indexes and noncached modes unsupported |
| `status` | Porcelain v1, staged/worktree changes, conflict stages, intent-to-add, individual untracked files | No rename detection. Rejects configured filters, submodule entries and sparse indexes. Compare with `git status --porcelain --no-renames --untracked-files=all`. |
| `update-ref` | Fully qualified ref, new OID, mandatory expected old OID, ref lock and reflog | No force/delete/symbolic-ref operation. All-zero expected OID means strict creation. Requires local `user.name`/`user.email`; branch targets must be commits. |

Repository discovery reads local configuration with isolated options. It does not
load global/system configuration or includes, or execute hooks, textconv, fsmonitor,
filter programs and credential helpers. This is a deliberately limited command
contract, not transparent behavior for every Git configuration. Native add,
commit, checkout, clone, fetch, push, merge, rebase, stash and general hooks are
still implementation work. Use ordinary Git for those operations today.

Git format support and GitHub remote format support are different claims. SHA-1
is the default compatibility mode. Local SHA-256 tests do not prove that a hosting
service accepts a SHA-256 repository. No authenticated GitHub push/clone roundtrip
has been claimed for this implementation.

## LFS and assets

| Operation | Implemented behavior | Remaining work |
|---|---|---|
| `lfs clean` | Stream into verified immutable cache and emit standard v1 pointer; empty files pass through; existing supported pointers preserved | Extended pointer transforms fail explicitly rather than losing fields |
| `lfs smudge` | Verify a cached object and write its bytes; optional `--skip` retains pointer | Missing objects fail; automatic remote discovery/download is not enabled |
| `lfs pointer/import/verify` | Exact SHA-256/size, standard cache paths, atomic publication | Custom `lfs.storage` requires an explicit `--storage` objects path |
| `lfs fetch/upload` | Explicit HTTPS endpoint, basic Batch transfer, streamed bytes, hash/size verification, optional upload verification | One object per command; credential-helper integration, automatic ref traversal, transfer scheduling, resume and retry policy are pending |
| `lfs lock/locks/unlock/verify-locks` | Server API, bounded pagination, owned/foreign lock reporting | Does not automatically enforce locks on ordinary Git pushes; server must support locking |
| `assets scan` | Metadata-only inventory, .gitignore-aware walk, bounded largest-file list, no symlink traversal | Worktree inventory, not a complete history-size audit. Ignored paths are excluded even if previously tracked. |
| `assets attributes` | Reviewable Unreal/Unity/generic LFS rules | No automatic profile installation or editor plugins |
| `assets import` | FastCDC local chunk dedup, immutable chunk cache, atomic versioned manifest | No remote chunk protocol or cache eviction/GC |
| `assets hydrate` | Verified chunk-to-standard-LFS reconstruction | Writes the LFS cache, not worktree files. GitHub transfers still use full objects. |

LFS authorization is read from `GITR_LFS_AUTHORIZATION` when needed. The value is a
complete HTTP authorization value supplied by the caller. Never put a secret in a
command argument or tracked file. Redirects are rejected, original credentials
are scoped to their origin, and server action headers are validated. HTTP is
allowed only with `--allow-insecure-loopback` and a literal loopback IP for a local
service or fixture. Remote errors omit signed URLs and untrusted response bodies.

Chunk profile: FastCDC 5.0.0, v2020, normalization level 1, seed 0, minimum 64 KiB,
target 256 KiB, maximum 1 MiB. Payload buffering is at most the chunker's buffer
plus the current chunk. Manifest metadata is capped at 1,000,000 chunks and
128 MiB of JSON. Unreferenced verified chunks may remain after failed imports;
they are safe to reuse. No incomplete manifest is published. The bounded first
format is not yet a recursive manifest format for unlimited asset sizes.

On Unix, cache publication syncs the file, created directory entries and the
containing directory before acknowledging an object or manifest. Reusing an
existing object completes those barriers too. External ancestors are assumed
trusted, stable and already durable. Non-Unix platforms retain verification and
atomic publication, but portable directory syncing is unavailable here; full
power-loss durability is not claimed there. Process-kill tests are not simulated
power-loss tests on any platform.

## Repository database

`gitr db index [revision]` atomically builds a catalog of a committed snapshot in
the shared Git common directory. Search and graph queries resolve their selected
revision and require that exact completed snapshot. Uncommitted worktree changes
are not included. Git remains authoritative, and ordinary Git refs do not depend
on this derived database.

The initial graph records factual commit/tree/path/object membership. Raw paths
retain a lossless hexadecimal representation. It does not claim resolved symbols
or call edges. Vectors use 512-dimensional normalized ASCII token feature hashing
with an explicit model identity. They are lexical vectors, not trained semantic
embeddings. Each indexed blob also retains its sorted, unique lowercase ASCII
tokens. Search excludes files with no exact query-token overlap and ranks the
number of matched query tokens before vector cosine, then path order. Results
report that overlap, so a vector hash collision alone cannot produce a match.
Search scans the selected snapshot and retains a bounded top-k list; it is not
an ANN or inverted index. It needs no model downloads or network inference.

Source blobs default to a 1 MiB indexing cap with an 8 MiB maximum. Binary,
oversized, symlink and LFS-pointer content has explicit excluded coverage. Git
metadata parsing and path lengths also have limits. Gix may need additional
memory for pack/delta decoding, so the file-size cap is not an OS memory quota.

redb 4.5.0 coordinates cooperating local writer processes and reader snapshots.
Transactions serialize. Distributed replicas, quorum writes, failover, lease
fencing, learned code embeddings, resolved source graphs and GitQL integration
are designed or researched but not implemented. See [architecture](architecture.md)
and [next implementation work](roadmap.md).

## Build and dependency honesty

Lustr's coordinator is pinned in
[`lustr-toolchain.json`](../../rust/gitr/lustr-toolchain.json). The regular gate
uses its explicit `rustc` engine. Native Lust does not currently compile arbitrary
gix/redb dependencies. A separate recorded Cranelift run must use the matching
reviewed backend and compiler; merely selecting a route is not a speed result.
The pinned Cranelift backend currently aborts in a dependency's SIMD float
formatting code; see the [recorded failure](qualification.md#experimental-cranelift-result).

The Gitr package authors Rust code and forbids `unsafe` in that code. It uses
Gitoxide and Rust `zlib-rs`, not libgit2 or the inherited C Git library. Its TLS
provider currently includes ring's native code, and OS bindings are transitive
dependencies. The entire inherited fork and dependency closure are not all Rust.
