# PipeSQL language

This guide defines user-visible query syntax and semantics. Start with the
[current manifest](#current-public-query-manifest) to write a query. The
detailed sections explain names, values, ordering, and evaluation. Explicitly
labeled target sections describe unimplemented forms and do not authorize public
syntax. [Historical frontend
boundaries](../notes/evidence.md#query-semantics-and-accepted-costs) retain the
retired experiment cases; they do not authorize public syntax.

## Authority

The initial upstream study baseline is GoogleSQL release `2026.7.2`, commit
`0e7d7073ed0360be587a5efa0fa78abeee00f17b`. At that revision:

- `docs/pipe-syntax.md` is 3,267 lines and has SHA-256
  `193c6759f11a5a195e0b262740418668ece55fecdaf725e2a8f9ce3a616e57c2`;
- the analyzer contains 46 `pipe*.test` files with about 50,000 lines; and
- the examples include all 22 TPC-H queries in pipe form.

Those facts make the upstream surface concrete; they do not make PipeSQL fully
GoogleSQL-compatible. The pinned TPC-H examples are not all accepted unchanged:
Q15 begins with a traditional `WITH` and contains a traditional scalar subquery,
while Q16 and Q22 also contain traditional scalar subqueries. PipeSQL must
reduce or rewrite those cases into its `FROM`-only profile before they can enter
an accepted manifest. The repository extracts a small manifest of accepted
syntax and semantics from the immutable revision. Changing the revision is a
reviewed language change with a before/after corpus.

Evidence is used in this order:

1. a reduced analyzer or reference-implementation result at the pinned commit;
2. pinned analyzer fixtures and grammar;
3. pinned reference documentation; and
4. the pipe-syntax paper for design intent.

Product documentation can omit cases, and grammar alone cannot establish name,
type, NULL, alias, or order behavior. A disagreement is preserved as a reduced
case and the disputed feature remains unsupported until resolved.

## Deliberate profile

GoogleSQL permits pipe operators after a traditional query and permits standard
and pipe subqueries to mix. PipeSQL deliberately does neither. Every relational
query and subquery starts from `FROM` and continues only with pipe operators.
There is no traditional-query parser followed by a translator.

The first-release target is narrower than GoogleSQL:

| Area | First-release direction |
|---|---|
| Query root | `FROM` with one relation, pipe subquery, `UNNEST`, or admitted table function |
| Column operators | `SELECT`, `EXTEND`, `SET`, `DROP`, `RENAME`, `AS` |
| Relational operators | `WHERE`, `AGGREGATE`, `DISTINCT`, pipe `JOIN`, `ORDER BY`, `LIMIT` |
| Set and naming | `UNION`, `INTERSECT`, `EXCEPT`, pipe `WITH` |
| Analysis | window expressions in projection operators, `ASSERT`, `DESCRIBE` |
| Persistent sinks | terminal `CREATE TABLE` and `INSERT` |
| External sink | terminal `EXPORT DATA` through explicit host authority |
| Completion | terminal `FINISH` |
| Deferred | `CALL`, recursive union, sampling, pivot/unpivot, pattern matching, `IF`, `FORK`, `TEE` |
| Rejected | traditional query blocks, mixed syntax, deprecated pipe `WINDOW`, graph syntax, scripting |

This table is direction, not parser authorization. Each row becomes accepted
only when its exact manifest cases and complete operator contract exist. A
missing feature is a parse or bind error, never an undocumented partial form.

Non-relational administration and transaction control may use compact statement
syntax. Any statement that consumes a relation receives a pipe query or is a
terminal pipe operator; it does not embed a traditional query block.

## Current public query manifest

Queries start with a named table or a parenthesized pipe query. Legacy databases
expose `lineitem`; declared-table queries may add sources through the equality
JOIN form below. Every declared source comes from the prepared query’s pinned
catalog.

Legacy `lineitem` has seven required stored columns: DOUBLE quantity, extended
price, discount and tax; one-character printable-ASCII STRING return flag and
line status; and DATE ship date. Their names are `l_quantity`,
`l_extendedprice`, `l_discount`, `l_tax`, `l_returnflag`, `l_linestatus` and
`l_shipdate`. The STRING domain is bytes `0x20..=0x7e` excluding delimiter
`0x7c`: space is a value and `|` is not. Storage and ingestion limits are
separate from this query manifest.

| Construct | Accepted public behavior |
|---|---|
| `FROM table [AS alias]` | Returns the named source’s columns, or feeds the following stages. Legacy databases expose only `lineitem`. The table name supplies the range name when AS is absent. Additional sources enter through JOIN. Comma-separated FROM inputs remain unsupported. |
| `FROM (pipe_query) [AS alias]` | Uses the child query’s ordinary outputs as an independent input. A JOIN may also use this form. See [table subqueries](#table-subqueries) for scope and ordering. |
| `AS alias` | Names the current row as a range and replaces earlier range names. It preserves values, ordinary output names and column identities. |
| `SELECT expression [AS alias], ...` | Selects visible columns or computes INT64/DOUBLE expressions using literals, parentheses, unary `+`/`-`, and binary `+`, `-`, `*`. Star expansion and other scalar expressions remain unsupported. |
| `WHERE name comparison constant` | Accepts `<`, `<=`, `=`, `!=`, `>=`, `>` over numeric, DATE or STRING columns and compatible constants. |
| `WHERE name IS [NOT] NULL` | Tests a visible column, including a computed or aggregate output. IS NULL retains NULL values; IS NOT NULL retains non-NULL values, including zero, empty text and NaN. Both tests preserve column demand and input order. |
| `WHERE` Boolean expression | Comparisons and NULL tests compose with NOT, AND, OR and parentheses. NOT binds above AND, which binds above OR. Each leaf consumes one normalized stage; inclusive BETWEEN consumes two. See the demand rules below. |
| `AGGREGATE SUM(expression) AS name`, `AVG(expression) AS name`, `COUNT(*) AS name` | Aggregate stages may repeat; each consumes the preceding relation. All aggregate stages together may introduce at most ten output identities, including grouping keys. Every entry requires an explicit alias. SUM/AVG accept INT64 or DOUBLE expressions; COUNT accepts only `*`. SUM preserves the argument type; AVG returns DOUBLE. |
| `DISTINCT` | Removes duplicate complete rows on declared tables. Preserves output names and shared identities through fresh replacements; clears order. See [equality](#values-null-and-equality). |
| `LIMIT count [OFFSET skip_rows]` | Selects a prefix on legacy or declared tables. Count and offset are non-negative INT64 constant expressions; see [LIMIT](#limit) for demand and error rules. |
| `GROUP BY key [, key]` | Legacy tables group by up to two distinct visible source STRING identities. Declared-table keys are specified below. Group aliases inherited from earlier projections are valid. |
| `GROUP AND ORDER BY key [, key]` | Additionally establishes ascending key order, preserved by following projections and filters. Ordinary GROUP BY establishes no semantic order. |

GROUP BY and GROUP AND ORDER BY are clauses of the aggregate stage. Declared
tables also admit `|> ORDER BY key [ASC|DESC] [NULLS FIRST|LAST], ...`. A key is
a visible direct column reference or a one-based visible-column ordinal. ASC
defaults to NULLS FIRST; DESC defaults to NULLS LAST. All four current scalar
types are orderable: INT64 and DATE use numeric order, STRING uses UTF-8 byte
order, and DOUBLE puts all NaNs below negative infinity and treats signed zeros
as equal. Sorting preserves raw values. Repeated keys are legal; later
occurrences of the same identity cannot distinguish an earlier tie. Expressions,
collations and legacy format-4 standalone sorting are rejected during
preparation.

Each order stage preserves the visible row and ranges. SELECT, WHERE, ORDER BY
and LIMIT may repeat before and after aggregation. Group columns precede
aggregate entries in the aggregate output. Aggregation replaces the visible
relation; earlier non-grouping source names are no longer available. A final
SELECT is optional, and every accepted relational prefix is executable.

Each SELECT resolves all names against its complete input before publishing any
alias. A column reference, including parentheses around it, retains its identity
and implicit name. Numeric unary `+` retains identity but has no implicit name;
other numeric computations receive fresh identities and are unnamed without AS.
`ResultColumn.name` is None for an unnamed output. Such outputs remain in the
row and can be used by ORDER BY ordinal, but cannot be referenced by name.
Duplicate names are permitted in output and become a bind error when
subsequently referenced ambiguously, even if both names refer to the same
identity. Dropped names disappear. An aggregate producer assigns new identities
to aggregate outputs; their aliases and later positions are not their identity.

Direct references may use `range.column` in projections, predicates, numeric
arguments and grouping keys. WHERE preserves ranges. SELECT and AGGREGATE remove
earlier ranges; a following AS names the resulting row. Duplicate range names
are rejected. Duplicate member names are ambiguous even when their identities
are equal.

Declared-table queries execute `|> JOIN table [AS alias] ON left.column =
right.column`. The condition must connect same-type columns from the two inputs;
either operand order is accepted. Projected, grouped and previously joined left
inputs compose with the join. Every matching pair is retained, including
duplicate keys. NULL and NaN do not match; signed zeros compare equal. Joining
establishes no output order. Later projections, filters and aggregate stages
operate on the joined relation.

Either input may be a parenthesized pipe query with an optional alias.
Non-equality or compound conditions, outer joins, USING and key coercion remain
unsupported. Legacy format-4 queries reject joins during preparation because
their storage path does not admit the required scratch owner. The declared-table
integration has scoped semantic and failure tests; full release qualification
and representative join performance remain open.

Numeric expressions admit visible numeric columns, INT64 and finite DOUBLE
literals, parentheses, unary signs, addition, subtraction and multiplication.
Constants in WHERE use that same arithmetic with no column references. Integer
subexpressions use checked INT64 arithmetic before any DOUBLE conversion; an
integer overflow is not rescued by a later floating operand. Finite-operand
DOUBLE overflow is `ArithmeticOverflow`. Runtime DOUBLE values preserve raw
IEEE-754 bits, including nonfinite values. Division, scalar calls beyond the
DATE forms below, other scalar expression forms and NULL literals are
unsupported.

DATE literals and constant DATE_ADD/DATE_SUB accept checked INT64 intervals in
DAY, MONTH or YEAR units, with at most eight nested calls. Month/year shifts
clamp to the target month's last day. Every literal and intermediate result must
remain in years 0001 through 9999. Numeric columns compare against numeric
constants; DATE compares against DATE. COUNT-to-INT64 comparison remains exact;
comparison with a DOUBLE constant uses DOUBLE conversion. STRING compares
against STRING constants in UTF-8 byte order, without normalization or
collation.

STRING constants use one single- or double-quoted token. Both its source payload
(excluding delimiters) and decoded UTF-8 value are limited to 32 bytes. Empty
strings, Unicode and escaped control characters are supported. Backslash escapes
are `\a`, `\b`, `\f`, `\n`, `\r`, `\t`, `\v`, `\\`, `\?`, `\"`, `\'`, `` \` ``;
three-digit octal `\000` through `\377`; two-digit hexadecimal `\xhh`/`\Xhh`;
four-digit `\uhhhh`; and eight-digit `\Uhhhhhhhh`. Numeric escapes encode
Unicode scalar values: surrogate values and values above U+10FFFF are rejected.
Unescaped newlines, unknown escapes, raw prefixes, triple quoting and adjacent
literal concatenation are unsupported. DATE uses the same quoted-token decoder
before calendar validation. Prepared constants own their bytes after query text
is released. These forms follow the pinned [literal
evidence](../notes/evidence.md#query-semantics-and-accepted-costs).

Ordinary comparisons with NULL produce UNKNOWN; a standalone WHERE comparison
therefore rejects those rows. IS NULL and IS NOT NULL instead produce TRUE or
FALSE for every current scalar type; they do not compare against a NULL literal.
Each test consumes one normalized predicate stage and may appear in a Boolean
expression. When demanded, it evaluates its referenced column even when semantic
nullability predicts the Boolean result: demanded arithmetic errors and source
corruption remain observable. IS TRUE/FALSE/UNKNOWN, IS DISTINCT FROM, Boolean
value columns and general scalar Boolean expressions outside WHERE remain
unsupported. The [NULL predicate
record](../notes/evidence.md#query-semantics-and-accepted-costs) preserves
pinned semantics and the independent-oracle boundary for NaN. DOUBLE comparisons
use IEEE unordered-NaN behavior; both signed zeros compare equal. Empty global
aggregation produces one row containing NULL SUM/AVG and zero COUNT. Empty
grouped input produces no rows. SUM preserves the first value's signed zero and
uses the aggregate exceptional/range rules below; AVG-only state must not
inherit a discarded SUM's overflow dependency.

Final outputs and later predicates determine which aggregate expressions are
demanded. An unused aggregate argument is not evaluated. Group membership and
required ordering remain demanded even when projections hide group keys. Later
WHERE expressions run in stage order; a rejected group does not demand later
WHERE stages or unused final SUM values. Every demanded argument error and every
retained final overflow is checked before any aggregate result batch is
published. Ordinary scan results may be a prefix followed by failure; see
`interfaces.md`.

Source is limited to 4,096 UTF-8 bytes, 160 tokens and 32-byte
identifiers/literal payloads. At most 16 normalized stages and 64 ordinary
output columns are admitted. Aggregate stages together introduce at most ten
output columns, counting each stage's grouping keys and aggregate entries; a
later projection may repeat those identities in up to 64 outputs. Each simple
predicate consumes one normalized stage; BETWEEN consumes two. Numeric programs
and their explicit parser stack each have at most 32 slots. Bare identifiers use
ASCII letters, digits and underscore; quoted identifiers remain unsupported. The
pinned 95 always-reserved and four conditionally reserved keywords (`ALIGN`,
`GRAPH_TABLE`, `MATCH_RECOGNIZE`, `QUALIFY`) are reserved, including as aliases.
Nonreserved syntax words such as DATE and AGGREGATE may be aliases. Text, syntax
and semantic bounds fail before a partial plan escapes. Parse, bind and
arithmetic errors carry source spans. Arithmetic failures name the operation and
the complete constant expression or enclosing aggregate call; see [Expressions,
errors, and effects](#expressions-errors-and-effects) for attribution and
ownership.

The retained Q1/Q6 sources are representative queries through these operators,
not separate parser or execution templates. The public corpus and pinned fixture
study support this admitted subset; they do not establish full GoogleSQL
conformance. Traditional query blocks, every unlisted operator and every
unlisted form within a listed operator remain parse/bind errors. The broader
direction above becomes public only after its complete contract and evidence
exist.

## Current declared-table queries

Databases created with `Database::create_empty` admit source projections,
numeric, DATE, and STRING comparisons, NULL tests, Boolean filter expressions,
and repeated global or grouped aggregate stages through the same pipe parser and
binder. Names resolve against the prepared query's pinned catalog generation;
later appends and declarations do not change that query's source. Stored INT64,
DOUBLE, UTF-8 STRING, DATE and NULL values retain their declared types. The
comparison and demanded-evaluation rules above apply. Global COUNT(*) counts
every row reaching the aggregate stage. SUM and AVG ignore NULL arguments and
return NULL for empty or all-NULL input; COUNT(*) returns zero only for empty
input. SUM(INT64) returns an exact INT64 result or overflow if the final sum is
outside INT64. Intermediate aggregate sums may exceed INT64; a later
cancellation can bring the result back into range. This does not suppress
checked overflow in a demanded scalar argument. AVG(INT64) returns DOUBLE and
does not require its sum to fit INT64. AVG and SUM on DOUBLE retain the
exceptional-value rules below.

Projection and filters may follow aggregation, preserving exact INT64
comparisons and demanded errors. The same aggregate signatures are available on
the legacy `lineitem` path. COUNT(expression) remains unsupported. Legacy
grouping retains its two bounded STRING-key restriction.

Declared GROUP BY accepts distinct visible INT64, DOUBLE, DATE and UTF-8 STRING
identities, including nullable columns. Keys retain their types and NULLability
and precede aggregate entries in the stage output. The query-wide ten-output
aggregate budget permits up to nine keys in a single aggregate stage; other
aggregate stages consume the remaining budget. Aliases from prior projections
are valid, including named computed columns. Listing two aliases for the same
identity is rejected. Inline expressions as grouping keys remain unsupported;
define them in a preceding SELECT.

NULLs form one group. All NaNs form one group, and both signed zeros form one
group. The executor retains the first scanned representation of an equivalent
key; this does not establish semantic input order. GROUP AND ORDER BY sorts keys
lexicographically ascending: NULL first, then NaN before other DOUBLE values,
numeric/date order for those types, and binary UTF-8 order for text. Ordinary
GROUP BY establishes no order. Following projections and filters preserve an
established key order, including when they hide keys. Memory and disk execution
preserve each group's argument fold sequence; neither promises exact floating
summation. Empty grouped input produces no rows. The demanded-error rule above
applies before any grouped result is published.

Catalog query binding admits the complete stored schema of up to 64 columns.
Only demanded columns receive payload buffers; projecting a late column does not
require admission for unrelated payloads. Memory admission may still refuse a
wide result before query I/O. These current limits do not replace the target
language profile.

## Table subqueries

A FROM or JOIN input may be a parenthesized pipe query, optionally followed by
`AS alias`. Its ordinary output names, identities, types and NULLability become
the input relation. Inner range aliases are not visible outside it. Without an
explicit alias, the subquery has no implicit range name. An explicit alias names
only its ordinary outputs; duplicate and unnamed outputs keep the ordinary
name-resolution rules.

A table subquery binds independently of sibling FROM/JOIN ranges. It cannot
reference an outer input as though that range were a catalog table. Correlated
expression subqueries have their separate target contract below.

Table-subquery boundaries preserve expression demand; they do not force unused
computations. The boundary does not preserve the child's semantic order. ORDER
BY and LIMIT inside the child still determine its selected rows and retain their
demanded-evaluation obligations. An outer ordering guarantee requires an outer
ordering operator. This follows the pinned table-subquery ordering witness
recorded in the [derived-relation
investigation](../notes/evidence.md#query-semantics-and-accepted-costs). The
implementation shares the outer query’s token, source, stage, projection and
aggregate limits. Each derived boundary consumes one normalized stage. Parsing
uses at most sixteen explicit child frames. See
[Verification](verification.md#required-composition-regressions) for required
scope, demand, stack, and admission checks.

## Relation state

The binder describes the state after every stage. That state contains:

- semantic columns keyed by stable identity, each with type and NULLability;
- ordered ordinary-output entries, each pairing an optional display name with
  one mandatory semantic column identity;
- visible range variables with explicit ordered `(member name, identity)`
  entries; and
- semantic ordering known at that point.

Scope and output visibility are separate binder facts. Future named relations,
value tables, and `JOIN ... USING` need additional rules described under the
[target relational model](#target-relational-model).

Display names belong to ordinary-output entries or named range memberships, not
to semantic columns. An expression output may be unnamed while retaining its
mandatory identity; unnamed entries do not participate in name resolution. Range
memberships are always named. One semantic identity may have several
simultaneous output names and may appear more than once in ordinary output.
Duplicate display names may exist where GoogleSQL permits them and become errors
only when a later reference is ambiguous, even if duplicate entries happen to
reference the same identity. A removed output name does not remain visible
through an accidental semantic or physical slot.

A pipe operator resolves names against the complete state immediately to its
left plus its explicit arguments. Alias preservation and replacement follow the
pinned analyzer for that operator; there is no one generic alias rule applied to
all operators.

## Values, NULL, and equality

PipeSQL uses SQL NULL semantics:

- ordinary comparison with NULL produces NULL;
- `AND`, `OR`, and `NOT` use three-valued logic;
- `WHERE` retains only rows for which its expression is TRUE;
- joins apply the admitted GoogleSQL condition semantics; and
- grouping, `DISTINCT`, and set operations use their specified not-distinct
  relation rather than ordinary `=`.

Bodyless `|> DISTINCT` groups all ordinary-output identities on declared tables,
including all 64 visible columns. Repeated output entries sharing one identity
use one key. Each unique input identity receives one replacement identity;
output names, positions and shared-identity relationships are preserved. Visible
ranges remap participating identities and remove hidden memberships. DISTINCT
clears semantic order and creates no rows for empty input.

NULLs group together, all DOUBLE NaNs group together, and signed zeros group
together. Other values use their typed equality; STRING compares UTF-8 bytes.
Each output row retains one input representative, including its original
floating bits. No particular representative or incidental output order is
promised. Every input grouping value is demanded before duplicate removal, even
if a later projection drops that field. Subsequent predicates and computations
consume the deduplicated relation. DISTINCT may repeat within the ordinary stage
bound and does not consume the aggregate-output budget. A body, BY clause,
DISTINCT ON, aggregate-call DISTINCT and legacy format-4 DISTINCT remain
unsupported.

Every admitted type separately defines literal syntax,
representation-independent range, casts, arithmetic, comparison, grouping,
hashing, ordering, storage, and interchange. These are distinct capabilities.
Supporting a type in projection does not automatically make it groupable or
orderable.

For `DOUBLE`, finite arithmetic and non-finite inputs follow the pinned
GoogleSQL IEEE-754 contract; an operation on finite inputs that would produce a
non-finite value returns an overflow error. Floating `SUM` ignores `NULL`,
returns `NULL` for empty or all-`NULL` input, returns NaN when any input is NaN,
and may return an infinity or NaN for infinity input as specified by the pinned
aggregate contract. Its floating result is nondeterministic because accumulation
order is not semantic order. Tests must check the documented outcome set and
special-value rules rather than require cross-engine bit identity. This
nondeterminism does not permit an answer unrelated to the admitted input values
and IEEE-754 operations.

Aggregate state is not a scalar addition expression. A temporary SUM exceeding
the DOUBLE exponent range must not hide a later NaN, infinity, or reduction by opposite-signed inputs.
For example, with `M` equal to the largest finite DOUBLE, SUM over rows
containing `M, M, NaN` returns NaN; SUM over `M, M, -M` returns `M`; and SUM
over `M, M` overflows. An average of finite inputs does not require their sum to
fit DOUBLE: AVG over two rows containing `M` returns `M`. These cases constrain
exceptional behavior, not the exact rounding of arbitrary floating aggregates.
Errors while evaluating demanded argument expressions remain errors; a later NaN
does not rescue an overflowing scalar multiplication.

The target type set also includes Boolean values, fixed decimal, bytes,
timestamp, interval, arrays, and structs. These additional types remain
unimplemented. A type enters only with edge cases and independent vectors for
every exposed capability.

PipeSQL does not inherit a moving compiler or operating-system Unicode,
timezone, decimal, or float conversion behavior as persistent semantics. Any
external semantic data is pinned, generated reproducibly, and versioned when it
can affect stored values or query results.

## Order

Rows are unordered unless the language contract establishes order. An `ORDER BY`
establishes a bounded ordered list of binder-resolved column identities,
directions, and effective NULL placements; display names and physical positions
are not order keys. `LIMIT` observes order only when order is known at that
point. Each operator has an explicit order-transfer rule. A nonanalytic
`SELECT`, `WHERE` and `AS` preserve the complete incoming order. `JOIN` and
ordinary `AGGREGATE` clear it; GROUP AND ORDER BY establishes its own key order.
A later ORDER BY replaces the earlier ordered-key list. Rows tied on all
declared keys have no promised relative order.

Preservation through WHERE is an explicit PipeSQL guarantee stronger than the
pinned GoogleSQL analyzer's order property. The retained [order-preservation
fixture](../notes/evidence.md#query-semantics-and-accepted-costs) marks its
input OrderByScan ordered but does not mark the resulting FilterScan ordered.
PipeSQL keeps its existing promise that filtering returns an ordered
subsequence; implementations and rewrites must preserve it. This difference does
not permit changing predicate evaluation or demanded errors.

A nonanalytic SELECT also preserves ordering when it hides keys. If projection
omits an order key, its identity remains hidden semantic state: it is neither
ordinary output nor name-resolvable, but the ordered-row fact does not disappear
or move to a projected expression identity.

Physical stability is never semantic order. Scan layout, hash iteration,
parallel scheduling, spill partitions, and coincidental input order cannot be
used to justify deterministic results. When output is semantically unordered,
tests compare the appropriate multiset rather than blessing one accidental
sequence.

## LIMIT

The current public form is `|> LIMIT count [OFFSET skip_rows]` on both declared
and legacy tables. Count and offset are non-negative INT64 constant expressions
using the existing bounded numeric grammar: literals, parentheses, unary signs,
addition, subtraction and multiplication. Column references, parameters, NULL,
DOUBLE results, LIMIT ALL and other expression forms remain unsupported. Binding
validates both constants and their arithmetic even when the query would produce
no rows. Offset defaults to zero. Each value may reach INT64_MAX; their sum need
not fit INT64.

The operator skips up to offset input rows, then returns at most count rows.
Exhaustion can return fewer. It preserves the visible row, identities, ranges
and the complete incoming order, including hidden keys. It creates no ordering
for unordered input. A later WHERE filters only the selected prefix; predicates
cannot move across LIMIT merely because they refer to the same columns.

LIMIT bounds output, not input processing. It permits consumption to stop after
the prefix. It does not remove bound aggregate demand or ordering keys, suppress
an encountered failure, or weaken a blocking producer's validation before
output. Skipped rows and work needed to produce the prefix can fail. Batch
prefetch, metadata admission and blocking operators can process more than the
returned rows. A successfully completed LIMIT 0 produces no rows; that
cardinality rule is not a general promise to suppress upstream errors or avoid
resource admission. The [boundary
study](../notes/evidence.md#query-semantics-and-accepted-costs) separates value
semantics from reference-engine evaluation timing.

## Expressions, errors, and effects

An expression is evaluated only when the relation demands it. Optimization may
remove an unused pure expression. It may not remove an assertion, constraint,
persistent sink, or another semantic effect.

### Boolean filter demand

WHERE comparisons and NULL tests compose with `NOT`, `AND`, `OR` and
parentheses. Comparison and NULL tests bind above NOT, followed by AND and then
OR. BETWEEN is inclusive and its internal AND belongs to its two bounds. All
names, types and constant operands are checked during preparation, including
skipped branches. The shared 160-token and 16-normalized-stage limits bound the
expression; parentheses and NOT do not consume additional normalized stages.

A filter determines whether its expression is TRUE. It visits operands in
written order and skips a subtree when the requested result is already decided.
UNKNOWN is neither TRUE nor FALSE. NOT reverses the requested truth value:
proving an AND TRUE requires both operands TRUE; proving it FALSE requires one
FALSE operand. For OR, proving TRUE requires one TRUE operand and proving FALSE
requires both FALSE. This rule applies recursively to the logical expression;
implementation must use bounded iteration.

For example, `NULL AND bad` cannot be TRUE, so a WHERE filter skips `bad` after
observing NULL. `NOT (NULL AND bad)` must inspect `bad`, because FALSE would
make the filter TRUE. `NOT (NULL OR bad)` cannot be TRUE and skips `bad`. Here
NULL illustrates an operand's value; a bare NULL predicate literal is not
admitted. These are PipeSQL evaluation guarantees within the upstream
three-valued truth table. They preserve the existing ordered AND behavior. They
do not promise which error wins between different rows in one bounded batch.

Negation preserves UNKNOWN and operates on the comparison result. It does not
reverse the comparison operator: for NaN, `NOT (n < 0)` is TRUE while `n >= 0`
is FALSE. A skipped branch does not demand its computed value or source payload.
A consumed storage block still receives its ordinary complete validation;
short-circuiting cannot selectively trust corrupt bytes in that block.

### Computed projection demand

Numeric SELECT expressions extend the demand behavior of direct projections and
aggregate results. A SELECT defines values; encountering its syntax does not
force their evaluation.

Within a consecutive sequence of nonanalytic SELECT, WHERE and AS stages, WHERE
expressions run in written order. Each expression demands leaf dependencies
under the Boolean rules above, only for rows retained by preceding WHERE stages.
A rejected row does not demand later WHERE stages or remaining projected values.
Surviving rows then demand the columns required by query output or the next
consuming relational operator. Dependencies resolve against each SELECT's input
scope; this rule does not make same-list aliases or later definitions visible.

For example, on INT64 input `[1, 2]`, this query returns INT64_MAX:

```sql
FROM facts
|> SELECT id, id * 9223372036854775807 AS bad
|> WHERE id < 2
|> SELECT bad
```

Putting `WHERE bad > 0` before `WHERE id < 2` instead demands the overflowing
multiplication on 2. Dropping `bad` entirely does not evaluate it. These are
PipeSQL guarantees, not a claim that every GoogleSQL optimizer configuration
uses this evaluation schedule. The [demand
review](../notes/evidence.md#query-semantics-and-accepted-costs) retains the
pinned source observations and counterexamples.

JOIN, AGGREGATE, ORDER BY and LIMIT retain their input-consumption boundaries.
Required input columns are materialized at those boundaries, including keys,
arguments and payload demanded downstream. A following filter does not
retroactively remove that demand. Join matching can therefore reject an input
whose required payload was already evaluated. Ordering retains hidden-key
demand. LIMIT retains its skipped-row, prefetch and blocking-producer rules;
LIMIT 0 with zero offset requests no input. Do not push a predicate or defer a
required input value across these boundaries unless the rewrite preserves the
documented values and failures.

Aggregate output finalization remains demand-driven through the following
SELECT/WHERE/AS sequence. In particular, replacing an INT64 output reference `s`
with `s + 0` cannot force final SUM overflow in a group rejected by an earlier
predicate that does not need `s`. Aggregate input arguments remain demanded
under the existing aggregate rule, even when a later predicate rejects the
group.

Binding checks every projection expression's syntax, names, types and literal
ranges, including unused expressions. Arithmetic inside a computed SELECT is
runtime work: an empty input or an unused result does not demand it, even for
constant expressions. Existing preparation-time evaluation of WHERE and LIMIT
constants is unchanged. Scalar operand evaluation and NULL behavior follow the
existing numeric rules; projection introduces no conditional short-circuiting
inside an arithmetic program.

### Failure reporting

Parse, bind, type, arithmetic, conversion, resource, cancellation, I/O, and
corruption failures are distinct stable categories. Diagnostic wording may
improve without becoming part of compatibility; category, source span when one
exists, and transaction outcome are part of the contract.

A streaming query can discover a runtime error after earlier batches were read.
Those batches are a consumed prefix, not a successful partial query. A database
sink publishes nothing unless its entire input and publication succeed. An
external sink must state separately whether its destination can be made
complete-or-absent.

Arithmetic diagnostics use the complete numeric constant expression for
preparation failures and the enclosing aggregate call for runtime argument or
finalization failures. A computed SELECT operation uses its complete expression
span, excluding an output alias. A failure in an aggregate supplying that
expression retains the aggregate call's span. Call spans exclude output aliases.
If demanded calls share argument evaluation, an argument failure names a
demanded call with that program. A final SUM overflow names SUM even when its
state is shared with AVG. Dropping an undemanded expression cannot introduce a
failure merely to attach provenance. Spans use half-open UTF-8 byte offsets in
the submitted query; they do not imply that the engine retains its text.

Where SQL does not define which of several possible row errors appears first,
PipeSQL promises the stable error category and a valid source span, not a row
chosen by incidental parallel order. Tests must not freeze unspecified discovery
order.

## Target relational model

This section describes the release target. Operators absent from the public
manifests above remain unsupported. Target rules constrain future
implementation; they do not authorize syntax before its complete admission
checks.

A query has one outer shape:

```text
FROM source [ |> table_operator ]* [ |> terminal_operator ]
```

Parenthesized relational subqueries use that same shape. Scalar expressions,
function arguments, and operator clauses are admitted separately by the profile
manifest; this sketch is not a replacement grammar.

A query transforms a relation from top to bottom:

```sql
FROM lineitem
|> WHERE
     l_shipdate >= DATE '1994-01-01'
     AND l_shipdate < DATE '1995-01-01'
     AND l_discount BETWEEN 0.07 AND 0.09
     AND l_quantity < 25
|> AGGREGATE
     SUM(l_extendedprice * l_discount) AS revenue;
```

Each prefix ending before a `|>` is itself a query. Written order defines a
sequence of relational transformations, not a physical execution schedule. The
optimizer may change the schedule only while preserving the semantic contract.

A query may end after `FROM` or after any non-terminal relational operator. Its
result is the complete relation at that point; a final `SELECT` is optional.

### Target row shape and name visibility

Named relations and value tables require explicit relation-shape facts. A column
may remain visible through a qualified range without appearing in ordinary
output. For `JOIN ... USING`, a right-side key must remain available through its
range even when unqualified output uses the left key. The pinned `EXTEND kv.key
AS key1` behavior preserves the original output and range membership while
adding a name for the same identity. These forms remain unsupported until their
complete contracts and checks are implemented.

## Unimplemented expression forms

The following forms are design requirements for future work. They are rejected
by the current public parser or binder.

### Target analytic expressions

The studied analytic boundary uses `COUNT`, `SUM`, `AVG` and `MIN` aggregate
signatures followed by `OVER` inside pipe `SELECT` and `EXTEND` expressions.
These analytic forms are not implemented:

```sql
FROM KeyValue
|> EXTEND
     COUNT(*) OVER () AS total_rows,
     COUNT(*) OVER (
       PARTITION BY value
       ORDER BY key DESC) AS running_rows
```

An empty `OVER ()` owns an explicit full-partition `ROWS` frame from unbounded
preceding through unbounded following. With window `ORDER BY`, the admitted
default is `RANGE` from unbounded preceding through the current row. Partition
and order expressions resolve against the relation immediately to the left of
the pipe stage; aliases introduced by the same stage are not visible. Window
order items independently record direction and effective NULL placement. They
are execution requirements, not semantic order promised by the resulting
relation.

Analytic evaluation preserves input cardinality. Each derived output receives a
new binder-owned identity, while ordinary `EXTEND` input outputs and ranges
remain visible. Any analytic expression in `SELECT` or `EXTEND` clears incoming
semantic relation order, even when its window specification has its own order; a
later nonanalytic projection preserves that cleared state. `COUNT(*) OVER` is
nonnullable `INT64`. Other admitted signatures retain their currently
conservative result NULLability.

The binder distinguishes scalar, reducing aggregate, and row-preserving analytic
expressions. An ordinary aggregate call remains invalid in `SELECT`/`EXTEND`;
`OVER` converts exactly one aggregate call to an analytic value. Aggregate and
analytic phases cannot be nested or mixed in one expression. Analytic calls in
`WHERE`, `AGGREGATE`, `ORDER BY`, and `LIMIT` are rejected.

Named windows, explicit frame clauses or bounds, ranking/navigation functions,
analytic `DISTINCT`, the deprecated pipe `WINDOW` operator, structured window
keys, and analytics execution remain unsupported. The historical syntax
experiment does not establish an executable analytic operator or its resource
contract.

### Target EXISTS pipe subqueries

The target relation-subquery predicate has this shape in a `WHERE` expression:

```sql
WHERE EXISTS(FROM source [ |> relational_operator ]*)
```

The child query uses the same pipe-only relational grammar. Binding resolves a
name against the child's ordinary output and ranges first; only a missing local
name may resolve through an explicit outer scope. Local ambiguity is terminal. A
correlated leaf retains the outer semantic column identity and its scope
provenance rather than copying or renumbering the column.

`EXISTS` has nonnullable `BOOL` type regardless of child output shape or column
count. It is true exactly when the child relation has at least one row and false
for an empty child. The expression owns a stable validated child-plan handle;
failed or incomplete children do not publish. Correlated evaluation is logically
per outer row (or equivalent distinct parameter set), while an uncorrelated
child may be evaluated once. Unary `NOT` follows comparison and precedes `AND`
and `OR`; it accepts only `BOOL` and maps TRUE/FALSE/NULL to FALSE/TRUE/NULL.
Consequently, `NOT EXISTS` remains nonnullable and reuses the same validated
child plan rather than defining a second subquery form.

### Target scalar pipe subqueries

The target scalar form is a parenthesized pipe query in a `WHERE` expression:

```sql
WHERE value < (FROM source [ |> relational_operator ]*)
```

The child must publish exactly one ordinary output identity; its display name is
irrelevant and may be absent. Zero or multiple output columns are bind errors.
The scalar type is that output's type. The result is nullable when the output is
nullable or the child may be empty. Exactly one child row yields its value, zero
rows yield NULL, and observing a second row is a runtime cardinality error
rather than permission to choose an arbitrary row. Correlation follows the same
local-first scope rule as EXISTS, but the scalar node retains both the child
query identity and selected output identity.

### Target IN predicates

The target local `IN` form is a nonempty expression list:

```sql
search_value IN (value [, ...])
```

Every list value must be equality-comparable with the search value under the
same binder contract as `=`. The result is TRUE if any comparison is TRUE, NULL
if none is TRUE and any comparison is NULL, and FALSE otherwise. Binding marks
the result nullable when the search value or any candidate is nullable. `IN` has
comparison precedence above `NOT`, `AND`, and `OR`.

The search value and candidates occupy one bounded search-first arena edge
range. Empty and malformed lists are rejected.

The target relation form uses a pipe-only child query:

```sql
search_value IN (FROM source [ |> relational_operator ]*)
```

The child must have exactly one ordinary output identity, and that output must
be equality-comparable with the search value. Zero child rows yield FALSE. One
or many rows use the same TRUE/NULL/FALSE search rule as a local list; duplicate
rows do not change membership and never trigger scalar-subquery cardinality
failure. The node retains the validated child and output identities. Nested
children bind through the same local-first flat query graph.

`NOT IN`, `IN UNNEST`, and structured keys remain rejected until their distinct
contracts are admitted.

The first release admits only pure functions and explicit database or external
sinks. It does not admit user functions with hidden I/O or mutation. Volatile
functions remain unsupported until their evaluation-count and optimizer
contracts are defined.

## Operator admission

An operator is ready only when one concise contract defines:

1. accepted syntax and pinned upstream cases;
2. input and output relation state;
3. scope, aliases, stable column identities, and order transfer;
4. NULL, equality, empty-input, duplicate-name, and boundary behavior;
5. expression demand, errors, and semantic effects;
6. memory ownership, external-memory behavior, cancellation, and backpressure;
   and
7. an independent result oracle plus invalid cases.

The contract should describe the operator, not every planned implementation. If
the facts cannot be stated clearly, the interface is not ready.

## Conformance claims

Use [verification.md](verification.md#claim-vocabulary) for implemented,
release-ready and stable claims. Language conformance adds two terms:

- **recognized**: the parser accepts the documented shape; binding or execution
  may still reject it. Recognition does not authorize advertising a capability.
- **conformant**: the implemented behavior also agrees with the pinned GoogleSQL
  evidence for every accepted case, with every divergence listed.

No release says “GoogleSQL compatible” without naming its profile revision and
publishing the accepted/rejected manifest. Passing positive examples alone is
insufficient: every grammar alternative and semantic classifier needs negative
and boundary witnesses.

The production engine will not link the GoogleSQL C++ analyzer. It is a test and
research oracle. PipeSQL owns its small parser and binder so the shipped
language surface, failure modes, dependency graph, and resource behavior remain
under project control.
