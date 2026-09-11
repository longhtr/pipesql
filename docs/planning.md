# Planning contract

Logical planning consumes an immutable bound plan and pinned catalog snapshot.
It cannot inspect query source, resolve display names, repeat binding, or mutate
the binder's relation states. Planning is deterministic for fixed semantic and
catalog inputs; a bound is exceeded by typed refusal, not an arbitrary partial
plan.

## Typed relational plan

Nodes reference stable `ColumnId` values and carry typed expressions,
three-valued logic, NULLability, row shape, semantic order, cardinality, catalog
origins, canonical values, resolved functions, and child identities. Rewrites
preserve values, NULLs, required order, demanded evaluation/errors, and effects.
Rows are unordered unless a language rule establishes order.

The current immutable plan retains bounded projection, filter and aggregate
nodes with explicit backward input references. Source and aggregate identities
survive aliases and reordering. Validation checks each node against its input
row shape and rejects unreachable nodes. Lowering follows producer ancestry to
classify source and aggregate filters. Final outputs and later predicates
determine aggregate demand. Join nodes record explicit source occurrences and
two inputs. Source identities are unique across occurrences, including repeated
reads of one table. A backwards validation pass checks that every producer is
reachable and has one consumer; execution cannot accidentally share a mutable
cursor between inputs. Relation widths are checked independently, and row-shape
lookup follows bounded earlier references without recursion. Join keys preserve
aggregate demand even when final projection drops those values. Physical
lowering represents both inputs. The runtime schedules scans, joins, ordering,
limits and aggregates through those edges; native construction retains a
separate cursor for each source occurrence. There is no cost optimizer. Declared
grouping and equality joins reuse the external sorter.

Order transfer follows `language.md`: nonanalytic SELECT, WHERE, AS and LIMIT
preserve the ordered-key list, including hidden identities; JOIN, DISTINCT and
ordinary AGGREGATE clear it. Ordered grouping and standalone ORDER BY establish
their declared keys. WHERE preservation is PipeSQL's explicit stronger
guarantee, not an inference from the pinned analyzer's FilterScan property.
Bound ordering terms retain identities, directions and effective NULL placement.
Ordered grouping derives equivalent terms from its fresh group-output
identities.

LIMIT preserves row shape, identities, ranges and known order. It separates
producer pipelines so a following predicate cannot be fused into its input.
Bound count and offset remain separate validated semantic values; planning does
not add them into a potentially overflowing signed extent. Preserve upstream
aggregate and order-key demand. A zero count is not permission to erase binding,
resource admission or metadata validation through an empty-plan rewrite.

## Physical plan

The physical planner combines the typed plan with one pinned generation. The
complete planning contract requires the following facts as corresponding
operators are admitted. The current representation described below stores graph
and mapping facts; execution admission still calculates resource requirements:

- exact operator graph and typed input/output identities;
- table, column, and immutable-unit mappings from catalog identity, never names;
- vector encodings, validity and selection requirements;
- required/preserved order and partitioning;
- nonrevocable memory minimum, revocable growth account, and batch/work bounds;
- spill algorithm, temporary minimum, fan-out/depth, and cleanup owner where needed;
- effect, cancellation, blocking, and wakeup behavior; and
- snapshot generation and unit identities retained through execution.

### Current representation

The private physical plan binds one database identity and generation to the
validated legacy root or pinned catalog and native-unit facts. Its bounded
vector contains scan, aggregate, join, ordering/DISTINCT, and limit producer
pipelines. Each pipeline records its input producers, ordered filters, output
identities and producer-local positions. Transparent aliases, projections and
filters fuse with their producer. Backwards demand retains join and ordering
keys and needed aggregate arguments while omitting unused source payloads. Root
projection preserves duplicate output entries; intermediate batches carry each
demanded identity once.

