# PipeSQL architecture

PipeSQL has one query path over immutable table generations. Query preparation
establishes meaning and pins its input generation. Physical planning decides
which values each producer needs. Execution moves typed batches through those
producers. A separate publication path constructs private files and acknowledges
a new generation only after the required durability steps.

This guide explains the relationships and invariants shared across those paths.
Use the [source map](source-map.md) to locate implementations and the
[documentation index](README.md) for detailed contracts. The product targets
macOS, Linux, and Windows; [platform status](testing.md#platform-status)
distinguishes implementation from native qualification.

## System

```text
query text -> parser/binder -> prepared plan + snapshot pin
                                      |
                                      v
                    physical planning and validation
                                      |
                                      v
                         producer graph -> batches

writer -> private immutable objects -> publish roots -> new generation
readers -> current and pinned graphs -> reclamation protection

shared memory authority       shared temporary-space authority
```

The library and CLI share one package. A private `filesystem` crate contains
native pathname/directory effects so their allocation and unsafe boundaries can
be verified independently. The engine remains unsafe-forbidden; the crate has no
semantic or publication authority. The binary's private `cli::arguments` and
`cli::sink` modules contain native startup capture and sink ownership. Package
linting denies unsafe code; the library root forbids it, and only those binary
modules opt in. The binary uses the same pinned libc ABI dependency. Argument
capture belongs to the CLI; embedded library callers supply their own inputs.
Public stack regressions use a native current-thread size observer in the
private filesystem crate, enabled only by the dev-dependency feature
`test-stack-observation`. Ordinary library/CLI builds omit it; the engine and
public integration test need no unsafe observer code or public FFI surface.

The database owns its immutable canonical path and process lease through
cleanup. Lifecycle inspection and query decoding share the complete unit codec
validator; inspection retains validated scalar facts, while queries decode
demanded metadata into admitted storage. These boundaries avoid redundant path
and metadata owners. [Resources](resources.md) owns their charges and
construction limits; [storage](storage.md) owns validation and persistence.

Aggregate arithmetic has a bounded input count independently of join fan-out.
Scalar evaluation, accumulation and finalization remain separate semantic
operations; an accumulator is not a user-visible scalar addition.
[Language](language.md#values-null-and-equality) owns numerical results;
[Execution](execution.md#numeric-kernels) owns accumulation strategies and work
bounds; [Resources](resources.md) owns admission and storage requirements.

## Design decisions

[README](../README.md#product-requirements) owns the product choices: an
embedded, single-node analytical database with one serialized writer and
concurrent snapshot readers. The implementation uses immutable column chunks,
typed plans, bounded batches, and external sorting when a blocking operation
exceeds memory. DOUBLE storage preserves its raw IEEE-754 bits.

Publication and recovery use the same authoritative roots and write-ahead
record. Effect injection surrounds production transitions so tests exercise the
actual operation; it does not substitute a simulator's parser, executor, or
publisher. [`effects`](../src/effects.rs) owns operation names and the
attempted-effect counter. Tests supply a separate fault schedule and inspect the
counter through an accessor; they cannot overwrite it. A refusal still counts as
an attempt. Callbacks run before the attempt is counted or at an explicit
completion point. The lifecycle, query, and publication callers own effect order
and recovery. Revisit implementation choices under the [engineering
guide](engineering.md#choose-and-finish-work) when concrete evidence identifies
a better design, preserving the affected contracts together.

Physical plans live under `execution/planning`. `lower` selects producers and
maps demanded semantic identities into their row positions. `validate_physical`
checks graph edges, computations, filters, and outputs; its inverse mapping is
separate from the builder's lookup. It can inspect only earlier, validated input
pipelines while checking a consumer. Both phases use the same backward demand
analysis, so that analysis is not itself an independent oracle for query demand.

## Cross-system invariants

1. Query meaning enters through one parser and binder path.
2. Display names, physical positions, catalog ordinals, and `ColumnId` are distinct.
3. The optimizer never reparses or resolves names; execution never guesses facts.
4. A physical plan fixes producer edges, source mappings, and demanded values.
   Independent validation precedes execution admission. Admission calculates
   resource requirements; the current plan has no general operator resource graph.
5. Every significant byte, temporary extent, task, queue, snapshot, and result has
   one owner, one account, and finite terminal behavior.
6. Publication has one authority and explicit point of no return. Recovery never
   hides a known newer generation by opening an older valid one.
7. Readers observe one immutable generation. Buffers/results die before unpin;
   reclamation never removes a pinned generation. Shared reader and resolver
   calls may inspect authoritative state but cannot repair or reclaim it.
   Publication, maintenance and exclusive recovery own their respective
   mutations; query scratch uses separate disposable-state authority.
8. Cancellation is cooperative at named bounded transitions and makes no claim
   about interrupting an in-progress bounded syscall.
9. Invalid input, resource refusal, I/O, cancellation, contention, corruption,
   and ambiguous commit return typed outcomes. Assertions guard internal
   programmer invariants; native interfaces prevent unwinding across a foreign ABI.
10. No test adapter, format bridge, or convenience path duplicates semantics or
    durability behavior.

## Query and publication paths

Preparation resolves names into query-local column identities and pins a catalog
snapshot. A snapshot is the immutable table graph selected for that query.
Physical planning maps those identities to source columns and producer outputs,
then validates the mappings before payload I/O. Execution retains the pin and
moves borrowed batches through the validated producer graph. The [frontend
walkthrough](frontend.md#trace-a-query-through-preparation) follows a query
through these phases; [Planning](planning.md) explains demand and validation.

Legacy `lineitem` and declared tables use the same frontend, numeric kernels,
and result interface. Their source readers and grouping strategies differ.
Declared tables also support scratch-dependent joins, ordering, and DISTINCT.
The [language manifest](language.md#current-public-query-manifest) owns accepted
forms and limits. [Execution](execution.md) explains the producer controllers
and their shared sorter.

A writer constructs private immutable objects and publishes a new generation.
Prepared catalog queries keep their older generation while the writer publishes.
Legacy loading instead needs an exclusive mutable database reference, which
prevents loading while a prepared query borrows the handle. Both storage paths
use the same publication and recovery authorities.
[Transactions](transactions.md) explains the publication point after which a
failure may mean the commit occurred.

Reclamation protects the current graph, pinned generations, and retained
receipts. It acquires maintenance authority rather than a publication slot or
transaction identity. Disposable query and reclamation scratch has separate
creation authority. A scratch failure cannot grant permission to repair
authoritative files. See [Concurrency](concurrency.md#authorities-and-locks) for
admission and [Storage](storage.md#catalog-reclamation-scratch-and-recovery) for
cleanup.

## Filesystem effect boundary

The private filesystem crate translates checked names, descriptors, and buffers
into native operations. It returns observations and errors. The engine decides
whether those observations authorize a read, publication, or repair.

### Directory traversal

`filesystem::Directory` owns a freshly opened, unshared read-only directory
cursor and borrows one caller-owned, 8-byte-aligned 8-KiB buffer. It returns
borrowed raw OS names, not allocated paths or catalog facts. Names cannot
survive the next mutable step. EOF and error are terminal; drop closes the sole
descriptor. Every step consumes a raw record or terminates; at most 64 raw
records and 65 bounded reads are admitted per cursor, including skipped
dot/vacant entries. The engine retains stricter namespace limits, identity
checks, effect ordering and all repair authority. Callers own cancellation
between steps; no kernel call is claimed to be interruptible.

The crate confines unsafe code to its private syscall and mutex modules.
Syscalls use checked C-string storage, native ABI bindings, an owned descriptor
and exclusive initialized buffers. macOS uses public `getattrlistbulk`; Linux
uses `getdents64`. There is no unbounded directory-cache fallback. Native SDK
checks, safe decoder mutations, Miri, ASan, real threads and public allocator
refusal challenge the boundary. Linux has scoped native filesystem and catalog
runtime coverage; [platform status](testing.md#platform-status) identifies the
remaining qualification gaps.

### Paths and object identity

Path inspection, open/create, canonicalization, link/rename and deletion also
pass through this crate for the public library and CLI query-file reader.
Open-file inspection uses `file_metadata(&File)`, returning the same normalized
identity, type, length, link count, and timestamps as pathname inspection.
Storage and publication compare these facts without importing native metadata
traits. An opened descriptor still identifies its original object after its name
is replaced. Each native pathname uses at most 4,097 checked stack bytes;
dual-name operations validate both before one syscall. Canonicalization requires
an absolute bounded path and uses a fixed result buffer plus one fallible
PathBuf allocation. The macOS resolver owns its root-device observation, suffix
cursor, link count and scratch per call; no shared root-device cache or engine
mutex spans native I/O. Its native helpers share a 65,536-call admission
counter, including optional mount lookups. Exhaustion propagates distinctly
rather than becoming a naming fallback; no partial name is published. Native
naming preserves case/normalization, firmlink spelling and the reviewed
33-symlink rule within admission. Linux uses libc resolution; its resource
bounds remain unqualified. Local regression evidence includes the native path
checks, but does not establish whole-platform qualification.
[Verification](verification.md) owns the remaining release obligations.

### Byte I/O and synchronization

File descriptors transfer once into safe File ownership; direct std descriptor
I/O and nonblocking locking use their standard-library interfaces. Exact byte
I/O uses `file_io`'s concrete-file loops rather than std's interruption-retrying
convenience methods. Positive calls consume remaining bytes; zero progress and
every error terminate. Positional extents are checked before native
signed-offset conversion. Callers retain buffer admission, effect placement,
cancellation and outcome authority. Synchronization uses one borrowed-descriptor
native attempt: F_FULLFSYNC on Darwin, fsync on Linux. Interrupted/unsupported
calls return the original error without retry or weaker fallback; std sync_all's
unbounded EINTR loop is not used. No flush is hidden inside pathname mutations.
The publisher still owns the point of no return.

Pathname metadata and opened-descriptor metadata normalize to one opaque
physical FileIdentity and explicit byte/time units. The engine independently
compares them; the OS boundary does not decide catalog identity, input stability
or fence ownership. [Resources](resources.md#native-paths-stack-and-io) owns
native storage and work bounds.
[Verification](verification.md#linked-native-effects) specifies linked failure
checks; [platform status](testing.md#platform-status) identifies which
implementations and campaigns have been exercised.
