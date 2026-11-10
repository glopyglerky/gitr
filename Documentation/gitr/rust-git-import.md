# Historical Grit source import and redb integration

Gitr now contains the historical GitButler Grit Rust CLI, engine and Rust test
support as a coherent source import. The product calls that code in process.
This is a substantial source and implementation expansion; it is **not evidence
of 100% Git parity or a general performance improvement**.

## Why this donor

The selected revision is
[`gitbutlerapp/grit@e197103087b1110db2d629c4c269ef9258508169`](https://github.com/gitbutlerapp/grit/tree/e197103087b1110db2d629c4c269ef9258508169).
It is the last coherent revision before upstream
[`95f8ed02be11bea8d6b3849d728ce30c54263114`](https://github.com/gitbutlerapp/grit/commit/95f8ed02be11bea8d6b3849d728ce30c54263114)
removed the compatibility CLI and its Git shell-suite infrastructure. Importing
current upstream would not supply that CLI.

The historical family contains more than 140 command handlers, object/pack/index
code, revisions, diffs, merging, transport, refs/reftable and Rust tests. Its
historical 41,715/42,001 test headline is an upstream result, not a Gitr result,
and was below 100%. We also found test-specific behavior and correctness defects
that required local changes. A README headline cannot replace source review and
execution against the selected compiler and integration.

| Candidate | Evidence and decision |
| --- | --- |
| Historical GitButler Grit | Broadest coherent Rust Git CLI found; copied the complete Rust package family and retained tests/notices. |
| Current GitButler Grit | Useful ongoing library/storage work, but removed the Git-compatible CLI. Do not mix its changed APIs into this historical family without migration tests. |
| Gitoxide (`gix`) | Already supplies the isolated Gitr component profile. It provides strong Git libraries but does not supply a complete replacement CLI. |
| `cNameHitch/gitr` | Its reviewed source includes ignored parity tests and system-Git subprocesses in transport/submodule paths. It is not a proven complete native donor. |
| Libra | Useful Rust Git implementation and service components, but its database/native-dependency choices require more adaptation to the selected redb architecture. No complete-parity evidence justified importing a second competing engine. |

The earlier [upstream survey](research/2026-11-08-upstream-survey/README.md) and
[expanded source assessment](upstream-reuse-expanded.md) retain the broader
candidate evidence. This change imports an implementation instead of adding
another source-only catalog.

A source recheck on 2026-11-09 retained this choice. Gitoxide's
[current capability list](https://github.com/GitoxideLabs/gitoxide/blob/e2743fe28ef61f06585f68ec65e0e9033931e729/crate-status.md)
still lists push and history workflow orchestration as unfinished.
`cNameHitch/gitr` retains
[ignored known-gap parity tests](https://github.com/cNameHitch/gitr/blob/e7a45704365bcbd4f9f6a5d83c4df07efcd5c59d/crates/git-cli/tests/parity_flags_tier1_gaps.rs),
[installed-Git local transports](https://github.com/cNameHitch/gitr/blob/e7a45704365bcbd4f9f6a5d83c4df07efcd5c59d/crates/git-transport/src/local.rs)
and installed-Git submodule commands. Current
[Jujutsu remote operations](https://github.com/jj-vcs/jj/blob/caf05e3bd8fc172615cf34b0ffc3f13567e54bbe/docs/config.md)
also invoke Git. These are useful implementations to study, but none supplies
verified complete native CLI parity by copying its source.

The imported family is specifically `grit-lib`, `grit-git` and
`grit-test-support`. Other historical Grit workspace packages, including its
HTTP server and protocol service packages, are outside this import. Their
presence upstream is not a claim that Gitr includes or qualifies those services.

## Source and license composition

`vendor/grit` includes `grit-lib`, `grit-git`, `grit-test-support`, their complete
Rust sources/tests, declarative manifests and embedded help data. There are 450
original imported file records. The source manifest records the exact upstream
SHA-256 for every original path; local patches/additions have separate hashes.
The actual path inventory is authoritative when counts change.

| Scope | Retained terms |
| --- | --- |
| `grit-git` and Git-derived CLI/help material | GPL-2.0-only; retained original notices and `COPYING` |
| `grit-lib` | MIT; retained split-license declaration and full MIT notice |
| `grit-test-support` | MIT, as declared by its upstream manifest |
| Gitr root executable and inherited Git scope | Existing GPL-2.0-only declaration retained |

`LICENSE-MIT` is the full upstream notice from
[`f615c13628313bd63f25d157d55da24a79ad9d68`](https://github.com/gitbutlerapp/grit/tree/f615c13628313bd63f25d157d55da24a79ad9d68),
supplementing the historical split notice for the already MIT-declared crates.
It does not relicense the historical CLI. The source manifest records its
separate origin, blob and SHA-256. Preserve those files with redistributed source
and comply with GPL source obligations for distributed combined executables.
No broad relicensing of inherited code was performed.

The upstream C Git reference tree and executable shell libraries/corpus are not
copied into this Rust-only repository. They remain available at their pinned
upstream revision. Rust tests formerly containing executable shell fixtures use
compiled Rust fixture programs. A genuine Rust include file was renamed from
`.rs.inc` to `.rs`; its original source identity remains in the manifest.

The import is not a claim that every transitive dependency is pure Rust or free
of unsafe code. Platform FFI, operating-system tools and cryptographic native
code remain separate dependency/portability boundaries. System Git is a test
oracle, not an implicit production implementation.

## Product entry and capability boundary

The root `gitr` executable selects the native library entry for ordinary Git
commands. `actions`, `assets`, `db`, `doctor` and `lfs` remain Gitr extensions.
`--isolated-git` explicitly selects the earlier Gitoxide implementation; default
`update-ref` also retains its mandatory expected-old CAS interface.

The native product entry applies its write gate **after alias and global-option
expansion**, so an alias cannot re-enable a disabled native mutation. Internal
subprocesses run the current Gitr executable and pass through that entry again.
`--native-git` selects the engine but cannot disable its product write gate.

The imported standalone `grit-git` binary remains an upstream conformance and
development target. It exposes unconverted donor paths and must not be presented
as the qualified Gitr product or a way around its storage/ref authority.

Core enabled paths include object plumbing, staging, ordinary commits, simple
branch creation/deletion, local branch checkout/switch, detached checkout,
lightweight/annotated tags, and inspection. New checked
ref APIs capture the original value before computing a change and compare it
while holding the same lock that protects publication. Unborn refs require
absence; detached HEAD is locked; reftable comparisons reload under the stack
lock. Detached `commit --amend` remains detached.

The remaining history and transport writers still contain unconditional legacy
publication paths. Gitr rejects them before their handlers mutate a repository:
merge/rebase, cherry-pick/revert, reset/stash, clone/fetch/push/pull,
receive-pack, several import/maintenance/ref commands, branch rename/copy and
recursive branch creation. The exact list and permitted read-only variants are
in `vendor/grit/grit-git/src/write_safety.rs` and the branch entry checks. Their
implementation source is retained for the next transaction migration.

Ordinary checkout holds the original HEAD and sorted source/destination branch
locks while computing and publishing the change. It acquires the real index
lock before changing worktree files, stages the new index privately, and runs
`post-checkout` after ref publication. `post-index-change` retains its earlier
index-publication timing while ref locks remain held. Existing local branches,
new local branches and detached HEAD use this path. Advanced checkout modes,
reftable, promisor repositories and split/sparse/alternate indexes fail explicitly.
Separate index, new-branch and HEAD renames do not provide rollback of arbitrary
I/O failures across all files. A worktree is not a database transaction.

Ref/reflog/config changes are not yet one cross-file atomic transaction. A
single locked ref update does not prove distributed consensus or multi-ref
rollback. No HA claim follows from redb multi-process coordination.

### Inspection and argument boundaries

Read-only inspection refuses effective configured filter, textconv, external
diff, fsmonitor, pager and signature-verifier programs. An inspection alias
cannot execute a shell/external helper or redirect to a modifying builtin.
The guard preserves inspection intent through alias `-C`/`-c` transitions.
Errors name the configuration key without printing command contents.

Explicit write commands retain Git's user-configured hook/filter interfaces.
This is broader configuration behavior than the isolated profile. Non-UTF-8 OS
arguments are rejected before traces or repository side effects, rather than
silently converting a path to a different filename. Full byte-path command-line
compatibility remains an acceptance gap.

Product worktree writers also check index paths, worktree names and link targets
before mutation. The metadata checks stop at 100,000 entries, depth 128, 16 MiB
of path bytes and 64 MiB of index input per phase. They do not follow symlinks or
walk `.git`. Restore validates incoming tree paths/links and rejects unsupported
bytes at its actual path conversions. This preflight does not lock the whole
worktree; other donor handlers still need lossless conversion work for names
introduced concurrently after the check.

`tag -v` performs real signature verification and evaluates the trust result;
the presence of a PGP armor header is not successful verification. A nonexistent
object ID formerly synthesized for an upstream empty-tree typo is now rejected.
SHA-256 tree validation and tag-target parsing preserve 32-byte identities.

## redb object storage

Enable the object store explicitly:

```sh
gitr -C /path/to/repository config gitr.redbObjects true
gitr -C /path/to/repository hash-object --no-filters -w path/to/blob
```

The store lives at `<common-git-dir>/gitr/objects.redb`. It is separate from the
existing catalog/action databases and does not migrate or remove them. It uses
the exact reviewed redb 4.5.0 dependency in cooperating multi-writer mode.

Git object IDs retain their SHA-1 or SHA-256 meaning. The database stores exact
raw Git header/payload representations in 1 MiB chunks, with length and a
secondary SHA-256 checksum. Object metadata and every chunk commit in one
immediately durable transaction. Duplicate writes compare bytes; conflicting
identities, missing/trailing chunks and damaged records fail explicitly. A
corrupt authoritative record does not silently fall back to a loose mirror.

After the database commit, Gitr publishes a verified standard loose-object
mirror without clobbering another writer's file. Existing/racing mirrors are
verified and synchronized, as are Unix directory entries, before success. A
failed mirror publication can leave an unreferenced durable database object;
retry verifies and completes publication. References are published separately.

Write initialization always uses redb's create/validate transaction, including
when a concurrent creator has already created the file. Reads do not create a
missing store or import loose/packed objects. Opening an existing database uses
redb's committed-page integrity check, which can rebuild allocator bookkeeping
and requires write access; it is not a byte-for-byte read-only file operation.
Shared handles cache this open cost within one process.

Raw blob writes and unknown-length hashing snapshot input to a temporary file
and stream through fixed buffers. Hash-only regular files use their opened
file's length and one pass, without a temporary copy or secondary storage hash;
short or extra input is rejected before returning an identity. This does not
promise an atomic snapshot of concurrent same-length edits.
Database inserts, duplicate verification and loose-file
publication likewise stream that snapshot. The snapshot stabilizes the length
for unknown-length stdin and concurrently resized inputs. Structured or filtered
`hash-object` inputs are limited to 64 MiB pending streaming conversion.

The database is **not yet the sole repository authority**:

- Refs, reflogs, indexes and existing packs retain their Git filesystem formats.
- Existing packs are read without automatic database import; pack ingestion does
  not yet enter all objects into redb.
- Database pruning, reachability GC, pack/repack integration and backup/restore
  coordination remain work. Records are currently retained.
- Other donor `Vec` object-read/write APIs still materialize complete objects;
  chunked persistence alone does not establish bounded memory for every command.
- Windows directory durability and full cross-platform execution remain
  unqualified. Linux process tests do not simulate arbitrary power loss.

## Build and qualification

Application qualification uses Lustr
`6339fbb104d6adda4be7e6be7dc637a6a9e5586b` and the **produced** BlueFe compiler
`7005dc02c9fb6674380eade443022bbc04215276`. The hosted bootstrap checks the source
revisions, stage-zero pin, native/Wasm sysroots and executable compiler identity.
The bundle contains source notices, file hashes and smoke-test evidence.

Stock Rust 1.99.1 compiles only dependency-free CI coordinators. Stock Cargo
metadata resolves lockfiles without compiling application code. Neither action
is a substitute application build or a fallback compiler qualification.

The application workflow verifies the exact compiler archive and bundle before
running root feature/target tests and imported Rust tests through Lustr. New
regressions cover object identity, redb durability/corruption/initialization,
streaming, ref races, detached HEAD, forged signatures, unsafe helper settings,
aliases and non-UTF-8 argument rejection. The default CLI tests remove Git and
shell executables from its PATH; reference Git runs separately as the oracle.
Tests that verify explicit rejection are not counted as implemented parity.

The reusable compiler workflow runs first. Its cache key includes the complete
source pins and build-helper hash; every cache hit is verified again. A missing
cache causes a rebuild from those pins. The application job consumes only the
artifact ID returned by that same run and verifies its archive, file inventory,
source identity and executable smoke tests. Artifact retention therefore does
not make future qualification depend on a month-old download or a latest-run
fallback.

The [executed qualification record](native-git-qualification.md) documents the
2026-11-10 hosted pass: 146 root tests, 705 imported tests, source/policy checks,
the optimized build and four output-verified benchmark workloads. It retains the
exact tested tree, logs, source hashes, compiler provenance and timing samples.
Hashing was faster in that run; status, log and tree listing were slower.
The historical upstream shell-suite count must not be relabeled as Gitr's own
coverage. Authenticated GitHub transport, all Git options/error modes, non-UTF-8
paths, SHA-256 end-to-end operations and Windows/macOS still require qualification.

Performance work records exact outputs before timing and retains individual
samples, compiler/profile settings, reference Git version and workload shape.
An improvement for one workload cannot establish that every Git command is
faster. No general speed claim is made before those measurements.

## Updating the import

Choose a pinned upstream revision and read its actual source/license/command
inventory. Compare original file hashes with `upstream-source.json`; preserve
the imported crate family and its dependency lock together. Reapply only the
documented Gitr patches, regenerate local path/hash provenance, then run source,
dependency, object/ref, safety, interoperability and performance qualification.
Current upstream `main` must not silently replace this historical CLI snapshot.

Validate recorded local bytes with `lustr run --example verify_import --locked`.
After reviewing local changes, `-- --refresh` updates their local hashes and
addition inventory without changing original upstream identities or license
notices. Commit the resulting manifest with the code it describes.

The next implementation priority is migrating the remaining publication paths
to the checked ref transaction APIs, including nested commands and transports,
then moving pack ingestion/maintenance into the selected database model. Those
are concrete gaps to close before a full Rust Git replacement claim is valid.
