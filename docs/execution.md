# Execution contract

Execution turns a validated physical plan and a live snapshot into typed result
batches. The runtime schedules producer steps on the caller's thread. Numeric
kernels evaluate already-bound programs and have no parser, catalog, scheduling,
or filesystem authority.

[Language](language.md) owns query semantics. This guide explains the
controllers, work quanta, buffer lifetimes, and failure transitions that
implement them. [Resources](resources.md) owns admission equations;
[Verification](verification.md) owns coverage requirements.

Start at [`admission.rs`](../src/execution/admission.rs) for query validation
and reservation order. The [`scan` controller](../src/execution/scan.rs) owns
conditional demand and batch publication. Its
[legacy](../src/execution/scan/legacy.rs) and
[declared](../src/execution/scan/declared.rs) sources own stored layouts,
admission, and decoding. They construct admitted cursor/batch bundles for the
runtime to own while executing the producer graph.
[`execution.rs`](../src/execution.rs) exposes the borrowed result and handles
successful completion or terminal failure.

## Blocking operator ownership

[`blocking.rs`](../src/execution/blocking.rs) owns sorted inputs and the run,
merge, and sort state transitions. The [row codec](../src/execution/blocking/record.rs)
owns typed layouts and checked frames; [buffered I/O](../src/execution/blocking/io.rs)
owns bounded caches and borrows scratch-file effect authority for each call.

Consumers own the transitions that give records meaning:

- [Grouping](../src/execution/aggregation/grouping.rs) reduces equal keys and owns
  aggregate state, result spooling, and optional hash grouping.
- [Join](../src/execution/blocking/join.rs) owns equality matching, duplicate
  replay, and joining the two input lifetimes.
- [Order](../src/execution/blocking/order.rs) owns ordered output and DISTINCT's
  complete-row duplicate removal.

[`aggregation.rs`](../src/execution/aggregation.rs) selects dense or general
controllers. [`numeric.rs`](../src/execution/aggregation/numeric.rs) owns
expression sharing, typed accumulator arrays, NULL counters, and final overflow
checks. [`arguments.rs`](../src/execution/aggregation/arguments.rs) captures
demanded inputs and folds checked replay batches through those same numeric
cells.

After sorting, `RowSort::sorted_rows` lends the left cursor and retained
key/ordinal storage through a `SortedRows` view. Allocations and charges stay
with the sorter. The cursor validates bookmark/rewind positions; consumers need
no access to merge-pass internals. Grouping reuses the former merge writer and
right-input reader for its result spool on the other disposable file. This
handoff neither allocates replacement buffers nor transfers cleanup authority.
The shared record and run encodings, validation, capacity, and failure rules
still apply to every consumer.

## Producer execution

Physical plans describe producer pipelines for scans, aggregates, joins,
ordering and limits. The existing scan and aggregate controllers receive
separate pipeline mappings, and aggregate construction takes explicit
input/output schemas and demand. The runtime follows those input edges.
Declared-table construction admits all sources and producer outputs, then opens
the sources against the prepared query's pinned catalog. Equality joins compose
with projections, filters, other joins and aggregates.

Grouping's accumulator minimum and construction share one checked byte
calculation for typed arrays and expression scratch. The minimum uses one
scratch lane; construction retains the existing bounded adaptive lane count.
Fallback-storage charges remain separate and must be added before optional
growth.

Aggregate state keeps the immutable aggregate plan live through evaluation,
replay and finalization. Argument failures resolve through the existing
entry-to-state mapping to a demanded call; final SUM overflow uses the SUM
entry's own source span. The mapping is resolved on failure, without per-row
state, query-text reparsing or diagnostic fields in spill records.

Computed numeric SELECT runs within the existing producer: each predicate
requests its dependencies for surviving rows, followed by remaining output
requirements. In particular, aggregate final values must remain available on
demand until those predicates finish; a projection wrapper cannot first
materialize every aggregate output. The same checked numeric kernels must serve
source and derived values, without parser, catalog or I/O access inside a
kernel.

Any computation cache has one charged owner and a validated row/batch lifetime.
Its dimensions, scratch and NULL masks are admitted before use. Dependency walks
are bounded and iterative; repeated references cannot expand into exponential
work. Each resumable phase advances a dependency, predicate or output cursor
within named row/byte limits. Replay invalidates cached values with their input
position. Failure discards unpublished values; cancellation and drop release the
physical owners before their charges. Direct projections must retain their
existing path without allocating a computation cache.

