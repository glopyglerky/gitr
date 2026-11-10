# omdb snapshot catalog

Rit now uses the owner's omdb fork for a repository-local catalog of named,
immutable snapshot identities. This is an active application integration,
connected to the existing snapshot reader. Git objects, packs, refs, and normal
Git command behavior retain the Sley implementation. Full Git parity remains
unfinished.

The dependency is the `redb` package from
[`glopyglerky/omdb` at `4c7c6c3`](https://github.com/glopyglerky/omdb/tree/4c7c6c35d61372d0cd538ef3824a2f47eed2c4d5),
aliased as `omdb`, with default std features. The database library source is
unchanged from the previously reviewed `9273718` baseline. See the
[dependency identity and license record](omdb-dependency.json) and
[API fact check](omdb-redb-factcheck.md). The API-5 preview, fork's example,
no_std profile, and hardware-lane proposal are not part of this integration.

## Application interface

```sh
rit --rit-db init --repo /path/to/repository
rit --rit-db put --repo /path/to/repository --name build-source --revision HEAD --expect-generation 0 --request-id build-source-1
rit --rit-db get --repo /path/to/repository --name build-source
rit --rit-db list --repo /path/to/repository --limit 100
rit --rit-db pin --repo /path/to/repository --name build-source --expect-generation 1
rit --rit-snapshot read --repo /path/to/repository --saved build-source --path src/main.rs
```

`put` resolves and verifies the commit and root tree before taking a database
write transaction. It saves the object format, complete commit ID, and complete
root-tree ID. The JSON mutation response includes `request_id`, `replayed`, and
`record`. A record contains `name`, `generation`, and `snapshot`.

For a first mutation, `generation` is 1. The database has one global increasing
generation counter; subsequent successful mutations use its next value. To
replace a record, pass the generation returned for that name:

```sh
rit --rit-db put --repo /path/to/repository --name build-source --revision HEAD --expect-generation 1 --request-id build-source-2
```

`delete` requires a name, expected generation and request ID. It retains a
tombstone with `snapshot: null` and a new generation. `get` and `list` return
tombstones so callers can obtain the generation required to recreate a name.
Expected generation zero means the name has never existed, not merely that it
is currently deleted. Counter overflow fails before any mutation or receipt.

Names and request IDs contain 1 through 128 ASCII characters: letters, digits,
dot, underscore or hyphen, beginning with a letter or digit. Lists are ordered
by name, including tombstones, and bounded to 1 through 1,000 records per page.
`--after NAME` is an exclusive cursor. A full page returns its last name as
`next_after`; the following page can be empty. Pages are separate read
transactions, so pagination is not a frozen view across concurrent changes.

The public Rust entry point is `rit::catalog::SnapshotCatalog`. Its explicit
`initialize`, read-only `open`, and `open_writable` constructors separate the
filesystem effects. `put`, `delete`, `get`, `list`, and `snapshot` use the same
implementation as the CLI. The `git` and `scalar` executables continue their
existing Git-compatible dispatch; the extensions require the `rit` executable
and an explicit first option.

## Explicit Git reachability pins

`pin` protects the exact live name and catalog generation against **ordinary
subsequent Git garbage collection**. This is an opt-in action; `put` still
stores metadata only and does not automatically retain content. The Rust API
is `SnapshotCatalog::pin(name, expected_generation) -> PinResult` and can be
called from a read-only omdb catalog handle. It writes only a native Git ref.

The pin uses a deterministic, unique ref under `refs/rit/retained/`, with the
ASCII snapshot name encoded into length-bounded hexadecimal path components and
the full catalog generation as its final component. It does not use an
unchecked branch update. The Sley ref transaction requires `MustNotExist`
under the appropriate Git ref lock, for files and reftable alike. An existing
direct ref with the same commit ID makes an interrupted publication retry
idempotent; a symbolic ref or any other target is rejected without overwrite.

Before publishing, Rit verifies that the stored commit/tree identity still
matches Git. A strict native reachability walk refuses missing objects, and
Sley's fsck connectivity checker validates object hashes, parsed links and
expected object types. A second set of checks through a newly opened object
store after publication avoids accepting stale cached objects if content
disappeared. It then rechecks the catalog record and direct ref against their
original values. The graph includes parent history and excludes submodule
gitlinks (those are external repositories). This is deliberate whole-graph
work: memory grows with object count, and large objects must be decoded; it is
not a fast or bounded-memory CI checkout operation.

Pins require a locally owned Git object store. They fail closed if the Git
objects directory, any immediate child, a reachable loose object file or a
pack entry is a symlink; if `objects/info/alternates` exists; or if there are
promisor pack markers. A ref in one repository cannot prevent an alternate
repository's garbage collector from removing shared objects. The ordinary
`put` and saved snapshot reader still support their existing object storage
behavior; these restrictions apply specifically to `pin`.

The JSON result contains `name`, `generation`, `commit_oid`, `refname`, and
`already_pinned`. The caller can independently inspect the Git ref with
`git show-ref` or `git rev-parse`. An in-flight pin that detects a concurrent
catalog change or encounters an interrupted response can leave an extra ref.
Retry with the same name and generation while it remains live. The ref itself
persists independently of the catalog row.

**Pins are intentionally not automatically released**, including after
replacement and deletion. A safe release needs separate ownership, fencing,
checked deletion and reconciliation so it cannot free an object that still
has a live consumer. For now, users must account for the resulting storage
growth. There is no automatic sweep, lease expiry, or ref garbage collection.
Rit does not reserve the ref namespace from external Git commands: manually
removing or modifying a pin defeats its protection. Internal pin writes do
not currently invoke configured reference-transaction hooks.

Do not run Git GC or object deletion **concurrently with pin publication**.
Git's reachability scan and ref creation are not one transaction, so a running
GC can act on an earlier ref snapshot. This pin also cannot restore objects
already missing, establish physical power-loss durability on all filesystems,
or guarantee completeness for a remote/partial object store. It establishes
a local, verified, create-only ref anchor for later ordinary Git pruning, not
an atomic omdb/Git cross-store commit or universal object-retention lease.
The reused Sley fsck connectivity checker is recursive; pathologically deep
histories or trees may exhaust the process stack. An iterative, bounded-resource
checker is required before qualifying pins for arbitrarily large repositories.

## Atomic mutations and retry behavior

A mutation commits the new record, global generation, and immutable request
receipt in one omdb transaction with `Durability::Immediate`.

The request descriptor contains the name, expected generation and resolved
snapshot identity, or a deletion operation. An identical request ID and
descriptor returns the original record with `replayed: true`, even after later
updates or deletion. It never rewrites the current record. Reusing that ID for
different input fails. Failed CAS and invalid input do not consume request IDs.

An initial `--revision HEAD` is convenient, but HEAD can move before a retry.
Retry using the full `commit_oid` from the original result or the caller's
already resolved input. A changed resolved target under the same request ID is
rejected. Full-ID receipt replay also works after Git objects are removed: it
acknowledges the original metadata operation, not current content availability.
New puts still verify their Git commit and root tree.

Receipts and tombstones currently remain indefinitely. They deliberately have
no pruning operation until an expiration protocol can preserve retry and stale
writer guarantees. Input records are limited to 4 KiB and lists are bounded;
the total database can still grow with the number of successful requests.

`MultiWriter` permits multiple local processes to open the database. It still
allows only one write transaction at a time. `try_begin_write` returns Busy
when that writer is occupied; the catalog CLI uses exit status 75. Other errors
use status 128. The CLI has no unbounded retry loop. Database opening, recovery,
reads and successful commits are synchronous and can wait on I/O or internal
locks. This is not an asynchronous or hard-latency-bound API.

## Repository ownership, schema, and recovery

The only database location is `<common Git directory>/rit/metadata.omdb`.
Linked worktrees share that common directory. Bare repositories use the same
relative path inside the bare Git directory. Repository discovery uses the
same environment restrictions as the [snapshot reader](snapshot-reader.md).
The original linked worktree's Git directory is retained for resolving its HEAD.

Initialization is explicit and works in an unborn repository. It reserves the
database file with `create_new`, synchronizes that file and the metadata/common
directories, then initializes the application tables and commits with immediate
durability. A usable application header is published only after the path is
durable, so another process cannot acknowledge a mutation in an initialization
window before directory synchronization.
It never initializes, truncates or overwrites an existing file. An interrupted
initialization can leave an incomplete reserved file; subsequent opens fail
instead of silently adopting or deleting it. There is no migration or repair
command for an incomplete application schema in this slice.

Initialization currently requires the Unix directory synchronization path.
Execution has been qualified on Linux. Windows initialization needs a separately
implemented and qualified directory-publication guarantee; no Windows or macOS
execution is claimed here.

The header identifies `rit.snapshot-catalog`, schema version 1, the repository's
object format and the global generation. Reads reject missing/unknown headers,
future schemas, format mismatches, missing tables and malformed records.
Existing clean foreign databases fail read-only preflight before writable open.
Read commands use omdb's read-only handle and never recover or initialize a file.

An explicit writable open may invoke omdb recovery after an interrupted writer.
If a file requires recovery before any header can be read, that recovery happens
before Rit can validate its application identity. A dirty foreign file manually
placed at this reserved path could therefore be recovered and then rejected.
Rit never adopts its schema or writes catalog rows to it. A failed write can
have an uncertain outcome; the persisted receipt determines whether retry is a
replay or a new commit.

Metadata paths must be real directories and regular files; existing symlinks
are rejected. The enclosing repository filesystem is trusted. These pathname
checks do not protect against malicious concurrent path replacement and do not
create a multitenant authorization boundary. Archived Gitr databases are never
located, opened or migrated automatically.

## Git content remains independently verifiable

`--saved NAME` is mutually exclusive with `--revision`. It loads the stored
identity, opens the full commit through the native snapshot reader, and checks
that the actual object format and root tree equal the recorded values. It then
uses the existing path validation, object hashing and raw-byte read behavior.

Saving a record does not write Git refs or copy payloads. A saved record without
an explicit, successfully published pin can outlive its objects; source reads
then fail. The explicit pin protocol above uses Git reachability, not an omdb
payload copy. Retention leases, ref release/reconciliation, cross-store atomic
publication, bounded-memory streaming, writable overlays, Wasmtime workers,
authentication and distributed storage remain separate work.

## Qualification

All 33 root tests passed through pinned Lustr's explicit Rust 1.99.1 route.
They include 10 new catalog product tests, 5 new catalog contract tests,
11 existing snapshot product tests, 2 existing snapshot unit tests and 5
existing Git/grep workflows. All 9 storage-policy tests passed, along with
source import verification and inventory.

The retained tests are `tests/catalog.rs` and the private contract tests in
`src/catalog.rs`. They cover both hash formats through the actual binary with
Git absent from candidate PATH, plus same-process contention and injected
storage failures. The [evidence record](omdb-catalog-evidence/result.json)
links commands, compiler identities, logs, review outcomes, and limitations.
The [independent reviews](omdb-catalog-evidence/review.json) have no remaining
findings. They prompted read-only preflight for clean foreign files and the
initialization publication-order correction described above.

The first import-verifier run still prohibited any redb dependency. It was
updated to require the exact selected omdb fork revision, preserving all Sley
source and license checks and the historical Grit exclusion. The initial
rejection and passing final run are retained in the evidence.

The fault backend fails before individual write, resize and synchronization
operations at 24 boundaries. Reopening must find the record and its receipt
together or neither, retain every acknowledged write, and converge on one
generation when the request is retried. This exercises application transaction
composition against the pinned database; it is not a physical power-loss or
remote-storage certification.

### Follow-up pin qualification

The explicit pin extension passed 38 root tests (15 catalog product tests,
7 root unit tests and 16 other product workflows), 9 storage-policy tests,
and 111 selected Sley ref/GC tests. The source import verifier again checked
the 680-file Sley inventory without donor changes. Local commands used pinned
Lustr's Rust 1.99.1 route; this does not establish produced-BlueFe execution.

The tests exercise native ref publication and replay for SHA-1 and SHA-256,
files and reftable, linked worktrees, ordinary Git 2.55 GC and Rit's native GC,
stale generations, deliberate missing blobs, mismatched ref targets, malformed
graph types, promisor markers, borrowed alternate stores, symlinked loose
objects and symlinked packs. They confirm that pinned content survives
subsequent pruning while a separate unprotected commit is collected, both with
Git-first and Rit-first collection. They do not prove concurrent-GC or
power-loss safety. The [pin evidence](omdb-catalog-evidence/pin-qualification.json)
records the exact qualification scope and review corrections.

Root Git workflows, source import verification and the updated composition
policy remain required gates. Produced BlueFe qualification is a separate
hosted workflow on the published commit. Local Lustr's explicit rustc route
does not constitute produced-BlueFe execution.
