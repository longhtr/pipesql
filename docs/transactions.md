# Transactions and recovery

PipeSQL has one serialized writer and concurrent snapshot readers. A transaction
constructs private immutable files, then publishes a complete graph. Readers do
not observe partially built files or mutable pages.

Legacy format-4 loads and namespace-format-7 catalog transactions use the same
[publisher](../src/publication.rs). This guide defines its outcomes, ordering,
and recovery contract. [Storage](storage.md) owns the byte formats, and
[verification](verification.md) owns the required evidence.

## Outcomes

- An ordinary pre-publication `Error` is a definite abort only after private
  rollback and its required synchronization complete.
- `Commit { transaction, generation }` means the named commit is visible and the
  advertised durable sync event completed successfully.
- `Error::CommitAmbiguous { transaction, source }` means publication may have occurred
  but durability or acknowledgement could not be resolved in-process.
  Reopen plus `resolve_commit` resolves it.
- `Error::CleanupRequired` means definite rollback could not be established; it
  is neither success nor a clean abort.

No error after possible publication is reported as definite abort. Cancellation
is honored before publication begins. After the point of no return, the
operation finishes or returns the same honest ambiguous outcome.

Temporary construction ownership follows the outcome. Successful publication
transfers the unit to the persistent generation; confirmed rollback removes it.
Until either is established, `CommitAmbiguous` and `CleanupRequired` retain the
original temporary reservation on the unavailable handle. Closing releases the
handle and lease without settling the files or outcome. Exclusive recovery must
settle the persistent graph and orphan cleanup before exposing a fresh usable
handle. The reservation observable is a live handle account, not an on-disk byte
count; the bounded namespace carries the recovery obligation after close or
process death. See `resources.md` for the account and admission rules.

## Attempt identity and resolution

A transaction identity denotes one attempt, not a reusable generation slot.
Distinct attempts cannot share an identity. Once a retained attempt resolves as
aborted, a later successful load must not turn that resolution into durable
success. Outcome retention has a named bound and explicit expiry behavior;
recovery must preserve it across reopen. An unknown identity is not evidence of
abort.

Public format 4 encodes a nonzero 128-bit DatabaseId followed by a nonzero u64
attempt sequence, in little-endian order. `TransactionId::from_bytes([u8; 24])`
validates shape, not issuance or authenticity; invalid shape returns
`InvalidTransactionId`. In a settled view, foreign or above-prefix tokens are
`NotFound`, the complete retained success receipt resolves `Durable`, and every
other issued sequence resolves `Aborted`. No outcome expires. The current
one-load capability admits at most one success; supporting another success
requires retaining all earlier successful outcomes, not overwriting the inline
receipt. An exhausted u64 issuance prefix refuses a new attempt before temporary
reservation or issuance. The final representable attempt can still commit; no
summed publication rank is stored or narrowed. Declared tables use the same
token encoding with the external success history described below.

`resolve_commit(&self, ...)` is read-only. An uncertain handle or namespace
needing repair returns `RecoveryRequired`; close and reopen under exclusive
recovery before retrying. It cannot acquire repairing authority through a shared
reference. The current load path makes its handle unavailable before writer
admission can repair the namespace. Any admission error or unwind requires
reopen; a visibly repaired name does not prove a failed barrier completed. This
applies even without temporary-byte debt. Construction remains unavailable until
success or confirmed clean rollback; an unwind cannot silently restore a usable
handle.

## Shared publication protocol

The shared [`publish_snapshot`](../src/publication.rs) function validates the
old/new transition, writes and verifies its fence, prepares ROOT.A, then
replaces and synchronizes both roots. `write_fence` owns fence ownership checks,
write/readback, and synchronization; `write_root_next` owns equivalent
preparation for a root replica. The main function keeps the first replacement
and all later uncertainty visible. `FailureStage` describes whether replacement
may have occurred; the caller still must establish cleanup before reporting a
definite abort.