A scan with demanded computations uses a compact workspace and 256-row quanta.
It resets cached readiness for each selection and after source replay. Output
batches copy values rather than borrow scratch. Direct scans use the 4,096-row
selection quantum without a computation buffer.
[Resources](resources.md#computed-scan-workspace) owns workspace dimensions and
admission. Other producers use the row evaluation path below.

Other producers evaluate one demanded row with fixed scratch and the same
numeric kernels. The result reserves the explicit scratch payload for one live
producer step; native frames and compiler spill space remain separately observed
by the small-stack tests. Predicate and output phases may repeat pure
calculations, but each dependency walk is iterative and visits each definition
at most once. Completion or failure releases the extra scratch charge. These
numeric buffers are internal kernel data; batch, spill and public value
encodings are unchanged.

## Batches

A query owns reusable bounded batches containing equal-row typed fixed-width
vectors, validity bitmaps, and optional selection indexes. Variable-width
values, when admitted, add checked offsets and one byte payload with independent
row and byte capacities. No row object or full-column owner exists on the
ordinary scan path. A producer may not retain uncharged output while blocked.

## Numeric kernels

The [language contract](language.md#values-null-and-equality) defines scalar and
aggregate results. The kernels implement those rules with different intermediate
representations. For scalar DOUBLE arithmetic, finite-operand addition,
subtraction, and multiplication producing nonfinite results return
`ArithmeticOverflow`. Nonfinite operands retain the specified IEEE behavior;
native bytes preserve raw IEEE-754 bits. Nullable aggregates distinguish no
demanded value from a numeric zero. The first SUM value is not seeded through
`+0.0`, preserving signed zero. Floating aggregate ordering is not promised
unless the language says so.

SUM preserves ordinary binary64 addition while a partial sum fits. On finite
overflow it retains the same significand precision in units of 2^32, returning
to ordinary units when opposite-signed inputs bring the partial sum back into range. The
shared 134,217,728-row maximum leaves ample exponent headroom. Nonfinite inputs
remain observable, and only a still-scaled finite sum at completion returns
aggregate overflow. This is bounded extended-range accumulation, not exact or
order-independent summation.

AVG-only state keeps ordinary sum/divide while its private sum fits. On finite
sum overflow it switches to a running mean, with opposite-sign updates avoiding
an overflowing difference. Identical demanded SUM/AVG expressions share state.
Dropping SUM removes its final-overflow demand; AVG alone must still succeed
when its mean fits. A SUM required by a later predicate remains demanded by that
predicate, even when omitted from final output.

INT64 aggregates retain a checked signed 128-bit sum, bounded by the admitted
row count times the maximum input magnitude. Only a demanded final SUM narrows
to INT64; AVG converts its wide sum to DOUBLE and divides by the nonnull count.
Identical SUM/AVG expressions share one state. Counters for nullable expressions
advance only when the evaluated argument is valid. An expression with no valid
arguments remains NULL, independently of the number of rows in its group.

## Pipeline state

### Bounded steps

The scan controller owns Begin, Filter, Output and Done phases. It produces
borrowed typed batches and finishes independently of its consumer. Dense/global
aggregation owns Read, Consume, Check, Emit and Done phases. Read requests
input; Consume folds the supplied batch or begins finalization after end of
input. Declared grouping consumes the same scan boundary with the additional
phases described below. Each `step` advances a declared cursor, returns borrowed
rows, ends, or fails. It checks cancellation before work and bounded reads. One
step reads at most one demanded block, filters at most 4,096 selected rows,
evaluates at most 256 output/input rows, or inspects at most 4,096 group slots.
The schema and expression bounds also limit byte and instruction work. No
syscall is claimed to have bounded elapsed latency or be interruptible.

### Predicate evaluation

STRING predicates compare validated UTF-8 bytes without normalization. The
constant has at most 32 bytes, so comparison inspects at most that common prefix
before using lengths; a large input string does not cause a full-value
comparison. Stored one-byte legacy keys and borrowed intermediate text use the
same predicate semantics. A NULL comparison is UNKNOWN and matches neither
requested truth value. The enclosing Boolean decision determines whether another
leaf is needed.

An IS NULL/IS NOT NULL predicate inspects the evaluated value's NULL tag and
returns a Boolean decision. Empty text, zero and NaN are non-NULL. It uses the
same bounded selection loops and column-demand paths as comparison predicates;
no nullability-based shortcut skips demanded evaluation or payload validation.

Boolean filters use validated forward decisions over the same leaf kernels. Row
producers share the lazy `RowValues::retains` interpreter. A linear AND scan
keeps its existing selection path. A branching scan adds one next-decision byte
and one temporary row index per quantum row, allocated once before source I/O.
The row limit is 4,096 without computed work and 256 with computed work. Each
predicate phase loads only its active rows' required columns, evaluates its
bounded selection, then advances their decisions. Zero rejects; forward offsets
skip leaves; the continuation retains the row. One phase returns Progress before
the next leaf. At completion, compaction preserves input row order. Replay
resets all decisions, and failure/cancellation drops the owners with the scan
account.

The scan retains its cursor and input buffers under the query's memory account.
A consumer borrows a complete batch until the next scan step; consuming it adds
no allocation or row copy. Dense/global aggregation folds each input batch
before advancing the scan, checks all demanded final values before emission, and
writes to the separately admitted output batch. A scan error or consumer error
fails the query and drops both owners before releasing their reservations.
Cancellation is checked during scanning and during aggregate checking and
emission.

### Runtime scheduling

Consumers receive a batch, an end-of-input flag and their output buffer. They
request the next batch or a complete replay without accessing source cursors.
The runtime graph performs one producer work quantum per call and retains each
node's input until its consumer requests more. Requests select an earlier node;
Rows or Finished return control to its validated parent. Grouping waits in Await
while the runtime obtains input. Replay is a separate quantum. Driver failure is
terminal, including cancellation during scanning or replay. Scans, aggregates
and declared-table joins and ordering use this scheduler. Each node owns its
output batch. Construction reserves every source, output, join and ordering
minimum before optional hash-grouping growth. Only fully opened source owners
reach query execution.

Each aggregate producer indexes its own controller in a runtime-owned vector;
its node owns its output batch. The vector is admitted before controller
creation and source I/O. Construction reserves the sum of aggregate minima
through the database memory account. Before constructing one controller it
releases only that controller's minimum; future minima remain charged during
optional buffer and hash growth. Allocation or competing resource admission can
still fail, with partial owners released normally. No independent memory budget
is introduced.

### Replay and terminal cleanup

An aggregate can replay its retained output once after accumulation and
validation, including after a prefix has been consumed. Dense state resets its
emission cursor; hash grouping reloads retained groups; disk grouping resets its
result offset and clears cached bytes so rereads validate the checked spool.
Replay does not restart the aggregate's inputs. Invalid or repeated replay and
cancellation fail the runtime. The public query result destroys runtime owners
on completion or failure. [Composition
checks](verification.md#required-composition-regressions) must cover replay
through the production runtime, including downstream grouping fallback.

### Legacy scans

The format-4 scan caches demanded native columns in checked 256-KiB DOUBLE/DATE
or 32-KiB STRING buffers. A valid 28,672-byte metadata workspace exists even
when no source values are demanded. Filters shrink one 4,096-index selection in
stage order; fully filtered rows do not demand later source buffers. Every
observed block passes its checksum before decoding. The stored key and DATE
domains are checked on demanded values. Ingestion, key decoding and dense group
indexing share the validated fixed-text type: ASCII space through tilde,
excluding `|`. Source and aggregate output use the same typed batch owner. Each
batch contains at most 64 columns and 256 rows, with a validity bitmap per
column; its row count is published only after all demanded values are written.

### Scalar expression evaluation

Scalar execution consumes borrowed numeric columns carrying stable query
identities, INT64/DOUBLE payload slices and validity. Source storage positions
do not index the evaluator inputs. A validating constructor checks payload type,
row extent, validity extent and declared NULLability. The explicit stack uses
8-byte raw numeric payloads, one INT64/DOUBLE type and four validity words per
vector, with at most 32 vectors and 256 lanes. NULL lanes propagate through
arithmetic without interpreting their payloads; demanded child expressions still
execute and can fail before a parent produces NULL. Checked integer operations
occur before required floating conversion. No per-row allocation, row-array
adaptation, parser lookup or catalog lookup occurs in scalar kernels. Aggregate
expression width adapts from 256 down to one lane under the same memory
authority; insufficient room for one lane returns typed resource refusal before
effects.

### Dense aggregation

Legacy grouping directly indexes the complete domain of zero, one or two
admitted printable-ASCII STRING keys. At most 8,836 groups own bounded value,
count and flag vectors. Positions preserve each group's fold order across
expression batches. Key lookup and numeric accumulation have separate owners.
`Groups` resolves dense keys; `AggregateState` admits a positive slot capacity
and evaluates demanded expressions. Aggregate keys, layouts and numeric state
retain semantic input identity, type and NULLability, without storage positions
or catalog IDs. Independent validation compares the entire numeric input map,
including unused slots, with the input relation before a batch reaches a kernel.
The scan's separate validation owns catalog and physical-position agreement. Its
`AggregateCells` owner counts a row when lookup assigns its slot, then folds
evaluated numeric values using that slot and row count. Expression scratch and
numeric cells can be borrowed independently. The lookup and counting pass reuses
one bounded position array; no per-row allocation is introduced. Declared
grouping uses bounded hash storage and disk reduction through the same public
query path. Counts fit the validated source maximum and are checked before
increment. Only demanded expressions own numeric states. Before any aggregate
rows escape, the engine checks every retained group's demanded final values.
Emission then follows declared key order; without GROUP AND ORDER BY, incidental
slot order is not a semantic promise.

### Admission and release

All retained query buffers are fallible and admitted before namespace effects.
Scratch construction uses previously reserved memory for its bounded transient
pathnames when the general controller enters the disk path. Workspace, aggregate
state, the prepared plan and the result handle report to one database account.
See `resources.md` for equations and measured limits. Physical owners are
destroyed before their reservations are released.

## Catalog scans

Ordinary builds connect catalog storage to the same Begin/Filter/Output
controller. `Source` owns either the legacy reader or one declared-table reader.
Catalog queries use `Database::prepare`, `Database::execute` and
`QueryResult::step`. Global COUNT(*), SUM and AVG use the same aggregate owner
and finalization phases as legacy queries. When the scan finishes, the aggregate
consumer enters finalization, including empty tables. Declared grouping selects
the general controller below.

The prepared query pins its complete catalog commit. Execution checks database
ownership, generation, table identity, stable column IDs, storage positions,
types and NULLability against that pinned schema. A later append does not change
an old query's source. The result's borrow keeps the prepared query and its pin
alive.

Declared-table source read failures are terminal, including a unit-open failure
after the index cursor advanced. A failed source cannot expose prior payloads or
restart. General grouping can restart a healthy source once, retaining the same
index descriptor, schema and snapshot pin. It clears unit and page caches and
rechecks admitted page checksums while replaying. A second restart returns
resource refusal.

Declared sources allocate payload buffers once and reuse them across units.
[Declared scan admission](resources.md#declared-scan-admission) owns their
per-type bounds. Each source uses the existing metadata and payload validators.

Declared-table source construction has two phases. `declared::admit` allocates
its complete retained buffers without I/O and borrows the prepared query that
owns its snapshot. Consuming that admission opens and validates the source using
shared, previously reserved catalog scratch. The admission cannot be opened
against another query. This lets a runtime reserve multiple sources before
optional aggregate growth. `ScanCursor` owns only scan state and buffers; its
step fills a supplied batch. The admission workspace transfers its cursor and
independently charged batches into runtime nodes before query execution. It is
not a second execution driver.

A declared-table step admits one unit, reads one demanded payload, filters at
most 4,096 rows, or fills at most 256 result rows. Unit admission may refill one
bounded index page and read fixed unit metadata. Each text result column also
has a 65,536-byte capacity. If the next row fills a text buffer, only completed
rows are published; the next step clears partial writes and retries that row.
Nullable INT64, DOUBLE, DATE and UTF-8 values use the same typed result batch.
Filters preserve SQL NULL behavior and stage order; an empty selection does not
demand later payloads. Declared-table admission reserves output batches and
transient catalog validation scratch before sizing aggregate expression lanes.
At the exact minimum it uses one lane; a one-byte shortfall returns resource
refusal before query I/O. Aggregate state and batches share the database memory
authority.

Filtering an ordered producer must emit a subsequence in the same relative
order. Direct projection and AS preserve that order even when keys are hidden.
This is the PipeSQL guarantee in `language.md`; incidental ordering from a join
or an ordinary aggregate remains insufficient to establish it.

## Declared grouping

Declared grouping consumes batches from the runtime through the shared scan
boundary. It captures demanded arguments, tries bounded hash storage, and on
refusal discards that storage and restarts the pinned source once. The replay
builds sorted runs and reduces their checked argument records through the same
aggregate kernel. Before returning any rows, the memory path checks every
retained group's final values; the disk path evaluates filters in order and
writes retained final values to a private checked spool. A later demanded
overflow prevents all publication. Final projection types determine the spool
buffer bound, including 64 repeated maximum-width text values. Hash ordering
reuses the finished hash buckets as bounded index arrays; result spooling reuses
the sort I/O buffers. It currently emits one completed group per step.
QueryResult owns either the dense aggregate or one fallibly allocated general
controller, and destroys it on finish, failure or drop.

## Equality joins

A declared-table join materializes each input through the shared external
sorter. Its checked row frame retains every demanded typed field and compares
only the join key. Each side owns its layout, reusable record, sorter and
two-file scratch owner. The query retains both sorted inputs until completion,
failure or drop. The join controller always sorts; it has no in-memory hash-join
path.

The controller creates scratch, requests an input batch, captures and pushes one
row, and advances spill or merge work when required. After both inputs finish,
Seek compares loaded keys. Emit writes one complete output row; Right advances
within the matching right group; Left advances the left row and either rewinds
the right group or resumes Seek. A bookmark comes only from a loaded, validated
record boundary and retains the original run's end and remaining-row bound.
Every reread checks the frame, layout, lengths and checksum. Duplicate groups
produce their full Cartesian product without retaining that product in memory.

Sorting uses the shared total ordering, but matching applies ordinary equality:
NULL and NaN never match, and signed zeros match. Filters and projections use
the validated concatenated-input positions. A filter may suppress an output
pair. No row order is promised. An output prefix may precede an I/O or resource
failure; callers must observe successful completion.

A step captures, reads or emits at most one bounded row, compares one key pair,
or advances one existing sorter quantum. A row has at most 64 fields, each
STRING at most 65,536 bytes. Output currently contains one row per emitted
batch. Each sort input is capped at 134,217,728 rows; the duplicate product is
finite, and a later join or aggregate independently enforces its input-row
bound. These are work bounds, not elapsed-time guarantees for filesystem calls.

Grouping may request one replay after exhausting hash storage. A join replays
its retained sorted inputs and resets matching; it does not reopen sources or
acquire a new snapshot. A second replay refuses. Errors poison the controller,
and query teardown releases all buffers and disposable extents. Already finished
work remains finished after late cancellation. Scratch bootstrap failures retain
the ordinary recovery-debt rules in the storage contract.

## Standalone ordering

Declared-table ORDER BY uses the same sorted-input owner and checked row codec
as joins. It places unique ordering fields first in the frame, followed by the
remaining demanded payloads. Repeated identities retain their first comparison
policy. The layout fingerprint binds the field permutation, types, NULLability,
directions and NULL placements. NULL placement is applied independently of
ascending or descending comparison. Encoded payloads preserve original values.

Create reserves ordinary scratch; Read/Await requests the child batch;
Capture/Push consumes one bounded row; Spill and Sort advance the shared sorter.
After collection and merging complete, Load checks the next record and Emit
verifies monotonic keys and original ordinals before applying ordered filters
and publishing one complete row. Equal-key stability is an implementation
detail. Done and Failed are terminal. No row escapes before sorting completes;
later I/O failure or cancellation may follow an output prefix.

One input owns a capture record, run buffer, bounded span arrays, merge records,
three I/O buffers, previous-key storage and two disposable files. Construction
admits that minimum, the controller and output batch before optional grouping
growth. The row, field and byte-work limits are shared with join sorting. A
single grouping fallback replays the retained final run, resets monotonicity
state and preserves the pinned snapshot. Failure and drop release these owners
through ordinary scratch cleanup. Legacy storage refuses standalone ordering at
preparation because it cannot admit this scratch publication authority.

## Duplicate removal

Declared-table DISTINCT uses the standalone ordering controller with all unique
input fields as ascending, NULL-first keys. Construction selects this mode and
admits the same checked row, run, merge, previous-key and scratch owners. The
semantic plan publishes fresh identities and no ordering guarantee; physical
output positions still address the retained input values.

Emit independently checks key/ordinal monotonicity, then compares the full key
with the previous key. Equal rows are consumed without evaluating downstream
predicates or computations. The first row of each group may publish one complete
output row. The previous key advances even when a downstream predicate rejects
the representative, so duplicate rows cannot re-enter the relation. Empty input
finishes without output. Raw payloads remain unchanged.

All keys fit the shared 64-field row frame, including 65,536-byte STRING values.
Each step consumes at most one bounded row or one sorter quantum. The
134,217,728-row input bound, checked temporary records and two-file cleanup
apply. A single replay resets the cursor and previous-key state over the
retained run; it neither opens another snapshot nor repeats upstream
computations. No row escapes before sorting finishes, but a later failure can
follow an output prefix. Legacy storage refuses this scratch-dependent operator
at preparation.

## Prefix consumption

The LIMIT controller consumes offset rows before count rows, retaining separate
decreasing counters. It counts selected input rows before applying predicates
fused after LIMIT; rejected rows do not extend the quota. Zero offset and zero
count require no input-batch request from the limiter, but ordinary query
admission and metadata validation still apply. A positive offset is consumed
first even when count is zero. Bounded batches can prefetch beyond the final
selected row, and an entered blocking producer still performs the work required
by its own output contract.

Early completion is an ordinary finished state, not cancellation of the shared
query token. Root completion drops the full runtime graph; a downstream consumer
retains any upstream owners needed for its bounded replay. Grouping fallback
must reset the limiting counters and intermediate batches and rewind the
underlying source or retained sorted-input producer. Resetting only a limiter's
counters would reuse an advanced input and produce the wrong prefix. Replay must
reproduce the same selected rows from the same pinned generation. The walk to
its replay owner is bounded by the validated producer graph. Both legacy and
declared-table construction admit the required outputs before optional grouping
growth. Legacy aggregation takes its actual input pipeline's schema and has its
own output batch, including when LIMIT appears on either side.

The controller has read, await, done and failed states. An await step processes
at most one 256-row batch. Each STRING output has the admitted per-column byte
bound; copying a subset of a batch cannot exceed that bound. Filtering happens
before copying, and the row count is published only after all retained cells are
written. The limiter adds no file, spool, heap collection or shared cancellation
authority. Its counters live in the charged runtime node; its reusable output
batch has a separate reservation. Replay resets one node per step and follows
strictly earlier input indices until reaching the scan or retained sorted
producer.

## Requirements for additional blocking operators

The legacy bounded key domain fits its fixed state under the admitted query
memory limit; it is not general grouping or spill evidence. Declared grouping
has the bounded hash/replay/sort path; declared equality joins retain sorted
inputs and replay duplicate groups as described above. Standalone ordering uses
the same checked sorter and can replay its final run. Additional blocking
operators and platform implementations require nonrevocable minima, forced-cap
external execution, bounded fan-out/depth, a measured decreasing fallback,
checksummed temporary state and complete cleanup.

## Result ownership

`execute` admits a validated plan and its retained owners. Data reads, scalar
work and finalization may then fail during `QueryResult::step`. Rows borrow the
running query; the borrow forbids another mutable step. A caller may copy a
`Value<'batch>` without allocation. Numeric/DATE payloads are copied; STRING
text borrows batch storage until the caller releases that borrow. Progress means
bounded work advanced without publishing rows. Finished and Failed are terminal
and distinct. Earlier scan rows form only a prefix if a later step fails. No
aggregate output precedes demanded arithmetic validation, but cancellation or a
sink failure can still interrupt result consumption.

General UTF-8 batches in declared tables use fixed row spans and a separate
String buffer of at most 65,536 bytes per column. Both owners are reserved
before allocation. Cell writes consume existing capacity; a full buffer returns
a typed resource refusal before changing that cell. Clear resets validity and
text length while preserving capacity. The producer publishes only complete rows
and must handle byte capacity as well as the 256-row bound. Fixed grouping keys
retain their compact encoding; general text does not enter that dense-key
algorithm.

Each execution batch carries its own reservation in `OwnedBatch`. Construction
transfers its exact payload charge from the admitted workspace without releasing
capacity to other queries. Moving the batch transfers that charge; dropping its
source cannot release it. The batch drops its physical columns before releasing
the reservation. Kernels borrow the underlying `Batch` and do not own admission.

Finished/failure/drop release workspace, descriptors and groups. The small
result reservation remains until its handle is dropped; prepared ownership
remains until the prepared plan is dropped. Failure retains a bounded owned
error that can be moved out with `into_error`. Neither successful admission nor
receipt of a row establishes successful completion. The CLI prints success only
after Finished. See `interfaces.md` for the caller contract.

## Required evidence

Test empty/all-NULL, NaN, infinities, signed zero, subnormals, finite overflow,
predicate NULLs, batch boundaries, byte widths, backpressure, all transitions,
and resource denial. Run optimized stock artifacts over SF1 and compare result
bits, bytes, RSS, CPU, and wall time with independent controls and retained
alarms.
