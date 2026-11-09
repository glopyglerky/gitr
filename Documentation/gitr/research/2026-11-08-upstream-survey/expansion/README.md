# Expanded review: import complete components, retain one Gitr authority

Reviewed 2026-11-08 America/Denver against the [476-candidate baseline](../README.md)
at `6d447b40d511cc87e8580a092278b184454779b3`. The owner permits complete source
copies, substantial module imports and maintained application forks when the
applicable licenses and source provenance allow them. Do not reduce reuse to
small snippets merely because the current package says GPL-2.0-only. Conversely,
changing that declaration does not relicense inherited Git or donor code.

The [12 pinned follow-up reviews](reviews.json) deepen the original priorities
and add adjacent capabilities. They are **not twelve automatically new unique
repositories**: the catalog tool compares canonical identities with the baseline,
preserves every original assessment field, and reports new versus reassessed
entries separately. Six reviews sample selected implementation/provider code;
six inspect manifests, documentation or export surfaces. None is a whole-repository
audit. No donor was compiled, benchmarked or integrated in this review. The catalog
validator's tests are not donor qualification.

## Reproduce the complete expanded list

From the Gitr root:

```sh
python3 ci/gitr-upstream-review.py --self-test
python3 ci/gitr-upstream-review.py --output /tmp/gitr-expanded-survey
```

This produces `catalog.csv`, `catalog.json` and `coverage.json`. CSV retains all
baseline columns and adds `review_*` evidence; JSON retains complete baseline
rows and detailed follow-up records. The original catalog files are immutable
inputs, verified against their Git blob hash. The [read-only workflow](../../../../../.github/workflows/gitr-upstream-review.yml)
retains all three files in the `gitr-expanded-upstream-survey` artifact. Its logs
report deduplicated counts. Subcrates are components, not extra repositories.

## Revised adoption order

| Track | Import-sized choice | What remains Gitr-owned |
|---|---|---|
| Workflow planning and safety | **Entire rbit-workflow and tests**, plus Zizmor expression/model libraries and security fixtures | Immutable workflow identity, authorization, secrets, redb claims, fencing and publication. Parsing is not permission to run arbitrary actions. |
| Native Git and recovery | Gitoxide plus reviewed git-zero/girt/Libra modules and Enroute regressions; **diffy and repository test/fuzz corpus** where it fills a measured gap | Quarantine, connectivity validation, expected-old ref transactions, durable acknowledgment and operation journals. |
| Queries and code understanding | GitQL and Syntext first; maintained Basemind graph derivatives; **Salsa** incremental analysis; **Oxigraph query crates** as an alternative graph-query pilot | Authorized immutable snapshots, raw paths, exact spans, and the distinction between inferred and resolved edges. No database replacement. |
| Semantic retrieval at scale | **Microsoft's current Rust DiskANN/provider family**; compare its older INFINI derivative | Versioned derived indexes, canonical Git IDs, repository filtering and exact fallback. ANN adjacency is not code/ref authority. |
| Large/game assets | Existing LFS/FastCDC; substantial **Xet data/client family** or **iroh-blobs** pilot; retain LFSX/Rudolfs/rustutils candidates | Standard GitHub LFS interoperability, authorization, streaming limits, verification, cache retention and SSD-write budgets. Benchmark alternatives before combining them. |
| Forge product | Whole **IronForge** or **Plombir** forks remain legitimate options, not just snippet sources | Replace incompatible ref/job authority and storage assumptions; preserve each lineage's license. A large UI/auth/review-model import is compatible with retaining Gitr's authority interfaces. |
| Distributed authority | OpenRaft plus reviewed Rift redb implementation and fault tests | Quorum durability, stale-leader fencing, idempotency retention, restore and object GC. Keep standalone operation useful; local redb coordination is not HA. |

Native Git completion remains parallel to forge/CI development. A larger catalog
must not defer add/commit/checkout or clone/fetch/push indefinitely. The original
[core](../core-vcs.md), [forge](../forge-ci.md) and [storage/search](../storage-search.md)
reports retain their evidence and exact donor pins.