Computed numeric definitions form a bounded, acyclic graph over semantic
identities. Each pipeline borrows that graph and maps identities to its raw
producer positions or local computed slots. A materialized input maps to a raw
position and is not recomputed across the boundary. Demand uses a fixed bitset
sized from the frontend identity ceiling, including fresh DISTINCT outputs and
hidden keys. Repeated references never expand the graph into expression trees.
Within each consecutive SELECT/WHERE/AS sequence, demand follows predicate order
and then surviving-row outputs, as specified in `language.md`. Column pruning
alone is insufficient: the planner must preserve which rows demand a definition.
A separate eager projection producer cannot force aggregate finalization before
a following predicate that does not need that value.

Lowering exposes each predicate's required dependencies and the remaining output
requirements to the producer. Both validators must check types, NULLability,
earlier-definition references, identity capacity and demand against the bound
sequence. Keep source-storage mappings separate from derived facts. JOIN,
aggregate input, ordering and LIMIT retain their documented materialization and
consumption boundaries. No rewrite may erase their required values or failures.

Validation checks source occurrences, both join inputs, aggregate demand,
ordering slices, limit bounds and reachability. Ordering validation checks each
physical key position and comparison policy against its bound term. It resolves
physical positions back to semantic identities instead of repeating the
planner's identity-to-position search. Source text and display names are not
physical inputs. Native admission compares each source occurrence with its own
pinned catalog schema. `ScanLayout` calculates demanded buffers, metadata
scratch and typed batch payloads before memory admission and namespace I/O.
Aggregate state admits bounded group cells and adaptive expression scratch from
the same database memory authority. Current resource requirements are calculated
during execution admission from the validated plan and available database
budget; the physical plan does not yet store a general operator resource graph.

Declared grouping derives key types and NULLability from bound source
identities. It calculates aggregate layout and projected result widths before
allocation, then reserves the complete sort/reduction minimum before adaptive
buffers and optional hash state. A full hash table or key arena permits one
replay of the complete input into argument sorting. A join or ordering input
replays retained sorted rows; a scan input restarts its pinned cursor.
Intervening LIMIT stages reset their counters and batches as the runtime walks
to that replay owner. The sort path retains each group's original argument order
and validates final aggregate values before publishing rows. `resources.md` owns
the admission equation and sizing policy; `execution.md` owns the controller
transitions and cleanup.

DISTINCT is a materialization boundary with a separate physical producer.
Backwards demand retains every unique input field, even when the consumer drops
its replacement. Output and computation mappings resolve fresh identities to the
child's demanded positions; independent validation resolves those positions back
to identities and checks complete input coverage. It cannot fuse a downstream
predicate into the input or substitute ordinary equality for duplicate grouping.
The sorted-input owner retains all keys and can replay the same deduplicated
relation for one grouping fallback.

## Independent validation

Execution accepts only a validated plan. For the complete planning contract,
validation must check identities, types, NULLability, schema agreement, acyclic
ownership, operator arity, required properties, resource arithmetic, external
paths, legal effects, and snapshot membership through a bounded graph walk.
Validation never trusts optimizer-produced estimates for allocation arithmetic.

The current validator independently checks semantic stages, snapshot/root/unit
facts, source mappings, output shape, filter types, aggregate sharing/demand,
grouping keys and workspace geometry. Q1/Q6 are compositions through this same
path. Legacy tables admit up to two printable-ASCII STRING keys and a complete
domain of at most 8,836 groups; direct indexing is bounded under the existing
memory cap. Declared tables admit up to nine distinct typed, nullable visible
keys within the ten-column aggregate-stage limit. Their hash and sort paths
share the same checked aggregate arithmetic. These are separate physical
strategies for the bound grouping semantics, not alternate parsers or binders.

## Verification

Compare public compositions against an independent semantic result and a simple
no-rewrite control. Corrupt every plan field class, deny every next owner, and
prove that no native payload reaches a kernel before plan and unit validation.
Record optimized stock plan shape, bytes, work limits, and resource equation.
