# Initial implementation review

Two independent static reviews compared upstream base
`fb255012acbcbe81189cb8014da0280b09c66ecc` with local implementation checkpoint
`93f5c19`, whose exact tree is preserved in published commit
[`2a0c5ee`](https://github.com/glopyglerky/gitr/commit/2a0c5ee3bbd710f55d512e1168bf85c5bd7006b8).
The contracts were the user requirements recorded in `AGENTS.md`,
the architecture, compatibility matrix and roadmap. Review did not substitute
for the executed qualification.

## Standards

The review found one contract mismatch, one durability concern and one
maintainability observation:

- Database extraction prefiltered LFS version lines using literal LF endings,
  while the shared pointer decoder also accepted CRLF. Supported pointers could
  therefore be reported as indexed source. Extraction now uses that decoder
  directly, with regression coverage for CRLF and legacy Hawser forms.
- A writer observing an identical object installed by another process could
  return after verification but before the winning process synced its directory.
  Newly created ancestor directories also needed durable publication. Both
  immutable object and manifest acknowledgement paths require directory syncing,
  including reused chunks. This is distinct from byte integrity and atomic rename.
- The two caches duplicated directory creation and syncing helpers. Sharing this
  policy reduces the risk of their durability semantics diverging.

No additional critical credential-forwarding, repository-command execution or
ref compare-and-swap defect was found in the inspected implementation. That is a
bounded review result, not proof that those areas contain no defects.

## Spec

The review independently found the same LFS-pointer coverage mismatch and no
material unrequested scope. It identified four substantial, disclosed gaps:

- The native everyday Git workflow still needs `add`, `commit`, `checkout`,
  `clone`, `fetch` and `push` and complete merge/rebase behavior.
- The repository database is a rebuildable local catalog. Database-native
  authoritative refs, distributed replication, fencing and failover remain work.
- Search uses lexical vectors and Git membership. Learned embeddings, resolved
  code relationships and a relational query language remain work.
- Asset deduplication is local. Negotiated chunk transfers, sparse hydration,
  transfer resume and automatic pre-push lock enforcement remain work.

The implementation is an initial working layer for the requested alternative.
The [roadmap](roadmap.md) retains these requirements and their acceptance gates.

Review counts: Standards — one contract mismatch, one durability concern and one
heuristic observation; Spec — one implementation mismatch and four documented
requirement gaps. The [qualification record](qualification.md) records validation
after review corrections.

## Repository smoke check

A subsequent search over this repository exposed an additional quality defect:
`gitk` and `atomic` collide in the first model's 512-bin vector. Short unrelated
files could outrank relevant material for `atomic publication`. The revised
catalog stores exact normalized tokens alongside vectors, filters zero-overlap
candidates and ranks query-token coverage before cosine. A regression fixture
captures this observed collision. This remains lexical search, not learned
semantic retrieval.
