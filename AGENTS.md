# Rit development contract

## Product and canonical branch

Rit is a Git-compatible implementation being ported to Rust. This public
repository contains the Git port, upstream source, compatibility support,
compiler integration, and the documentation and tests needed to develop them.
Keep unrelated products and integration plans outside this repository.

Active work belongs on the default branch, `master`. Commit tested, reviewable
changes there when safe. Scope each turn to a concrete change with evidence.
The intended result is complete Git behavior implemented in Rust, assembled
from reusable upstream projects and completed incrementally with Lustr.
Full native Git parity is an acceptance goal, not the current status.

## Native and compatibility execution

The native engine is the pinned Sley import. Preserve its coherent crate layout,
Git-compatible command implementations, and useful tests. Rit's entrypoints
must execute the engine rather than merely inventory it.

Connected upstream Git source at `compat/git` covers behavior while the Rust
port remains incomplete. Keep its exact revision, source notices, build inputs,
and runtime selection recorded. The compatibility executable runs as a separate
process. It must never be described as Rust execution.

Choose the backend before invoking any command. Never retry a failed native
command with another engine because it may already have changed the repository.
Do not search PATH for an arbitrary fallback executable. Reject recursive
routing and unqualified compatibility installations.

`RIT_BACKEND=native` explicitly exercises the native Sley implementation.
The default compatibility route remains in place until command and option
semantics have evidence sufficient to admit native routing. Retain connected
Git implementations until their Rust replacements work through the real CLI,
with matching output, exit status, repository effects, and relevant platforms.
Removing source or passing a small smoke test does not establish parity.

## Upstream source and licensing

Import coherent upstream modules and their tests instead of rewriting code that
already works. Record exact commits, imported paths, original blobs, current
hashes, exclusions, local patches, dependency locks, and license declarations.
Keep donor updates reviewable and preserve every donor notice.

New independently authored Rit code uses MIT OR Apache-2.0. Sley retains
Apache-2.0. Connected and inherited Git material retains its applicable Git
licenses. Do not combine separately licensed implementations by relabeling
source, removing notices, or importing incompatible code into one executable.
A separate compatibility process does not waive distribution obligations.

The native production algorithms should be Rust. Keep explicitly inventoried
helper adapters, caller-configured hooks and filters, generated donor fixtures,
and the connected Git implementation when required for compatibility. Do not
rename or embed foreign programs to conceal their language. Do not claim the
complete artifact is exclusively Rust or wholly memory safe.

## Git correctness

Git object bytes and IDs, refs, indexes, worktrees, configuration, CLI output,
exit status, and wire protocols are compatibility contracts. Ref writers need
checked expected-old-value publication under the appropriate lock. Reject stale
writers and preserve data on partial failure. Do not acknowledge references to
incomplete objects or replace missing content with empty data.

Test malformed objects, stale writes, concurrent publication, missing descendants,
interrupted I/O, and authentication boundaries when a change touches them.
Preserve raw Unix filename bytes. Never infer general byte-path parity from one
staging fixture. Keep credentials out of logs and command arguments. Do not
forward endpoint authorization across origins.

## Lustr and qualification

Use the pinned Lustr coordinator. Its ordinary rustc route can validate local
development; record the actual compiler and version. Produced BlueFe
qualification uses `lustr-toolchain.json` and remains a separate CI gate.
Native Lust does not compile arbitrary Rust dependencies. Do not silently
change compiler routes after a failure.

Measure native Rust execution independently with `RIT_BACKEND=native`.
Compatibility tests must select and verify the connected Git installation.
Run the changed engine tests, source-import verification, and composition checks.
Record executed tests separately from configured or queued CI. The selected
Sley library and CLI tests are not the complete upstream Git suite. Retain
failing and excluded cases in the evidence. Performance claims need reproducible
equal-work measurements; platform support needs execution on that platform.

Run the composition and inventory commands through Lustr:

```sh
lustr run --manifest-path xtask/Cargo.toml --locked -- policy
lustr run --manifest-path xtask/Cargo.toml --locked -- audit
```

The inventory classifies tracked files and the exact Git source connection.
Its labels do not prove the absence of native dependencies or embedded programs.
