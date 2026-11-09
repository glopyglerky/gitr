# Native checkout-free Git authoring

Current root layout and produced BlueFe qualification: [rust-root.md](rust-root.md).
Compiler versions and measurements below retain their historical scope.


## Implemented and qualified on Linux

`gitr mktree`, `gitr write-tree` and `gitr commit-tree` are part of the normal CLI.
The implementation and tests were activated from the exact passing source blobs
of [run 42541753408](https://github.com/glopyglerky/gitr/actions/runs/42541753408),
job `128918003599`. This replaces the earlier uncompiled downloadable draft.
The reviewed integration script and candidate workflow have been removed: normal
builds and the existing cross-platform Rust/API gates compile committed source.
No dependency, lockfile or existing database was changed.

The initial run used pinned Lustr `62494e27717a64c8612b538409de27048fa6adc2`, explicit
`rustc` engine, Rust 1.99.1, Gitoxide 0.90.0, redb 4.5.0 and reference Git 2.56.0.
It passed 96 default-profile Rust tests, 99 API-enabled Rust tests, strict Clippy
and five backend-policy self-tests. Profile counts overlap; these are not 195
unique tests. New coverage consists of three unit tests and twelve integration
tests. The controlled process helper is deliberately invoked by parent tests.
[Machine-readable evidence](native-history-evidence.json) records exact source
hashes and scope. This initial run does not prove Windows/macOS or GitHub remote
transport support; those require their own observed tests.

## Behavior

`mktree` creates an immutable tree from ls-tree-style records. `-z` preserves
literal raw path bytes including newlines and non-UTF-8. Newline mode does not
implement quoted filenames. Entries are validated for names, modes, hash format,
object existence/type and duplicates; Gitlinks may reference external commits.
Git directory-aware ordering is used rather than ordinary lexical order.
Input is limited to 16 MiB and 100,000 entries. Unlike Git's broader option set,
batch mode and general missing-object mode are not implemented.

`write-tree` consumes a valid stage-zero index without changing it or the
worktree. It checks entry integrity, rejects unresolved stages and unsupported
sparse/split indexes, excludes intent-to-add entries and does not trust a cached
TREE extension to determine content. Child trees are built bottom-up with at most
128 path components and 100,000 entries. The aggregate path bound is 16 MiB;
these limits do not bound Gitoxide's initial index decode allocation or provide
an OS-level memory limit. Index paths use the repository's own Git index, not
an arbitrary caller-provided database path.

`commit-tree` creates root or ordered multi-parent commit objects, deduplicates
repeated parent IDs, accepts repeated -m paragraphs or -F/file/stdin input, and
requires UTF-8 without NUL. The message is limited to 1 MiB and parent arguments
to 64. It does not invoke a signer or synthesize an anonymous identity.
Repository-local configured signing is rejected instead of silently ignored;
non-UTF-8 commit encodings are not implemented. This respects Gitr's existing
isolated discovery/configuration profile, not every C Git configuration or option.
Parents must resolve directly to commits; peel annotated tags explicitly.

These commands do not move a ref, touch HEAD or stage/check out a worktree. Publish
an authored commit separately using guarded `update-ref` with its expected old
value. Native ordinary add/commit/checkout, merge/rebase and clone/fetch/push remain
separate unfinished requirements. Creating a multi-parent commit is not performing
a content merge. Failed object construction may leave unreachable valid immutable
objects, but cannot publish a partially created branch through these commands.

## Example: author without checking out source

With Gitr built and on PATH, in a fresh repository with local `user.name` and
`user.email` already configured (the global/system configuration is not loaded):

```sh
blob=$(printf 'fn main() {}\n' | gitr hash-object -w --stdin)
tree=$(printf '100644 blob %s\tmain.rs\0' "$blob" | gitr mktree -z)
commit=$(gitr commit-tree "$tree" -m 'Native root commit')
# SHA-1 example: all-zero expected old ID means create, never overwrite.
gitr update-ref refs/heads/main "$commit" 0000000000000000000000000000000000000000
gitr db index "$commit"
gitr db search main --revision "$commit" --limit 10
```

Use 64 zeroes for SHA-256 repositories. For an existing branch, supply its actual
expected old ID and pass the intended parent to `commit-tree -p`. Do not substitute
an unconditional ref update when a compare-and-swap fails. Standard Git can read
these objects; the checkout-free example deliberately does not materialize files.

## Executed end-to-end coverage

The tests compared native output to reference Git for SHA-1 and SHA-256, including
raw names, executable/symlink modes, directory ordering, exact paragraph bytes,
stdin/file messages, ordered parents and external Gitlinks. Invalid object types,
unsafe names, missing objects, bad encodings, configured signing and missing
identity fail explicitly. Staged nested content is read without changing index
bytes or including dirty worktree changes. Unmerged/corrupt indexes are rejected.

A native-only pipeline ran with an empty PATH: it created a blob/tree/commit,
published a guarded ref, indexed that commit into redb and queried it, installed a
Wasm echo action, submitted the exact created commit/blob, executed it and verified
an exact source-byte artifact. Neither the source checkout nor a Git index was
created. Both redb catalog and durable action files were present. Reference Git
was used only to configure/check the fixture and `git fsck --strict` passed.
The tests did not perform a GitHub push, benchmark speed or simulate power loss.
