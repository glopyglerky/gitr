# Rit development contract

## Current priority and canonical branch

Rit is Matt Mayhugh's Git-compatible Rust implementation. Active work belongs
on the default branch, `master`; commit tested, reviewable changes there when
safe. The owner's current instruction is full Git behavior first, assembled
from reusable upstream Rust source, then database and performance improvements.
The existing implementation and layout are replaceable. Scope each ChatGPT.com
turn to a concrete, finishable change, with evidence and a commit.

The active engine is the pinned Sley import. Preserve its coherent crate layout,
Git-compatible CLI implementation, and useful tests. The CLI currently contains
real command behavior; do not discard it as presentation-only code. Rit's
entrypoints must execute the engine, not merely inventory it.

Full Git parity is an acceptance goal, not the current status. A command name,
large source import, passing smoke test, or upstream assertion count does not
prove complete command semantics. Never silently fall back to C Git and call
that native Rust execution. Keep a reproducible pinned reference oracle outside
the tracked product source. Do not reimport the C Git implementation to fill gaps.

## Database, forge, and preserved work

Redb remains the selected future database, with the reviewed 4.5.0 pin preserved
in `Documentation/gitr/archive/database-first`. The archived root source,
manifest, lockfile, and tests retain the catalog, object-store, Git LFS, chunking,
Wasm actions, and forge work. They are not active product capabilities today.
Do not delete, migrate, or open users' existing databases as part of this source
reorganization. Later integration must preserve exact Git identities, durability,
compare-and-swap, idempotency, worker fencing, atomic receipts and artifact
publication, bounded-memory payloads, and independently verifiable exports.

Do not activate SurrealDB, SurrealKV, or SurrealDS. Keep Rit independently
installable without mandatory Omnidia services. Search indexes are rebuildable
and revision-addressed; embeddings never identify Git objects or decide merges.
Retain the eventual self-contained Git, forge, CI/CD, Wasm action, and stateless
API/MCP goals. They follow compatibility work; they do not justify an ever-growing
research catalog instead of a working Git implementation. Multi-host HA needs
real replication and consensus; sharing a writable redb file is not HA.

## Source, licensing, and upstream reuse

Import coherent upstream modules and their tests instead of rewriting code that
already works. Record the exact commit, every imported path and original blob,
current SHA-256, exclusions, local patches, dependency locks, license declarations,
and qualification. Keep donor updates reviewable. Preserve every donor notice;
modifying imported code does not relicense it.

New independently authored Rit code uses MIT OR Apache-2.0. Sley remains
Apache-2.0. Historical Grit's GPL-2.0-only CLI must remain outside this executable's
dependency graph. The root license file describes the mixed source repository;
no permissive label overrides inherited Git/Grit material. Do not copy private
or proprietary source into this public repository without explicit authorization.

The current functionality-first integration supersedes the former blanket
removal rule for foreign compatibility assets. Production Git algorithms should
be Rust. Retain explicitly inventoried small native-dispatch shell adapters,
caller-supplied Git hooks/filters, and generated donor test fixtures where needed
for the Git contract. The copied shell filter-branch engine and git-sh-i18n
library are excluded pending a resolved source-origin/license composition or
qualified native replacements. Do not restore those fragments under the donor's
blanket Apache label. Do not rename files or
embed foreign code to conceal its language. Document these exceptions and native
transitive dependencies; do not claim an exclusively Rust or wholly memory-safe
artifact. Standalone upstream development scripts and the C Git oracle remain
external. See `Documentation/gitr/rust-only-source.md`.

## Git correctness

Git object bytes/IDs, refs, index/worktree behavior, configuration, CLI output,
exit codes, and wire protocols are compatibility contracts. Writable refs require
checked expected-old-value publication under the appropriate lock. Reject stale
writers and preserve data on partial failure. Do not acknowledge references to
incomplete objects or silently replace missing content with empty data.

Test malformed objects, stale writes, concurrent publication, missing descendants,
interrupted I/O, and authentication boundaries where touched. Preserve raw Unix
filename bytes; never claim global byte-path parity from a staging-only fix.
Keep credentials out of logs and command arguments, and do not forward endpoint
authorization across origins. Git-compatible configured hooks, filters, aliases,
and tools must remain explicit and auditable rather than triggering a blanket
ban on otherwise unrelated commands.

## Build and qualification

Use pinned Lustr. Its ordinary rustc route can validate local development; record
that compiler/version honestly. Produced BlueFe qualification uses the exact
bundle in `lustr-toolchain.json` and remains a separate CI gate. Native Lust does
not compile arbitrary Rust dependencies; do not claim it does or fall back
silently after a failed compiler route.

Run root workflows, changed engine tests, import verification, and source/storage
policy checks. Record executed tests separately from configured or queued CI.
The selected Sley library/CLI subset is not the full upstream Git suite. Preserve
failing or excluded cases in the evidence. Performance claims need reproducible
equal-work measurements, and platform support needs execution on that platform.

Run the Rust policy and inventory commands through Lustr:
`run --manifest-path xtask/Cargo.toml --locked -- policy` and `audit`.
The inventory's extension labels do not prove the absence of embedded programs.