## Findings that change implementation decisions

### Rbit: whole-crate reuse, not a new planner from scratch

The pinned [limits implementation](https://github.com/GitRbit/rbit/blob/39f932a00b7aa35b324e06a9ff2343a57d4bed76/crates/rbit-workflow/src/limits.rs)
already bounds YAML bytes/depth/nodes/aliases, source jobs, matrix combinations
and expanded run jobs. However, `Deadline::after` stores `Instant::checked_add`
directly in an Option: a failed addition becomes the same representation as no
deadline. This is a source-level finding, not a reproduced exploit. Reject an
unrepresentable budget and prohibit absent deadlines at untrusted planning entry
points; retain donor tests and add the boundary regression.

The [Secret type](https://github.com/GitRbit/rbit/blob/39f932a00b7aa35b324e06a9ff2343a57d4bed76/crates/rbit-workflow/src/secret.rs)
zeroizes its owned string and redacts Debug, but Serialize intentionally emits
the value for the runner wire protocol. Do not store/log whole JobSpec messages
expecting redaction. Persist secret handles and scoped capability grants, then
resolve short-lived credentials at the worker boundary. Import coherent protocol
and planner types without importing a second scheduler or trusting caller receipts.

### Zizmor: maintained monorepo crates plus security fixtures

The current [workspace](https://github.com/zizmorcore/zizmor/blob/16c4859553a1815178301b33b274009ce2ef0ed8/Cargo.toml)
contains github-actions-expressions, github-actions-models, subfeature and YAML
utilities under MIT. Older standalone model/YAML repositories are archived; see
the [organization listing](https://github.com/zizmorcore). The sampled
[expression library](https://github.com/zizmorcore/zizmor/blob/16c4859553a1815178301b33b274009ce2ef0ed8/crates/github-actions-expressions/src/lib.rs)
retains spans, distinguishes contexts from values flowing into expansion, and
exposes a depth-limit error.

Reuse whole relevant crates and their regression corpus. Keep Rbit's execution
semantics unless a shared parity corpus justifies consolidating parsers. Add tests
for template injection, fork-event secrets, token permissions, unpinned actions,
credential persistence and cache trust. Offline Gitr must not silently depend on
remote lookup rules. Static analysis is neither a sandbox nor authorization proof.
The full CLI has a larger dependency closure, including Tree-sitter tooling, than
the small expression library.

### DiskANN: compare today's upstream, not historical descriptions

Microsoft's pinned [provider interface](https://github.com/microsoft/DiskANN/blob/eaeb12578547a84c160909ec639d1a8564cc7216/diskann/src/provider.rs)
defines external/internal ID mapping, operation context, failure guards, soft
deletion and eventual release. This supports reuse of the Rust ANN implementation
with a Gitr redb-derived provider rather than another database service. Publish
coherent index generations, retain model identity, prevent recycled-ID results,
and measure filtered recall and deletion races.

The INFINI derivative's [README](https://github.com/infinilabs/diskann/blob/d94f863f35e075a0d888535282bb7ed1d056b96a/README.md)
identifies ancestry from Microsoft's former Rust directory and NSG and claims
complete disk queries. Its [workspace](https://github.com/infinilabs/diskann/blob/d94f863f35e075a0d888535282bb7ed1d056b96a/Cargo.toml)
comments out a disk-index driver. Neither fact establishes tested completeness or
incompleteness. Evaluate today's Microsoft API first; carry a fork delta only
when its unique benefit and provenance are demonstrated. This is a pinned
architecture comparison, not a verified ahead/behind Git-history comparison.

### Oxigraph and Salsa: query/recomputation code without changing databases

The sampled [QueryableDataset](https://github.com/oxigraph/oxigraph/blob/6fd4584114ddc4e8168af68df2b6ed530ab9bb12/lib/spareval/src/dataset.rs)
and [spareval manifest](https://github.com/oxigraph/oxigraph/blob/6fd4584114ddc4e8168af68df2b6ed530ab9bb12/lib/spareval/Cargo.toml)
provide a real custom-storage boundary. Evaluate the entire parser/optimizer/
evaluator/term family over scoped redb indexes, not the Oxigraph store. This is an
optional graph-query pilot next to GitQL, not an instruction to ship two languages
immediately. Default graph enumeration/union deduplication allocate collections;
output LIMIT alone cannot bound intermediate work. Restrict enumeration, deny
ambient network SERVICE access, enforce memory/cancellation budgets, and keep raw
Git filename encoding reversible.

[Salsa's manifest](https://github.com/salsa-rs/salsa/blob/6d59529e8c299f18614d59e03f6ffb54e8c8dda6/Cargo.toml)
and [query model](https://github.com/salsa-rs/salsa/blob/6d59529e8c299f18614d59e03f6ffb54e8c8dda6/README.md)
justify a whole-library pilot for avoiding repeated code analysis. Its optional
persistence is not distributed consensus or a stable Gitr authority format. Keep
commit/blob/parser/model/config keys explicit; memoization remains regenerable.
Test rename/deletion/branch-switch invalidation before claiming freshness or parity.

### Moon: impact selection needs immutable-input adaptation

The [affected package](https://github.com/moonrepo/moon/blob/6ef65c505823486fcbcea6491de3f7313e3c5190/crates/affected/Cargo.toml)
has multiple workspace dependencies. Its sampled [task tracker](https://github.com/moonrepo/moon/blob/6ef65c505823486fcbcea6491de3f7313e3c5190/crates/affected/src/task_tracker.rs)
reads a global environment bag and live files for content-matched inputs. A
nonexistent file follows a false content-match branch. For Gitr, supply committed
before/after inputs and captured environment descriptors, and test deletion and
rename propagation explicitly. That is an adaptation requirement, not a claim
that Moon's intended workspace behavior is universally incorrect. Reuse a coherent
package family and tests; its process/Extism/toolchain execution is not automatically
Wasmi-compatible or safe inside a database transaction.

### Xet and Iroh: distinct asset-transfer candidates, distinct caveats

The current Xet [workspace](https://github.com/huggingface/xet-core/blob/5385084c6e15f0cedd2ca8a2e36279413b049e0c/Cargo.toml)
and [data package](https://github.com/huggingface/xet-core/blob/5385084c6e15f0cedd2ca8a2e36279413b049e0c/xet_data/Cargo.toml)
use consolidated data/client/core-structures/runtime packages. The workspace
includes git2; the [data exports](https://github.com/huggingface/xet-core/blob/5385084c6e15f0cedd2ca8a2e36279413b049e0c/xet_data/src/lib.rs)
include telemetry. Audit this package family's dependency closure, endpoint
assumptions and telemetry before reuse. A whole git-xet frontend does not establish
native Rust Git; a CAS client is not Gitr's self-hosted CAS server. Preserve LFS
SHA-256 and ordinary GitHub transfers. Benchmark compressed game assets as well
as insertion-friendly content; chunking is not an automatic speed or SSD-wear win.
The inspected Xet surfaces here are manifests/exports, not its algorithm internals.

Iroh-blobs [0.104.1's manifest](https://github.com/n0-computer/iroh-blobs/blob/15872075f567ef07a31b007f8e439acd262e6697/Cargo.toml)
uses Iroh 1.0 and optional redb 4.1.0, making it relevant to the selected storage
substrate. Its [README](https://github.com/n0-computer/iroh-blobs/blob/15872075f567ef07a31b007f8e439acd262e6697/README.md)
still calls this version non-production-quality and points production users to
0.35. Do not suppress that warning or downgrade without checking compatible API
families. BLAKE3 verified ranges are transport identities, not replacements for
Git or LFS IDs. Hash knowledge must not bypass repository permissions. Qualify
resume, corruption rejection, tickets, GC and retained bytes together.

### diffy: acquire its repository test corpus as well as the crate

The [0.5.2 manifest](https://github.com/bmwill/diffy/blob/eba5e533f3c385a9740bb446c7e0995ae50ca062/Cargo.toml)
excludes tests/fuzz from package distribution and gates compatibility/replay tests
behind binary. Its sampled [merge implementation](https://github.com/bmwill/diffy/blob/eba5e533f3c385a9740bb446c7e0995ae50ca062/src/merge/mod.rs)
distinguishes conflict styles and incomplete-line behavior. Import the complete
library and applicable upstream tests/fuzz files, then qualify default and binary
profiles through Lustr. Text merge does not implement mode/rename conflicts,
index stages, operation journals or semantic correctness. Structural-merge/diff
and JJ/branchless donors in the baseline remain eligible; resolve their exact
license and native/parser dependencies rather than blanket-excluding copyleft.

## Complete-copy and whole-fork license policy

The [baseline license review](../licensing-review.md) remains the starting point.
Complete copies and forks preserve original copyright, license texts, notices,
source-offer obligations and applicable modification requirements. Repository
badges or package declarations alone do not establish a transitive distribution
closure. Evidence and the remaining audit are recorded for each reviewed scope.

Practical choices, subject to actual provenance and distribution review:

1. Existing GPLv2-only composition: prefer eligible MIT/BSD/ISC code or the MIT
   side of a genuine dual license. Do not silently combine Apache-2.0-only or
   GPLv3-only implementation code into that build.
2. Independent Gitr-controlled Rust components: choose a deliberate permissive
   or compatible copyleft scope where ownership permits. A separate directory,
   single commit author or separate reference Git executable does not prove
   that every copied source/test fragment is independently relicensable.
3. Larger compatible GPLv3/AGPL compositions: legitimate options where substantial
   donor value justifies them and every incorporated component allows the result.
   Inherited GPLv2-only source is not automatically upgraded; a process boundary
   is not an automatic legal exemption.

See the [Apache Software Foundation](https://www.apache.org/licenses/GPL-compatibility),
[FSF GPL-version FAQ](https://www.gnu.org/licenses/gpl-faq.en.html#v2v3Compatibility)
and [FSF compatibility guidance](https://www.gnu.org/licenses/license-compatibility.en.html).
No declaration is changed here. Baseline GitButler/Oxigit/Frankengit restrictions
do not disappear because Gitr's own license is flexible.

For each substantial import record repository/commit, imported paths and hashes,
original/selected license alternatives, copyright/NOTICE files, local patch series,
required package closure, exact target/features, native/subprocess boundaries,
fixture terms, executed tests and the upstream-update method. Preserve whole
modules/tests when more maintainable than fragment extraction. Keep Gitr adapters
narrow and authority explicit; do not independently own the same refs/jobs twice.

Use the complete **cargo-about** and **cargo-deny** tools for release assurance,
not a new embedded licensing subsystem. cargo-about's [configuration](https://github.com/EmbarkStudios/cargo-about/blob/8dd595a270e21c52badb21adcf0f95dce3f0c808/docs/src/cli/generate/config.md)
can ignore private/transitive crates and override harvesting with clarifications;
those require reviewed exceptions, not switches to hide failures. cargo-deny's
[checks](https://github.com/EmbarkStudios/cargo-deny/blob/2ab89dcdbedda04f2b713820155fbddee16cb821/README.md)
cover licenses, sources, bans and advisories, not copyright ownership or legal
advice. Audit actual feature/target release graphs, vendored source/assets and
native dependencies; intentionally select permissible license alternatives.

## Import acceptance, not another endless research stage

Pin a coherent package family, retain donor tests, and compile through the reviewed
Lustr route. Compare its intended operations with Gitr/reference Git on normal
and hostile fixtures. Test authorization, stale writes, interrupted publication,
restart/restore and resource budgets. Keep parser output, vectors, impact heuristics
and action receipts outside ref authority until trusted policy binds them to exact
commits. Measure cold startup, package size, RSS, recall, transfer bytes and SSD
writes before claiming lighter/faster/better behavior. This review adds reusable
components and qualification criteria; it does not claim those integrations or the
full Git/forge/MCP/semantic-search/HA product are already implemented.
