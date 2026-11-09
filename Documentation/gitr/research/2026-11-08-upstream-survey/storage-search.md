# Search, code intelligence, storage and replication donors for gitr

Research observed 8–9 November 2026. This is the storage and search portion of the wider upstream survey for `glopyglerky/gitr`.

## Recommendation

Keep redb 4.5.0 as gitr's authoritative store. The best reuse opportunities are whole libraries, bounded protocol modules and language rules that can sit above it. Start with GitQL's query engine, Syntext's revision-content indexing interface, Basemind's maintained stack-graph crates, and an OpenRaft adapter derived from Rift Cluster. Qualify Mesquite as the Rust parser option. For LFS and peer transport, there is enough existing Rust code to avoid writing the wire protocols from scratch, although several donors still launch the Git executable.

The license assessment now considers product merit across MIT, Apache, MPL, GPL and AGPL donors. The user's willingness to change gitr's license makes these candidates worth evaluating. It does not by itself change rights to inherited source, grant permission for unlicensed repositories, or resolve conflicting upstream license files. The main report owns the final license strategy.

| Priority | Candidate | Concrete material to reuse | Integration decision |
| --- | --- | --- | --- |
| First | [AmrDeveloper/GQL](https://github.com/AmrDeveloper/GQL) | `gitql-parser`, `gitql-engine`, `gitql-ast`, `gitql-core`, standard functions | Implement a redb-backed, repository-authorized snapshot provider; impose query budgets. |
| First | [whit3rabbit/syntext](https://github.com/whit3rabbit/syntext) | `ContentSource`, trigram/regex indexing, revision-content indexing machinery | Feed immutable gix blob contents; keep derived index generations and their revision manifests separate from canonical Git data. |
| First | [Goldziher/basemind](https://github.com/Goldziher/basemind) | Maintained stack-graph forks, language rules, resolver components | Keep the selected crates and rules; adapt persistence to redb. |
| Qualification pilot | [mesa-hills-research/mesquite](https://github.com/mesa-hills-research/mesquite) | Rust Tree-sitter-compatible runtime and translated grammars | Evaluate parser and query compatibility before replacing Tree-sitter dependencies. |
| First HA prototype | [databendlabs/openraft](https://github.com/databendlabs/openraft) and [achird-labs/rift-cluster](https://github.com/achird-labs/rift-cluster) | Consensus library and actual OpenRaft/redb log, vote, snapshot and state-machine adapter | Pin a mutually compatible OpenRaft release and port storage to redb 4.5.0. Test crash recovery and read consistency. |
| First LFS prototype | [rustutils/git-lfs](https://github.com/rustutils/git-lfs), [jasonwhite/rudolfs](https://github.com/jasonwhite/rudolfs), [FerrLabs/LFSX](https://github.com/FerrLabs/LFSX) | LFS client/filter/protocol crates; streaming object storage; locking and backend components | Replace Git subprocess and standalone server assumptions with gitr services. |
| Selective | [wronex/iroh-git](https://github.com/wronex/iroh-git), [magik6k/git-remote-iroh](https://github.com/magik6k/git-remote-iroh) | Peer transport, stream forwarding, authorization, tickets and LFS custom transfer | Keep Git protocol identity; connect transport to gitr's native pack service. |
| Selective | [quickwit-oss/tantivy](https://github.com/quickwit-oss/tantivy), [jean-pierreBoth/hnswlib-rs](https://github.com/jean-pierreBoth/hnswlib-rs) | Full-text/BM25 and approximate nearest-neighbor indexes | Build replaceable, revision-addressed search indexes; neither becomes Git transaction authority. |

These are source-review recommendations, not claims that the donors build together today. No donor build, benchmark, fault-injection run or full security audit was performed during this survey.

## Scope and evidence

The accompanying [inventory](storage-search.json) records 96 candidates. It includes 94 GitHub repositories, one Tangled repository read directly and one inaccessible Codeberg lead. Inspection reached selected runtime or interface files in 13 repositories, Tree-sitter's native-build script in one more, manifests or READMEs for another 25, a primary README for the Tangled candidate, and metadata or search results for the remainder. The 83 unique repository/path pairs across 39 GitHub repositories comprise 17 runtime/interface Rust files, one build script, 44 Cargo manifests and 21 documentation, license or notice files. The weaker entries remain leads, not qualified substitutes for the shortlisted projects.

The [query ledger](storage-search-ledger.json) records 20 GitHub repository queries and six batches of web queries. Searches used `fork:true` where useful and included GitHub, GitLab, Tangled, Codeberg, SourceHut, crates.io, docs.rs and first-party project documentation. GitHub queries returned their first page of up to 100 results. The broad `codebase-memory-mcp in:name fork:true` and `"git lfs" language:Rust` queries reached that cap. This is a broad, reproducible search, not proof that every public, private, deleted or unindexed project was found.

Direct GitHub fork-list endpoints were rejected by the connector's URL policy. Fork-inclusive repository searches and repository parent metadata supplied the verified fork links. Codeberg and SourceHut retrieval encountered robots restrictions. Names, similar descriptions and copied READMEs were not treated as proof of ancestry.

All source links below point to first-party repositories. The inventory records observed head SHAs separately so a future import can pin the exact version. Metadata license labels are preliminary when the report has not inspected the relevant manifest or license text.

## Code query and indexing

### GitQL offers the cleanest query-engine integration

At commit `3a76cfee02a00ee6ce20eeac6447573b34f25d86`, GitQL is a MIT-licensed Rust workspace with independent parser, engine, AST, core, CLI and standard-function crates. Its current root manifest uses `gix 0.80.0`. The most useful boundary is the engine's `DataProvider` trait:

```rust
fn provide(&self, table: &str, selected_columns: &[String])
    -> Result<Vec<Row>, String>;
```

That permits gitr to supply rows from its own revision snapshot without adopting GitQL's complete CLI or its on-disk repository adapter. The SDK documents providers as an extension point. This is a strong candidate for whole-crate reuse. [Manifest](https://github.com/AmrDeveloper/GQL/blob/3a76cfee02a00ee6ce20eeac6447573b34f25d86/Cargo.toml), [provider trait](https://github.com/AmrDeveloper/GQL/blob/3a76cfee02a00ee6ce20eeac6447573b34f25d86/crates/gitql-engine/src/data_provider.rs), [SDK provider documentation](https://github.com/AmrDeveloper/GQL/blob/3a76cfee02a00ee6ce20eeac6447573b34f25d86/docs/sdk/provider.md).

The current provider returns a materialized `Vec<Row>`. The Git adapter also contains unchecked assumptions around object and path access. A public forge must impose result, execution and memory limits, preserve non-UTF-8 Git path identity and handle malformed repository data without panicking. Repository authorization belongs before row production, not after SQL evaluation. Those are adapter requirements, not reasons to rewrite the parser. [Git provider implementation](https://github.com/AmrDeveloper/GQL/blob/3a76cfee02a00ee6ce20eeac6447573b34f25d86/src/gitql/gitql_data_provider.rs).

### Syntext can index Git snapshots without a checkout

Syntext is a MIT-licensed Rust trigram/regex search project. At commit `d0fdd127e781142933d8747821de8ab4fc74c90c`, `src/index/source.rs` defines a `ContentSource` interface that enumerates source files and reads bytes subject to a limit. The external-content path does not read a working tree or launch Git. That matches gitr well: resolve a commit, enumerate its tree through gix, and supply its immutable blob bytes. [Manifest](https://github.com/whit3rabbit/syntext/blob/d0fdd127e781142933d8747821de8ab4fc74c90c/Cargo.toml), [content interface](https://github.com/whit3rabbit/syntext/blob/d0fdd127e781142933d8747821de8ab4fc74c90c/src/index/source.rs).

The repository also separates index manifests, snapshots, commits, deltas and freshness handling into modules. Those are concrete paths for follow-up extraction. Its optional symbol support brings Tree-sitter and bundled SQLite, so importing the whole default application would need a dependency review. The lexical index can be evaluated independently. It does not provide learned embeddings or fully resolved cross-file name binding. [Index module directory](https://github.com/whit3rabbit/syntext/tree/d0fdd127e781142933d8747821de8ab4fc74c90c/src/index).

### Tantivy is a strong search library, with a storage adapter still to write

Tantivy's MIT-licensed library supplies full-text indexing and BM25 ranking. Its `Directory` trait is an explicit storage abstraction. A gitr integration can first publish immutable index generations and store generation-to-revision mappings in redb. A later redb-backed `Directory` implementation is possible work, but no ready adapter was verified here. [Tantivy manifest](https://github.com/quickwit-oss/tantivy/blob/d66f88d8f88df5b5ab62129f4a367eebdc70dd32/Cargo.toml), [Directory trait](https://github.com/quickwit-oss/tantivy/blob/d66f88d8f88df5b5ab62129f4a367eebdc70dd32/src/directory/directory.rs).

Do not equate a Rust library with a binary free of native dependencies. The observed Tantivy manifest enables `columnar-zstd-compression` by default. Selected features and transitive codecs need an audit if gitr requires a fully Rust implementation. [Columnar manifest](https://github.com/quickwit-oss/tantivy/blob/d66f88d8f88df5b5ab62129f4a367eebdc70dd32/columnar/Cargo.toml).

For vector search, `hnsw_rs` from `jean-pierreBoth/hnswlib-rs` and `rust-cv/hnsw` are more appropriate Rust implementation candidates than C++ USearch or Chroma's C++ hnswlib fork. `hnsw_rs` declares MIT OR Apache-2.0 and has empty default features in the inspected manifest. This was a manifest-level assessment, not an algorithm audit. Persist model identity, version, dimensions, blob/revision keys and index generation in redb. Keep an exact search implementation as a correctness and recall reference. [hnsw_rs manifest](https://github.com/jean-pierreBoth/hnswlib-rs/blob/3669dc6ecee50f9cb4faa46b4439042fce30aa80/Cargo.toml), [rust-cv/hnsw](https://github.com/rust-cv/hnsw).

`nishu-builder/git-vdb` is a useful adjacent design for immutable, versioned vector indexes and canonical roots, but it uses git2/libgit2 and optional FastEmbed/ONNX dependencies. It is a design and selected-module donor, not a reason to replace redb or a verified benchmark result. [Pinned manifest](https://github.com/nishu-builder/git-vdb/blob/f51720bfcebe0248667b2c9598bbdc556e6559e1/Cargo.toml).

## Resolved code intelligence and a Rust parser option

### Basemind contains useful maintained stack-graph derivatives

The original `github/stack-graphs` project is archived. Basemind documents vendoring and maintaining its Rust crates and language rules. The inspected Basemind head is `88930cc70b09de11b6925cb7d8118d1adcdfe5b7`; its attribution file pins the original stack-graphs snapshot to `fcb7705d5b38ae13b3665a9b2c882e5a97243d44`. The forks have their own published crate names:

- `basemind-stack-graphs`
- `basemind-tree-sitter-graph`
- `basemind-tree-sitter-stack-graphs`

The app is MIT; the retained upstream fork code and rules are MIT OR Apache-2.0. Documented changes include removing the core C FFI and SQLite storage dependencies, turning two stitching panic paths into recoverable errors, and updating Python rules for newer syntax. This is one of the survey's clearest cases where a derivative has specific useful changes. [Attributions and fork history](https://github.com/Goldziher/basemind/blob/88930cc70b09de11b6925cb7d8118d1adcdfe5b7/ATTRIBUTIONS.md), [stack-graphs crate](https://github.com/Goldziher/basemind/blob/88930cc70b09de11b6925cb7d8118d1adcdfe5b7/crates/stack-graphs/Cargo.toml), [workspace](https://github.com/Goldziher/basemind/blob/88930cc70b09de11b6925cb7d8118d1adcdfe5b7/Cargo.toml).

Basemind's JavaScript/TypeScript path uses the Rust Oxc tooling. Python and Java use stack-graph rules; other languages use narrower Tree-sitter-based resolution. The inspected `resolve_file` returns empty facts for some unsupported or failed parses. Gitr should retain language coverage and resolution confidence in its data model so an empty result cannot be mistaken for proof that a symbol has no references. [Resolver implementation](https://github.com/Goldziher/basemind/blob/88930cc70b09de11b6925cb7d8118d1adcdfe5b7/src/intel/resolve.rs).

Adopt the maintained crates, selected resolver modules and rule files. The full application uses Fjall, optional LanceDB and other services or native libraries. Swapping gitr's authoritative store to match that application would defeat the useful separation already present in the source.

### Mesquite deserves a focused qualification project

Mesquite translates the Tree-sitter runtime and grammar implementation into safe Rust. At commit `440d3a8707da03bc4adeff0c1a0e12713890c6ed`, the runtime and the inspected Rust grammar both contain `#![forbid(unsafe_code)]`. The workspace includes 58 grammar packages. The runtime's library name remains `tree_sitter`, which helps some import paths but does not establish drop-in compatibility. [Runtime source](https://github.com/mesa-hills-research/mesquite/blob/440d3a8707da03bc4adeff0c1a0e12713890c6ed/runtime/src/lib.rs), [runtime manifest](https://github.com/mesa-hills-research/mesquite/blob/440d3a8707da03bc4adeff0c1a0e12713890c6ed/runtime/Cargo.toml), [Rust grammar](https://github.com/mesa-hills-research/mesquite/blob/440d3a8707da03bc4adeff0c1a0e12713890c6ed/grammars/rust/src/lib.rs).

Most of the code is MIT. The Astro, Elixir, Erlang and HCL grammar lineage requires Apache-2.0 notices. Keep the upstream NOTICE and per-grammar license records. The documented differences are material: C grammar crates cannot be mixed with this runtime, UTF-16/custom encodings are absent, the Wasm grammar loader is absent, parser/query APIs differ in places, and grammar crates do not expose the usual query-string constants. Basemind currently uses Tree-sitter 0.27 while the inspected Mesquite package is 0.25.10. A dependency alias alone will not solve that mismatch. [API differences](https://github.com/mesa-hills-research/mesquite/blob/440d3a8707da03bc4adeff0c1a0e12713890c6ed/docs/api-differences.md), [NOTICE](https://github.com/mesa-hills-research/mesquite/blob/440d3a8707da03bc4adeff0c1a0e12713890c6ed/NOTICE).

This is a recent project. Its own performance and compatibility claims were not reproduced. Qualify the exact grammar versions, error recovery, incremental reparsing, query captures and malformed-input limits that gitr needs before committing to it. That work can run in parallel with adopting the higher-level resolver interfaces.

The original Tree-sitter Rust bindings compile `lib.c` through `cc::Build`. `shadaj/tree-sitter-c2rust` is a verified fork with machine-translated Rust and little-endian limitations documented upstream. Rust-sitter and Adze are additional Rust parser-generator directions, but this survey did not qualify them as substitutes for the required language set. [Tree-sitter binding build](https://github.com/tree-sitter/tree-sitter/blob/master/lib/binding_rust/build.rs), [c2rust fork](https://github.com/shadaj/tree-sitter-c2rust), [Rust-sitter](https://github.com/hydro-project/rust-sitter), [Adze](https://github.com/EffortlessMetrics/adze).

### Avoid misleading Rust and license matches

`DeusData/codebase-memory-mcp` is currently a C implementation. Its graph model, tool behavior and regression cases can inform gitr's requirements, but it is not a wholesale Rust donor. The Rust-named port `gaoxinghui2014-beep/codebase-memory-mcp-rust` declares MIT and contains a Rust workspace, but uses bundled SQLite and Tree-sitter and has a local sibling path dependency on `rust-database-compatible-access-sdk`. The copied original repository/author metadata does not prove that it is an official upstream port. [Original](https://github.com/DeusData/codebase-memory-mcp), [Rust-port manifest](https://github.com/gaoxinghui2014-beep/codebase-memory-mcp-rust/blob/fc273f066bc62b12b4a1c1817511db9ac7884cad/Cargo.toml).

Bloop's archived Rust backend remains a valuable source of scope-resolution logic, code-navigation logic and language `.scm` rules. Pin the `oss` branch at `431e9e82c5a293c40f22aadb3615c3aa387af82e`. That branch is Apache-2.0 and its inspected tree has no `ee` directory. A separate `main` snapshot contains a proprietary enterprise license under `server/bleep/src/ee`; describing every Bloop branch as Apache-2.0 would be wrong. The full backend also brings native parser/tokenizer dependencies and a Qdrant service. [OSS manifest](https://github.com/BloopAI/bloop/blob/431e9e82c5a293c40f22aadb3615c3aa387af82e/server/bleep/Cargo.toml), [OSS intelligence directory](https://github.com/BloopAI/bloop/tree/431e9e82c5a293c40f22aadb3615c3aa387af82e/server/bleep/src/intelligence), [enterprise license on inspected main snapshot](https://github.com/BloopAI/bloop/blob/002854f4d956e1274e4da7fb2252f7cbbd96fa65/server/bleep/src/ee/LICENSE).

The parent research independently compared `BloopAI/gitoxide` with upstream. Its observed default head is one commit ahead and 5,687 behind, and the extra change materializes license and source symlinks. No new Git algorithm was found there. Preserve its provenance in the catalog; use current upstream Gitoxide for the engine. Bloop's `esaxx-rs` fork also wraps C++, so it does not solve a native-dependency requirement. [Bloop Gitoxide fork](https://github.com/BloopAI/gitoxide), [Bloop esaxx-rs fork](https://github.com/BloopAI/esaxx-rs).

Graphenium, codesearch, gossiphs, tsg_indexer, HyperAST, Houndr, mozsearch, Probe and ast-grep remain useful selective candidates. Their limits differ: some use Tree-sitter or git2, some assume SQLite/LMDB, some download ONNX runtimes, and Graphenium enables a hosted Claude semantic path by default. The inventory retains these rather than discarding them on language popularity or license alone. Import only the capability whose behavior and dependencies have been checked.

## Replication and redb

### Reuse a consensus library and a real storage implementation

OpenRaft has MIT OR Apache-2.0 licensing and separates network and storage concerns. Its storage traits explicitly require ordered, durable log and vote handling. State-machine recovery must preserve applied position, membership and snapshots. The observed main head is `e56b107bc3270da01de692686e81426c1507fa0b`, whose API is newer than the 0.9 line used by several concrete adapters. Pick one compatible release before porting code. [Log-storage trait](https://github.com/databendlabs/openraft/blob/e56b107bc3270da01de692686e81426c1507fa0b/openraft/src/storage/v2/raft_log_storage.rs), [state-machine trait](https://github.com/databendlabs/openraft/blob/e56b107bc3270da01de692686e81426c1507fa0b/openraft/src/storage/v2/raft_state_machine.rs).

Rift Cluster contains the strongest inspected direct donor: an Apache-2.0 implementation of OpenRaft 0.9 storage over redb. Its `store.rs` handles log entries, vote metadata, snapshots, applied positions, membership and response deduplication. This gives gitr concrete code to adapt rather than a toy consensus example. It also embeds domain-specific routing behavior, which must be removed from a general gitr adapter. [Pinned storage implementation](https://github.com/achird-labs/rift-cluster/blob/3e5dccfe5e69dccd508e1efa043e831d1e92fe80/crates/rift-cluster/src/raft/store.rs).

The donor keeps snapshot metadata in redb and snapshot payloads in plain sidecar files. Log and vote writes use `Durability::Immediate`; snapshot metadata and state-machine writes use `Durability::None`, with the durable log providing the recovery boundary. Deduplication entries expire after 24 hours, so the implementation does not promise permanent duplicate suppression. Gitr must prove its own snapshot publication, crash-replay and acknowledgment contract before copying those choices. The same applies to deduplication retention, membership changes, cancellation and reads after leader change. A successful happy-path replication demo is insufficient for a forge that will acknowledge pushes or execute durable jobs. [Pinned storage implementation](https://github.com/achird-labs/rift-cluster/blob/3e5dccfe5e69dccd508e1efa043e831d1e92fe80/crates/rift-cluster/src/raft/store.rs).

Two other real adapters merit comparison. Danube's Apache-2.0 `danube-raft` crate uses OpenRaft 0.9, redb 2 and Tonic/Prost. Hyperion Vault's AGPL-3.0-only code uses OpenRaft 0.9.25 and redb 4.1.0. The latter is closer to the selected redb major version, though its KMS domain is irrelevant to gitr. Neither was source-audited to the same depth as Rift Cluster. [Danube manifest](https://github.com/danube-messaging/danube/blob/9632acf6d875f65a50a8a4d5eafab3b2d518c5be/danube-raft/Cargo.toml), [Hyperion Vault](https://github.com/hyperiondb/hyperion-vault).

Redb remains the local durable database. Its current multi-process support is not a distributed consensus or multi-host replication protocol. Do not share a writable redb file among HA nodes. Clients may enter through several servers, but acknowledged authoritative state changes must follow the chosen replication and conflict policy. Search indexes can rebuild after log replay; Git ref compare-and-swap, object visibility, durable action claims and idempotency records cannot rely on approximate search state. [redb 4.5 source and README](https://github.com/cberner/redb/tree/b15a899d734150b8389e38ddcde7d042d3fa1a5c).

Fjall, Cozo, SurrealKV, LanceDB, Qdrant, IndraDB and agdb are retained as design references or optional derived-index candidates. The gitr contract explicitly keeps redb and excludes a Surreal-family database replacement. There is no need to reopen that decision to use good query, parser, ANN or replication code.

## LFS, object storage and peer transport

### Rust LFS has reusable client and server pieces

`rustutils/git-lfs` is a GitHub mirror of a GitLab project with a MIT-licensed modular Rust implementation. At commit `7e0ca1587e4ef9d58e75ff356ca423c2fc101585`, its packages separate pointer parsing, local storage, Git integration, API, transfer, credentials and filter handling. These are useful whole-module donors. The `git` crate explicitly shells out to installed Git, so its name and Rust implementation do not mean that it already integrates with a native gitr backend. [Workspace manifest](https://github.com/rustutils/git-lfs/blob/7e0ca1587e4ef9d58e75ff356ca423c2fc101585/Cargo.toml), [Git integration implementation](https://github.com/rustutils/git-lfs/blob/7e0ca1587e4ef9d58e75ff356ca423c2fc101585/git/src/lib.rs), [canonical GitLab repository](https://gitlab.com/rustutils/git-lfs).

Its published scoreboard reports 718 of 794 upstream cases across 104 suites on the date in that file. That is useful evidence of a compatibility program, not a current independent pass result or complete Git LFS parity. Reuse the compatibility corpus and rerun it against gitr's chosen implementation. [Upstream scoreboard](https://github.com/rustutils/git-lfs/blob/7e0ca1587e4ef9d58e75ff356ca423c2fc101585/tests/SCOREBOARD.md).

Rudolfs supplies a small MIT-licensed LFS server and composable streaming storage modules. Its storage directory includes disk, S3, cache, retry, encryption and verification wrappers. The inspected verification code checks data digests while streaming. Some corruption handling assumes disposable cached objects; wrapper ordering matters. Its namespace constructor also retains validation work. Gitr should use typed repository scopes and its own authorization before exposing a backend. [Storage trait and modules](https://github.com/jasonwhite/rudolfs/blob/83c7e9e8d0d4308afecc36c5cc9c2ef3fa0c0f05/src/storage/mod.rs), [verification implementation](https://github.com/jasonwhite/rudolfs/blob/83c7e9e8d0d4308afecc36c5cc9c2ef3fa0c0f05/src/storage/verify.rs).

LFSX is a broader MPL-2.0 server donor. Its inspected lock implementation includes owner IDs, per-path coordination, bounded local gates and object-store conditional-write authority. The source also separates canonical storage, staging, codec, encryption, deduplication and verification. The locking behavior is particularly relevant to a forge, but a gitr port must attach repository authorization and the chosen replicated authority. Its codecs/TLS dependencies also need a native dependency review. [LFSX manifest](https://github.com/FerrLabs/LFSX/blob/596518a8e535e0a1169b3cfe74d5e7cb9a253745/Cargo.toml), [lock implementation](https://github.com/FerrLabs/LFSX/blob/596518a8e535e0a1169b3cfe74d5e7cb9a253745/server/src/locks.rs), [storage directory](https://github.com/FerrLabs/LFSX/tree/596518a8e535e0a1169b3cfe74d5e7cb9a253745/server/src/storage).

The MIT-licensed Scutiger contains an experimental pure SSH LFS transfer implementation and is useful for protocol comparison. Its manifest uses git2/libgit2, and the README says locking is absent. Treat it as a protocol/test donor. [Scutiger](https://github.com/bk2204/scutiger/tree/614a4f01cfbe6aafa355ca92ed6d1581b5cfc088).

Fork-inclusive searches found `sercand/rudolfs`, `oursky/rudolfs` and `azataiot/fork-LFSX` with verified parent relationships. This survey did not establish a useful unique patch in those forks. They remain explicit follow-up leads, not automatically preferred upstreams.

### Two Iroh helpers offer different useful slices

Wronex's MIT-licensed `iroh-git` workspace includes a daemon, remote helper and LFS transfer implementation. The inspected daemon supports repository grants tied to a caller's cryptographic NodeId, live-session revocation, and connection/stream/time limits. That is more useful to gitr than a transport-only demo. It still invokes external Git, so import the peer transport and authorization components and connect them to native gitr pack services. [Workspace](https://github.com/wronex/iroh-git/blob/9e2cd220bac1fe4b33af507fef52e63b52653b88/Cargo.toml), [daemon implementation](https://github.com/wronex/iroh-git/blob/9e2cd220bac1fe4b33af507fef52e63b52653b88/crates/iroh-git-daemon/src/lib.rs), [protocol and LFS directory](https://github.com/wronex/iroh-git/tree/9e2cd220bac1fe4b33af507fef52e63b52653b88/crates/iroh-git/src).

Magik6k's Apache-2.0 `git-remote-iroh` is smaller. Its `bridge.rs` implements an important stream rule: EOF in one direction does not prematurely cancel traffic in the other; errors and explicit cancellation terminate the bridge. It also has ticket, protocol, server and helper modules. The server invokes Git upload-pack/receive-pack, and the ticket model is a bearer capability rather than a complete forge authorization system. [Bridge implementation](https://github.com/magik6k/git-remote-iroh/blob/15fb5390cf8dddcad6b8d6dd7b564831a33b92cf/src/bridge.rs), [source directory](https://github.com/magik6k/git-remote-iroh/tree/15fb5390cf8dddcad6b8d6dd7b564831a33b92cf/src).

The independently hosted `notplants.net/git-remote-iroh` on Tangled is a small Rust work in progress that also requires Git subprocesses. A license grant was not established. `cmars/git-remote-iroh` on Codeberg was found through a first-party discussion but could not be fetched. Neither belongs on the immediately reusable shortlist. Similar names do not establish that these are forks of Wronex or Magik6k. [Tangled candidate](https://tangled.org/notplants.net/git-remote-iroh), [Codeberg lead](https://codeberg.org/cmars/git-remote-iroh).

Iroh and iroh-blobs are useful underlying transport and content-transfer libraries, but transfer availability does not establish repository authorization or consensus. Preserve Git's object IDs and Git LFS SHA-256 identities even if an internal blob transport uses a different content hash.

### Object-store remotes and chunked storage

`dekobon/git-remote-object-store` is an Apache-2.0 remote helper with bundle/packchain and LFS components for cloud object storage. Its inspected manifest uses gix 0.83 and related subcrates. The project documents conditional object-store locking. That is a credible remote/backend donor, but it does not establish a replicated gitr database or cross-service transactional authority. External-Git command paths need a further source audit before claiming the entire helper is native. [Pinned manifest](https://github.com/dekobon/git-remote-object-store/blob/4b025d1c99f03d0d5b8e017bc4e16390abb4cd5f/Cargo.toml), [LFS modules](https://github.com/dekobon/git-remote-object-store/tree/4b025d1c99f03d0d5b8e017bc4e16390abb4cd5f/src/lfs).

Hugging Face's current Apache-2.0 `xet-core` contains a larger amount of reusable Rust: content-defined chunking, shard structures, reconstruction, caching and LFS agent protocol. The inspected chunking implementation uses Gearhash with explicit minimum/maximum chunk boundaries. The whole client assumes Hugging Face CAS interactions and also includes git2/native integration. Start with the chunking and format crates if object deduplication becomes a demonstrated need. [Workspace](https://github.com/huggingface/xet-core/blob/5385084c6e15f0cedd2ca8a2e36279413b049e0c/Cargo.toml), [chunking implementation](https://github.com/huggingface/xet-core/blob/5385084c6e15f0cedd2ca8a2e36279413b049e0c/xet_data/src/deduplication/chunking.rs).

Do not conflate that repository with the older BSD-3-Clause `xetdata/xet-core`. `ggoggam/openxet` is an Apache-2.0 self-hosted server lead, but its exact Xet crate pins and warning about unstable APIs make it a qualification project. Shardline, oxlfs, gitolfs3 and the Radicle/IPFS LFS derivatives are recorded for follow-up at lower inspection depth. [Older Xet repository](https://github.com/xetdata/xet-core), [OpenXet](https://github.com/ggoggam/openxet).

## MCP and the Aspen name collision

The official Rust MCP SDK is the preferable first source for shared Rust tool and transport integration. The inspected version's manifest declares Apache-2.0, but its LICENSE retains some older MIT contributions and licenses documentation under CC-BY-4.0. Keep those distinctions when importing files. Transport sessions remain separate from gitr's authorization and durable state. [Pinned manifest](https://github.com/modelcontextprotocol/rust-sdk/blob/f2b0ddf113d1ed426b77a81511eac6e4bffa0ac4/Cargo.toml), [license](https://github.com/modelcontextprotocol/rust-sdk/blob/f2b0ddf113d1ed426b77a81511eac6e4bffa0ac4/LICENSE).

`brittonr/aspen` requires special caution because search metadata describes an older architecture. Its current default branch is `molten`, and the inspected manifest/README describe Molten using Wasmtime, Iroh and redb, with pinned dependencies from SSH GitHub and Radicle URLs. The head is `35478ffb0a6291e2dad583f9bec62fb26b4b10ae` and the project is AGPL-3.0-or-later. It is not a ready, self-contained copy of an older Aspen Raft stack. Gitr currently uses Wasmi, so adopting this entire workspace would change more than storage replication. [Observed manifest](https://github.com/brittonr/aspen/blob/35478ffb0a6291e2dad583f9bec62fb26b4b10ae/Cargo.toml), [README](https://github.com/brittonr/aspen/blob/35478ffb0a6291e2dad583f9bec62fb26b4b10ae/README.md).

The sibling `aspen-plugins` repository contains forge, hook, coordination, SQL and secret-related Wasm plugin components and signing tooling. They depend on the Aspen host ABI. `aspen-wasm-guest-sdk` and `aspen-pijul` are relevant lineage leads, not drop-in Wasmi modules. `aspen-s3` has conflicting licensing: its manifest says MIT OR Apache-2.0 while its root license is AGPL-3.0. That conflict must be resolved at source even if gitr changes license. [Plugins](https://github.com/brittonr/aspen-plugins), [guest SDK](https://github.com/brittonr/aspen-wasm-guest-sdk), [Pijul component](https://github.com/brittonr/aspen-pijul), [S3 manifest](https://github.com/brittonr/aspen-s3/blob/main/Cargo.toml), [S3 license](https://github.com/brittonr/aspen-s3/blob/main/LICENSE).

The separate `reason-machines/aspen` and Enroute work belongs to the forge/CI report. Shared names do not prove shared implementation history.

## Concrete qualification sequence

1. Build a GitQL provider over one authorized, immutable gitr revision. Confirm bounded execution and byte-safe paths with malformed repository fixtures.
2. Implement Syntext's content source against gix object reads. Verify that an index generation is only published after all files are complete and that a failed build leaves the prior generation usable.
3. Extract Basemind's stack-graph crates and a small language set. Retain confidence and coverage metadata. Compare Mesquite against those exact parsers and queries before making it the default.
4. Prototype an OpenRaft/redb 4.5 adapter from Rift Cluster with explicit durable log, vote, apply and snapshot rules. Exercise leader change, crash replay, duplicate commands and linearizable reads before replicating pushes or durable action claims.
5. Port LFS pointer, batch, filter and transfer components from rustutils. Compare Rudolfs and LFSX storage modules against gitr's authorization and locking requirements. Rerun upstream compatibility fixtures.
6. Attach the selected Iroh transport to the same native pack/LFS services. Preserve the transport donors' cancellation, stream bounds and revocation behavior.
7. Add Tantivy or a Rust ANN library only for a measured search requirement, with redb retaining revision manifests and rebuild metadata.

This sequence can reuse substantial upstream code without forcing gitr to inherit an unrelated database, desktop UI, hosted model service or subprocess-based Git backend. License qualification, dependency qualification and behavioral qualification are separate gates; passing one does not imply the others have passed.

## Full candidate catalog

The table below is generated from the machine-readable inventory. `selected-source-files` means specific implementation files were read, not that the complete repository was audited. `metadata-or-search` means the license and capability descriptions still require source confirmation before import.

### Search and semantic code intelligence

| Repository | Decision | License evidence | Inspection |
| --- | --- | --- | --- |
| [DeusData/codebase-memory-mcp](https://github.com/DeusData/codebase-memory-mcp) | behavioral-reference | MIT | manifest-or-readme |
| [BloopAI/bloop](https://github.com/BloopAI/bloop) | selective-donor | Apache-2.0 on selected oss branch; enterprise restriction on main/server/bleep/src/ee | manifest-or-readme |
| [Goldziher/basemind](https://github.com/Goldziher/basemind) | priority-pilot | MIT app; MIT OR Apache-2.0 fork crates and rule files | selected-source-files |
| [gaoxinghui2014-beep/codebase-memory-mcp-rust](https://github.com/gaoxinghui2014-beep/codebase-memory-mcp-rust) | selective-donor | MIT declared | manifest-or-readme |
| [win4r/codebase-memory-mcp-pro](https://github.com/win4r/codebase-memory-mcp-pro) | behavioral-reference | MIT | metadata-or-search |
| [octacity-org/codebase-memory-mcp](https://github.com/octacity-org/codebase-memory-mcp) | behavioral-reference | MIT | metadata-or-search |
| [dajneem23/z-codebase-memory-mcp](https://github.com/dajneem23/z-codebase-memory-mcp) | behavioral-reference | NO_LICENSE_DETECTED | metadata-or-search |
| [dirmacs/deagle](https://github.com/dirmacs/deagle) | selective-donor | MIT manifest declaration; license file not verified | manifest-or-readme |
| [techquestsdev/houndr](https://github.com/techquestsdev/houndr) | unqualified-discovery | GPL-3.0 | metadata-or-search |
| [ast-grep/ast-grep](https://github.com/ast-grep/ast-grep) | component-donor | MIT | manifest-or-readme |
| [tree-sitter/tree-sitter](https://github.com/tree-sitter/tree-sitter) | compatibility-reference | MIT | selected-source-files |
| [github/stack-graphs](https://github.com/github/stack-graphs) | selective-donor | MIT OR Apache-2.0 | manifest-or-readme |
| [HyperAST/HyperAST](https://github.com/HyperAST/HyperAST) | unqualified-discovery | GPL-3.0 | metadata-or-search |
| [rust-lang/rust-analyzer](https://github.com/rust-lang/rust-analyzer) | component-candidate | Apache-2.0 | metadata-or-search |
| [salsa-rs/salsa](https://github.com/salsa-rs/salsa) | component-candidate | Apache-2.0 | metadata-or-search |
| [rust-analyzer/rowan](https://github.com/rust-analyzer/rowan) | component-candidate | Apache-2.0 | metadata-or-search |
| [oxc-project/oxc](https://github.com/oxc-project/oxc) | component-candidate | MIT | metadata-or-search |
| [petgraph/petgraph](https://github.com/petgraph/petgraph) | component-candidate | Apache-2.0 | metadata-or-search |
| [AmrDeveloper/GQL](https://github.com/AmrDeveloper/GQL) | priority-pilot | MIT | selected-source-files |
| [quickwit-oss/tantivy](https://github.com/quickwit-oss/tantivy) | priority-pilot | MIT | selected-source-files |
| [BurntSushi/ripgrep](https://github.com/BurntSushi/ripgrep) | component-candidate | Unlicense | metadata-or-search |
| [BurntSushi/fst](https://github.com/BurntSushi/fst) | component-candidate | Unlicense | metadata-or-search |
| [probelabs/probe](https://github.com/probelabs/probe) | unqualified-discovery | Apache-2.0 | metadata-or-search |
| [mozsearch/mozsearch](https://github.com/mozsearch/mozsearch) | reference-candidate | MPL-2.0 | metadata-or-search |
| [flupkede/codesearch](https://github.com/flupkede/codesearch) | selective-donor | Apache-2.0 | manifest-or-readme |
| [BrosnanYuen/opencodesearch](https://github.com/BrosnanYuen/opencodesearch) | unqualified-discovery | GPL-3.0 | metadata-or-search |
| [whit3rabbit/syntext](https://github.com/whit3rabbit/syntext) | priority-pilot | MIT | selected-source-files |
| [williamfzc/gossiphs](https://github.com/williamfzc/gossiphs) | selective-donor | Apache-2.0 | manifest-or-readme |
| [eas4ai/tsg_indexer](https://github.com/eas4ai/tsg_indexer) | selective-donor | MIT OR Apache-2.0 manifest | manifest-or-readme |
| [lambda-alpha-labs/Graphenium](https://github.com/lambda-alpha-labs/Graphenium) | selective-donor | MIT | manifest-or-readme |
| [TtTRz/graphify-rs](https://github.com/TtTRz/graphify-rs) | unqualified-discovery | NOASSERTION | metadata-or-search |
| [nguyenphutrong/agentlens](https://github.com/nguyenphutrong/agentlens) | reference-candidate | NO_LICENSE_DETECTED | metadata-or-search |
| [mesa-hills-research/mesquite](https://github.com/mesa-hills-research/mesquite) | priority-pilot | MIT runtime and most grammars; Apache-2.0 Astro/Elixir/Erlang/HCL, keep NOTICE | selected-source-files |
| [hydro-project/rust-sitter](https://github.com/hydro-project/rust-sitter) | component-candidate | MIT | metadata-or-search |
| [shadaj/tree-sitter-c2rust](https://github.com/shadaj/tree-sitter-c2rust) | reference-candidate | MIT | metadata-or-search |
| [EffortlessMetrics/adze](https://github.com/EffortlessMetrics/adze) | component-candidate | Apache-2.0 | metadata-or-search |
| [yhm-amber/bloop.ai](https://github.com/yhm-amber/bloop.ai) | fork-discovery | Apache-2.0 | metadata-or-search |
| [rmuller-ml/bloop-fork](https://github.com/rmuller-ml/bloop-fork) | fork-discovery | NOASSERTION | metadata-or-search |
| [nohehf/stack-graphs-python-bindings](https://github.com/nohehf/stack-graphs-python-bindings) | derivative-reference | MIT | metadata-or-search |
| [BloopAI/gitoxide](https://github.com/BloopAI/gitoxide) | fork-reference | Apache-2.0 | metadata-or-search |
| [BloopAI/hyperpolyglot](https://github.com/BloopAI/hyperpolyglot) | fork-reference | Apache-2.0 | metadata-or-search |
| [BloopAI/esaxx-rs](https://github.com/BloopAI/esaxx-rs) | native-boundary-reference | Apache-2.0 | metadata-or-search |
| [BloopAI/comrak](https://github.com/BloopAI/comrak) | fork-reference | NOASSERTION | metadata-or-search |
| [rust-lang/regex](https://github.com/rust-lang/regex) | component-candidate | Apache-2.0 | metadata-or-search |

### Embedded storage, vectors, replication and MCP

| Repository | Decision | License evidence | Inspection |
| --- | --- | --- | --- |
| [cberner/redb](https://github.com/cberner/redb) | retain-selected | MIT OR Apache-2.0 | manifest-or-readme |
| [fjall-rs/fjall](https://github.com/fjall-rs/fjall) | reference-only | Apache-2.0 | metadata-or-search |
| [databendlabs/openraft](https://github.com/databendlabs/openraft) | priority-pilot | MIT OR Apache-2.0 | selected-source-files |
| [tikv/raft-rs](https://github.com/tikv/raft-rs) | alternative-candidate | Apache-2.0 | metadata-or-search |
| [achird-labs/rift-cluster](https://github.com/achird-labs/rift-cluster) | priority-pilot | Apache-2.0 | selected-source-files |
| [hyperiondb/hyperion-vault](https://github.com/hyperiondb/hyperion-vault) | alternative-donor | AGPL-3.0-only | manifest-or-readme |
| [danube-messaging/danube](https://github.com/danube-messaging/danube) | alternative-donor | Apache-2.0 | manifest-or-readme |
| [tvolk131/fleet](https://github.com/tvolk131/fleet) | unqualified-discovery | NO_LICENSE_DETECTED | metadata-or-search |
| [cozodb/cozo](https://github.com/cozodb/cozo) | reference-only | MPL-2.0 | metadata-or-search |
| [surrealdb/surrealkv](https://github.com/surrealdb/surrealkv) | excluded-by-current-project-contract | Apache-2.0 | metadata-or-search |
| [nishu-builder/git-vdb](https://github.com/nishu-builder/git-vdb) | selective-donor | Apache-2.0 | manifest-or-readme |
| [lance-format/lance](https://github.com/lance-format/lance) | reference-candidate | Apache-2.0 | metadata-or-search |
| [lancedb/lancedb](https://github.com/lancedb/lancedb) | reference-candidate | Apache-2.0 | metadata-or-search |
| [qdrant/qdrant](https://github.com/qdrant/qdrant) | reference-candidate | Apache-2.0 | metadata-or-search |
| [chroma-core/chroma](https://github.com/chroma-core/chroma) | reference-candidate | Apache-2.0 | metadata-or-search |
| [chroma-core/hnswlib](https://github.com/chroma-core/hnswlib) | native-boundary-reference | Apache-2.0 | metadata-or-search |
| [rust-cv/hnsw](https://github.com/rust-cv/hnsw) | component-candidate | MIT | manifest-or-readme |
| [jean-pierreBoth/hnswlib-rs](https://github.com/jean-pierreBoth/hnswlib-rs) | priority-pilot | MIT OR Apache-2.0 | manifest-or-readme |
| [unum-cloud/USearch](https://github.com/unum-cloud/USearch) | native-boundary-reference | Apache-2.0 | metadata-or-search |
| [modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk) | priority-pilot | Apache-2.0 with retained MIT contributions; documentation CC-BY-4.0 | manifest-or-readme |
| [huggingface/candle](https://github.com/huggingface/candle) | component-candidate | Apache-2.0 | metadata-or-search |
| [indradb/indradb](https://github.com/indradb/indradb) | reference-only | MPL-2.0 | metadata-or-search |
| [agnesoft/agdb](https://github.com/agnesoft/agdb) | reference-only | Apache-2.0 | metadata-or-search |
| [kacy/ember](https://github.com/kacy/ember) | reference-only | Apache-2.0 manifest; metadata NOASSERTION | manifest-or-readme |
| [apache/opendal](https://github.com/apache/opendal) | component-candidate | Apache-2.0 | metadata-or-search |

### P2P transports and data platform derivatives

| Repository | Decision | License evidence | Inspection |
| --- | --- | --- | --- |
| [n0-computer/iroh](https://github.com/n0-computer/iroh) | component-candidate | Apache-2.0 | metadata-or-search |
| [n0-computer/iroh-blobs](https://github.com/n0-computer/iroh-blobs) | component-candidate | Apache-2.0 | metadata-or-search |
| [magik6k/git-remote-iroh](https://github.com/magik6k/git-remote-iroh) | selective-donor | Apache-2.0 | selected-source-files |
| [wronex/iroh-git](https://github.com/wronex/iroh-git) | priority-pilot | MIT | selected-source-files |
| [brittonr/aspen](https://github.com/brittonr/aspen) | research-candidate | AGPL-3.0-or-later | manifest-or-readme |
| [brittonr/aspen-pijul](https://github.com/brittonr/aspen-pijul) | unqualified-discovery | AGPL-3.0 | metadata-or-search |
| [brittonr/aspen-wasm-guest-sdk](https://github.com/brittonr/aspen-wasm-guest-sdk) | unqualified-discovery | AGPL-3.0 | manifest-or-readme |
| [brittonr/aspen-plugins](https://github.com/brittonr/aspen-plugins) | selective-donor | AGPL-3.0-or-later | manifest-or-readme |
| [brittonr/aspen-s3](https://github.com/brittonr/aspen-s3) | license-unresolved | Conflicting manifest MIT OR Apache-2.0 and root AGPL-3.0 | manifest-or-readme |
| [notplants.net/git-remote-iroh](https://tangled.org/notplants.net/git-remote-iroh) | reference-candidate | Unverified; no license grant established | primary-readme |
| [cmars/git-remote-iroh](https://codeberg.org/cmars/git-remote-iroh) | unverified-lead | Unverified | discovery-only |

### LFS, binary assets and object-store remotes

| Repository | Decision | License evidence | Inspection |
| --- | --- | --- | --- |
| [huggingface/xet-core](https://github.com/huggingface/xet-core) | selective-donor | Apache-2.0 | selected-source-files |
| [xetdata/xet-core](https://github.com/xetdata/xet-core) | historical-donor | BSD-3-Clause | metadata-or-search |
| [rustutils/git-lfs](https://github.com/rustutils/git-lfs) | priority-pilot | MIT | selected-source-files |
| [jasonwhite/rudolfs](https://github.com/jasonwhite/rudolfs) | priority-pilot | MIT | selected-source-files |
| [FerrLabs/LFSX](https://github.com/FerrLabs/LFSX) | priority-pilot | MPL-2.0 | selected-source-files |
| [dekobon/git-remote-object-store](https://github.com/dekobon/git-remote-object-store) | priority-pilot | Apache-2.0 | manifest-or-readme |
| [bk2204/scutiger](https://github.com/bk2204/scutiger) | selective-donor | MIT | manifest-or-readme |
| [STEXS-Technologies/shardline](https://github.com/STEXS-Technologies/shardline) | unqualified-discovery | Apache-2.0 | metadata-or-search |
| [ggoggam/openxet](https://github.com/ggoggam/openxet) | component-candidate | Apache-2.0 | manifest-or-readme |
| [hayatek-jp/oxlfs](https://github.com/hayatek-jp/oxlfs) | alternative-candidate | AGPL-3.0 | metadata-or-search |
| [rutgerbrf/gitolfs3](https://github.com/rutgerbrf/gitolfs3) | alternative-candidate | MIT | metadata-or-search |
| [mab122/radicle-heartwood-lfs-ipfs](https://github.com/mab122/radicle-heartwood-lfs-ipfs) | derivative-candidate | Apache-2.0 | metadata-or-search |
| [mab122/radicle-lfs-ipfs-transfer-agent](https://github.com/mab122/radicle-lfs-ipfs-transfer-agent) | component-candidate | Apache-2.0 | metadata-or-search |
| [sercand/rudolfs](https://github.com/sercand/rudolfs) | fork-discovery | MIT | metadata-or-search |
| [oursky/rudolfs](https://github.com/oursky/rudolfs) | fork-discovery | MIT | metadata-or-search |
| [azataiot/fork-LFSX](https://github.com/azataiot/fork-LFSX) | fork-discovery | MPL-2.0 | metadata-or-search |

