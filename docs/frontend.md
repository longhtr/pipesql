# Frontend contract

The frontend turns query text into a typed, immutable plan. Parsing recognizes
syntax and records source locations. Binding resolves names against the catalog
and the visible row, assigns column identities, and checks types and supported
operations. Validation checks the resulting graph independently. Execution uses
these established facts; it never reconstructs them from query text or names.

## Implementation owners

The [semantic types](../src/frontend.rs) hold column identities, relation state,
immutable plans, and prepared-query ownership. The phase boundaries are:

| Owner | Input and responsibility |
| --- | --- |
| [Lexer](../src/frontend/lexer.rs) | Source bytes to bounded tokens and spans; also owns the pinned reserved-word policy. |
| [Parser](../src/frontend/parser.rs) | Tokens to bounded parsed syntax. Boolean lowering is a parser child; no database is read. |
| [Binder](../src/frontend/binding.rs) | Parsed syntax and admitted source facts to a typed plan. Owns catalog reads, scope storage, names, and prepared-plan reservations. |
| [Binding admission](../src/frontend/binding/admission.rs) | Calculates retained plan bytes and allocates exact descriptor capacities after the caller reserves memory. |
| [Validator](../src/frontend/validation.rs) | Checks the complete semantic graph independently of parsing and binding. |

The DISTINCT descriptor maps old identities to fresh ones and validates its own
representation. Scope membership changes belong to the binder. Read-only column
facts are shared semantic data; mutable scope tables do not escape binding.

Binding and malformed-plan regressions live in
[`binding/tests.rs`](../src/frontend/binding/tests.rs). Tests may construct invalid
private state to challenge validation; production consumers receive validated
immutable plans.

## Trace a query through preparation

Use the `sales` table from [the library walkthrough](getting-started.md):

```sql
FROM sales
|> SELECT amount AS subtotal
|> SELECT subtotal + 1 AS adjusted
|> AGGREGATE SUM(adjusted) AS total
```

With the walkthrough's three rows, this query produces `38`. Follow
`Database::prepare` into `prepare_catalog`, then `bind_plan` in the
[binder](../src/frontend/binding.rs).

1. `parse_query` records the three transformations and their source spans. A
   parsed reference such as `subtotal` is still a name, not a storage position.
2. `prepare_catalog` pins a snapshot and builds `SourceBindings` from its schema.
   Each source occurrence has distinct query identities. Reading the same table
   twice in a join therefore creates separate inputs.
3. `bind_plan` initializes the visible row, calculates and reserves the plan's
   memory, and allocates descriptor storage. `Binder` borrows source facts and
   owns the name scope and descriptors being constructed.
4. `bind_stage` dispatches each transformation. `bind_projection` resolves
   `amount AS subtotal` to the existing column identity: a rename does not create
   a new value. The second projection binds an arithmetic expression and assigns
   a fresh identity to `adjusted`. Each SELECT replaces the visible row only
   after all its output definitions bind successfully.
5. `bind_aggregate` binds its numeric arguments and grouping keys through
   separate operations. `SUM(adjusted)` refers to the computed identity; its
   output `total` receives another fresh identity.
6. The binder releases temporary child-scope storage, completes the immutable
   plan, and calls `validate`. Only the validated plan enters `PreparedQuery`.
   The prepared handle retains the memory reservation and snapshot pin.

