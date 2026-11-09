# Native checkout-free Git authoring

## Qualification candidate

The recovered native authoring implementation is committed for qualification,
not yet enabled in the ordinary CLI. Its workflow uses the pinned Lustr
coordinator and existing Gitoxide 0.90.0 / redb 4.5.0 without changing dependencies
or their lockfile. Promotion requires observed passing Rust execution, not the
previous draft's Python/reference-Git checks.

## Behavior

`mktree` creates an immutable tree from ls-tree-style records. `-z` preserves
literal raw path bytes including newlines and non-UTF-8. Newline mode does not
implement quoted filenames. Entries are validated for names, modes, hash format,
object existence/type and duplicates; Gitlinks may reference external commits.
Git directory-aware ordering is used rather than ordinary lexical order.

`write-tree` consumes a valid stage-zero index without changing it or the
worktree. It checks entry integrity, rejects unresolved stages and unsupported
sparse/split indexes, excludes intent-to-add entries and does not trust a cached
TREE extension to determine content. Child trees are built bottom-up with bounded
path nesting. The entry/path limits do not bound Gitoxide's initial index decode
allocation or provide an OS-level memory limit.

`commit-tree` creates root or ordered multi-parent commit objects, deduplicates
repeated parent IDs, accepts repeated -m paragraphs or -F/file/stdin input, and
requires UTF-8 without NUL. It does not invoke a signer or synthesize an anonymous
identity. Repository-local configured signing is rejected instead of silently
ignored; non-UTF-8 commit encodings are not implemented. This respects Gitr's
existing deliberately isolated discovery/configuration profile, not every C Git
configuration or command option.

These commands do not move a ref, touch HEAD or stage/check out a worktree. Publish
an authored commit separately using guarded `update-ref` with its expected old
value. Native ordinary add/commit/checkout, merge/rebase and clone/fetch/push remain
separate unfinished requirements. Creating a multi-parent commit is not performing
a content merge.

## End-to-end evidence required

Tests compare actual objects with reference Git for SHA-1 and SHA-256, preserving
raw names, modes, paragraph bytes and parent ordering. They check stale ref writes,
unmerged/corrupt indexes, unchanged index/worktree data and explicit refusals.
A native-only authoring pipeline runs with an empty PATH, then indexes the commit
in redb, submits its source blob to the existing Wasmi action engine and verifies
an exact source-byte artifact. Git is used only to configure/check the fixture.

The source and tests remain a candidate until the workflow reports success and
its exact tested blobs are activated. No existing database or user repository is
converted, deleted or used by the qualification fixture.
