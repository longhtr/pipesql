# Storage and recovery

A database directory contains immutable objects and a small set of records that
choose which objects are committed. Creating a file does not commit it. Updating
the authoritative roots does.

This separation makes a large append possible without rewriting the old table.
It also gives recovery a concrete question: which complete graph do the surviving
records authorize? Recovery must answer that question before removing anything.

## Follow the roots

```text
ROOT.A / ROOT.B
  ├─ catalog
  │    └─ table
  │         ├─ schema
  │         └─ data index
  │              └─ native units
  │                   └─ column payloads
  └─ success history
```

The catalog describes tables and their row and unit counts. An empty table has
a schema but no data index. Each index names native units; each unit describes
its independently readable columns. Success history records which transaction
attempt produced each generation, including schema declarations.

Objects live in `units/`. Their names combine the creating attempt number with
an ordinal within that attempt. References bind identities, lengths, and checksums;
readers validate the relationship before following the next reference. A checksum
can detect changed bytes but cannot, by itself, establish that those bytes belong
to this table or database.

Native-unit metadata must describe the entire file without gaps, overlaps,
duplicate column IDs, or trailing bytes. Physical column order can differ from
schema order because persistent IDs identify the columns. Metadata and payload
checksums are separate. Opening validates metadata; scans validate demanded
payloads. An unread column can still contain undetected damage.

## Keep authority small

| Name | Responsibility |
| --- | --- |
| `LOCK` | Hold the exclusive process lease. |
| `CONTROL` | Establish database identity and namespace version. |
| `ROOT.A`, `ROOT.B` | Select committed data and the highest issued attempt. |
| `WAL` | Record the attempted transition between snapshots. |
| `units/` | Store immutable objects. |
| `private/` | Hold recognized construction names while work is unfinished. |

The lease prevents two processes from independently changing the same database.
It belongs to an open descriptor. Before repair, its identity must still match
the LOCK pathname: a descriptor for an unlinked old lock does not authorize
changes to a replacement directory. Writers also check the saved database identity
against CONTROL.

Names must not be symlinks. WAL must have one hard link because it is rewritten
in place; an alias could otherwise let recovery overwrite a file outside its
ownership. The name, opened reader, and eventual writer are checked at their
respective boundaries.

## Issue an attempt before constructing data

An attempt number counts writes, including writes that abort. A generation counts
successful publications. They cannot be interchangeable: an aborted write must
not cause a later write to reuse its identity.

Admission checks the resources and representable capacity needed for success.
Issuance then publishes a higher attempt prefix while retaining the old data and
history. Both roots and their synchronization finish before the token is exposed
or construction starts. Failure during issuance requires reopen and exposes no
token. Once issued, an attempt stays spent even if all its private files disappear.

