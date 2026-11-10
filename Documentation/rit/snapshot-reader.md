# Immutable source reads from Git storage

Recorded November 10, 2026, continuing from
[`c57d3c2c854ccd611738c80c87edae37f5ecdc7d`](https://github.com/glopyglerky/rit/commit/c57d3c2c854ccd611738c80c87edae37f5ecdc7d).
This implements the local source-reader portion of the
[storage architecture assessment](storage-architecture-review.md). It uses
the existing native Sley object store. It does not change storage formats or
activate the archived redb, forge, LFS, or Wasm implementations.

## What is implemented

The public [`rit::snapshot`](../../src/snapshot.rs) module opens a repository,
resolves a revision once, and retains its commit and root-tree IDs. Later reads
from the same handle keep that identity even if a branch or working tree
changes. Directory metadata and file bodies come from ordinary Git loose
objects or packs; no checkout is created and no C Git process is called.

```rust
use std::path::Path;
use rit::snapshot::{Snapshot, DEFAULT_MAX_BLOB_BYTES};

fn read_source() -> sley::Result<()> {
    let source = Snapshot::open(Path::new("/path/to/repository"), "HEAD")?;
    let entries = source.list(b"src")?;
    let file = source.read(b"src/main.rs", DEFAULT_MAX_BLOB_BYTES)?;
    println!("{} entries at {}", entries.len(), source.commit_id());
    assert_eq!(file.entry().name, b"main.rs");
    // file.bytes() borrows the verified object body retained by this handle.
    Ok(())
}
```

`SnapshotEntry` exposes a byte basename, Git mode, and object ID.
`SnapshotBlob` retains the loaded object and exposes its bytes and entry.
Returning that handle avoids an additional full-body copy; the underlying
object reader still materializes the object.

The [`rit --rit-snapshot`](../../src/snapshot_cli.rs) prefix exposes the same
operations from the product executable:

| Action | Result |
| --- | --- |
| `inspect` | JSON containing `object_format`, `commit_oid`, and `root_tree_oid`. |
| `list` | JSON snapshot identity, requested path, and the entries in one directory in Git tree order. It does not recurse. |
| `stat` | JSON snapshot identity, requested path, and one entry's metadata. |
| `read` | The verified raw blob body on stdout, with no JSON envelope or extra newline. |

```sh
rit --rit-snapshot inspect --repo /path/to/repository --revision HEAD
rit --rit-snapshot list --repo /path/to/repository --revision HEAD --path src
rit --rit-snapshot stat --repo /path/to/repository --revision HEAD --path src/main.rs
rit --rit-snapshot read --repo /path/to/repository --revision HEAD --path src/main.rs --max-bytes 1048576
```

Each CLI invocation opens a new snapshot. To keep one revision across several
invocations, pass the full `commit_oid` returned by `inspect` as `--revision`.
The API handle does this automatically. Defaults are repository `.` and
revision `HEAD`; an empty path selects the root for `list` and `stat`.

JSON names and paths contain a lossless lowercase `hex` string and a `utf8`
string when valid, otherwise `null`. Modes are JSON integers, preserving the
Git values, and `kind` is `tree`, `blob`, `symlink`, or `gitlink`. `--path` uses
the original Unix argument bytes. `--path-hex` accepts those same bytes in
hexadecimal, including on platforms whose command-line API cannot represent
them directly. The two path options cannot be combined. This is a new Rit
extension, not an emulation of an existing Git command's output format.

Only an executable named `rit`, with `--rit-snapshot` as its first argument,
selects this interface. Ordinary commands and aliases continue through the
Git frontend. Executables named `git`, `scalar`, and `git-*` retain their
existing dispatch. `rit snapshot`, for example, can still select a configured
Git alias; it is not reserved by this extension.

## Identity and traversal contract

The reader accepts a full object ID, a unique hexadecimal prefix of at least
four digits, `HEAD`, an exact ref, or an unambiguous short ref. Verified
annotated tags can peel to a commit. Ambiguous names fail. Revision expressions
such as `HEAD~1`, `tag^{commit}`, ranges, reflog selectors, and `rev:path` fail;
resolve them to a full commit ID before opening this interface.

Every loaded commit, tag, tree, and selected blob must match its canonical
object hash. Replacement refs are disabled. The reader checks the advertised
type and size before loading and checks the loaded type, size, and hash before
using the body. Selected commits and root trees must be available on open.
Child trees and file bodies are loaded only when requested.

Malformed selector headers are rejected before commit or tag traversal.
Commits must begin with their single `tree` header. Tags must begin with
`object`, `type`, and `tag` headers in order, without later duplicates of
those selectors. This closes a reviewed parser ambiguity where the general
Sley parser takes a later duplicate while reference Git takes the first.
Unknown and continuation headers remain supported, and message text is not
treated as headers.

Every loaded tree is checked for canonical encoding, valid modes and names,
duplicate names, and Git ordering. Path lookup preserves bytes and case;
it does not normalize Unicode, reinterpret backslashes, or resolve `.` or
`..`. Absolute paths, empty interior components, trailing slashes, NUL bytes,
and traversal syntax are rejected. A symlink is a leaf whose read returns its
stored target bytes. A gitlink is a leaf carrying its foreign commit ID;
neither can be traversed. The foreign submodule commit need not be present
to list the containing tree.

Listing and stat establish tree membership, not payload availability or full
repository connectivity. An unrequested missing blob does not prevent another
file from being read. A missing, corrupt, oversized, or wrong-type requested
object fails; it is never substituted with empty content. CLI object validation
finishes before the first payload byte is written. An output-device error can
still interrupt a write after validation.

## Limits and environment

| Limit | Value |
| --- | --- |
| Default accepted uncompressed blob | 8 MiB |
| Maximum caller-selected blob limit | 64 MiB |
| Each commit, tag, or tree body | 8 MiB |
| One path or revision name | 64 KiB |
| Path depth | 256 components |
| Entries in one loaded tree | 100,000 |
| Symbolic-reference hops / annotated-tag hops | 32 / 32 |

`--max-bytes` applies only to `read`; zero permits an empty blob. These are
accepted-content limits, not a strict process-memory or execution-time bound.
The Sley reader may allocate while loading indexes, decompressing malformed
input, or reconstructing deltas. Streaming, cancellation, and host allocation
limits require additional work before exposing this as an untrusted worker API.

Repository discovery ignores `GIT_DIR` and `GIT_WORK_TREE`. The following
variables must be unset, including empty assignments:

- `GIT_COMMON_DIR`
- `GIT_OBJECT_DIRECTORY`
- `GIT_ALTERNATE_OBJECT_DIRECTORIES`
- `GIT_REFERENCE_BACKEND`

Sley still consults these below its discovery-isolation option. The new reader
fails explicitly instead of accepting ambient storage redirection or changing
the process environment. Repository-configured storage remains the ordinary
Sley repository behavior. This local API is not a tenant authorization boundary.

## Qualification

The root [`snapshots.rs`](../../tests/snapshots.rs) integration target constructs
fixtures with pinned reference Git and invokes the actual Rit executable with
an empty `PATH`. Both SHA-1 and SHA-256 are exercised. Exact comparisons cover
tree entries from `ls-tree -z` and file bodies before and after a real delta
repack. Additional cases cover stable API identity, replacement refs, raw Unix
names, modes, symlinks, gitlinks, missing/corrupt objects, strict paths and
limits, bare repositories, linked worktrees, environment isolation, and command
dispatch. The test compares index and reference state around reads.

| Executed local check | Result |
| --- | --- |
| Root all-feature, all-target suite | 18 passed: 2 snapshot unit tests, 11 snapshot integration tests, and 5 existing Git workflow/grep tests. |
| Independent failure-path matrix | 88 of 88 checks passed across SHA-1 and SHA-256 on the final binary. |
| Imported-source verification | Passed: 680 files, with 36 existing adaptations and 11 existing additions. No vendored source changed in this slice. |
| Composition policy and staged-source inventory | Passed. |
| Changed Rust file formatting and whitespace checks | Passed. |

The initial targeted run had one test-fixture failure: copying a loose Git
object preserved read-only permissions, so the test could not overwrite it
with corrupt bytes. The fixture now unlinks it before replacement. The initial
log is retained as intermediate evidence; both final test runs pass.

Executed results and exact source/binary identities are recorded in
[`snapshot-reader-evidence/result.json`](snapshot-reader-evidence/result.json).
Local application checks use pinned Lustr's explicit rustc route, Rust 1.99.1,
and Git 2.55.0. Hosted produced-BlueFe qualification is a separate gate on the
published commit. Its existing root-test phase includes this new target.

## Remaining architecture work

An immutable ID is not a retention lease. Garbage collection can remove
unreferenced objects after a handle opens; a later read then fails. This API
does not pin refs, fetch promised objects, load LFS assets, apply checkout
filters or line-ending conversion, or create a working tree. Raw blob access
is not complete checkout compatibility.

The next storage-service work needs retained snapshot ownership and recovery,
bounded streaming and cancellation, a job-owned writable overlay, and scoped
worker access. Later redb integration must use short metadata transactions and
a defined publication/reconciliation protocol with Git refs and payloads.
Wasm execution, native job profiles, artifact publication, remote backing
storage, and multi-host replication remain separate unfinished stages.

No performance gain, platform matrix, full Git-conformance increase, or
zero-copy CI execution is claimed by this change. Existing Git parity gaps
remain recorded in the earlier qualification reports.
