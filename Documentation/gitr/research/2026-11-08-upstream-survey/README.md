# Broad upstream reuse survey for gitr

**Research window: 8–9 November 2026.** This survey covers similarly named Rust Git implementations, established VCS engines, Git servers and forges, workflow runtimes, storage, search, code intelligence, and useful forks and derivatives.

## Decision

**Keep gitr's current Gitoxide, redb and Wasmi foundation, and import substantial upstream components with clear interfaces.** The strongest immediate whole-crate candidates are Rbit's workflow planner, GitQL's parser/engine, FerrisLabs gitr's commit-graph layout, maintained Basemind stack-graph libraries, and selected Git/LFS protocol libraries. Girt and git-zero deserve competing native-protocol experiments. IronForge/Plombir, Enroute, Mega2, Jujutsu and the public Scion implementations provide larger bodies of useful code, but require deliberate adaptation.

No inspected complete product satisfies all of gitr's Git/GitHub compatibility, local/offline operation, redb authority, safe concurrent publication, large-asset streaming, integrated forge, Wasmi execution and code-intelligence requirements. That is a finding about the inspected snapshots, not a claim that no such project could exist. Replacing the application wholesale would also import another product's unsupported operations, storage ownership and deployment assumptions. The tables below identify where wholesale **component** reuse is realistic.

The user's latest instruction authorizes choosing a different license where lawful to obtain the best product. Candidates are ranked by engineering merit, including Apache-, GPL- and AGPL-licensed code. The existing GPL-2.0-only declaration is not treated as a permanent product constraint. The [scoped licensing review](licensing-review.md) explains which source can be relicensed and what provenance evidence is still needed.

## Evidence and navigation

- [Combined filterable catalog](catalog.csv) and [structured catalog](catalog.json): deduplicated candidate identities, separate per-track assessments and source links.
- [Coverage summary](coverage-summary.json): reproducible counts, inspection levels and search scope.
- [Name families and small donors](name-search.md): gitr, rgit, gitrs, rsgit, rs-git, git-rs, git_rs, rust-git, rusty-git, rustgit, girt and related names.
- [Forge, server and CI donors](forge-ci.md): full forge families, protocol servers, collaboration models, workflow planning and Wasm execution.
- [Storage, search and replication donors](storage-search.md): query engines, revision-content indexing, resolved code graphs, parsers, LFS, peer transport and redb/consensus adapters.
- [Established engines and Scion implementations](core-vcs.md): Gitoxide, Jujutsu, Josh, Sapling/Mononoke, workflow tools and independent Scion projects.
- [Licensing assessment](licensing-review.md): ownership, GPLv2/Apache boundaries, permissive and copyleft options.

Each track has a JSON inventory and query ledger beside its Markdown report. Exact inspected commits and file paths appear in those inventories. The existing [Libra/git-internal source review](../../libra-integration.md), [asset upstream review](../../upstreams.md) and [similar-project comparison](../../similar-projects.md) remain relevant; this survey extends them.

### Coverage

The combined catalog contains **476 candidates**: **473 repository URLs and three project/documentation surfaces**. It preserves **485 track assessments**; nine candidates occur in two tracks. The recorded GitHub searches comprise **106 pages across 66 distinct queries**, returning **5,925 result rows and 4,227 distinct repository identities**.

| Track | Candidate records | Search pages | Distinct queries |
| --- | ---: | ---: | ---: |
| Name families | 296 | 42 | 15 |
| Forge and CI | 64 | 28 | 22 |
| Storage and search | 96 | 20 | 20 |
| Core VCS | 29 | 16 | 9 |
| **Combined, deduplicated** | **476** | **106** | **66** |

The strongest recorded evidence for each candidate is: **62 selected-source samples** (including one native-build-script check), **eight fork comparisons**, **62 manifest/license/README/documentation inspections**, and **344 metadata/search/discovery leads**. None of these categories means a comprehensive implementation audit. Counts and exact normalization rules are in [coverage-summary.json](coverage-summary.json).

The name sweep alone returned **1,620 distinct discovery identities** and enriched 296 candidates, including **81 explicitly verified GitHub fork ancestries**. A separate Gitoxide fork-name sweep returned **585 identities** before an empty page. These are search counts, not code-audit counts. Broad searches retain irrelevant results in the ledgers so another researcher can inspect the selection.