Preparation checks the meaning of the query; it does not read table rows or
evaluate each row's arithmetic. The expression's source span survives binding
so execution can identify a demanded overflow. See
[language semantics](language.md#current-public-query-manifest) for demand rules.
The `computed_definitions_reject_invalid_scope_identity_and_provenance` and
`repeated_aggregate_binding_preserves_identity_and_transitive_demand` tests in
[binding tests](../src/frontend/binding/tests.rs) exercise these distinctions.

## Bounds

The shared token and stage budgets constrain the whole query. Individual limits
are not independently attainable maxima.

| Representation | Limit |
| --- | ---: |
| Source text | 4,096 bytes |
| Tokens and shared parsed numeric operations | 160 each |
| Normalized stages and nested-input frames | 16 each |
| Source columns across all occurrences | 64 |
| Columns in one relation or final output | 64 |
| Projection entries across the query | 80 |
| Ordering terms across the query | 80 |
| Operations in one numeric expression | 32 |
| Aggregate output identities across the query | 10 |
| Identifier bytes | 32 |

Reject excess input before allocation or partial publication. Parsing and binding
use explicit bounded work stacks for input-controlled depth. Literal decoding
also checks source and decoded byte bounds; the accepted forms and limits belong
to [Language](language.md).

## Expressions and predicates

Quoted constants share one decoder for STRING and DATE. It checks source-payload
and decoded byte bounds and rejects unsupported quoting or escapes. The binder
stores STRING bytes inline in the logical literal; no caller text survives as a
borrow. Validation checks type, UTF-8, length and unused bytes before execution.
Logical predicates distinguish comparison from an optional-negation NULL test;
each leaf retains its resolved column identity and source span. A separate
four-byte control record gives requested truth, forward match/miss offsets and
the remaining WHERE extent. Zero rejects a row; a positive offset reaches a
later leaf or the expression's continuation. Validation rejects out-of-range
edges and branches across relational or scope boundaries. Physical filters
borrow each immutable predicate and independently check its copied control and
column mapping against the logical owner.

The parser uses bounded operator/value stacks and a temporary topological syntax
array. NOT and parentheses do not enlarge each stage's parsed literal. Reverse
iteration lowers the tree to one decision per leaf, without recursion or Boolean
distribution. All leaves bind even if runtime control can skip them. Nullability
does not substitute for demanded evaluation.

## Names, identities, and relation state

A relation separates the following facts:

- semantic `ColumnId -> (type, NULLability)` entries;
- ordered ordinary outputs `(optional display name, ColumnId)`;
- named ranges with ordered `(member name, ColumnId)` memberships;
- bounded semantic order keys; and
- base-column catalog origin `(query, source occurrence, table identity, ordinal)`.

Names are not identities. One identity may have several output names; duplicate
names remain ambiguity candidates; unnamed outputs remain typed and ordered.
Hidden range members remain semantic columns only while reachable. Every output,
range, expression, and origin reference is checked against its owning query state.

Bound expressions retain canonical values and resolved operation identities.
The current numeric programs use checked INT64 constants, finite literal DOUBLE
bits and closed arithmetic operations. DATE constants retain validated epoch-day
values after interval folding. General function calls are not supported.
Runtime DOUBLE columns may contain
every IEEE-754 value even though nonfinite numeric literal spellings are not
admitted.

For computed SELECT, binding validates each definition without evaluating
its arithmetic. Definitions reference earlier semantic columns and retain their
own source spans; source catalog ordinals are not a representation for derived
values. Names and types remain checked even when later demand removes the value.
`language.md` owns row demand and the distinct preparation-time constant rules.

The binder's order-transfer policy is owned by `language.md`. In particular,
WHERE preserves established order in PipeSQL. SELECT can hide an order key
without making it visible for later name resolution. Ordering metadata must keep
the original identity and must not be rebuilt from output positions or aliases.

The parser retains each aggregate call's source extent on its bound entry and
each numeric constant's extent in parsed syntax.
Executable Expression equality excludes provenance so equivalent SUM/AVG
arguments still share state. Binding validates nonempty ranges inside the original
source byte extent. No parser or execution component retains query text merely
to render an arithmetic error.

## Atomicity and failure

Each stage resolves its inputs before exposing its output to the next stage.
The candidate plan stays private until capacities, identity arithmetic,
references, and invariants validate. Unsupported or malformed syntax fails at parse/bind with bounded
source diagnostics. Resource refusal is distinct from invalid input. A failed
root or child cannot leave an executable partial plan.

## Plan graph and shared storage

The public path uses 160 fixed token slots, a direct parser, explicit bounded
expression stacks and at most 16 normalized stages. It accepts exactly the
composition manifest in `language.md`. Every source occurrence receives distinct
query identities, including repeated reads of one table. Aggregate outputs
receive fresh identities. Bound stages retain resolved identities, typed
constants and aggregate programs; execution never reads query text.

Relation zero is the initial source; node outputs are one through the admitted
stage count. Additional source nodes name separate occurrences. Each join names
its left and right inputs, which must precede it. Every producer must reach the
final output; each nonfinal producer has one consumer. A join consumes two
normalized nodes: its right source and the join itself. Aggregate producers
share the query-wide budget of ten output identities.

Row-shape lookup follows earlier source, projection, aggregate or join relations;
filters, aliases, ordering and limits preserve their input shape. Each lookup
decreases the relation index and visits at most sixteen nodes. No per-stage schema copy or recursive
walk is retained. Projection entries occupy one bounded list: the token limit
permits at most 80 total entries. Validation checks complete, disjoint projection
ranges, visible identities and unused tails before execution. Ordering terms use
a separate inline list of at most 80 entries across the query, also bounded by
the 160-token limit. Each term stores its resolved identity, direction and effective
NULL placement. Ordinals resolve against the current visible row. Order lookup
walks transparent nodes to the latest ordering producer; hidden order identities
remain metadata and cannot be resolved by name. Ordered grouping supplies fresh
group-output identities with ascending, NULL-first comparison. Validation checks
complete disjoint ordering slices and their visible input identities.

SELECT expressions, WHERE constants, aggregate arguments and LIMIT count/offset
share one parsed 160-operation array. Each operation consumes a distinct token;
each expression still has its own 32-operation limit. Stages and aggregate entries hold bounded
ranges with independent source extents. Binding reconstructs one expression at
a time into a fixed local buffer. This avoids multiplying complete scalar stacks
by every stage or aggregate entry, without a heap allocation or a larger stack
allowance. DATE shifts retain their separate bounded representation. LIMIT
binding publishes separate non-negative INT64 cardinalities, and validation
checks their range before execution.

### Prepared memory and nested inputs

The prepared handle owns one fallible allocation for the immutable plan. A
query with aggregation adds a descriptor vector and one entry vector per aggregate.
Both use exact admitted capacities. Stages reference their descriptor by index;
validation checks ownership and fresh output identities. Repeated aggregates bind
against the preceding relation, including any intervening computed projections.
New computations add one flat vector sized to their exact count; direct projections
add no computation vector. The database reservation precedes these allocations
and includes a 4,096-byte allocator allowance for each. Physical owners drop
before the reservation. Keeping the plan behind a small handle prevents declared
source metadata from expanding every caller's stack frame.

During binding, one private plan buffer holds the rows and stage arrays being
constructed. `Binder` borrows that buffer and immutable source facts. It owns
the temporary name ranges, pending child scopes, and descriptors; completing a
stage does not copy another maximum-width plan onto the stack. The existing
small-stack regressions exercise this preparation path.

Parenthesized FROM/JOIN inputs share the parser's bounded pools and use an
explicit sixteen-frame stack. Binding suspends pending left inputs in two
charged, exact-capacity vectors: ordinary outputs and range members share one,
and range descriptors use the other. Frame indexes restore each parent at its
JOIN. Child scopes cannot see sibling ranges. The semantic Derived node exposes
ordinary outputs, replaces ranges with its optional alias and clears the
ordering guarantee. Physical lowering fuses that boundary without allocating
another producer.

### Identity allocation

Output, projection and filter references carry plain four-byte query IDs, with
zero reserved for unused slots. Source facts live in the source-column table;
computed definitions, aggregates and DISTINCT descriptors own their semantic
type/NULLability facts.
After source IDs, binding assigns fresh IDs to computations, aggregate outputs
and DISTINCT replacements in stage order. Validation independently walks that
sequence, checks each definition against its complete input scope, and rejects forward references,
cycles, overlapping IDs and invalid spans. The conservative query bound is
1,178 identities:
64 source columns, at most 80 computations, ten aggregate outputs and sixteen
sets of at most 64 DISTINCT replacements. The shared token and stage bounds
usually admit fewer.
The 64-source-column bound covers all occurrences together, including repeated
table schemas. Each relation and final output is independently bounded to 64
columns; these are current limits, not the full release profile.

### DISTINCT

DISTINCT owns one descriptor per stage in a single charged, exact-capacity vector.
A descriptor records each unique input's semantic facts, fresh output range and
one small mapping per visible output position. Duplicate output entries share a
replacement; range names survive with remapped participating members. Validation
checks descriptor ownership, complete input coverage, canonical sharing, copied
types/NULLability and fresh identity ranges. Relation lookup reads that descriptor
without recursive identity expansion. Its identity budget is independent of
aggregation and ordering-key syntax limits.

### Catalog binding

Query-local source identities are distinct from stored column IDs. The binder
assigns IDs 1 through the total source width. Each occurrence owns a contiguous range
and its table identity; a private plan-owned array retains catalog IDs by source
slot. Its unused tail is zero. Semantic validation checks canonical query identities and the catalog map's shape;
execution checks mapped IDs, positions, types and NULLability against the pinned
schema before consuming native data. A well-shaped but false catalog ID is still
rejected at that independent check.

Numeric programs, borrowed batch inputs and aggregate key/state inputs carry only
semantic identity, type and NULLability. They cannot inspect a catalog ID or
storage slot. These facts belong to one immutable relation input; a future operator that changes NULLability
must publish new input facts, not mutate an earlier relation.

Join binding checks that same-type equality keys belong to opposite inputs.
Joining concatenates their visible columns and ranges, preserving identities.
SELECT and AGGREGATE clear earlier ranges; AS names the resulting row. WHERE
preserves ranges. Anonymous ordinary outputs have no named range membership;
an AS range can therefore have no members. Result metadata represents absence
with `Option<&str>` and never exposes generated names. Duplicate range names and
ambiguous member references fail binding, as specified in `language.md`.

The prepared handle borrows the database; running queries also borrow the
prepared plan. `accounted_memory_bytes()` reports prepared ownership separately
from running-query charges. Fixed parser stacks and caller source remain
separately bounded and observed.

The declared-table path uses the same parser and stage binder with a
row shape read from a pinned schema. It retains stable column IDs, storage
positions, types, NULLability and table identity. A prepared query owns its
generation pin. `Database::prepare` selects this source for a declared-table
database. Binding admits up to 64 source columns and global
COUNT/SUM/AVG and grouping over typed, nullable visible keys. Up to nine distinct
keys fit the query-wide ten-output aggregate budget when there is one stage. These are integration limits,
not the release profile. Source/output capacity is separate from aggregate
state capacity; a following projection may expose up to 64 output entries.

## Verification

Q1/Q6, admitted prefixes, duplicate aliases, empty input and invalid forms pass
through the public parser/binder. Independent plan validation walks the stage
sequence and checks visibility, types, provenance, output mappings, aggregate
programs, demand and bounds. Malformed-byte campaigns and exact-next allocation
checks must preserve typed outcomes and release prepared ownership. Table
subqueries have local positive, negative, stack and admission tests.
See [Language](language.md), [Planning](planning.md), and
[Verification](verification.md).

## Unimplemented relation forms

Value tables, expression subqueries, general function calls, and broader range
behavior remain unsupported. Supporting them requires preserving the existing
identity, scope, provenance, and order guarantees, and explicitly representing
regular versus value-table row shape and conservative cardinality. These are
future obligations, not fields or capabilities of the current plan.
