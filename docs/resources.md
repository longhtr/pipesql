# Resource contract

One database memory authority and one database temporary-space authority govern
all production subsystems. Parsers, queries, caches, ingestion, maintenance,
adapters, and foreign runtimes may not invent shadow budgets.

## Admission protocol

[The resource authorities](../src/resources.rs) own shared counters and their
mutation. Callers request reservations and inspect admitted bytes through
methods; they do not manipulate atomics or reservation fields.
`MemoryAuthority::reserve` returns a `Reservation` whose drop releases its
charge. `split` transfers part of that charge without making it available to
competing callers, and `shrink_to` returns unused capacity. Temporary storage
has a separate authority with explicit release because unresolved files can
outlive the operation that created them.

For significant retained growth:

1. calculate logical bytes with checked arithmetic;
2. reserve from the responsible authority;
3. perform fallible physical allocation;
4. publish mutation only after allocation succeeds; and
5. drop the physical owner before releasing its charge.

Reject the next value before overflow, narrowing, allocation, I/O, or partial
publication. Accounts identify owner, purpose, unit, lifetime, revocability, and
cleanup state. Reservation transfer is explicit; blocked producers retain no
uncharged output.

One database `MemoryAuthority` uses checked atomic reservation, a 64-attempt
contention bound, and exact drop reconciliation. Configuration reports the
limits held by the two authorities rather than retaining duplicate limit fields.
Legacy lifecycle state and recovery requirements belong to [the lifecycle
contract](interfaces.md#legacy-lineitem-lifecycle); an unavailable handle can
retain cleanup debt even when no temporary bytes are charged.

The shared authorities charge logical ownership. The stock [composed-ownership
checks](../notes/evidence.md#resource-ownership-and-admission) reconcile these
charges with live requested allocations and separately observed allocator
rounding. Logical admission does not charge every allocator size-class increment,
so configuration is not a hard usable-heap or RSS limit. Reducing optional hash
allocation does not change that distinction. Closing this physical-memory gap
remains a release obligation.

## Interpret composed memory observations

Run `python3 -B tools/check-diagnostic-allocation.py --ownership-only` from the
repository root. The caller holds an ORDER BY reader, a DISTINCT reader, and an
append together. Its constructed input determines the complete old/new results.
Readers park at barriers while one owner changes; the caller compares their
separately observed allocation changes with the global live totals. It checks
allocation refusal, temporary-space refusal, cancellation, commit, completion,
and final release. These serialized transitions exercise overlapping ownership,
not arbitrary concurrent schedules.

The observer distinguishes three quantities:

- **Logical charge** admits an owner's allocations, inline state, and bounded
  scratch capacity before use. A reservation need not be a live heap allocation.
- **Requested bytes** are the sizes of live allocations through the caller's Rust
  global allocator. Database and caller allocations share this observer.
- **Usable bytes** are the allocator-reported extents of those same allocations.
  Their difference from requested bytes is allocator rounding, not engine data.

For this numeric workload, the caller checks the following equations. `R` is
an owner's live requested heap bytes, and `P` is the byte length of its canonical
`database/units` path. The pathname bounds are 4,096 bytes each.

| Owner at the checkpoint | Logical charge |
| --- | --- |
| Prepared ORDER BY | `R + size_of::<PreparedQuery>() + 4096` |
| Prepared DISTINCT | `R + size_of::<PreparedQuery>() + 2 * 4096` |
| Append after writing | `R + size_of::<Append>() + 8192 + 3 * 16384` |
| Reader parked after its first spill | `R + size_of::<QueryResult>() + 4096 + 8192 - P` |
| Reader after completion or cancellation | `size_of::<QueryResult>()`, with no retained heap allocation |
| Dropped owner | Zero |

Preparation retains one plan allocation; DISTINCT also retains a descriptor
allocation. Each receives its documented allowance. Append paths are transient
at the sampled point; its catalog/schema, reference array, and reusable workspace
each retain the [qualified rounding ceiling](#streaming-append). A running reader retains one exact-capacity source path;
its source account covers two maximum paths, and its physical plan has a separate
allocation allowance. These equations are specific to the named fixture and
checkpoints. Aggregates, computed expressions, and other producer graphs add
their own owners. The caller does not infer their charges from this table.

The parked baseline includes the database, prepared plans, and caller thread and
synchronization state. Completion releases engine allocations before the caller
drops its barriers and observation mutexes. The caller measures synchronization
release separately and requires the original baseline after closing the database.
A negative control changes a preparation allowance by one byte and must fail
attribution; a separate control rejects an incorrect complete query result.

Logical allowances do not guarantee that usable allocations fit under the same
limit. The native allocator can round a request beyond its logical allowance.
The observer also excludes direct foreign allocations, allocator metadata and
retained free pages, mapped or resident stack, and other process mappings. Parked
samples do not measure transient peaks. `/usr/bin/time` reports process-level
observations separately; its high-water mark cannot be assigned to one query
owner or equated with the sum of these checkpoints. See the
[current measurements](../notes/evidence.md#composed-query-owners) for executed
platforms and concrete limits.

## Query preparation

Computation descriptors retain their logical count separately from allocated
capacity. Large vectors use the existing native buffer-capacity geometry, rounded
down to whole descriptor slots. Admission includes every requested slot; the
independent allocation allowance remains 4,096 bytes per retained vector.

Prepared queries reserve their handle, immutable heap-owned plan, and any
aggregate descriptor, aggregate entry and computation vectors. Each allocation
includes a 4,096-byte allocator allowance. Running queries borrow that plan.
`PreparedQuery::accounted_memory_bytes()` reports its actual charge;
`memory_requirement_bytes()` is a conservative bound, not an exact query
minimum. Parser stacks and caller source remain separately bounded and observed.

Query preparation releases catalog scratch before binding the semantic plan. Its
peak is the larger of catalog scratch and retained-plan plus transient-scope
charges. Queries with multiple source occurrences reserve two exact-capacity
scope vectors before binding, including a 4,096-byte allowance per allocation.
Their capacities follow the shared source, projection and aggregate pools;
pending inputs do not allocate a maximum-width scope for every nesting level.
Scope storage drops before preparation returns.
`PreparedQuery::memory_requirement_bytes()` bounds retained ownership, not the
complete preparation peak. The derived-query admission fixture independently
checks both the public peak and isolated scope allocation at their exact limits
and one byte below.

## Legacy scan and aggregate admission

For the legacy format-4 source, before namespace I/O execution reserves the
demanded scan buffers, metadata, selection, both typed batches, physical
pipelines, the concrete result-handle size and aggregate state. The scan
workspace equation is 106,496 fixed bytes plus demanded native-column arena
bytes and the two typed batches. Each column has one 256-element payload and
four validity words; the descriptor vector is included. The arena retains at
least 28,672 metadata bytes. Missing columns add no native buffer. Source
buffers and selection retain the workspace reservation. Each `OwnedBatch`
receives its exact payload bytes by splitting that reservation without changing
the authority's total. The sum of source and batch reservations remains the
admitted workspace total; handle storage is covered by the enclosing owner.
Selection has 4,096 u32 entries. Aggregate state owns compact arrays by semantic
type: `groups * double_states` f64 values, `groups * integer_states` i128 values
and `groups * nullable_states` u32 nonnull counts. Group row counts always
occupy `groups` u32 cells; a group flag word is needed only when DOUBLE states
exist. Identical demanded expressions share one state. Nonnullable expressions
reuse row counts, so Q1 needs no integer storage or nullable counters. The
expression stack depth times admitted lanes times eight bytes determines scratch
payload. Zero numeric states require no scratch. Lanes shrink to one before
typed refusal. The scalar call also uses 1,024 fixed stack bytes for 32 validity
bitmaps, plus its type stack. At most 128 borrowed numeric-input descriptors
carry identity, payload and validity references; they allocate nothing and die
after consumption. The result carries four validity words and borrows the
existing scratch payload.

## Runtime and result admission

Runtime admission reserves an exact-capacity node vector before aggregate sizing
or source I/O. Its capacity equals the physical pipeline count, bounded by
seventeen. Nodes own their output batches and, for scans, their cursor state.
The native source account owns the reader allocation, paths, selection and
payloads; it excludes the inline admission workspace. That workspace's retained
fields live in the charged runtime nodes. The public scan sizing ceiling
includes the maximum runtime vector. Finish, failure and drop destroy the nodes
before releasing the vector reservation. All arithmetic is checked before
reservation and physical allocation.

Workload-specific charged-byte observations live in result records; they are
neither universal minima nor whole-process bounds. Exact-next admission,
one-lane fallback, allocation refusal and reconciliation remain ordinary tests.
`Database::execution_memory_requirement_bytes()` covers the larger of legacy and
declared scan/result admission, including overlapping catalog scratch. It
excludes database, prepared, aggregate, join, ordering and additional
producer-output ownership. The declared ceiling uses 64 maximum native column
buffers and two maximum-text batches; it is not a per-query minimum. The
physical plan separately reserves its exact pipeline vector capacity plus a
4,096-byte allocation allowance before allocation. There are at most seventeen
producer pipelines under the current normalized-node bound. The scan sizing
ceiling includes that maximum plan allocation. Finish, failure and drop destroy
the vector before releasing its reservation; the terminal result keeps only its
handle charge. Exact-limit tests include the plan in the complete equation and
retain their one-byte-short refusal checks. Multi-producer runtime admission
reserves all sources, outputs, join owners and catalog scratch before optional
grouping growth. The scan sizing helper is not a bound for an arbitrary producer
graph. Unused source columns allocate no payload buffers. Earlier layout
measurements are retained in the [semantic-input
study](../notes/evidence.md#query-semantics-and-accepted-costs); they do not
measure the current physical plan or establish allocator/RSS bounds.

Intermediate batches and row layouts admit up to 128 values: at most 64 visible
columns and 64 original values retained by qualified ranges. Actual batch charges
follow demanded types and widths; native source buffers still follow the 64-column
schema bound. Slot, layout, and scratch arrays use the larger internal capacity
and remain included in their owners' charges.

Result steps reuse admitted data buffers. Bounded native-unit and scratch
pathname construction may allocate from memory reserved before execution. All
workspace/group owners are destroyed on finish, failure or drop before releasing
their account. A terminal result retains its handle-size reservation until
dropped; a prepared plan retains its own charge until dropped.
`QueryResult::memory_requirement_bytes()` includes the target-specific handle
charge and the per-step row-computation scratch allowance. It excludes
separately owned physical-plan, workspace, and aggregate allocations. The
scratch charge is released at completion or failure; the terminal handle retains
only its own size.

The query's accounted-memory report includes every live hash text reservation.
During growth this includes both the retained arena and its replacement; copying
must finish before the old allocation and reservation are released.

A text batch keeps its String and span-vector handles inline in the column.
Its separately allocated 256 start/end span pairs occupy 2,048 bytes; the text
arena retains its admitted capacity. This avoids a per-column singleton heap
object combining the spans and String handle, whose 2,072-byte request occupied
2,560 usable bytes on the measured macOS allocator. Batch admission includes the
column metadata, actual span capacity and text capacity. Sparse writes and
replacement retain independent start/end spans until clear; neither owner grows
during result steps. Physical owners are destroyed before their reservation.

## Declared scan admission

Declared-table source admission reserves the reader owner, bounded paths,
selection, demanded payload buffers and result columns through the database
account. Encoded column limits include validity and text offsets. Admission
uses the [large-buffer capacity policy](#blocking-buffer-capacity):

| Demanded column type | Maximum encoded bytes | Allocated payload capacity |
| --- | --- | --- |
| INT64 or DOUBLE | 266,240 | 278,496 |
| DATE | 135,168 | 147,424 |
| STRING | 524,288 | 540,640 |

Validity, offsets and value bytes retain their encoded limits. Admission includes
the actual physical capacity before source I/O; missing columns allocate nothing.
The maximum scan workspace includes 540,640 bytes per demanded payload. This is
16,352 more bytes per STRING column than the previous physical limit, while
fixed-width payload requests decrease by 32 bytes. The encoded STRING limit
remains 524,288 bytes. These are capacity/accounting changes, not throughput or
RSS improvements.

Payload buffers are allocated once and reused across units. Padding is never an
encoded value: the existing metadata and payload validators still enforce the
stored length and canonical representation. The ownership campaign exercises
ORDER BY and DISTINCT with INT64, DOUBLE, DATE and STRING at one and 64 columns.
Short and 384-byte pathnames cover complete nullable rows, requested/usable
attribution, and release on the qualified
stock macOS and GNU/Linux allocators. These checks and the composed-reader
checkpoints retain the equation above; they do not qualify custom allocators,
every producer graph, transient allocation peaks, or process/RSS bounds.

## LIMIT admission

Each limiter owns one output batch and inline counters in its runtime node.
Declared STRING columns reserve the same 65,536-byte per-column ceiling as
source batches. Legacy queries without text constants retain fixed-key storage;
when a text constant is present, each STRING output reserves 8,192 bytes
(256 rows × 32 literal bytes). This query-wide bound survives copies, grouping
and later producers, including when a constant becomes undemanded. Output reservation precedes optional aggregate growth and source
I/O. Selecting zero rows does not bypass these admission obligations. Source and
temporary-file owners retained by a completed interior limiter remain charged
until replay or query cleanup releases them. LIMIT introduces no separate memory
account or spool.

## UNION ALL admission

Each binary union retains one positional descriptor in the prepared plan. Its
vector capacity is admitted from the parsed union count, with one allocation
allowance for the vector. Output identities belong to positions; mapping the same
left identity twice does not merge distinct right-input values.

Each runtime union owns inline branch state and slot mappings plus one output
batch. Declared STRING outputs reserve the same 65,536-byte per-column ceiling
as source batches. The controller copies at most one admitted input batch per
step and introduces no spool or separate resource authority. Both input producers
and their minima are admitted before source I/O or optional aggregate growth.

The scheduler retains both children until query cleanup. Replay resets visited
branches when they are next requested; an unvisited aggregate must remain fresh.
A downstream LIMIT can leave later branches unvisited, but does not avoid their
admission. Branch sorting, grouping, and joins retain their own temporary-space
and replay obligations. Completion, cancellation, failure, and abandonment release
the query's retained owners through the existing result lifecycle.

UNION DISTINCT normalizes to the same binary union owners followed by one
ordinary DISTINCT owner for the complete argument list. The extra parsed stage
admits its descriptor, sorted input, and output through the existing equations
below. It can require temporary storage even when UNION ALL would stream. All
comparison fields remain demanded regardless of later projection. It adds no
new resource account, spool implementation, or replay authority.

## Join, ordering and DISTINCT admission

DISTINCT uses one complete-row sorted-input owner with every unique field as a
key. Its prepared descriptor vector charges the exact stage count times the
host's descriptor size plus one 4,096-byte allocator allowance; queries without
DISTINCT allocate no such vector. Demand and producer slot arrays follow the
frontend's conservative identity ceiling, including sixteen full-width identity
replacements. Those arrays remain part of ordinary physical-plan admission.

Each join owns two sorted inputs and one producer output. Standalone ordering
and DISTINCT each own one sorted input and one output. All use the same
admission equation. Let R be the 32-byte frame header plus the schema's maximum
typed row bytes, C its field count, and K its maximum encoded key bytes. The
side reserves one capture record of R bytes, two merge records of R bytes each,
three 65,536-byte I/O buffers, a K-byte previous-key buffer, and a run buffer of
`R + 255 * (32 + C)` bytes with two arrays of 256 record spans. Inline
controller and sorter storage is charged once. Each side also reserves the
ordinary scratch creation pathname allowance before execution. Released run
buffers and completed scratch creation return their charges; retained final runs
keep their temporary extents charged. Source payloads and producer outputs have
separate owners in the same database account.

The run admits at most 256 rows and stops sooner when its byte capacity fills.
It always fits one maximum row. Each input has the shared 134,217,728-row bound;
skew changes duplicate-output work, not retained group memory. Temporary space
is reserved before writes, and rereading a duplicate group reuses existing
buffers. There are four scratch file descriptors per join, under the physical
producer-count bound; standalone ordering and DISTINCT each own two. All source,
join, ordering and DISTINCT minima are admitted before an aggregate can consume
optional memory. Ordering retains its final run for a possible grouping replay
and charges its unique sort-key prefix once, even when the query repeats an
ordering identity.

Exact-admission tests check the complete equation, refusal one byte short before
I/O, and account reconciliation through public steps. A fixture's successful
configured limit is not a universal minimum for other schemas or producer
graphs.

## Blocking buffer capacity

Declared payloads, sort frames, run bytes, prior keys and hash lookup keys use
`resources::buffer_capacity`. Above 16 KiB, it chooses the smallest capacity at
least as large as the requirement that ends 32 bytes before a 16-KiB boundary.
Smaller buffers retain their exact requested capacity. Run-span widths must
divide the resulting byte capacity. Every owner reserves the actual capacity
before construction; no extra rounding allowance is added to the equation.

The space before the boundary accommodates native allocation headers/alignment.
GNU libc's mapped chunks otherwise can acquire an extra page for a request
ending exactly on that boundary; its arena and mapped paths have different
usable extents ([glibc 2.36 allocator source](https://raw.githubusercontent.com/bminor/glibc/glibc-2.36/malloc/malloc.c)).
Darwin rounds the tested requests back to their 16-KiB class. The native caller
checks the complete buffer-size range independently. GNU/Linux reader campaigns
also fix low and high mmap thresholds in fresh processes and require an observed
allocation-path control before checking all reader shapes.

Encoded limits remain separate. A sort frame retains its original encoded-byte
limit, and a run retains its original byte and row limits. Padding cannot admit
another row or a larger frame, including a valid checksummed frame. This capacity
policy reduces native allocation-class tails; allocator-usable extents still
require the stock caller's observations and are not a general process-memory
bound.

## Declared grouping admission

Declared grouping calculates immutable aggregate layout before allocation. Its
minimum includes the complete native source and output batches, transient
catalog scratch, one scalar lane, one captured argument row, one maximum-width
run record, two run spans, two merge records, three 65,536-byte I/O buffers, one
prior key, one final-result frame, controller fields and 8,192 bytes for scratch
paths. The final frame is sized from projected types, including repeated text
keys. Every retained field and allocation capacity belongs to exactly one of
these owners; AggregateState charges its arrays while its enclosing owner
charges its inline fields. Count-only arguments own nullable counters where
needed, but no typed sum cells. An identical numeric argument shared with
SUM/AVG owns one sum state. Sharing with MIN/MAX retains the argument value;
COUNT-only STRING/DATE arguments capture validity only.

Each demanded MIN/MAX slot owns an eight-byte word per admitted group. Numeric
slots store value bits; text slots store lengths and own separate reusable byte
storage. The fixed accumulator reserves 65,536 bytes per declared STRING slot
per group, including legacy queries that introduce UTF-8 constants; legacy
fixed-key-only queries reserve one byte. Admission and independent validation
receive this query text domain explicitly. Disk reduction reuses one
fixed group's slots.

Optional hash grouping stores explicit byte spans for STRING extrema. Each
span's capacity is a power of two, bounded by the source width. Replacements
reuse the span when they fit; a larger value receives a new region. Because
capacities grow geometrically and never shrink, superseded regions total less
than the current region's capacity for each slot. Repeated equal or shorter
replacements consume no additional arena space.

Before folding captured rows, hash grouping admits a conservative bound for all
possible region growth in the batch. Arena growth charges the entire new buffer
while the old buffer remains live. Each copying step moves at most 65,536 bytes
and checks cancellation. The old buffer is destroyed before its charge is
released. This temporary overlap can raise peak reservations above the fixed
representation for wide values. Admission refusal destroys optional state and
replays the pinned source through the retained disk minimum. It does not turn
partially updated hash cells into output.

Captured STRING arguments each reserve a 65,536-byte arena per batch, in addition
to eight-byte row spans and validity masks. The minimum includes these arenas
even with one admitted row. A spill frame contains its header, encoded keys,
eight bytes per argument, and retained text bytes. Its maximum trailer is
65,536 bytes times the number of retained text arguments. Replay flushes before
an append exceeds a text arena or the row capacity, so one full-width row always
fits. Folding does not allocate per row. Fixed extrema reuse their admitted
slots; hash extrema use the bounded region-growth protocol above.

Available memory first increases captured arguments up to 256 rows. Run row and
encoded-byte limits then grow together up to 4,096 slots, reserving the maximum
first record and the minimum encoded width for each additional slot. Admission
selects the largest row limit whose rounded buffer allocations fit; padding does
not consume the independently retained fallback minimum. Scalar lanes use
the remaining budget up to 256. Optional hash storage starts with half the
remaining capacity for group slots and rounds the slot count down to a power of
two. Large cell arrays request power-of-two physical capacities while retaining
exact logical state lengths. Key slots use 16-byte alignment so their width is
also a power of two; that padding is part of the charge. Sizing halves the group
count further if those physical arrays would exceed the metadata half-budget.
Buckets retain their power-of-two bound. Above 16 KiB, the available key budget
rounds down to a power of two. The encoded-key limit is then capped at the bytes
needed for all admitted groups; its allocation rounds up to a power of two
within that budget.
The query reserves and requests those physical capacities before allocation;
unused capacity cannot admit more groups or state lanes. Fewer hash slots or key
bytes can cause earlier fallback; this is a capacity policy, not a throughput
guarantee. When STRING extrema are retained, metadata admits at most 4,096
groups; larger cardinalities use the external path. Numeric-only hash sizing
retains its row-bound ceiling. Text arena allocations are admitted as needed
rather than reserving maximum-width text for every metadata slot. The encoded-key limit
is also bounded by the group-slot capacity multiplied
by the maximum encoded key width: each occupied slot stores one key, and a full
slot array already forces fallback. Additional encoded-key space cannot increase
the admitted workload; physical allocation padding is charged separately. These are sizing policies, not distribution or performance
guarantees.
Admission may refuse if concurrent reservations change availability. After
admission, replay, sorting and reduction never reacquire their retained minimum.
Hash storage and the run collection arena are destroyed before their charges are
released.

The public exact-minimum test includes transient catalog scratch and compares
retained fields plus allocation capacities with charged bytes. One byte less
refuses before any query effect. Logical accounting does not bound allocator
overhead, thread stacks, filesystem caches or whole-process RSS.

## Memory classes

Account separately for operator minima, optional query growth, retained results
and snapshots, database state, ingestion and maintenance, and caller/host
buffers. Allocator metadata, thread stacks, executable mappings, foreign
runtimes, and kernel page cache require their own bounds or observations.
Returning live engine allocations to baseline does not imply that allocator
pages or RSS return to baseline. The [physical-memory
counterexample](../notes/evidence.md#resource-ownership-and-admission) remains
an open release obligation.

### Diagnostics and expression state

`ErrorCause` stores contextual leaf facts inline; error construction does not
allocate recursive boxes or cleanup strings. Arithmetic errors retain an inline
`SourceSpan`, not query text. Native I/O diagnostics render the error kind and
exact OS code without allocating an OS message. Opaque custom formatting and
caller-owned strings remain separate owners. Retaining an error does not release
unresolved temporary-file charges.

Text literals retain at most 32 UTF-8 bytes in the semantic plan,
covered by its ordinary charge. Physical filters borrow their predicates and
literals. Decoding and comparison introduce no heap owner or per-row allocation.
Projection constants share the admitted computed-descriptor allocation with
numeric expressions. Their values bypass numeric scratch buffers; materialized
STRING outputs use the batch capacities above. DATE shifts fold during binding
with the same calendar checks as predicate constants.
Boolean controls occupy four bytes per logical leaf; copied physical controls
belong to pipeline admission. Temporary Boolean syntax arrays are bounded by 160
tokens and do not survive preparation.

Optional scan-branch scratch reserves five bytes per quantum row plus two
4,096-byte allocator allowances: 28,672 bytes for direct scans or 9,472 bytes
with computed work. Linear AND scans allocate neither vector. Actual admission
and the public conservative execution ceiling include this scratch. Both vectors
drop before their reservation.

### Computed scan workspace

A scan with demanded computations uses 256-row work quanta. Its compact
workspace contains only needed numeric source/definition buffers, each with 256
raw payload words and four validity words, plus the maximum required
expression-stack depth. One fallible allocation is reserved through the scan
account, including its 4,096-byte allocator allowance. The conservative
workspace ceiling is 369,152 bytes; actual admission uses the dependency count
and stack depth. A direct scan retains its existing 4,096-row selection quantum
and allocates no such buffer. Every evaluation resets readiness for its
selection; source replay also invalidates cached values. Batch output owns
copies, never references into this scratch.

### Native paths, stack, and I/O

Path construction uses checked, fallible allocation and bounds each resulting
path and capacity to 4,096 bytes. Native arguments use 4,097 stack bytes per
name, or 8,194 for dual-name operations. Canonicalization also holds a fixed
output buffer before fallibly allocating its returned `PathBuf`. Directory
readers borrow an 8-KiB stack buffer and own one descriptor.

The macOS resolver additionally uses fixed 1,024-byte path slots, a 1,052-byte
native name record, and bounded scalar metadata scratch. A suffix cursor avoids
input-controlled recursion and repeated suffix shifting. At most 33 symlink
expansions refill that suffix.

The Linux resolver traverses components using `lstat` and reads each encountered
symlink once. It allows forty symlink expansions; the next returns `ELOOP`.
Its fixed output, pending-input, and link-target arrays total 12,290 bytes.
This array total excludes scalar state, compiler temporaries, and caller frames.
Each link target is bounded to 4,096 bytes. A target exceeding that bound returns
`ENAMETOOLONG`.

Expanded pending suffixes may exceed the final pathname limit. Their capacity
is bounded to `(40 + 1) * 4096 = 167936` bytes. The engine's pathname scratch
owner admits overflow through the database memory authority before allocating.
Growth uses power-of-two capacities capped at that bound, with a 4,096-byte
allocator allowance per buffer. Admission includes old/new buffer overlap during
copying; physical storage drops before its reservation. Ordinary paths need no
overflow allocation. Allocation refusal propagates without a partial result or
an unaccounted fallback.

Both resolvers admit at most 65,536 native helper calls, including macOS's
optional mount-name and prefix checks; the next call returns
`Resource { owner: "pathname native calls", required: 65537, limit: 65536 }`
before native entry. This bounds admitted calls, not kernel work or elapsed
time. Scratch remains private and no partial resolved path escapes.

Bounded-thread regressions qualify exercised operations against a native-reported
thread extent. They request 48 KiB on each target, then check the actual size
inside the worker before entering the scenario. The private test helper owns
these constants; production code neither creates these workers nor changes a
caller's stack.

| Target | Maximum reported extent | Rationale |
| --- | --- | --- |
| macOS and other existing targets | 64 KiB | Retains the existing envelope; macOS reports 61,440 bytes for the 48-KiB request. Other targets require their own runtime evidence. |
| GNU arm64 Linux | 144 KiB | The native minimum is 128 KiB; at most 16 KiB more is permitted for runtime/TLS and alignment overhead. The reviewed Rust executable reports 137,152 bytes. |

The Linux ceiling explicitly replaces the impossible 64-KiB native-thread
premise. It does not establish that Linux engine frames fit within 64 KiB.
Runtime changes that exceed the allowance fail qualification. The independent
native control checks the minimum and an oversized thread; the Rust helper's
negative control verifies that a real 2-MiB thread is rejected.

Requested size, native extent, usable call-stack space, live frames, guard pages,
VM mappings, and resident memory are different quantities. Native reports do not
measure maximum live frames or isolate runtime/TLS storage. Passing scenarios
establish execution within the recorded target envelope, not every call path,
a guaranteed caller-stack minimum, or a whole-process memory cap. Native and
failure-phase attribution remain separate from logical reservations.

A separate Linux pathname test requests a 128-KiB thread and checks a 144-KiB
reported ceiling during expanded-path creation, refusal, and reopen. Its scope
remains pathname handling; it does not replace the other operation scenarios.

Synchronization borrows an existing file, allocates no buffer, and makes one
native attempt using the platform's required durability primitive. Interruption
returns to the caller without an implicit retry or weaker fallback. Exact byte
I/O borrows already-admitted buffers and descriptors; it makes at most the
initial nonempty byte count in native attempts. Each successful call consumes at
least one byte; zero progress or any error terminates. Positional extents must
fit the native signed 64-bit range before the first call. No pathname
marshalling allocation occurs after entry into link, rename, or deletion.

These bounds do not limit kernel-call latency, foreign-runtime memory, or
process RSS. The [native evidence
record](../notes/evidence.md#native-boundaries-and-diagnostics) retains
maintained allocation-refusal controls, stack distinctions, and unresolved
attribution. It does not enlarge the contracts above.

## CLI startup owners

The CLI captures at most 12 native entries (program name, operation and five
option/value pairs). It ignores program-name contents, bounds every other entry
at 4,096 bytes before fallible copying, and uses an inline 12-slot argument
array. At most 45,056 argument bytes are requested; valid dispatch retains at
most two 4,096-byte path owners. Parsing transfers these owners and allocates no
error text. Resolve transfers a validated 24-byte transaction token into its
command variant; hex decoding uses a fixed array and allocates nothing. It needs
no extra path owner or larger native argument allowance. Each command variant
owns exactly its operation-specific input. Resolution uses ordinary
open/recovery and read-only resolution under the database's existing resource
accounts. These are separate bounded CLI startup owners, not a second database
budget. macOS reads CRT arguments only at private executable startup, before
threads or callbacks; linked code must not mutate argv during capture. No
foreign pointer escapes. Linux instead reads `/proc/self/cmdline`, with a 4-MiB
total work cap including ignored argv[0], and refuses inspection failure without
fallback.

Sinks are duplicated into owned descriptors before engine entry (at most two
extra fds). Stdout has a 1,024-byte inline line buffer; diagnostics use 4,096
inline bytes. Writes advance a byte cursor or return, including on interruption;
drop never flushes. Unavailable stderr preserves the command exit status without
a secondary sink. Rust startup sanitizes shell-closed standard slots to
`/dev/null`; closure after bootstrap is a distinct tested effect. Neither sink
uses lazy stdio heap state. The ordinary gate covers allocation prefixes for
create/open/load/query and durable/aborted/unknown resolution, including
repairing open. It checks allocation-free parser control/refusal pairs, with
requested Rust owners and descriptors restored after entry returns. Exact counts
belong to the current result record, not this resource contract.

CLI capture does not release the original process argument/environment storage.
Runtime bootstrap, resident stack, allocator-retained pages, and foreign-runtime
allocations remain separate owners. The [native evidence
record](../notes/evidence.md#native-boundaries-and-diagnostics) retains stock
argument-padding and heap/residency counterexamples. The macOS and GNU/Linux
gates exercise capture/decoding, publication, allocation refusal, and native
sink failure. Their recorded cases do not establish a whole-process memory cap
or qualify every runtime and filesystem.

## Temporary and persistent bytes

Spill/import bytes are reserved before writes. Each extent has a query or
transaction owner, integrity metadata, and bounded cleanup after cancellation,
I/O failure, or process death. Fan-out, run count, retry depth, and retained
cleanup debt are finite. Persistent free-space accounting is not confused with
the memory account.

For the public load's retained native geometry with validated row count `r`,
seven staging streams contain exactly `38*r`, unit content is `28,672 + 38*r`,
and peak charged temporary content is `28,672 + 76*r`. The 6,500,000-row
admission maximum therefore requires 494,028,672 temporary bytes. Filesystem
metadata and allocation-unit overhead are observed separately. One fixed load
arena requests 1,725,440 bytes and is charged 1,867,776 bytes: a 1,802,240-byte
allocator-capacity ceiling plus 65,536 bytes of fixed engine stack/metadata
headroom. Construction reuses charged arena regions for encoded metadata and
readback and decodes records into caller-owned fixed slots to avoid large
by-value stack temporaries. Namespace scanning releases its 8-KiB scratch frame
before unit-metadata validation. Load, Q6 and Q1 share a public small-stack
regression with native size observation; its retained
[counterexample](../notes/evidence.md#native-boundaries-and-diagnostics)
explains why requested thread size alone is insufficient. Local stack success
does not establish total stack/allocator attribution. Reservation precedes
namespace inspection/recovery as well as `try_reserve_exact`; reported capacity
above the charged ceiling refuses before private files exist. The implemented
load reserves that full memory charge before allocation and rejects observed
arena capacity outside 1,725,440..=1,802,240 bytes before input or private-file
work. After pass one determines `r`, it reserves exactly the checked temporary
peak before creating staging. Loads must enforce that admission during
construction: each staging writer keeps the first-pass row allowance and refuses
the next row before changing any column. Each flush checks its next byte extent
against the admitted rows times the column width before I/O. These are
subdivisions of the reserved temporary peak, not new accounts. Final
source-identity and CRC comparisons remain required. Memory is released after
its physical owners are gone. The temporary reservation is released after
confirmed rollback has removed construction state, or successful publication has
transferred the unit to the persistent generation. Moving a unit into `units/`
alone does not establish that transfer.

An ambiguous publication or cleanup failure retains the full original temporary
reservation on the unavailable database handle. Further construction is refused
before effects. Closing consumes the handle and releases its lease; it does not
resolve the outcome or remove the remaining files. The bounded on-disk namespace
retains the recovery obligation after close or process death. A fresh open
exposes a zero-reservation handle only after exclusive recovery confirms the
persistent graph and completes required orphan cleanup and synchronization.
Failed recovery exposes no usable handle. `reserved_memory_bytes` and
`reserved_temp_bytes` report live handle reservations, not filesystem usage or
process RSS. Issuance failure before construction can require recovery without
retaining a temporary charge.

## Streaming append

An append reserves its handle and two bounded paths, a 65,536-byte
catalog/schema admission buffer, and `(existing units + batch limit) *
size_of::<UnitRef>()` bytes for references. The sum of existing and permitted
new units cannot exceed 4,096. No caller input is retained between writes.

Encoding and commit reuse the same workspace in separate phases. Its requested
size is `max(metadata bytes + largest encoded column bytes, 65536)`, at most
526,400 bytes. Commit needs the 65,536-byte minimum for catalog/history construction;
those bytes are not added to the earlier encoding requirement. It is reused when
large enough. Growth drops the old
buffer and its charge before reserving a larger one; a failed allocation leaves
the stream abort-only and its earlier private files owned. Construction buffers
are released before publication validation allocates its scratch.

Before allocating or issuing the transaction, append admission also reserves
16,384 bytes for allocator rounding on each of its three retained allocations:
catalog/schema, references, and the workspace. This is a scoped native-allocator
premise. The ownership campaign checks all 460,865 workspace sizes from 65,536
through 526,400 bytes and all 4,096 reference counts (32-byte entries), including
return to the allocation baseline after each case. The tested Darwin allocator
can add 16,383 bytes to a workspace and 16,352 to a reference array; the tested
GNU allocator also fits the ceiling. A control without the ceiling must fail.
No allowance is silently borrowed from another owner or subtracted from observed
usable memory. Adding a retained allocation or overlapping workspace growth
requires revisiting both admission and the independent caller.

The full-width public case combines maximum encoded-column size with reference
counts at a Darwin size-class crossing and at the 4,096-unit limit. It checks
small/large/small writes, exact charges, usable bytes within the append charge,
publication results, and release. Internal tests reject one-byte-short admission
before allocation/issuance effects and check exact growth after releasing the old
workspace. The ceiling is qualified for the exercised stock allocators and size
ranges; custom global allocators, untested platforms, allocator metadata/retention,
and transient whole-process peaks are not covered by those observations. Other
engine owners retain their separately documented physical-memory limitations.

The upfront temporary reservation is the encoded-data limit plus the maximum
resulting table index (`64 + 48 * unit count`), unchanged catalog extent, next
success history (`64 + 8 * generation`), and one 4,096-byte root. A successful
commit or durably completed abort releases the reservation. Failed cleanup keeps
it on the unavailable database, including when files are visibly gone but the
directory sync failed. At most 4,099 private objects belong to one append.

These are logical reservations and scoped stack checks, not a whole-process
memory or ingestion-throughput qualification.

## Backpressure and refusal

Admission may reduce concurrency or return typed resource refusal. It cannot
silently violate semantics, use unbounded fallback, or wait forever without a
named wakeup and fairness premise. Every supported blocker makes progress under
a forced cap or refuses before execution if its declared minimum is unavailable.

## Catalog capacity

The catalog admits 64 tables, 64 columns per table, and 4,096 native units per
table. Each unit admits 32,768 rows, so a table cannot exceed 134,217,728 rows.
Native columns admit at most 524,288 encoded bytes and each STRING cell at most
65,536 bytes. Schema names admit at most 32 bytes. These pre-release limits
bound metadata validation, reference storage and batch work; exceeding them
returns a typed refusal before the corresponding value or extent is published.

The registry has four generation slots shared by current state, pinned queries
and receipt-resolution views. Writer admission needs a noncurrent, unpinned
slot; otherwise it returns a Resource error before claiming writer authority.
Each slot admits at most eight data pins and eight resolution pins. Pin
admission tries at most eight atomic updates, returning Resource at capacity or
Contention after retry exhaustion. Transaction issuance is bounded by u64, and
success history by 1,048,576 generations. Exhausted issuance or success capacity
returns Unsupported before issuing another attempt. Success-history capacity
never permits old receipts to expire.

These limits are owned by `catalog`, `catalog_schema`, `native_unit`,
`catalog_snapshot` and `storage_format`. The corresponding codec, admission and
snapshot tests retain exact/next-boundary checks. A larger limit requires
rechecking its downstream byte, work and arithmetic bounds.

## Database resident ownership

The database retains its fallibly allocated canonical pathname. Create/open
reserve 4,096 bytes before allocation, shrink the reservation to the actual
capacity after construction, and transfer the charge with the path. The path
cannot grow after publication and drops before its charge. An idle legacy
database therefore has a nonzero resident memory charge. Handle layout is
target-specific; moving the handle does not copy a maximum-length inline path.

The catalog registry reserves `size_of::<Registry>()` plus
`filesystem::Mutex::<State>::allocation_bytes()` before allocating either owner.
The latter includes the native mutex, protected state, poison flag and padding.
The initialized pthread object never moves; lock/unlock has no lazy Rust heap
owner. Physical owners drop before the charge is released. Allocator rounding is
observed separately by the public allocation caller; this is not a bound on
foreign-runtime allocations or process RSS.

## Catalog reclamation

Maintenance reserves no publication slot or transaction identity. It charges two
maximum-size pathname allowances (8,192 bytes) during each of three
nonoverlapping phases: namespace validation, scratch construction, and inventory
work. Each phase owns at most one directory and one transient child path.
Namespace validation and scratch construction finish before inventory
allocation. The inventory reserves 206,884 bytes for 8,192 sort records, bounded
run descriptors, merge buffers and cached page checksums. Its reference walk
separately admits at most 16,448 bytes. These reservations coexist with
database, query and receipt owners; they do not replace those charges.

A conservative 2,097,672-record input bound permits 257 initial runs and at most
three eight-way merge passes. Two scratch files have a combined maximum admitted
extent of 67,125,504 bytes. Reserve growth before writing and retain the maximum
physical extent when overwriting earlier runs. Close the exclusive, unlinked
scratch owners before releasing their temporary charge. Scratch descriptors,
fixed cursor arrays, directory buffers, filesystem metadata and kernel cache are
separate bounded/observed native costs; logical reservations are not process
RSS.

Failures before a scratch name is durably absent may leave at most two
recognized empty files. They retain no encoded temporary content but require
exclusive reopen. A failed object-deletion barrier can leave persistent obsolete
names; it requires recovery even though unlinked scratch has closed and its
temporary charge is zero. Cleanup cannot run past memory or temporary admission
limits.

Shared scratch admission stores one atomic byte in the database handle and uses
no allocated lock. Each owner has two descriptors and two byte extents.
Reclamation and declared grouping each own at most one pair; a join owns one
pair per input. Concurrent consumers reserve memory and temporary bytes from the
same database authorities. Scratch bootstrap is serialized, but already unlinked
pairs may remain live concurrently. Descriptor ownership and whole-process
limits require their separate resource evidence. Every growth reserves the
difference from the same database temporary authority before writing.
Failed/partial writes retain the admitted extent; overwrites do not release
charge. Reads cannot exceed that extent. Owners close both descriptors before
releasing the combined charge. Bootstrap path reservations end before a returned
owner can grow. A construction failure can retain only the two recognized
zero-byte names and a finite recovery state, never uncharged scratch payload.
Read/write work is bounded by the caller's admitted batch; reclamation uses
4,096-byte pages and bounded initial runs.