The search included both web engines, GitHub repository search with forks and archived projects retained, source trees, package manifests, actual license files, commit comparisons, crates.io/docs.rs, GitLab, Tangled, Pijul Nest and official project documentation. Codeberg and SourceHut searches supplied additional leads, but some pages could not be retrieved. Mixed-language repositories were examined through targeted searches and cross-links.

**Exhaustive across the entire internet is not a verifiable claim.** GitHub's punctuation matching broadened the rs-git and git-rs searches; both reached the 1,000-result ceiling. Several other broad queries stopped at their first 100 results. Direct fork-list access was unavailable, so fork-inclusive searches and parent/source metadata were used. Private, deleted, unindexed and inaccessible repositories, nondefault branches, and mixed-language projects remain possible gaps. The ledgers record these limits.

## Fit to gitr as it exists

The initial repository review was pinned to [69a16ea](https://github.com/glopyglerky/gitr/tree/69a16ea1506d8efa20dff0e559b5716dc95743bd). Before publication, the survey rechecked [3d7c777](https://github.com/glopyglerky/gitr/tree/3d7c7775ce7d3ee9f06cd0eea9bdb950aafec8df), which includes concurrent implementation work. Its manifest still pins **gix 0.90.0, redb 4.5.0 and Wasmi 2.0.0**. The optional local authenticated action API is activated, and the native object-authoring module exposes mktree, write-tree and commit-tree. These write immutable objects; ref publication remains a separate guarded step. Sources: [manifest](https://github.com/glopyglerky/gitr/blob/3d7c7775ce7d3ee9f06cd0eea9bdb950aafec8df/rust/gitr/Cargo.toml), [action API](https://github.com/glopyglerky/gitr/blob/3d7c7775ce7d3ee9f06cd0eea9bdb950aafec8df/Documentation/gitr/action-api.md), [native authoring](https://github.com/glopyglerky/gitr/blob/3d7c7775ce7d3ee9f06cd0eea9bdb950aafec8df/rust/gitr/src/git_write.rs).

Full native clone/fetch/push, everyday add/commit/checkout, complete merge/rebase, learned code embeddings, resolved code intelligence, multi-host consensus and a full forge still have explicit qualification gaps. The local action API is useful existing functionality; it is not yet the whole multiuser forge or MCP product. Sources: [current scope](https://github.com/glopyglerky/gitr/blob/3d7c7775ce7d3ee9f06cd0eea9bdb950aafec8df/README.md), [development contract](https://github.com/glopyglerky/gitr/blob/3d7c7775ce7d3ee9f06cd0eea9bdb950aafec8df/AGENTS.md).

Preserve canonical Git object identities, expected-old-value ref updates, durable fenced jobs, atomic artifact/receipt publication and existing redb data. Search indexes are derived state. A protocol library must not become a second, competing authority for refs or jobs.

## Highest-value whole-component imports

These are evaluation targets, not claims that their complete dependency graphs are already qualified together.

| Order | Donor and license scope | Code to reuse substantially or wholesale | Fit and required adaptation |
| --- | --- | --- | --- |
| 1 | **GitRbit/rbit — MIT OR Apache-2.0** | [crates/rbit-workflow](https://github.com/GitRbit/rbit/blob/39f932a00b7aa35b324e06a9ff2343a57d4bed76/crates/rbit-workflow/src/lib.rs), plus conformance fixtures | Independent planner with no I/O, async or other Rbit crates. Bounded YAML, expressions, triggers, filters, cron, matrix planning, job specifications and runner protocol. Translate plans into gitr redb jobs and Wasmi capabilities. |
| 2 | **AmrDeveloper/GQL — MIT** | Parser, engine, AST/core and standard-function crates; [DataProvider](https://github.com/AmrDeveloper/GQL/blob/3a76cfee02a00ee6ce20eeac6447573b34f25d86/crates/gitql-engine/src/data_provider.rs) | Query gitr's authorized revision snapshots. Its provider materializes rows: impose memory/result/time limits and preserve byte paths. Keep gitr storage instead of the complete CLI adapter. |
| 3 | **ferrislabs/gitr — Apache-2.0** | [crates/graph](https://github.com/ferrislabs/gitr/blob/40f634624b639c27e6bdf99bc2020e1a8d6bafe5/crates/graph/src/lib.rs), necessary domain types and layout tests | Pure commit-lane layout, separated from GUI/I/O/clock. Preserve topological ordering and qualify large-history behavior. Settle compatible license scope before import. |
| 4 | **Goldziher/basemind — app MIT; graph forks MIT OR Apache-2.0** | Maintained stack-graphs, tree-sitter-graph, tree-sitter-stack-graphs and language rules; [documented changes](https://github.com/Goldziher/basemind/blob/88930cc70b09de11b6925cb7d8118d1adcdfe5b7/ATTRIBUTIONS.md) | Meaningful derivative: panic handling and removal of C FFI/SQLite core dependencies. Adapt persistence to redb; retain explicit language coverage and failed/partial-resolution states. |
| 5 | **whit3rabbit/syntext — MIT** | Trigram/regex index and [ContentSource](https://github.com/whit3rabbit/syntext/blob/d0fdd127e781142933d8747821de8ab4fc74c90c/src/index/source.rs) | Feed immutable Git blobs without checkout or a Git process. Publish revision-keyed index generations. Evaluate lexical modules without optional Tree-sitter/SQLite symbol support. |
| 6 | **meta/sapling DAG library — separately MIT** | [eden/scm/lib/dag](https://github.com/meta/sapling/blob/f7a6003b0bbc083ddef7a40f34f0b13f3f420fcb/eden/scm/lib/dag/src/lib.rs), after following local dependencies | DAG sets, ancestry, segments and dense internal IDs. A whole-library evaluation is reasonable. Keep Git OIDs canonical; this survey reproduced no performance claims. |
| 7 | **no-std-rs/git-zero — MIT OR Apache-2.0** | Core/client/server modules; [repository interface](https://github.com/no-std-rs/git-zero/blob/9efd2f70da9247411feb6bd79d4b291cd618c6b6/crates/git-zero-core/src/repository.rs), [receive boundary](https://github.com/no-std-rs/git-zero/blob/9efd2f70da9247411feb6bd79d4b291cd618c6b6/crates/git-zero-server/src/receive/mod.rs) | Safe no_std core with host-owned I/O/persistence. Host must authorize, quarantine, validate, publish and acknowledge. Selected-fetch storage scales with total object count. |
| 8 | **josh-project/josh — MIT** | [josh-gix-ext](https://github.com/josh-project/josh/blob/fbc53a53c147c5c454f0e983fe26d728b5f84bb4/josh-gix-ext/src/lib.rs), filter/projection components | Current core uses gix. Useful object/tree helpers and sparse views. Align gix versions; in-memory StagingOdb is not a streaming asset store or durable ref authority. |
| 9 | **rustutils/git-lfs / Rudolfs / LFSX — MIT / MIT / MPL-2.0** | Client pointer/filter/API/transfer crates; streaming object storage; server locks and backend modules | Extend standard LFS. Replace Git subprocess adapters and preserve credential isolation, hashes, atomic publication and bounded memory. [Source assessment](storage-search.md). |

## Larger codebases worth adopting in parts

| Family | Most useful source | Decision |
| --- | --- | --- |
| **joshka/girt — MIT OR Apache-2.0** | Async [ObjectRead/ObjectWrite](https://github.com/joshka/girt/blob/5558e436cd050a33034f55e88c5d923aa0046fd2/src/storage.rs), pack/protocol and serving modules | Strong native candidate beside gix. Object storage is pluggable; refs are not. Files-ref transactions can partially publish and do not promise fsync/crash survival. Keep gitr ref authority. |
| **IronForge / Plombir Git — MIT / AGPL-3.0-or-later** | rg-git, forge routes/models, MCP, CI and runners | Closest compact full-forge family. Plombir, formerly ForgeKeep, explicitly derives from IronForge. IronForge's [receive path](https://github.com/lengyuqu/ironforge/blob/56893d83fc6d39551623ace292cad725c8b6fdd5/crates/rg-git/src/protocol/receive_pack.rs) publishes sequentially with PreviousValue::Any; replace that with expected-old-value CAS. |
| **Enroute, opendata git-server, reason-machines/aspen** | Ledger/compute contracts, immutable snapshots, pack quarantine/promotion, native protocol and interop fixtures | Strong server donors with different persistence/deployment choices. This Apache Aspen is distinct from brittonr's current Molten/AGPL stack. [Detailed boundaries](forge-ci.md). |
| **Libra / git-internal — MIT** | Push planning, strict remote-status validation, thin-pack delta encoding and interop fixtures | Useful native-push work. Preserve gitr's standard repository boundary, byte paths, guarded refs and bounded transfers. [Existing source review](../../libra-integration.md). |
| **Jujutsu — Apache-2.0; git-branchless — MIT OR Apache-2.0** | Operation store, repository-view reconciliation, undo/recovery, restack/revset algorithms and tests | Strong workflow material once license scope is settled. JJ still has Git subprocess integration; branchless has git2/SQLite coupling. Preserve ordinary Git semantics. [Source review](core-vcs.md). |
| **OpenRaft + achird-labs/rift-cluster — MIT OR Apache / Apache** | Actual redb log/vote/state-machine/snapshot adapter | Best inspected HA adapter starting point. Snapshot payloads use sidecars; dedup expires after 24 hours; relaxed state-machine persistence relies on the durable log. Port and qualify these choices. [Source](https://github.com/achird-labs/rift-cluster/blob/3e5dccfe5e69dccd508e1efa043e831d1e92fe80/crates/rift-cluster/src/raft/store.rs). |
| **Mesquite — runtime/most grammars MIT; four grammar lineages Apache** | Safe Rust parser runtime and 58 translated grammar packages | Promising pilot. Targets Tree-sitter 0.25.10 while current Basemind uses 0.27; cannot mix C grammars and lacks some APIs. [Compatibility differences](https://github.com/mesa-hills-research/mesquite/blob/440d3a8707da03bc4adeff0c1a0e12713890c6ed/docs/api-differences.md). |
| **Mega2; Radicle; GitArena issues** | Object/artifact/OCI/LFS services; signed collaboration objects; git-bug issue operations | Substantial domains with different SQL, native-library and transaction boundaries. Import focused models/services. [Forge review](forge-ci.md). |
| **bnsd55/openscion; CarterT27/scion** | Typed metadata/blob contracts, backend identity, durability refusal and reference-model tests | Independent implementations, not Anthropic releases. One offers MIT OR Apache despite GitHub's Apache-only label. Keep redb/local Git capabilities. [Source review](core-vcs.md). |

## Forks and derivatives that actually matter

The survey distinguishes GitHub fork ancestry, documented detached derivatives, repository renames and projects that merely share a name.

| Fork/derivative | Verified difference at inspected snapshots | Reuse consequence |
| --- | --- | --- |
| **zed-industries/enroute** | One ahead/one behind Enroute. [Patch d29138a](https://github.com/zed-industries/enroute/commit/d29138ad3a8806effc7bbe72ddf6a1ad214401cc) adds wait-for-done and a push.negotiate regression. | Extract the patch/test rather than the older fork. |
| **mayhemheroes/git-rs** | Three ahead/zero behind chrisdickinson/git-rs; adds a libFuzzer target and Mayhem integration. [Fuzz manifest](https://github.com/mayhemheroes/git-rs/blob/12a603036f29d5a73f0fab7f7da60dcdbf74c44b/fuzz/Cargo.toml). | Reuse the fuzz target independently of the hosted CI product. |
| **Basemind stack-graph derivatives** | Documented ancestry, panic fixes, dependency removal and rule updates. | A meaningful maintained derivative. |
| **Plombir Git** | IronForge derivative retaining upstream MIT notices; additions AGPL. | Compare actual fork modules. Ancestor licensing does not relicense later changes. |
| **joshka/jj experimental branches** | Girt backend branch observed at 591187f9bff9545ea39fb2382ff345c9f13f4aea. | Integration experiment and author-reported evidence, not a verified mainline or survey test result. |
| **BloopAI/gitoxide** | One unique commit, 5,687 behind. Materializes license/source symlinks. | Packaging reference; no demonstrated new Git algorithm. |
| **withgraphite/gitoxide; BurntSushi/gitoxide** | Zero unique commits and behind upstream at compared heads. | No demonstrated default-branch donor advantage. |
| **arbruijn/gitr; pombredanne/gitrs** | Same default heads as cNameHitch/gitr and haltode/gitrs. | Do not count as new implementations. |
| **Walgit → Gitcask / Walgit-D1** | Explicit detached lineage with different storage/collaboration choices. | Useful publication/event models; review language mix and license additions. |

Exact comparisons and caveats are in the track reports. A fork behind upstream may still have useful branches; the survey only claims the differences inspected.

## License decision

**Use the best lawful donor, then choose the license for the actual composition.** For new independently authored reusable Rust components, MIT OR Apache-2.0 is a practical default. Keep GPL/AGPL candidates in contention where their benefit justifies the corresponding obligations.

The current Rust manifest declares GPL-2.0-only, and inherited Git is GPLv2-only unless individual notices say otherwise. Visible commit authorship does not prove every source fragment and fixture can be relicensed. Establish file provenance and the resolved build/release composition before changing existing declarations. A Rust translation of protected Git source is not made independent merely by changing language. See the [evidence and proposed audit](licensing-review.md).

Apache-2.0 is compatible with GPLv3 combinations but not GPLv2-only combinations, according to the [Apache Software Foundation](https://www.apache.org/licenses/GPL-compatibility.html). A GPLv3 option helps only where all included source permits it; it does not upgrade inherited Git. Directory/process boundaries alone do not establish independent programs.

Restricted and unresolved candidates remain in the inventory so exclusions are explicit: current GitButler uses FSL-1.1-MIT with version-specific conversion dates; Oxigit uses BUSL with a Git-hosting restriction; Frankengit carries a custom rider; some small projects lack a verified grant. Changing gitr's license cannot erase those terms. Pinned evidence is in the [forge report](forge-ci.md) and inventories. No license declaration was changed by this research commit.

## Concrete adoption sequence

1. **Import a contained win first.** Evaluate rbit-workflow with notices and conformance fixtures. Compile plans into redb jobs. Require bounded parsing/matrix expansion and demonstrate restart, duplicate submission, cancellation and stale-worker refusal using the existing runtime. Specify supported Actions constructs and host capabilities.
2. **Extend native Git transport.** Compare gix plus Libra's push planning against girt and git-zero adapters. Use Git's test corpus, strengthened cNameHitch fixtures, Mayhem fuzzing and Zed's negotiation regression. Require exact SHA-1/SHA-256 objects, byte paths, valid pack closure, malformed-input budgets, stale-ref rejection and crash-safe acknowledgment.
3. **Add revision-based query/search.** Integrate GitQL over authorized snapshot rows and Syntext over immutable blobs. Keep indexes rebuildable and keyed by repository, commit, schema/model version and generation. Test stale-index exclusion, deletion, byte paths and cross-repository authorization.
4. **Add resolved code intelligence.** Use maintained Basemind crates/rules and a Mesquite pilot. Measure resolution against a labeled language corpus. Persist unsupported/failed/partial analysis explicitly; lexical similarity alone does not satisfy Codebase Memory parity.
5. **Build forge services over one authority.** Reuse IronForge/Plombir route/domain code, GitArena/Radicle models and focused artifact APIs. Replace donor persistence/ref paths with gitr services. Qualify authorization, multi-ref updates, protection rules, event/job atomicity and bounded uploads.
6. **Add HA through a dedicated prototype.** Adapt OpenRaft/Rift to selected redb. Exercise partitions, leader loss, stale workers, replay, membership changes, interrupted snapshots and restored idempotency. A local lock or operation-head union is not distributed consensus.

These are recommendations, not additional code changes made during this survey. Prefer maintained crate dependencies where interfaces fit; vendor modules when narrow patches are necessary, recording upstream commit, path, license, local changes and a refresh/test procedure.

## Validation and limits

This source research package records selected source reads, metadata, manifests, licenses, tree paths and commit comparisons. It does not claim every function or candidate was audited. Inspection levels remain visible; finding a path is distinguished from reading its contents.

No donor code was executed, compiled, benchmarked or imported. Proposed tests are future adoption gates. This commit changes documentation/inventories, preserves concurrent implementation work, and records license flexibility without modifying COPYING, Cargo license fields, dependency pins or runtime behavior.

