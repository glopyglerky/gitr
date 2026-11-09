# Gitr

[![Gitr Rust qualification](https://github.com/glopyglerky/gitr/actions/workflows/gitr-rust.yml/badge.svg?branch=master)](https://github.com/glopyglerky/gitr/actions/workflows/gitr-rust.yml)

A Rust Git implementation with verified large-file storage, game-asset chunk
reuse, a transactional repository graph and vector catalog, and bounded Rust/Wasm
actions against immutable Git snapshots. Active development
lives on the default branch, [`master`](https://github.com/glopyglerky/gitr/tree/master).

**Status: initial implementation. Gitr is not yet a complete Git replacement.**
The new implementation is in [`rust/gitr`](rust/gitr). This fork also retains the
upstream Git source and test corpus, so the entire repository is not all Rust.

## What runs today

- Native Gitoxide commands for repositories, objects, refs, history, index and
  status. Guarded `update-ref` uses expected-old-value compare-and-swap.
- Standard Git LFS clean/smudge filters, verified immutable cache, explicit HTTPS
  basic transfers and the file-lock API.
- FastCDC local chunk reuse across large asset revisions, with verified hydration
  into the standard Git LFS cache.
- Large-asset inventory and candidate Unreal/Unity `.gitattributes` rules.
- redb snapshots keyed by exact Git commits, factual Git graph queries, and
  automatically generated lexical vectors with exact token matching for local search.
- Content-addressed Wasm action modules, immutable committed-source inputs,
  idempotent job submissions, durable fenced worker claims, and atomic
  artifact/receipt publication in the local database.
- An optional authenticated local HTTP action API over the same redb database
  and execution methods as the CLI; no external database server or data migration.
- Builds and tests through a pinned version of [Lustr](https://github.com/glopyglerky/lustr).

Learned semantic embeddings, resolved call graphs, distributed HA, native
clone/fetch/push, add/commit/checkout and complete merge/rebase support remain work.
Local redb multi-process writers serialize transactions; that is not distributed
consensus. The first search model uses lexical feature hashing, not a trained code
model. The action runner is not yet a DAG scheduler, standard MCP server, remote
multi-tenant service, full forge, or GitHub Actions compatibility implementation.
Unsupported Git commands fail explicitly.

## Build with Lustr

Use Rust 1.99.1 and the compiler revision in
[`lustr-toolchain.json`](rust/gitr/lustr-toolchain.json). From this repository root:

```sh
git clone https://github.com/glopyglerky/lustr .gitr-ci/lustr-src
git -C .gitr-ci/lustr-src checkout 62494e27717a64c8612b538409de27048fa6adc2
rustup toolchain install 1.99.1 --profile minimal
cargo +1.99.1 install --path .gitr-ci/lustr-src/tools/cargo-lust --locked
rustup run 1.99.1 lustr --engine rustc build --manifest-path rust/gitr/Cargo.toml --locked --release
rustup run 1.99.1 lustr --engine rustc test --manifest-path rust/gitr/Cargo.toml --locked
```

The executable is `rust/gitr/target/release/gitr`, or `gitr.exe` on Windows.
The explicit `rustc` route uses Lustr's full-Rust Cargo coordinator. Native Lust
does not currently compile arbitrary Gitoxide/redb dependencies. Cranelift needs
Lustr's matching backend and compiler; see [qualification](Documentation/gitr/qualification.md).

## Try it

With the built executable on `PATH`:

```sh
gitr -C /path/to/repo doctor
gitr -C /path/to/repo log --oneline -n 10
gitr -C /path/to/repo status --porcelain
gitr -C /path/to/repo assets scan --threshold-bytes 52428800
gitr assets attributes unreal

gitr -C /path/to/repo db index HEAD
gitr -C /path/to/repo db search "asset upload" --revision HEAD --limit 10
gitr -C /path/to/repo db graph --revision HEAD --path src/main.rs

gitr -C /path/to/repo assets import Content/World.uasset
# Use the returned full-file OID and size to materialize the standard LFS cache:
gitr -C /path/to/repo assets hydrate OID SIZE
```

`db index` reads committed objects, not uncommitted worktree files. Search requires
that exact indexed commit and reports excluded content. The graph currently
describes repository membership, not language-resolved calls. Commands emit JSON
or JSON Lines where a structured report is useful.

The large-file layer preserves whole-file SHA-256 Git LFS pointers. GitHub still
receives whole LFS objects. Chunk transfer requires a separately negotiated server
protocol. To use explicit standard LFS transfers:

```sh
gitr -C /path/to/repo lfs import Content/World.uasset
gitr -C /path/to/repo lfs upload OID SIZE --endpoint https://github.com/OWNER/REPO.git/info/lfs
gitr -C /path/to/repo lfs fetch OID SIZE --endpoint https://github.com/OWNER/REPO.git/info/lfs
```

Authenticated requests read `GITR_LFS_AUTHORIZATION` from the environment. Do not
put a token in source files or command arguments. Endpoint discovery, credential
helpers, automatic pre-push traversal and enforcement of asset locks are not yet
wired into a complete native push workflow.

## Database: redb

**Gitr uses redb.** The current reviewed pin is **4.5.0**. The repository catalog
and action state already use it; redb is the selected backend, not a temporary
fallback. The earlier SurrealDB/SurrealKV/SurrealDS migration direction is retired.
No existing repository data is converted or deleted.

The rebuildable catalog and durable action state retain their separate files in
`<common-git-dir>/gitr/`. Relational/code-graph queries, learned vector retrieval,
forge services and distributed replication remain Gitr implementation work over
this storage substrate. Selecting redb does not make those capabilities complete
or turn local writer coordination into multi-host HA.

See the [redb decision](Documentation/gitr/redb-decision.md). Earlier database
experiments are preserved in a [non-active archive](Documentation/gitr/archive/surrealdb),
outside the active Rust packages and GitHub workflows. A
[backend-policy check](.github/workflows/gitr-redb-policy.yml) rejects their
reintroduction into the active dependency manifests or migration paths.

## Run a database-resident Rust/Wasm action

Build the [example Rust action](rust/gitr-action-example) for
`wasm32-unknown-unknown` using the same pinned toolchain:

```sh
rustup target add --toolchain 1.99.1 wasm32-unknown-unknown
rustup run 1.99.1 lustr --engine rustc build --manifest-path rust/gitr-action-example/Cargo.toml --locked --release --target wasm32-unknown-unknown
WASM_PATH="$(pwd)/rust/gitr-action-example/target/wasm32-unknown-unknown/release/gitr_action_example.wasm"
gitr -C /path/to/repo actions install "$WASM_PATH"
# The install response prints module_sha256:
gitr -C /path/to/repo actions submit whitespace-policy MODULE_SHA256 --revision HEAD --path src/main.rs
gitr -C /path/to/repo actions run whitespace-policy
gitr -C /path/to/repo actions status whitespace-policy
gitr -C /path/to/repo actions artifact whitespace-policy
# Export includes source and action module bytes: store the backup privately.
umask 077
gitr -C /path/to/repo actions export > /secure/backups/gitr-actions.jsonl
```

The export verifies action object hashes and their receipts; it refuses snapshots
with running workers. It is a read-only export, not an authority cutover or
proof of a fully qualified restore facility, and does not change stored records.

Action inputs are the bytes in the named **committed Git revision**, not dirty
worktree contents. A failed policy test has no downloadable successful artifact.
The local `actions control` JSON command is not a standards-compliant MCP/HTTP
transport and assumes OS-level authorization. In-process Wasm fuel and guest
memory limits do not impose a hard whole-process memory or wall-time budget.

See [action execution evidence](Documentation/gitr/actions-qualification.md)
for the measured tests and remaining restrictions.

## Optional local action API

Build with `--features action-api` to add `gitr actions serve`. The default CLI
build does not enable this feature and ordinary commands never start a listener.
The API uses the same action records as native CLI processes, not a replacement
datastore or in-memory session state.

```sh
rustup run 1.99.1 lustr --engine rustc build --manifest-path rust/gitr/Cargo.toml --locked --features action-api
export GITR_API_TOKEN="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
rust/gitr/target/debug/gitr -C /path/to/repo actions serve --repository my-repository
```

Startup prints the selected loopback endpoint. Every request requires the bearer
token and matching repository header. The service supports installation,
submission, status, cancellation, execution and artifact retrieval. It rejects
non-loopback listeners, browser Origin headers, oversized requests, raw queries
and caller-fabricated worker completion records. See the
[protocol, executed Linux qualification and limitations](Documentation/gitr/action-api.md).
This is a trusted-local-user profile, not an Internet-facing forge or standard MCP
transport. Platform success must be read from its actual CI run, not inferred from
the configured Linux/macOS/Windows matrix.

## Design and evidence

- [Compatibility matrix and exact limits](Documentation/gitr/compatibility.md)
- [Executed qualification](Documentation/gitr/qualification.md)
- [Selected redb backend](Documentation/gitr/redb-decision.md)
- [Repository database, multi-writer and HA design](Documentation/gitr/architecture.md)
- [Git, Lore and asset upstream catalog](Documentation/gitr/upstreams.md)
- [Rust database and retrieval comparison](Documentation/gitr/database-research.md)
- [Closest similar projects: Libra, Git-vdb, GitQL and Codebase Memory](Documentation/gitr/similar-projects.md)
- [Libra component review and native push integration plan](Documentation/gitr/libra-integration.md)
- [Next implementation work and acceptance gates](Documentation/gitr/roadmap.md)

The authored Rust package is GPL-2.0-only, following this fork's Git license.
Upstream licenses remain intact. Gitoxide uses its MIT alternative; no libgit2 or
inherited C Git library is linked into this package. TLS currently uses rustls
with ring, whose cryptographic implementation includes native code. See the
catalog before incorporating differently licensed upstream source.

## Upstream Git reference

The original upstream README follows. Its installation commands build the
reference Git implementation, not the standalone Rust port above.

<details>
<summary>Original Git README</summary>

[![GitHub build status](https://github.com/git/git/workflows/CI/badge.svg)](https://github.com/git/git/actions?query=branch%3Amaster+event%3Apush)
[![GitLab build status](https://gitlab.com/git-scm/git/badges/master/pipeline.svg)](https://gitlab.com/git-scm/git/-/pipelines?ref=master)

Git - fast, scalable, distributed revision control system
=========================================================

Git is a fast, scalable, distributed revision control system with an
unusually rich command set that provides both high-level operations
and full access to internals.

Git is an Open Source project covered by the GNU General Public
License version 2 (some parts of it are under different licenses,
compatible with the GPLv2). It was originally written by Linus
Torvalds with help of a group of hackers around the net.

Please read the file [INSTALL][] for installation instructions.

Many Git online resources are accessible from <https://git-scm.com/>
including full documentation and Git related tools.

See [Documentation/gittutorial.adoc][] to get started, then see
[Documentation/giteveryday.adoc][] for a useful minimum set of commands, and
`Documentation/git-<commandname>.adoc` for documentation of each command.
If git has been correctly installed, then the tutorial can also be
read with `man gittutorial` or `git help tutorial`, and the
documentation of each command with `man git-<commandname>` or `git help
<commandname>`.

CVS users may also want to read [Documentation/gitcvs-migration.adoc][]
(`man gitcvs-migration` or `git help cvs-migration` if git is
installed).

The user discussion and development of Git take place on the Git
mailing list -- everyone is welcome to post bug reports, feature
requests, comments and patches to git@vger.kernel.org (read
[Documentation/SubmittingPatches][] for instructions on patch submission
and [Documentation/CodingGuidelines][]).

Those wishing to help with error message, usage and informational message
string translations (localization l10) should see [po/README.md][]
(a `po` file is a Portable Object file that holds the translations).

To subscribe to the list, send an email to <git+subscribe@vger.kernel.org>
(see https://subspace.kernel.org/subscribing.html for details). The mailing
list archives are available at <https://lore.kernel.org/git/>,
<https://marc.info/?l=git> and other archival sites.

Issues which are security relevant should be disclosed privately to
the Git Security mailing list <git-security@googlegroups.com>.

The maintainer frequently sends the "What's cooking" reports that
list the current status of various development topics to the mailing
list.  The discussion following them give a good reference for
project status, development direction and remaining tasks.

The name "git" was given by Linus Torvalds when he wrote the very
first version. He described the tool as "the stupid content tracker"
and the name as (depending on your mood):

 - random three-letter combination that is pronounceable, and not
   actually used by any common UNIX command.  The fact that it is a
   mispronunciation of "get" may or may not be relevant.
 - stupid. contemptible and despicable. simple. Take your pick from the
   dictionary of slang.
 - "global information tracker": you're in a good mood, and it actually
   works for you. Angels sing, and a light suddenly fills the room.
 - "goddamn idiotic truckload of sh*t": when it breaks

[INSTALL]: INSTALL
[Documentation/gittutorial.adoc]: Documentation/gittutorial.adoc
[Documentation/giteveryday.adoc]: Documentation/giteveryday.adoc
[Documentation/gitcvs-migration.adoc]: Documentation/gitcvs-migration.adoc
[Documentation/SubmittingPatches]: Documentation/SubmittingPatches
[Documentation/CodingGuidelines]: Documentation/CodingGuidelines
[po/README.md]: po/README.md

</details>
