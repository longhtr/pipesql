# Explore queries on a declared table

Use the same four-row sales table to explore column transformations, numeric
expressions, set operations and partition count. Each section gives a query,
its expected result and a path through the implementation. Choose a section
after completing the setup; the queries do not change the stored rows.

For the first create, append, reopen and query flow, start with
[Create and query a declared table](getting-started.md). This guide uses the same
[example program](../examples/declared.rs) to create a fresh database. Use the
[build prerequisites](testing.md#prerequisites) and run commands from the
repository root on macOS or Linux. The [platform matrix](testing.md#platform-status)
owns the qualification limits.

## Set up the sales table

Build the CLI, then create a temporary directory and run the example. Keep this
shell open so the directory variable remains available for the later queries:

```sh
cargo build --release --offline --locked
pipesql_example_dir=$(mktemp -d)
cargo run --release --offline --locked --example declared -- "$pipesql_example_dir/sales"
```

Require successful exit and these two lines before continuing:

```text
north total=15 rows=3 present=2
south total=20 rows=1 present=1
```

The database contains amounts 10, 20, 5 and NULL. Repeating setup requires a new
path. Each query below reopens this database and must complete with successful
exit and `status=queried`; printed rows alone can be an incomplete result.

## Follow one query from names to results

Run [query-flow.sql](../examples/query-flow.sql), the query used in the
[preparation walkthrough](frontend.md#trace-a-query-through-preparation):

```sh
cargo run --release --offline --locked --bin pipesql -- query \
  --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/query-flow.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

It renames `amount` to `subtotal`, adds one to each present value, then sums the
results. The arithmetic is `11 + 21 + 6 = 38`; the fourth value remains NULL.
Require one `row=int64:38`, `row_count=1`, `status=queried` and successful exit.
The [execution trace](execution.md#trace-a-query-through-execution) follows this
same query from producer admission to the final borrowed batch and cleanup.

## Transform columns while retaining the original values

Before removing the database, run [examples/extend.sql](../examples/extend.sql)
through the CLI:

```sh
cargo run --release --offline --locked --bin pipesql -- query \
  --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/extend.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

EXTEND keeps `region` and `amount` and appends `doubled`. SET replaces the ordinary
`amount` with that value. RENAME changes its name to `subtotal`, and DROP removes
the temporary `doubled` output. The final EXTEND computes `adjusted` from the
subtotal. Each expression list sees its complete input before publishing changes.

The range `s` still names the original columns, so `s.amount` returns the amount
before SET. SET assigns a new identity; RENAME preserves that identity. The NULL
amount produces NULL computed values, and the filter removes that row before
sorting.

The CLI reports schema, rows, and completion. Its decoded result is:

| region | amount | subtotal | adjusted |
| --- | ---: | ---: | ---: |
| north | 5 | 10 | 11 |
| north | 10 | 20 | 21 |
| south | 20 | 40 | 41 |

Require `status=queried` and a successful process exit before accepting the
result. To follow the implementation, read `bind_extend`, `bind_set`,
`bind_rename`, and `bind_drop` in the
[binder](../src/frontend/binding.rs), then read the [projection demand
rules](language.md#computed-projection-demand). These transformations share their
producer's execution controller; only demanded numeric definitions are evaluated.

## Add constant labels and dates

Run [examples/constants.sql](../examples/constants.sql) against the sales database:

```sh
cargo run --release --offline --locked --bin pipesql -- query \
  --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/constants.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

Each non-NULL amount receives label `reported` and date `2000-02-29`.
The rows are north with amount 5, north with 10, then south with 20. Require
successful exit and `status=queried` before accepting the result.

Follow `projection_expression` in the [parser](../src/frontend/parser.rs) and
`bind_computation` in the [binder](../src/frontend/binding.rs): binding decodes
text and folds the calendar operation into an owned constant. The
[computed-value reader](../src/execution/computed.rs) returns that value without
numeric scratch, and the output batch copies the text into admitted storage.
The prepared query does not retain its caller's SQL string.

## Measure text in bytes

Run [byte_length.sql](../examples/byte_length.sql) after the sales-table setup:

```sh
target/release/pipesql query --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/byte_length.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

The query projects each region's UTF-8 byte length and the byte length of the
literal `雪`, then aggregates those numeric identities. Both `north` and `south`
occupy five bytes. The literal occupies three bytes, so its total across four
rows is twelve. Require one row with INT64 values `5, 5, 12`, `row_count=1`,
`status=queried` and successful exit.

Byte length measures encoded size, not displayed characters. BYTE_LENGTH is a
complete projection expression in this profile; compute it first, then use its
identity in a later arithmetic or aggregate stage. Follow
[STRING byte-length evaluation](execution.md#string-byte-length-evaluation) to
see where borrowed text becomes an owned numeric value. The
[language manifest](language.md#current-public-query-manifest) owns accepted forms
and NULL behavior.

## Filter by membership

Run [examples/membership.sql](../examples/membership.sql) against the same sales
database:

```sh
cargo run --release --offline --locked --bin pipesql -- query \
  --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/membership.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

The result is north with amount 5, then south with amount 20. The list's NULL
candidate does not match the NULL amount. A match yields TRUE; a nonmatch with
NULL yields UNKNOWN, which WHERE excludes. Negating this membership test returns
no rows because NOT preserves UNKNOWN. Require successful exit and
`status=queried` before accepting the result.

To exclude a list and a range, run
[examples/negated-membership.sql](../examples/negated-membership.sql):

```sh
cargo run --release --offline --locked --bin pipesql -- query \
  --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/negated-membership.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

The result is south with amount 20. `NOT IN (5)` excludes 5, and
`NOT BETWEEN 0 AND 10` excludes the inclusive range. The NULL amount remains
UNKNOWN and is excluded. Require successful exit and `status=queried`.

Follow `boolean_leaf` in the [Boolean parser](../src/frontend/parser/boolean.rs)
to see each candidate become an equality decision joined by OR. The
[row predicate](../src/execution/predicate.rs) preserves UNKNOWN under negation;
the existing forward decisions retain branch demand without a separate membership
execution engine. The [membership contract](language.md#literal-list-membership)
owns type and size limits.

## Classify measurements by sign

Run [examples/sign.sql](../examples/sign.sql) against the same sales database:

```sh
cargo run --release --offline --locked --bin pipesql -- query \
  --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/sign.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

SIGN classifies each amount relative to 10: -1 below, zero equal, and +1 above.
NULL remains a separate group. The result is:

| direction | total | n |
| ---: | ---: | ---: |
| NULL | NULL | 1 |
| -1 | 5 | 1 |
| 0 | 10 | 1 |
| 1 | 20 | 1 |

Require `status=queried` and successful exit. Follow `Op::Sign` in the
[scalar program](../src/scalar.rs) and [demand cursor](../src/scalar/evaluation.rs)
to see the same numeric rule used by batch and row evaluation. The
[language contract](language.md#current-public-query-manifest) owns type,
exceptional-value and argument-demand rules.

## Group measurements into buckets

Run [examples/rounding.sql](../examples/rounding.sql) against the same sales database:

```sh
cargo run --release --offline --locked --bin pipesql -- query \
  --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/rounding.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

Dividing by 15 and applying FLOOR puts amounts in intervals of width 15.
Bucket zero contains amounts from zero up to, but excluding, 15. NULL remains
separate. Require successful exit and `status=queried`, with these rows:

| bucket | total | n |
| ---: | ---: | ---: |
| NULL | NULL | 1 |
| 0 | 15 | 2 |
| 1 | 20 | 1 |

FLOOR rounds downward; CEIL (also spelled CEILING) rounds upward. Both return
DOUBLE even for integer arguments. Follow `Op::Floor` and `Op::Ceil` in the
[scalar program](../src/scalar.rs): validation changes the result type, and batch
evaluation reuses the argument's scratch slot. The
[demand cursor](../src/scalar/evaluation.rs) applies the same rule when a row
requests the value. The [language contract](language.md#current-public-query-manifest)
explains conversion precision and exceptional values.

To assign each amount to the nearest multiple of 15, run
[examples/nearest_rounding.sql](../examples/nearest_rounding.sql) on the same database:

```sh
cargo run --release --offline --locked --bin pipesql -- query \
  --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/nearest_rounding.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

ROUND assigns 5 to bucket zero, and 10 and 20 to bucket one. Require successful
exit and `status=queried`, with these complete rows:

| bucket | total | n |
| ---: | ---: | ---: |
| NULL | NULL | 1 |
| 0 | 5 | 1 |
| 1 | 30 | 2 |

Halfway values round away from zero: an amount of 7.5 would enter bucket one,
and -7.5 would enter bucket minus one. `Op::Round` uses the same scalar and demand
owners as FLOOR and CEIL, with a different rounding primitive. Decimal-position
and rounding-mode arguments are outside the accepted profile.

## Compute a root-mean-square amount

Run [examples/square_root.sql](../examples/square_root.sql) against the same database:

```sh
cargo run --release --offline --locked --bin pipesql -- query \
  --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/square_root.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

The three present amounts have squares 25, 100 and 400. AVG ignores the NULL
amount and returns 175. SQRT produces one DOUBLE row, approximately
13.228756555322953, with bits `402a751f9447b724`. Require successful exit and
`status=queried`. Squaring uses checked INT64 multiplication; sufficiently large
amounts would fail before AVG. This example's small inputs avoid that boundary.

`Op::Sqrt` uses the scalar batch and demand cursor already used by rounding.
A negative demanded argument raises a typed domain error; NULL propagates.
The [language contract](language.md#current-public-query-manifest) owns conversion,
exceptional-value and argument rules.

## Compare amounts on a logarithmic scale

Run [examples/logarithm.sql](../examples/logarithm.sql) against the same database:

```sh
cargo run --release --offline --locked --bin pipesql -- query \
  --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/logarithm.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

Require successful exit and `status=queried`, with one nullable DOUBLE row,
approximately 2.302585092994046. The three positive amounts are 5, 10 and 20;
their mean natural logarithm is ln(10). Equal multiplicative changes become
equal additive distances on this scale. The NULL amount does not contribute.
The predicate also excludes nonpositive amounts, where a finite logarithm would
raise a domain error. This is a log-scale summary; it is not the logarithm of
the arithmetic mean.

Trace `ParsedOp::Ln` through the [binder](../src/frontend/binding.rs) to `Op::Ln`
in the [scalar program](../src/scalar.rs). The scalar owner converts INT64 before
evaluation and checks the domain before calling the native logarithm. Batch
evaluation and the [demand cursor](../src/scalar/evaluation.rs) share that kernel;
NULL validity prevents an irrelevant payload from reaching it. AVG consumes the
result through the ordinary aggregate path. The
[language contract](language.md#current-public-query-manifest) owns exceptional
values and precision limits; this example's decimal output is approximate.

## Express a power ratio in decibels

Use the existing amounts 5, 10 and 20 as a small numeric fixture for power
measurements. [examples/decibel_scale.sql](../examples/decibel_scale.sql) compares
each value with a reference power of 10. For actual measurements, the power and
reference must use the same units. The decibel value is ten times the base-ten
logarithm of their ratio.

```sh
cargo run --release --offline --locked --bin pipesql -- query \
  --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/decibel_scale.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

Require successful exit and `status=queried`, with three rows ordered by amount:

| amount | relative_db (approximately) |
| --- | --- |
| 5 | -3.010299956639812 |
| 10 | 0 |
| 20 | 3.010299956639812 |

Doubling power adds about 3.01 dB; equal power gives zero. The filter excludes
NULL and nonpositive amounts before LOG10 is demanded. A zero reference would
fail in division before reaching LOG10.

Trace `ParsedOp::Log10` through the [binder](../src/frontend/binding.rs) to
`Op::Log10` in the [scalar owner](../src/scalar.rs). Division first forms a DOUBLE
ratio; LOG10 checks its domain and applies the native base-ten logarithm. Batch
evaluation and the [demand cursor](../src/scalar/evaluation.rs) reuse the unary
kernel and existing scratch. The [language contract](language.md#current-public-query-manifest)
owns exceptional values and precision limits. Independent scalar vectors and the
[public calculation](../tests/catalog_lifecycle/computed.rs) check the result.

## Compute a geometric mean

Run [examples/geometric_mean.sql](../examples/geometric_mean.sql) against the same
sales database to express the logarithmic average in the original units:

```sh
cargo run --release --offline --locked --bin pipesql -- query \
  --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/geometric_mean.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

Require successful exit and `status=queried`, with one nullable DOUBLE row
approximately equal to 10. The geometric mean of 5, 10 and 20 is the cube root
of their product, 1,000. Averaging logarithms and then applying EXP computes the
same mathematical quantity without forming that product. This is useful when
multiplicative changes matter. The filter selects positive amounts; NULL amounts
do not contribute. An empty positive input produces NULL.

The final pipe stage consumes AVG's nullable DOUBLE result and evaluates EXP
through the same [scalar program](../src/scalar.rs) and
[demand cursor](../src/scalar/evaluation.rs) as other numeric expressions. The
kernel handles zeros and infinities explicitly, preserves input NaN bits, and
reports overflow created by finite inputs. It retains representable subnormals
and permits underflow to zero. The [language contract](language.md#current-public-query-manifest)
owns these rules and the approximate precision limits; the composed answer can
vary slightly from 10.

## Compound a rate over several periods

For this calculation, interpret the same positive amounts as percentage rates
per period. Run [examples/compound_growth.sql](../examples/compound_growth.sql)
to compound an initial quantity of 1,000 for three periods:

```sh
cargo run --release --offline --locked --bin pipesql -- query \
  --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/compound_growth.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

Require successful exit and `status=queried`, with nullable INT64/DOUBLE columns
and these three rows in order. DOUBLE values can differ slightly from the
mathematical amounts shown:

| rate_percent | compounded |
| --- | --- |
| 5 | 1157.625 |
| 10 | 1331 |
| 20 | 1728 |

Dividing the percentage by 100 gives a fractional rate; adding one gives the
growth factor for one period. POWER raises that factor to the third power before
multiplying by the starting quantity. POW is an equivalent spelling. This
calculation uses approximate DOUBLE arithmetic and does not define exact decimal
rounding for money.

Follow the binary call frame in the [parser](../src/frontend/parser.rs), then
the typed operation in the [binder](../src/frontend/binding.rs). The
[scalar owner](../src/scalar.rs) promotes both operands, handles exceptional
values and evaluates the finite power. Its batch evaluator and
[demand cursor](../src/scalar/evaluation.rs) share that kernel. The
[language contract](language.md#current-public-query-manifest) owns NULL behavior,
domain/overflow errors and precision limits; the
[public calculation](../tests/catalog_lifecycle/computed.rs) checks complete
compounded results against independent mathematical answers.

## Combine pipeline results

Run [examples/union.sql](../examples/union.sql) against the same database:

```sh
cargo run --release --offline --locked --bin pipesql -- query \
  --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/union.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

The first branch selects north's sales. The second selects every sale of at least
10. UNION DISTINCT keeps one copy of the north sale worth 10. It matches columns by
position: the second branch's `area` and `value` feed the first branch's `region`
and `amount`. Grouping therefore produces:

| region | total | n |
| --- | ---: | ---: |
| north | 15 | 3 |
| south | 20 | 1 |

Change `UNION DISTINCT` to `UNION ALL` in the example to retain both copies of
the north sale worth 10: north then has total 25 and count 4. DISTINCT compares
the complete `(region, amount)` row before grouping, so the north sale worth 5
and its NULL amount remain separate rows.

The NULL amount contributes a row to COUNT(*) but no value to SUM. GROUP AND ORDER
BY establishes the displayed order; union itself establishes none. Require
`status=queried` and successful process exit before accepting the output.

Follow `bind_union` in the [binder](../src/frontend/binding.rs) to see how each
output position gets a fresh identity and two input mappings. The
[demand pass](../src/execution/planning/demand.rs) translates required outputs
into branch inputs. The [union consumer](../src/execution/union.rs) copies one
batch at a time while the [scheduler](../src/execution/runtime.rs) retains child
ownership. The [parser](../src/frontend/parser.rs) adds one ordinary DISTINCT
stage after the complete argument list for UNION DISTINCT. Its complete-row
comparison demands every input field and uses the existing bounded
[distinct owner](execution.md#duplicate-removal), including temporary storage
when necessary. See the [union contract](language.md#union-distinct) for scope
and errors.

## Compute a ratio after grouping

Run [ratio.sql](../examples/ratio.sql) against the same sales table:

```sh
target/release/pipesql query --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/ratio.sql" \
  --memory-limit-bytes 4000000 --temp-limit-bytes 2000000
```

The grouped stage produces a sum and a count of non-NULL amounts. The next
projection divides them, producing DOUBLE values: north/7.5 and south/20.0.
Require `status=queried` and successful process exit. `COUNT(amount)` excludes
the missing north amount; using `COUNT(*)` would include it in the denominator.

Follow [binding](../src/frontend/binding.rs) into the bounded postfix program in
[scalar.rs](../src/scalar.rs). Division converts its operands to DOUBLE at that
operation. Earlier integer expressions still use checked arithmetic. Each lane's
validity is checked before dividing, so a NULL operand produces NULL and a
non-NULL zero denominator reports a source-spanned error.

## Group amounts by integer quotient

Run [quotient.sql](../examples/quotient.sql) against the same sales table:

```sh
target/release/pipesql query --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/quotient.sql" \
  --memory-limit-bytes 4000000 --temp-limit-bytes 2000000
```

The decoded rows are NULL/1/NULL, 0/2/15 and 1/1/20 for bucket, n and total.
Require three rows, `status=queried` and successful process exit. Amounts 5 and
10 share quotient zero; amount 20 has quotient one. The NULL amount retains its
own group. DIV computes an exact INT64 quotient in the [numeric evaluator](../src/scalar.rs)
before the ordinary [grouping owner](../src/execution/aggregation/grouping.rs)
consumes the column. It truncates toward zero: negative values near zero share
bucket zero, so signed buckets are not mathematical floor intervals.

## Group amounts by remainder

Run [remainder.sql](../examples/remainder.sql) against the same sales table:

```sh
target/release/pipesql query --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/remainder.sql" \
  --memory-limit-bytes 4000000 --temp-limit-bytes 2000000
```

The decoded rows are NULL/1/NULL, 0/2/30 and 5/1/5 for remainder, n and total.
Require three rows, `status=queried` and successful process exit. The amounts
10 and 20 share remainder zero; amount 5 has remainder five. A missing amount
retains its own NULL group. `COUNT(*)` counts that row while SUM remains NULL.

The [numeric evaluator](../src/scalar.rs) computes the remainder in the existing
INT64 lane. The [grouping owner](../src/execution/aggregation/grouping.rs) then
consumes that ordinary computed column. MOD keeps a negative dividend's sign;
it does not turn negative inputs into positive bucket numbers.

## Measure deviations from a reference amount

Run [deviation.sql](../examples/deviation.sql) against the same sales table:

```sh
target/release/pipesql query --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/deviation.sql" \
  --memory-limit-bytes 4000000 --temp-limit-bytes 2000000
```

The decoded rows are north/NULL/NULL, north/5/5, north/10/0 and south/20/10.
Require four rows, `status=queried` and successful process exit. ABS preserves
the INT64 type of `amount-10`; a missing amount retains a missing deviation.
The [numeric evaluator](../src/scalar.rs) evaluates subtraction first, then
changes the same lane in place. An overflowing subtraction still fails before
ABS, and the absolute value of minimum INT64 also fails because it cannot fit.

## Keep a ratio when its denominator is missing

Run [safe-ratio.sql](../examples/safe-ratio.sql) against the same sales table:

```sh
target/release/pipesql query --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/safe-ratio.sql" \
  --memory-limit-bytes 4000000 --temp-limit-bytes 2000000
```

This query divides ten by each recorded amount and retains the missing amount.
The decoded rows are north/NULL/NULL, north/5/2.0, north/10/1.0 and south/20/0.5.
Require four rows, `status=queried` and successful process exit. A zero denominator
would also produce NULL. SAFE_DIVIDE converts errors from division itself to NULL;
an overflowing argument such as `SAFE_DIVIDE(9223372036854775807+1, 0)` still fails.

The [numeric evaluator](../src/scalar.rs) records NULL in the lane's existing
validity bitmap. Later operators retain that distinction: `COUNT(ratio)` excludes
a missing ratio, while `COUNT(*)` still counts its row. No exception-catching
wrapper or separate execution path is involved.

## Count the complete input beside each row

Run [window-count.sql](../examples/window-count.sql) against the same sales table:

```sh
target/release/pipesql query --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/window-count.sql" \
  --memory-limit-bytes 4000000 --temp-limit-bytes 2000000
```

The query excludes the NULL amount, counts the three remaining rows and then
orders them by amount. The decoded result rows are:

| region | amount | total_rows |
| --- | --- | --- |
| north | 5 | 3 |
| north | 10 | 3 |
| south | 20 | 3 |

Require `status=queried` and successful process exit. Moving LIMIT before the
analytic stage counts only that prefix; moving it after the stage limits output
while keeping the complete count. Analytic evaluation clears semantic order, so
the example places ORDER BY afterward.

Trace `Plan::computation_producer` in [the semantic plan](../src/frontend.rs) to
see why ordinary expressions in this stage evaluate at its output. The
[physical planner](../src/execution/planning/lower.rs) retains their demanded
input values. The [sorted-input consumer](../src/execution/blocking/order.rs)
counts captured rows, then emits each row with that count using checked scratch
storage. When no input values are demanded, the [counter](../src/execution/count.rs)
retains only the row count and emits that many rows without scratch storage.
Both preserve cardinality instead of reducing the relation to one row.

The retained-row consumer separates the controller's inline storage from its
sort buffers. Follow `Order::with_layout` to see both admitted before execution:
the runtime retains the inline charge until it frees the controller vector,
while the sorter releases each buffer's charge after freeing that buffer. This
also protects cleanup when source opening fails. The
[resource contract](resources.md#runtime-and-result-admission) explains the shared
ownership rule for ordering, joins and sorted set operations.

## Finish and clean up

When finished, remove only the temporary example directory created by this guide:

```sh
rm -r -- "$pipesql_example_dir"
```

For the rules behind these examples, see the [language contract](language.md).
For errors and query lifetimes, see [public interfaces](interfaces.md#query-lifecycle).