Public format 4 fixes one 512-byte fence in WAL, two 4096-byte roots, and at
most one generation-1 unit. A snapshot contains DatabaseId, issued prefix, and
the complete inline success receipt with immutable unit references. Its
publication order is `(issued, successful_count)`: issuance increments only the
prefix and preserves the graph; commit adds the currently active last attempt to
the success index. Readers validate equal snapshots or legal adjacent
transitions, not generation alone.

After memory, input shape, temporary bytes, next sequence and possible success
are admitted, the writer publishes an issuance-only snapshot to BOTH roots.
Until both barriers complete, no token is exposed and no data construction
starts. Issuance failure requires reopen and is not a data-commit ambiguity. A
subsequent abort retains the prefix. Recovery has no active attempt and cannot
resume an abort gap.

Issuance and data use the same publisher: write/verify/sync the complete
snapshot fence, write/verify/sync `ROOT.A.next`, then replace ROOT.A. Before
data publication, the verified private unit and its directory link are
synchronized. Data publication enters its point of no return at the first root
replacement. It syncs the database directory before repeating the root
write/replacement for `ROOT.B` and a final directory sync. Cancellation is
ignored after entering that first replacement; every later failure is
`CommitAmbiguous`. A successful return updates the in-memory generation only
after the final barrier. The publisher still requires cut, process-death, and
filesystem qualification; checkpoint cadence remains production work. The
protocol must preserve these properties: one publication authority;
generation/incarnation wrap refusal before mutation; duplicate roots survive one
damaged copy; every successful prefix either fails closed or recovers an allowed
state; and prior ambiguous commits remain resolvable through another failure and
healing.

The publisher derives issuance versus commit from the validated old/new snapshot
transition, using the same rule as recovery selection. An unchanged data graph
with the next issuance prefix is issuance even when it contains committed data.
The derived kind governs fence and both root effect labels; invalid transitions
refuse before filesystem effects. The legacy format-4 API still permits only one
successful load. Declared tables use repeated catalog publication through this
same authority.

## Declared-table transactions