A token encodes the nonzero database identity and attempt number in 24 bytes.
It is an identifier, not proof of commit or an authentication credential. After
recovery, history distinguishes committed attempts from gaps. Outcomes remain
available; reaching history capacity refuses another write instead of forgetting
old results. [Operations](operations.md#resolve-a-write) explains caller handling.

## Publish a complete graph

The writer builds new objects, verifies their contents, synchronizes their files,
and synchronizes the directory entries naming them. It then transfers cleanup
authority to publication. From that handoff onward, a builder cannot delete files
that recovery might select as committed.

Issuance and data commit use the same publication sequence:

1. Validate that the candidate is a legal successor of the current snapshot.
2. Write WAL, read it back, verify it, and synchronize it.
3. Write `ROOT.A.next`, read it back, verify it, and synchronize it.
4. Replace ROOT.A and synchronize the database directory.
5. Prepare and replace ROOT.B in the same way, then synchronize the directory again.

Before the first root replacement, cancellation can stop publication. After it,
the publisher attempts to finish without further cancellation checks. An error
at that point may follow a commit, so the result is ambiguous. Only the final
barrier permits changing the live registry's current generation.

Root order is determined by `(issued attempt prefix, successful count)`. Legal
adjacent states represent issuance or the corresponding commit. A larger number
alone is insufficient. WAL constrains the transition but is not a redo log:
recovery does not replay a commit solely because WAL names it.

## Recover from surviving evidence

Reopen first acquires the lease, then selects authority, validates its graph, and
repairs the namespace. The order matters. Choosing an older readable graph when
a known newer graph is damaged would silently lose a committed write.

| Evidence | Recovery decision |
| --- | --- |
| Two valid equal roots | Select that state. |
| Two valid roots describing one legal transition | Select the newer state. |
| Authoritative roots with a damaged WAL | Repair from the roots. |
| Authoritative roots with an intact WAL | Require WAL to match or describe the legal next transition; do not replay that next transition. |
| One valid root | Require an intact WAL matching it exactly. |
| Insufficient or contradictory evidence | Refuse to open; do not guess or clean up. |

Unsupported formats, foreign identities, and checksum-valid but invalid record
contents are not ordinary damage. Recovery refuses them rather than overwriting
something it does not understand. Root readers inspect available fields before
classifying a wrong file length as damage, so truncation cannot conceal one of
these conditions. Only an exact-length record can become an authoritative root.

Repair constructs and verifies replacement roots separately from the survivors.
It synchronizes the directory even when the root bytes already look correct: a
previous process may have changed visible names and then failed to synchronize.
Both roots must be durable before unreferenced objects are removed or WAL is reset.
The matching WAL, `units/`, and `private/` are synchronized before recovery finishes.
Catalog construction cleanup precedes allocation of the live registry.

Another interruption may require another recovery attempt. A failed attempt
returns no usable handle. Shared inspection never repairs; only exclusive reopen
has that authority.

Creation follows a stricter path: it validates exactly the empty state it intended
to create, then synchronizes the directory and its parent. It does not repair away
an initialization mistake. Failed creation retains its lease through cleanup and
refuses cleanup if the LOCK name no longer identifies the held file.

## Remove only unneeded objects

Reclamation protects the current graph, pinned reader graphs, retained history,
and active outcome lookups. It builds and checks a complete inventory before
unlinking unprotected names. Deletion does not depend on a directory cursor whose
view might change while names are removed.

An inventory error stops cleanup. After any attempted unlink, `units/` must be
synchronized even if cancellation or another error prevents finishing. Failure of
that barrier makes the handle unavailable until reopen. An absent name is not
proof of durable removal.

## Make scratch disposable

Query scratch carries no recovery authority. A constructor creates and unlinks
`private/SCRATCH.A` and `private/SCRATCH.B`, then synchronizes the directory before
writing payloads. Unix descriptors keep the unnamed files usable until close.
Thus a leftover scratch name can only be valid if it names an empty, single-link
regular file. Other shapes or unknown names cause refusal, not deletion.

Only one constructor uses those names at a time. After their durable removal,
another constructor can proceed while the first pair remains in use. Construction
requires neither writer admission nor a mutex held across I/O.

Failure after the first namespace attempt but before the directory barrier disables
new scratch construction until reopen—even if the names already appear absent.
Drop records this state without I/O. Ordinary publication can still proceed;
reclamation's failed cleanup has the stronger unavailable-handle rule above.
Live queries check directory identity without listing these changing names.

## Formats and bounds

Declared tables use namespace version 7 and object codecs version 6. Legacy
`lineitem` uses namespace version 4. Other namespace versions are rejected.
None is a stable format promise; backup and format upgrades are not implemented.

| Declared-table capacity | Maximum |
| --- | ---: |
| Tables / columns per table | 64 / 64 |
| Units per table / rows per unit | 4,096 / 32,768 |
| Rows per table | 134,217,728 |
| Encoded bytes per column in a unit | 524,288 |
| Bytes per STRING value / schema name | 65,536 / 32 |
| Committed generations | 1,048,576 |

The last representable attempt can still commit. Exhaustion must be detected
before issuing an attempt whose successful outcome could not be represented.

The legacy loader reads its input twice. The first pass validates and measures;
the second must reproduce identity, size, timestamps, counts, and checksums while
writing bounded column streams. Assembly compares the streams with independently
retained checksums, then verifies the finished unit. Calculating new checksums
from damaged staging would merely certify that damage. An empty legacy database
admits its eight recognized construction files; after publication, only scratch
names are allowed in `private/`. Linking its unit into `units/` does not commit it.

Exact layouts belong beside [snapshot codecs](../src/storage/format.rs),
[catalog schemas](../src/storage/schema.rs), and [native units](../src/storage/unit.rs).
The cross-file decisions live in [publication](../src/storage/publication.rs),
[recovery](../src/storage/recovery.rs), and [reclamation](../src/storage/reclaim.rs).
