# Revision-bound literal search

Qualification must identify the exact source revision. CI evidence belongs in
the change's pull request; authored tests alone are not a passing result.

`rit::snapshot::Snapshot::search` and `rit --rit-snapshot search` search regular
and executable files directly in a committed tree. A query is a nonempty,
case-sensitive literal byte sequence of at most 4096 bytes with no LF newline.
The CLI accepts UTF-8 query text. The library also accepts other bytes.
Binary file bytes are included. There is no regex, text conversion or index.

```sh
rit --rit-snapshot search --repo /path/to/repository --revision HEAD --query TODO
rit --rit-snapshot search --repo /path/to/repository --saved build-source --path src --query Result
```

The existing omdb catalog supplies `--saved`. Search does not create or mutate
that catalog. It resolves the selected commit once and retains that commit
and root tree even if a branch or working file changes during the operation.
Catalog identity is not object retention. Use the existing explicit retention
pin when needed; concurrent GC protection is not established by search.

## Result and failure semantics

JSON reports the exact object format, commit and root tree. Each occurrence
includes raw path hex and optional UTF-8 text, blob ID, zero-based byte offset,
query byte length, one-based LF-delimited line and one-based byte column.
Columns are bytes, not Unicode characters or display columns. Overlapping
occurrences are included in depth-first Git tree order and then byte order.

Search accepts a directory or a regular file. The empty path selects the root.
Within directories it counts but never reads or follows symlinks and gitlinks.
Selecting either directly is an error. `scope: regular-file-bytes` makes this
exclusion explicit. `complete: true` means that entire selected scope was
searched, not that a submodule or symlink target was searched.

Every visited tree and file body uses the existing object hash verification.
Missing objects, malformed trees, hash mismatches, invalid paths or exceeded
limits fail the whole call. The CLI prepares the entire report before writing
JSON; those failures leave stdout empty. A later stdout I/O failure can still
interrupt delivery, as with other CLI output. No partial success is advertised.

Search never reads working files, applies filters, runs hooks, fetches missing
objects, executes C Git, follows replacement refs, writes refs or changes data.
The existing snapshot environment restrictions remain in force.

## Bounds

| Limit | Default | Maximum |
| --- | --- | --- |
| One blob, `--max-bytes` | 8 MiB | 64 MiB |
| Total accepted blob bytes, `--max-total-bytes` | 32 MiB | 256 MiB |
| Visited entries including selected root, `--max-entries` | 10,000 | 100,000 |
| Occurrences, `--max-matches` | 1,000 | 10,000 |
| Traversal tree bytes | 64 MiB | 64 MiB |
| Combined returned path bytes | 8 MiB | 8 MiB |

The existing path-depth, path-length and per-tree limits also apply. Metadata
used to resolve the initial path is checked under the snapshot reader's limits;
the traversal byte counter starts after that resolution. Reused blobs and
trees are charged for each logical path visited. Accepted byte budgets do not
promise peak process memory, decompression/delta memory or a wall deadline.

The matcher uses a prefix table and linear-time byte scanning. Each occurrence
copies its path under a separate aggregate path budget. No performance gain
over another tool is claimed.

## Acceptance coverage

`tests/snapshots/search.rs` exercises the public library and real CLI with the
candidate's PATH containing no Git executable. It checks both Git object
formats, loose/packed storage, exact identities and byte locations, saved
snapshot selection after HEAD moves, binary input, overlapping literals,
raw Unix names, ignored symlink/gitlink targets, subtree scope, empty inputs,
strict budgets and no output after missing/corrupt-object failures.

Run the existing pinned Lustr/BlueFe qualification workflow. No new CI schedule,
runtime dependency, database schema or donor source change is required.
This is revision-addressed literal search, not full Git parity, semantic/vector
retrieval, a crawler, an authenticated service or a forge/action executor.