Catalog namespace format 7 supports repeated declarations and appends. Each
successful publication records the issued transaction in the bounded external
success history and installs a complete immutable catalog generation. Issuance
preserves the prior graph and success history; an aborted attempt remains a gap
in the issued prefix. Resolution returns Contention for the active transaction.
In a settled view, issued gaps are Aborted and retained successes are Durable;
foreign or unissued tokens are NotFound. Success receipts do not expire.
Exhausting history, generation or issuance capacity refuses admission before an
unrepresentable transition; [Resources](resources.md#catalog-capacity) owns the
corresponding bounds.

A prepared query pins the catalog generation selected at preparation. New
writers build private objects and publish a later generation without changing
that query's source. Reclamation uses maintenance authority, preserves all
current and pinned data/history anchors, and neither issues an attempt nor
publishes a new generation. Failed cleanup or uncertain publication requires
exclusive reopen as specified by the public interface.

Declaration and append share private-object creation, synchronization, and
rollback in [`construction.rs`](../src/catalog_snapshot/construction.rs). Append
admission retains the selected table and reserves its owners before issuing the
transaction. A failed admission releases its buffers before classifying or
aborting the writer. After all dependencies are closed and synced,
`commit_prepared` transfers outcome authority to publication; builders must not
remove objects after that handoff.

## Snapshot ownership

A reader pins exactly one published generation, captures immutable catalog/unit
descriptors, and releases only after query buffers and results no longer borrow
it. Writers may publish newer generations. Maintenance may reclaim only objects
not reachable from current or pinned generations. Pin capacity and retained
generations are bounded with typed refusal.

## Follow snapshot pins and retained outcomes

Run [the snapshot example](getting-started.md#keep-an-old-snapshot-readable) with
[snapshots.rs](../examples/snapshots.rs) open. It commits amounts 10 and 20,
prepares an old reader, explicitly aborts an empty append, then commits 30.
The new reader sees all three values while the old reader keeps its original
two. It also resolves the two successful receipts and the aborted attempt after
reclamation, old-plan release and reopen.

Three public types carry different responsibilities in this flow:

| Value kept by the example | Responsibility |
| --- | --- |
| `PreparedQuery` | Borrows the database and pins the data generation chosen during preparation. |
| `Commit` | Copies a successful attempt's identity and published generation. The database's success history preserves the outcome. |
| `TransactionId` | Identifies an issued attempt whose outcome can be resolved, including an attempt that was explicitly aborted. |

Follow the write and lookup owners in order:

1. `begin_append_named` in [append admission](../src/catalog_snapshot/append/admission.rs)
   retains the selected table and reserves construction bounds. Only then does
   `Writer::issue` in [catalog_snapshot.rs](../src/catalog_snapshot.rs) publish
   the next issuance prefix while preserving the existing data graph. The empty
   append therefore has an identity before it writes any rows. Its successful
   explicit `abort` settles writer ownership and leaves an issued gap.
2. For a successful append, `write_batch` in
   [append.rs](../src/catalog_snapshot/append.rs) constructs private typed units.
   `build_commit` creates the next table index, catalog and success history,
   closes and synchronizes dependencies, then returns a complete candidate graph.
   `commit_prepared` validates that graph and hands outcome authority to the
   [shared publisher](#shared-publication-protocol). The returned `Commit`
   identifies the completed publication.
3. The old prepared query continues to borrow its original pinned catalog.
   `prepare` after the second commit selects the new catalog. The example's
   literal row checks exercise both views before and after reclamation.
4. `resolve_catalog` takes a short-lived resolution pin on the current registry
   view, checks issuance and active-writer state, then searches
   [success_index.rs](../src/success_index.rs). A recorded success returns its
   original generation. The known issued gap resolves as `Aborted`, even after a
   later success. Foreign or unissued identities still return `NotFound`.
5. `Reachable::open` in [reclaim.rs](../src/catalog_snapshot/reclaim.rs) captures
   data and history reachability separately. It preserves current data and
   prepared-query data pins, plus current history and active resolution pins.
   Dropping `old` releases its data pin. The copied `Commit` values impose no
   data pin, and their outcomes remain in the current success history.

The final close/reopen checks rows and outcomes through a fresh database handle.
This is a healthy sequential lifecycle. The [outcome contract](#outcomes) still
requires reopen and resolution after uncertainty; the example does not establish
crash recovery, concurrent scheduling or power-loss durability.

## Platform contract

On macOS, the durable-success contract requires ordered writes followed by the
documented full-device synchronization primitive (`F_FULLFSYNC`) for the
relevant file effects and directory/publication ordering established by the
production protocol. This premise is conditional on OS, filesystem, and device
honoring their documentation; no controlled-power certification is claimed.
Linux durability remains unqualified until its exact primitive/filesystem matrix
passes. Declared-table operations, legacy loading, and receipt resolution use
the same publication and recovery authorities on both native implementations.
Their Linux runtime tests include simulated effect failures and process death;
those checks do not establish power-loss durability or native failure coverage.
The [platform matrix](testing.md#platform-status) separates these scopes.
For a virtualized filesystem, the virtual disk's host synchronization policy is
also a premise; see the [verification contract](verification.md#linked-native-effects).
A guest flush acknowledged under a weaker host policy cannot establish the
full-device guarantee required here.

Every engine synchronization barrier performs one native call through the
private filesystem boundary. Darwin uses F_FULLFSYNC with no fsync/barrier-only
fallback; Linux's unqualified implementation uses fsync. EINTR, I/O failure and
unsupported operation return typed errors immediately, not a hidden retry loop.
The publisher and recovery preserve the distinction between cleanup debt,
required recovery, and a possibly committed transaction. A bounded call count
does not bound the time spent inside a kernel call or establish durable cleanup
after a failed flush.

## Creation

Fresh creation and repairing reopen have different authority. After writing and
synchronizing LOCK, the empty child directories, CONTROL, both roots and WAL,
[`finish_create`](../src/database.rs) calls
[`validate_created_namespace`](../src/namespace.rs). The validator uses the normal
independent readers to check the held lease, database identity and exact initial
state. Both roots and the fence must agree, the issued prefix must be zero, and
no pending roots or construction debris may remain. Unexpected state causes
failure and creation cleanup runs while the lease is still held. Before deleting
files, cleanup checks that the pathname still names the held LOCK identity. If
that check fails, creation returns `CleanupRequired` and leaves the files alone;
an open descriptor does not authorize deleting a replacement database.

The validator performs no recovery writes or synchronization. Creation still
synchronizes the database directory and its parent before returning a handle.
Those barriers make the already synchronized children reachable. Recovery's
additional barriers below remain necessary when reopening existing state, where
an earlier synchronization may have failed despite apparently settled bytes.

## Recovery

`recover_namespace` requires the database lease; inspection entry points cannot
repair files. Their shared coordinator, [`check_namespace`](../src/namespace.rs), runs
three phases: `read_namespace_authority` verifies the held lease, CONTROL,
roots, and fence; `validate_namespace_contents` checks the selected graph and
admissible construction names; then read-only callers require a settled
namespace while exclusive callers enter `repair_namespace`. The cleanup enum
distinguishes legacy loading debris from disposable scratch. Empty legacy
namespaces admit both name sets, with the scratch files separately required to
be empty and single-link. Published legacy and catalog namespaces admit only the
two scratch names. Live query inspection tolerates bootstrap names while writer
inspection and exclusive reopen retain their own checks. Validation failures precede
repair; repair failures become `RecoveryRequired` at the coordinator.

Recovery acquires the database lock, reads all authoritative root copies,
validates checksums, versions, lengths, issued prefixes, generations, and the
selected root/unit graph, then chooses only the newest provably published state.
It never silently opens an older valid generation when a newer known generation
is corrupt or unresolved. Generation 0 reconciliation, duplicate-root repair,
and generation-1 graph selection are implemented through closed bounded effects.
Qualification still requires the independent and stock-host campaigns below.
Reads are bounded before allocation. Corruption, unsupported format/generation,
recovery debt, and unresolved commit are distinct outcomes.

All namespace admission precedes destructive cleanup: validate file and
directory types without following symlinks, supported authoritative revisions,
bounded WAL length, root identity/agreement, and selected graph integrity. Two
valid roots establish their equal or adjacent published snapshots independently
of a damaged fence. With one invalid/missing root, an intact fence must match
the surviving root exactly; an ahead fence or missing evidence requires refusal
without cleanup. An intact fence must equal or legally succeed the selected
snapshot. It is not replay permission. Unsupported or foreign records fail
closed. Checksum-valid unknown or invalid semantic fields are not repairable
damage; checksum validation precedes that distinction. Repair writes are read
back and checked before replacement.

Recovery is idempotent in durable state, not merely visible bytes. A failed
barrier can leave a namespace that looks clean. Exclusive recovery makes both
root names durable BEFORE reclaiming unreferenced objects or resetting the fence
to the selected snapshot. It re-establishes the database-directory barrier even
without visible repair, verifies/syncs the matching fence, and syncs
units/private directories. A torn fence reset can then be retried using both
durable self-contained roots. Generation-one unit durability precedes root
publication. These barriers rely on the platform premise above; a
process-visible read cannot certify power-loss durability.

## Required evidence

Drive production write/sync/root/checkpoint/recovery transitions through narrow
fault effects and an independent abstract history. Exhaust bounded cuts, growth,
snapshot schedules, maintenance, and repeated failure/healing. Independently run
stock process-kill, APFS ENOSPC/quota, alias/lock, positional I/O, and sync
tests. Hardware power interruption is separate evidence and never inferred from
simulation.
