# Concurrency and cancellation

Declared tables use one serialized writer and concurrent snapshot readers. Query
steps run synchronously on caller threads; an engine worker scheduler remains
unimplemented. The target worker set is bounded. There is no row-lock manager,
multiwriter transaction graph or distributed scheduler.

## Authorities and locks

The writer owns transaction mutation until publication. Readers own immutable
snapshot pins. Query tasks own local operator state; shared state is introduced
only after local/shared transitions are explicit. Lock order is documented and
checked. No engine lock crosses blocking I/O, host callbacks, task entry, thread
join, sleep or allocation.

A long-lived reader/writer capability is an explicit state token, not a mutex
guard held across an operation. Generation publication is atomic according to
`transactions.md`; readers never observe mutable construction.

### Current ownership implementation

The database handle owns one exclusive cooperating-process file lease for its
entire lifetime. Queries and commit resolution use shared references. The legacy
`load_lineitem` API requires an exclusive mutable reference, so borrowed
prepared queries and results prevent a concurrent legacy load.

Declared-table declarations and appends use shared database references and
acquire the registry's serialized writer capability. Prepared queries pin
immutable catalog generations and may execute while a later append publishes.
Reclamation acquires maintenance authority and preserves current and pinned
generations. Registry guards protect finite admission/publication transitions
and do not remain held during I/O. Shared references do not grant concurrent
writer authority. Borrowed queries and writers prevent consuming
`Database::close`.

The process lease couples an acquired file lock with its opened device/inode
identity. Exclusive namespace recovery borrows that capability and checks the
current LOCK identity before repairing anything. Creation retains its lease
through failure cleanup; otherwise a peer can commit in the unlock/cleanup gap
and have acknowledged data removed. Ordinary regressions retain both that
schedule and a stale-descriptor replacement-namespace refusal. The lease does
not confer mutation authority over outside hard-link names: the in-place WAL
fence must be singly linked, checked at namespace admission and again on opened
readers/writers. This is not protection against arbitrary concurrent changes by
noncooperating filesystem users.

Shared queries and resolution inspect authoritative state without repairing it.
They return typed recovery debt when required repair is found. Publication,
maintenance and exclusive-open recovery own authoritative mutations. Grouping
and joins may create disposable scratch through the separate bootstrap
capability described in
[Storage](storage.md#catalog-namespace-format-7-scratch-ownership); scratch does
not grant publication or repair authority. Tests run Q6, Q1, and resolution on
real threads against a damaged replica and require refusal without
creating/replacing files; subsequent exclusive reopen performs the repair.

macOS pathname resolution keeps traversal/root-device/work state local to each
call. It adds no shared cache, callback or mutex. Canonical names are
observations of a namespace, not atomic snapshots or ownership capabilities;
opened identity and physical lease checks remain necessary. Cancellation is
checked around the whole synchronous resolution, not between its at-most-65,536
native helper calls. The call bound and fixed scratch do not imply a wall-clock
bound or make kernel calls interruptible. Linux's resolver and native-runtime
internals remain separate qualification obligations.

### Registry admission

The registry uses a fallibly initialized `filesystem::Mutex`. The mutex and
protected state occupy one stationary heap allocation. Admission makes one
nonblocking attempt; only an admitted writer may wait to finish a transition.
Guards cannot cross threads, I/O, callbacks, or allocation. Lock/unlock performs
no Rust allocation. Native initialization and lock errors return typed failures.
Unwinding poisons the protected state, so subsequent acquisition refuses it.
Progress requires finite holder work and host scheduling/acquisition fairness.
[Resources](resources.md#database-resident-ownership) owns the allocation
charge.

The active mutation is either a publication writer or maintenance. Publication
reserves a noncurrent free snapshot slot and an attempt; maintenance reserves
neither. Cleanup must remain possible when all slots are pinned or issuance is
exhausted. Both capabilities exclude another mutation.

Maintenance captures current and pinned graph anchors under the guard. Releasing
a pin during cleanup leaves conservative protection. New readers can pin
current, which is already protected. A failed cleanup barrier makes admission
unavailable until reopen. Dropping unfinished maintenance also makes admission
unavailable and performs no cleanup I/O. The inventory and deletion rules belong
to [Storage](storage.md#catalog-reclamation-scratch-and-recovery).

## Cancellation

Cancellation is cooperative. The public `CancellationToken` owns one monotonic
atomic requested flag and may be shared across threads; cancellation never
resets or implies completion. Checks occur before each at-most-64-KiB
input/staging call, before each at-most-256-KiB unit block call, before publication effects until the point of no return, and at declared row
and byte work intervals. An in-progress
bounded syscall may complete before cancellation is observed. Within an exact
byte request, each positive native transfer consumes remaining bytes; zero
progress or any error, including EINTR, terminates rather than retrying.
Cancellation is checked at the caller's declared boundaries, not between each
positive fragment of that bounded request. Synchronization makes one native
attempt per declared barrier; EINTR returns to the owning transition rather than
starting a hidden retry loop. This does not interrupt an in-progress flush or
establish a wall-clock deadline. Cancellation is not a timeout and is not proven
while work, I/O, worker ownership, or cleanup remains live.

Query cancellation releases execution buffers and disposable scratch. The
terminal result retains its handle reservation until drop; a prepared query
keeps its snapshot pin until it is dropped. Cancellation after completion
preserves the terminal completed outcome and performs no new work.

Writer cancellation before publication attempts abort and private cleanup;
failed cleanup remains explicit debt. Once publication may have begun,
cancellation cannot overwrite commit truth; the operation completes or returns
`CommitAmbiguous { transaction }`.

## Target scheduler state

A scheduler task is `Ready`, `Running`, `Blocked(reason,wakeup)`, `Finished`,
`Cancelled`, or `Failed`. One step advances a declared row/byte/chunk/effect
cursor, blocks, finishes, or fails. Queues, workers, and wakeups are bounded.
Scheduler fairness and healthy-resource premises are stated separately from
safety. These are target scheduler requirements, not current public enum
variants. The current `QueryStep` interface is specified in `interfaces.md`.

## Target backpressure and callbacks

A full queue blocks its producer on a named consumer wakeup. The producer may
retain only admitted state. Host callbacks run without engine locks, have
explicit reentry policy, and cannot borrow buffers beyond the call unless
ownership is transferred and charged.

## Required evidence

Exercise every supported publication, pin and cancellation transition with
deterministic schedules and independent real-thread tests. When a scheduler is
implemented, also test task transitions, queue saturation, worker failure and
healed liveness. Check publication versus readers, cancellation versus finish,
close with live owners and teardown. Race/sanitizer tools are required where
supported; a simulator does not establish compiler or hardware race behavior.
