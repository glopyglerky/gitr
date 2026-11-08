# Closest upstream projects to the repository database idea

Inspected 2026-11-08. These complement the broader [Git and asset catalog](upstreams.md)
and [database comparison](database-research.md). Design lessons and code imports
are separate decisions. None of the projects below has been bulk-vendored.

## Libra and Mega

[Libra](https://github.com/libra-tools/libra/tree/f7d27a268b13df353d1e556f0c4e3b108679e5b8)
is the closest existing Rust Git client with transactional repository metadata.
The inspected version is 0.30.162 under MIT. Its `.libra` repository retains Git
object/index/pack formats and stores refs, HEAD, configuration and additional
metadata in SQLite. Its command compatibility varies; the detailed
[compatibility matrix](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/COMPATIBILITY.md)
is more useful than a blanket Git-compatible label.

Learn from its transaction boundaries, ref/worktree scoping, remote protocol
implementation and explicit unsupported-command policy. Evaluate its MIT
`git-internal` crate and small protocol components for gaps in Gitoxide, especially
push. Do not import its entire application: the
[manifest](https://github.com/libra-tools/libra/blob/f7d27a268b13df353d1e556f0c4e3b108679e5b8/Cargo.toml)
includes SQLite, vendored OpenSSL, tree-sitter and mandatory GPUI dependencies.
That conflicts with a small Rust storage core. Its current README also says the
older `libra code` runtime and `libra publish` host were removed in 0.23.x; old
search results describing those interfaces are stale.

[Mega](https://github.com/gitmono-dev/mega/tree/aecc11a98b2748eeabc73f163f7dc5fa796f53b4)
is a Git-compatible centralized monorepo design related to Libra. The repository
is archived as of 2026-10-19. Study subtree-oriented access, remote storage and
monorepo boundaries. Do not adopt an archived server as a maintained default or
repeat its scale claims as measurements of gitr.

## Git-vdb

[git-vdb 0.4.0](https://github.com/nishu-builder/git-vdb/tree/f51720bfcebe0248667b2c9598bbdc556e6559e1)
stores a vector database in immutable Git trees. It uses versioned snapshots,
compare-and-swap writes, optional historical roots and exact/approximate queries.
This is a direct precedent for combining Git history with vector retrieval.

Learn from deterministic snapshots, model identity and explicit query provenance.
Keep embeddings separate from source history by default: adding every derived
vector revision to normal Git history can increase clone, pack and GC costs.
Shared derived artifacts should use an opt-in namespace or separate repository.
Its [manifest](https://github.com/nishu-builder/git-vdb/blob/f51720bfcebe0248667b2c9598bbdc556e6559e1/Cargo.toml)
is Apache-2.0 and uses `git2`; optional FastEmbed downloads native ONNX Runtime
binaries. It is not an all-Rust Git engine or inference stack. Do not copy it into
this GPLv2-only port without resolving the license and dependency boundaries.

## GitQL

[GitQL 0.43.0](https://github.com/AmrDeveloper/GQL/tree/3a76cfee02a00ee6ce20eeac6447573b34f25d86)
is an MIT Rust query engine with separate parser, AST, execution and data-provider
crates. Its [manifest](https://github.com/AmrDeveloper/GQL/blob/3a76cfee02a00ee6ce20eeac6447573b34f25d86/Cargo.toml)
uses Gitoxide rather than libgit2. A gitr data provider could expose versioned
catalog relations to its SQL-like language without replacing Git's storage.
Evaluate that integration before writing an SQL parser. GitQL is a query engine,
not the transactional or replicated repository database itself.

## Codebase Memory

[Codebase Memory](https://github.com/DeusData/codebase-memory-mcp/tree/612eee2518ef84cc573e491fdee5fa5c90264cb5)
is an MIT code-intelligence reference. Its current README describes a native C
backend, persistent graph, parser grammars, language-specific resolution, MCP
queries, source provenance and indexed coverage. The Rust port should learn from
its distinction between source facts and inferred relationships, targeted query
results, incremental updates and coverage diagnostics.

It is not a Rust implementation to import wholesale. A hashed lexical vector is
not equivalent to its symbol resolver or call graph. Before claiming comparable
or better retrieval, build a corpus of known symbol references, overloaded calls,
cross-module imports, stale snapshots and deleted files. Measure precision, recall,
freshness, latency, memory and bytes reprocessed against the same revisions.

## Decisions this inspection changes

- Keep Gitoxide as the initial standard-object engine; inspect Libra's small MIT
  protocol components before implementing its missing operations from scratch.
- Use redb's local transaction model now and keep distributed authority separate.
- Reuse FastCDC for chunk boundaries instead of inventing a rolling hash.
- Keep the first catalog commit-addressed and reconstructable. Do not turn
  embeddings into the authority for source or history.
- Evaluate GitQL for query syntax and rust-analyzer/SCIP for resolved source facts.
- Use Lore's storage/version-control separation and binary workload tests while
  preserving the Git/LFS protocol at GitHub's boundary.
