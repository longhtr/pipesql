# Storage contract

Storage owns immutable native units, catalog descriptors, integrity validation,
private construction, reclamation, and format lifecycle. Transaction visibility
and commit outcomes belong to [Transactions](transactions.md).

| Stored representation | Current use |
| --- | --- |
| Namespace format 7 | Declared tables, repeated appends, and retained transaction history. |
| Object codecs version 6 | Catalogs, schemas, table indexes, and native units inside namespace 7. |
| Legacy format 4 | One fixed-schema `lineitem` load and its retained outcome. |

These versions identify incompatible bytes. None is a stable compatibility
promise. Start with the catalog graph for declared tables. The legacy sections
specify the separate fixed-schema representation.
[Verification](verification.md#persistence-and-recovery-evidence) owns required
checks and the distinction between runtime and durability evidence.

## Declared-table catalog graph

Catalog namespace format 7 uses the same lease, root replicas and WAL
publication protocol. Each committed root anchors two immutable objects: the
complete catalog and the success history. Catalog entries name a table identity,
schema object, row and unit counts, and an optional data index. An empty table
has no data index. The index contains native-unit references; each reference
anchors unit metadata, which in turn anchors the separately checksummed column
payloads. Success history retains the successful transaction sequence for every
committed generation.

Catalog, schema, table-index and native-unit codecs use format 6 inside
namespace format 7. Their versions are distinct from the namespace version.
Objects live in `units/` under identities derived from transaction sequence and
object ordinal. Readers validate identity, length, checksum and relationships
before trusting the next reference. Writers publish a new graph while pinned
readers retain their old graph. Reclamation protects both data and receipt
history as described below. [Resource limits](resources.md#catalog-capacity)
bound this graph; these codecs are pre-release and have no compatibility
promise.

## Pre-release format-4 namespace

An empty database contains a zero-byte regular `LOCK`, 128-byte `CONTROL`, 4,096-byte
`ROOT.A` and `ROOT.B`, 512-byte `WAL`, and empty `units/` and `private/` directories.
All are non-symlinks. WAL must have exactly one hard link: it is rewritten in
place, unlike replaced root replicas and immutable published units. Admission
checks this ownership condition on the name and opened reader, and recovery and
publication check the opened writer again before modifying bytes. An external
alias is corrupt namespace state, not authority to repair the aliased file.
CONTROL names format 4 and a random nonzero 128-bit database identity. Genesis roots and fence name that identity, issued prefix 0, and generation
0. The roots have distinct replica roles; the fence uses canonical role byte 0.
Every reserved byte is zero. These bytes are pre-release with no compatibility
promise. Formats 1 and 2 are retained as rejected fixtures, not compatibility paths.

Roots and fence share the following snapshot fields. Byte ranges are half-open;
multibyte integers use little-endian encoding.

| Bytes | Field |
| --- | --- |
| 16..32 | Database identity |
| 32 | Replica role |
| 40..48 | Generation |
| 56..80 | Optional success token |
| 80..88 | Unit ordinal |
| 88..96 | Row count |
| 96..104 | Unit extent in bytes |
| 104..108 | Unit metadata CRC32C |
| 108..112 | Record CRC32C |
| 112..120 | Issued attempt prefix |

The record CRC32C covers the entire record with its own field zeroed. Magic and
encoded extent distinguish root from fence. Generation 0 has no success or unit
fields but can retain an arbitrary issued prefix. Generation 1 retains its
nonzero same-database success sequence at or below that prefix. The
unit-metadata CRC anchors the graph independently of WAL.
`tools/snapshot-fixtures.py` independently produces current vectors, including
an aborted gap before success. Native unit geometry is unchanged from format 2;
its header discriminator and dependent checksums change. Input CRC remains a
two-pass construction check, not a persisted fence field.

Create obtains identity before creating the target, holds the
cooperating-process LOCK, writes and full-syncs every metadata file, validates
the complete namespace, then full-syncs the database and parent directories.
Every host transition passes a closed effect operation. Failure after target
creation invokes bounded cleanup of only recognized names and full-syncs the
parent; cleanup failure is an explicit `CleanupRequired` outcome. The acquired
lease remains owned by the outer create operation through cleanup, not by a
fallible initialization helper. Before lease acquisition the incomplete
namespace cannot admit a public handle. A failed creator must never release
authority, admit another writer, and then remove that writer's data. This
establishes the implemented create contract, not a load-commit or power-loss
durability claim.

Open identity-checks and acquires LOCK before complete structural validation or
recovery. A lease owns both the opened file and its device/inode identity;
recovery compares the current namespace LOCK with that held identity before any
mutation. A previously admitted writer also supplies its saved DatabaseId;
CONTROL must match before recovery effects, not merely before returning from
load admission. A descriptor for an unlinked or renamed old LOCK is not
authority over a new namespace at the same path. It checks CONTROL, all root
replicas, WAL length, bounded subdirectory contents, and the complete unit
metadata prefix including zero padding without allocating from persisted
lengths. Root/fence selection follows `transactions.md`: one missing, truncated,
or corrupt root is repairable only with a matching intact fence and a valid
surviving graph. Two valid roots must be equal or adjacent. Generation-0
private/unit debris is reclaimed only after both selected root names are
durable; the fence is then rewritten, verified, and synced. Known root-next
debris is removed. Every changed directory is full-synced before return. A
recovery-effect failure returns no usable `Database` and a subsequent healthy
open can retry. Unknown names, both bad roots, foreign identities, and newer
known format/generation values fail closed rather than being hidden. Root reads
inspect at most 4,096 bytes, including recognizable version prefixes in short
copies and complete checksummed prefixes in overlong copies. A wrong extent
cannot hide unsupported versions/generations or checksum-valid foreign identity;
only an exact extent can be admitted as a valid root. Changed opened identity is
an admission failure, not permission to repair the replacement object.

## Native units

Native units contain immutable column chunks that can be read independently.
Authoritative metadata names format/version, generation, table, column/logical
type, NULLability, row start/count, encoding, byte extent, and integrity value.
Relationships are validated before dependent reads or allocation. Canonical
coverage rejects gaps, overlap, duplicate identities, out-of-range offsets,
trailing authoritative bytes, and inconsistent row counts.

DOUBLE must preserve all IEEE-754 bits; DATE has one documented canonical unit.
Validity is independent of value payload. Integrity detects accidental
corruption and is not authentication. Checksums cover semantic context as well
as bytes so payloads cannot be silently exchanged. Current native codecs use
CRC32C and bounded, uncompressed column payloads. Different chunk sizes,
encodings and compression require their own measurements and admission; current
bytes have no compatibility status.

Projected scans read and validate only demanded chunks. Undemanded corruption
may remain undiscovered until that column is demanded; metadata corruption and
any demanded corruption fail closed before values reach kernels. A scan never
requires a row adapter or full-column residency.

The format-6 unit reader owns 2,112 bytes of fixed metadata; its table index
cursor owns a 3,072-byte page and bounded page checksums. These values can move
between query steps without borrowing workspace scratch. A retained owner
accounts for their complete sizes. Stack use is a separate bound; the snapshot
validation scratch remains 65,536 bytes for success-history validation.

`ColumnBuffer` owns one preallocated native payload of at most 524,288 bytes.
The caller reserves its capacity and handle before allocation. Refill clears the
validated state before calling the existing unit reader; only a successful read,
checksum and value validation make a borrowed column view available. Failure
leaves no view. Refill adds no allocation, and callers cannot mutate the bytes
through the view. Catalog scans use these reader interfaces through the shared
scan controller.

## Construction and publication

The legacy `load_lineitem` operation accepts one absolute, no-dot, no-NUL,
at-most-4096-byte path to a local regular non-symlink file of at most 1 GiB. The
file has TPC-H lineitem `.tbl` shape: exactly 16 pipe-delimited fields per row,
a required trailing `|`, LF row termination, no CR, and at most 512 bytes
including LF. The projected fields use these one-based source positions:

| Position | Column |
| --- | --- |
| 5 | quantity |
| 6 | extended price |
| 7 | discount |
| 8 | tax |
| 9 | return flag |
| 10 | line status |
| 11 | ship date |

Ordinal tests use distinct sentinels in all 16 positions. The four DOUBLE fields
use the canonical grammar below. Each key is exactly one printable ASCII byte
`0x20..=0x7e` except the unescaped delimiter `0x7c`; all seven projected fields
are nonnullable. At most 6,500,000 rows are admitted independently of the byte
bound.

DOUBLE input is optional ASCII `-`, one or more decimal digits, and optional `.`
followed by one or more digits, or exactly `NaN`, `inf`, or `-inf`; exponent,
leading `+`, empty integer/fraction, and other spellings are rejected. Each
DOUBLE field is at most 32 ASCII bytes, including any sign and decimal point.
DATE is exactly ten ASCII bytes `YYYY-MM-DD`, a real Gregorian date in years
0001..=9999, and is encoded as signed days from 1970-01-01. The remaining
unprojected fields may be empty but must satisfy the row/delimiter/ newline
bounds. The input must not mutate during load.

Pass one identifies the file, validates every row without writing, and computes
row count, whole-input CRC32C, and a projected canonical-byte CRC32C ordered per
row as quantity, extended price, discount, tax, returnflag, linestatus, ship
date, plus checked resource requirements. Only after memory and temporary
admission does pass two reopen the same identity, repeat validation, and write
seven fixed-name private staging streams. It must reproduce
device/inode/length/time observations, row count, and both CRCs before unit
assembly. Every offset, length, row count, and file extent uses checked
arithmetic. A rejected next row publishes no staging value. The staging producer
enforces the first-pass row allowance before changing any column and checks each
next flush extent against that allowance before I/O. Final input validation
cannot substitute for enforcing the temporary reservation during construction.
The producer also retains one CRC per column stream. Assembly checks those
expected CRCs against the bytes it consumes; merely computing new unit CRCs from
possibly corrupt staging would certify the corruption rather than detect it.

The implementation reserves 1,867,776 memory bytes before its one 1,725,440-byte
arena allocation and reserves the checked temporary peak before creating seven
staging files. It assembles one `UNIT.next`, independently rereads and checks
its complete metadata prefix (including all 3,072 reserved zero-padding bytes)
and every payload block, full-syncs it, and removes staging before publication.
Encoded metadata and readback bytes reuse charged arena regions. The metadata
decoder constructs into caller-owned fixed storage and returns an immutable
reference only after complete validation. A failed constructor exposes no
trusted view; input byte buffers can be reused after a successful return.
Decoded records contain no resource-bearing fields, so partial scratch needs no
destructor. The unit has a 28,672-byte metadata prefix followed by quantity,
extended price, discount, tax, returnflag, linestatus, and ship-date regions.
DOUBLE and DATE payload blocks are at most 262,144 bytes; one-byte key blocks
align to 32,768 rows. The retained native geometry permits at most 1,344
descriptor slots and 6,500,000 rows; the row maximum uses 1,294 descriptors,
247,028,672 unit bytes, and 494,028,672 temporary bytes.

Failure removes private state through the same generation-0 recovery path or
returns bounded `CleanupRequired` debt. The issued prefix survives rollback.
Hard-linking the private unit into `units/` is not a transaction commit; the
unit becomes visible only through the publication protocol in `transactions.md`.
[Verification](verification.md#persistence-and-recovery-evidence) defines the
required fixture, corruption, interruption, and native checks. Historical
format-2 measurements remain scoped to their recorded bytes in the [evidence
record](../notes/evidence.md#query-semantics-and-accepted-costs).

## Recovery and lifecycle

Recovery validates version, lengths, checksums, identities, and graph shape
before trusting references. Unknown authoritative features fail closed.
Reclamation starts from published roots and respects generation pins;
correctness never requires background cleanup to run.

No format is stable before released mutation, reclamation, backup, restore,
upgrade, corruption, and recovery transitions exist. Each stable release retains
old byte fixtures and documents forward/backward behavior. Backup is not
implemented. When admitted, it must capture one pinned consistent generation and
verify copied units before success.

## Catalog reclamation scratch and recovery

Catalog scratch ownership follows namespace format 7 below. Reclamation uses the
shared constructor and extent owner; each inventory read remains bounded and
checksummed. Its scratch bodies are disposable and introduce no stable format.

Cleanup uses the existing lease/namespace validator and a bounded, checksummed
external name/reference inventory. It never unlinks while depending on a live
namespace enumeration cursor. It protects current data/history, every pinned
data graph and every pinned resolution history. Only exact absence from that
complete protected set permits deletion. Input-run and later output-page
corruption stop cleanup. Any attempted object unlink is followed by a units
barrier, even after cancellation or an earlier unlink failure. A failed barrier
requires reopen; visible missing filenames alone do not establish durability.

## Catalog namespace format 7 scratch ownership

Format 7 replaces catalog namespace format 6; format-6 CONTROL/ROOT/WAL records
are rejected. Catalog object codecs retain their existing versions and layouts.
The namespace still has one `units/` object directory and one `private/` scratch
directory. No new persistent directory or file body format is added. Format 4
retains its legacy private staging and recovery rules.

In format 7, live authoritative inspection validates the scratch directory owner
without enumerating its changing disposable contents. Exclusive open validates
the two recognized names `private/SCRATCH.A` and `private/SCRATCH.B`. Any
surviving name must identify an empty, single-link regular file. Open removes
these names and synchronizes the directory before returning a handle. Unknown
names, nonempty named scratch, aliases and wrong object kinds fail closed at
that boundary. Scratch payloads never carry recovery or commit authority.

A database-owned bootstrap capability serializes scratch creation without taking
writer authority or holding a mutex across I/O. Both names are unlinked and the
directory barrier completes before any payload write. Owners then hold only
unlinked descriptors. An interrupted construction or failed unlink barrier
retains one bootstrap debt until exclusive reopen, even when no name is visible.
There is no live cleanup retry. This debt refuses another scratch constructor;
it carries no publication authority. Reclamation conservatively also refuses new
database admission after its `RecoveryRequired` outcome. Neither case
reinterprets data or receipts.

[`scratch::Admission`](../src/scratch.rs) owns idle, creating, and
reopen-required states. Acquisition makes one nonblocking attempt; a competing
constructor gets contention. `Scratch::create` checks eligibility and acquires
that capability. `create_files` performs the ordered file effects. The bootstrap
guard records recovery debt on failure or unwind and performs no I/O in drop. A
failure after the first namespace attempt leaves the capability unavailable
until reopen. Successful durable removal returns it to idle before payload use.

Each returned scratch owner borrows its database and holds exactly two distinct
unlinked descriptors. Owners may coexist with a writer and pinned readers after
construction. General grouping reserves creation memory before optional hash
state and transfers that reservation into the shared constructor on fallback. It
does not acquire bootstrap authority until construction starts. This prevents
fallback from needing fresh pathname admission after a competing query reserves
the released hash budget. [Resources](resources.md#catalog-reclamation) owns
extent accounting, descriptor counts, and reservation release.

## Retired snapshot fixtures

The fixed two-table prototype used pre-release format 5. Its implementation is
retired. `tools/candidate-fixtures.py` still reproduces format-3 and format-5
vectors for provenance and public unsupported-version rejection checks. Neither
format is admitted by public open. The [retirement
record](../notes/evidence.md#retired-implementations-and-gate-isolation)
identifies the source revision and production owners of transferred guarantees.
