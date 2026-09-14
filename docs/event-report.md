# Compare a report across appends

Build a yearly report over sixteen events and four dimension rows. Keep the first
report readable while appending more events, then close and reopen the database
and verify the new report. Complete the [build prerequisites](testing.md#prerequisites)
and run these commands from the repository root on macOS or Linux:

```sh
cargo build --release --offline --locked --example event_report
pipesql_report_dir=$(mktemp -d)
target/release/examples/event_report "$pipesql_report_dir/events"
```

Require successful exit. The output begins with:

```text
old groups=7 joined_rows=10; pinned report unchanged
new groups=13 joined_rows=20; reopened report unchanged
```

It prints thirteen groups and ends with:

```text
measurements=16; original and doubled bits verified
status=finished
```

The [program](../examples/event_report.rs) checks every group's schema and values,
all numeric projection rows, terminal `Finished`, and released query reservations
before printing success. `Some(value)` means a present value; `None` means SQL NULL.
A successful process exit is required even if you redirect the output.

## Understand the input and answer

Read the sixteen literal [events](../examples/support/event_data.rs). Each event
has an INT64 identity, a nullable dimension key, a nullable DATE, a nullable INT64
amount, and a nullable DOUBLE measurement. DATE input uses validated days since
1970-01-01. The writer transposes these records into borrowed typed columns and
validity bitmaps. It reuses bounded buffers after each `Append::write` returns;
the caller owns the append transaction and its limits.

Dimension key 1 has label `north`. Key 2 has two rows, labeled `south` and `南`.
Key 3 has an empty label. Key 99 is absent. The first append contains events 0–7;
the second contains events 8–15. Run this [query](../examples/event_report.sql):

```sql
FROM events AS f
|> LEFT JOIN dimensions AS d ON f.dimension_id = d.id
|> EXTEND EXTRACT(YEAR FROM f.happened) AS calendar_year
|> AGGREGATE COUNT(*) AS entries, COUNT(f.amount) AS present, SUM(f.amount) AS total
   GROUP BY calendar_year, d.label
|> ORDER BY calendar_year NULLS FIRST, label NULLS FIRST;
```

Each event with key 2 contributes to two joined rows. A missing or NULL key
contributes one row with a NULL label. That label is distinct from the empty
STRING attached to key 3. NULL dates form groups with a NULL year. `entries`
counts joined rows, `present` counts non-NULL amounts, and `total` is NULL when
all amounts in that group are NULL.

For example, the first report's year-2000/NULL-label group combines events 2 and
3: two entries, one present amount, and total 30. Event 10 adds amount 3, so the
new group has three entries, two present amounts, and total 33. Each year-2000
`south` and `南` group totals 15 from events 1 and 6. These duplicate dimension
matches explain why sixteen events produce twenty joined rows.

The numeric projection orders events by identity and returns the original DATE, `measurement`, and
`measurement * 2`. It checks literal DATE offsets and IEEE-754 bit patterns, including negative
zero and NULL propagation. Integer report totals have a separate literal oracle;
floating-point summation does not define their expected answers.

## Follow the owners

Start with `run` in the [program](../examples/event_report.rs). It prepares `old`
before the second append. That plan retains its original snapshot, so executing
it again produces the same seven groups. Preparing `new` selects the newer
snapshot and produces thirteen groups. Results borrow their plans; dropping a
result releases execution state while its plan retains the snapshot pin. The
program drops both plans before closing, then prepares again after reopening.
The [snapshot lesson](getting-started.md#keep-an-old-snapshot-readable) explains
pins, reclamation, and the distinction between snapshots and commit receipts.

Follow names through the [binder](../src/frontend/binding.rs): `f` and `d` select
relation scopes, and each bound column has a semantic identity independent of
its displayed name. The join preserves fact columns and makes right-side output
nullable. EXTEND assigns `calendar_year` a computed identity. Aggregation consumes
that identity and `d.label` as keys and defines the count/total outputs. Ordering
consumes those outputs. The [preparation walkthrough](frontend.md#trace-a-query-through-preparation)
explains these boundaries and independent validation.

The [execution guide](execution.md) follows producers and their resource owners.
For this query, typed scans supply join inputs; the join supplies rows to year
extraction and grouping; sorting establishes the final order. `date_year` in
[computed execution](../src/execution/computed.rs) reads a checked DATE and
returns a nullable INT64. The [calendar lesson](calendar-year.md) traces that
conversion. The [resource contract](resources.md) owns admission and release
rules; this small example does not establish spill or whole-process memory bounds.

## Scale the report and observe spill

The [scaled caller](../examples/scaled_report.rs) extends the same schema and
query to 131,072 events. Build it and compare two query budgets:

```sh
cargo build --release --offline --locked --example scaled_report
target/release/examples/scaled_report "$pipesql_report_dir/even-high" 32000000 even
target/release/examples/scaled_report "$pipesql_report_dir/even-low" 8000000 even
```

Each successful run checks all twenty groups against an independent row model,
cancels another execution after observing temporary storage, and reruns the same
plan with a fresh cancellation token. It also checks the complete ordered DOUBLE
projection and released reservations. The final line is `status=finished`.
`sampled_temp_bytes` reports the largest logical temporary reservation observed
after a query step. Both budgets can spill; the high budget is not a promise of
in-memory execution.

The caller creates the database with a separate ingestion budget, then reopens
it with the requested query budget. Input construction writes 512 batches of
256 events with reused caller buffers. It uses different intervals for NULL
keys, dates, amounts and measurements. The `skewed` profile sends most non-NULL
keys to the duplicated dimension key 2. The `empty` profile declares both tables
and populates only the dimensions; the report must return no groups. The `small`
profile reuses the sixteen literal events.

The reference model enumerates the dimension rows for each fact. It maps the
four fixture DATE offsets to literal years and accumulates nullable integer
counts and sums in an ordered map. It neither parses the SQL nor calls engine
evaluation. DOUBLE checks remain separate from the integer totals. The original
small example retains its fully literal group and numeric-bit expectations.

Run the bounded public scenarios with:

```sh
cargo test --release --offline --locked --example scaled_report -- --test-threads=1
```

The tests compare all four profiles at both budgets. They also require typed
memory and temporary-space refusals, reject an altered expected group, abandon
a result after its first batch, and verify healthy reuse and release afterward.
The separate [grouping replay test](../src/execution/aggregation/grouping/tests/replay.rs)
starts the same report with room for one hash group. It requires disk fallback,
observed replay of the retained join, complete literal rows and final release.
This distinguishes the mechanism from the public caller's observation of spill.

## Challenge the answer and clean up

Run the example's public-API checks:

```sh
cargo test --release --offline --locked --example event_report
```

One test changes the expected total 30 to 31. The verifier must reject group 3,
release the partial result, and successfully rerun the same plan against the
correct answer. The other test checks the complete append/reopen workflow.
Expected rows are literal and separate from input construction; neither test
computes its reference answer using the engine's evaluator.

After the example exits, remove its database:

```sh
rm -r -- "$pipesql_report_dir"
unset pipesql_report_dir
```
