# Group dates by calendar year

Create a five-row events table and group its amounts by Gregorian year. This
lesson shows how a DATE becomes a numeric grouping key without changing the
stored date. Complete the [build prerequisites](testing.md#prerequisites), then
run these commands from the repository root on macOS or Linux:

```sh
cargo build --release --offline --locked --example calendar_year
pipesql_calendar_dir=$(mktemp -d)
target/release/examples/calendar_year "$pipesql_calendar_dir/events"
```

Require successful exit and exactly this output:

```text
year=NULL total=40 entries=1
year=1999 total=10 entries=1
year=2000 total=25 entries=2
year=2001 total=30 entries=1
status=finished
```

The [example program](../examples/calendar_year.rs) appends December 31, 1999;
February 29 and December 31, 2000; January 1, 2001; and NULL. Their amounts are
10, 20, 5, 30 and 40. It closes and reopens the database before running
[calendar_year.sql](../examples/calendar_year.sql):

```sql
FROM events
|> EXTEND EXTRACT(YEAR FROM happened) AS calendar_year
|> AGGREGATE SUM(amount) AS total, COUNT(*) AS entries GROUP AND ORDER BY calendar_year;
```

EXTEND preserves `happened` and gives the extracted INT64 a new identity.
The aggregate consumes that identity as its grouping key. Both dates in 2000
join the same group, yielding 25. NULL remains NULL and forms a separate group;
its amount still contributes 40. The example checks the schema, every group,
Finished and released reservations before printing success.

## Follow the conversion through the implementation

1. Open [date.rs](../src/date.rs). `DateValue` validates signed day offsets in
   years 0001 through 9999. The existing `components` method decodes a Gregorian
   date in 400-year cycles. Its March-based calculation places February's leap
   day at the end of a cycle year. The final adjustment restores January and
   February to their calendar year. `year` returns that decoded component;
   dividing the epoch offset by 365 would be wrong.
2. Follow `projection_expression` in the [parser](../src/frontend/parser.rs) and
   `bind_computation` in the [binder](../src/frontend/binding.rs). The parser
   accepts the bounded YEAR form; the binder requires DATE input and records
   `Computation::DateYear`. A DATE literal or bounded constant DATE_ADD/DATE_SUB
   instead folds to an owned INT64 during preparation.
3. Read `date_year` and `BatchLayout::new` in
   [computed execution](../src/execution/computed.rs). Runtime extraction reads
   a checked DATE or owned DATE constant. Numeric scratch holds the resulting
   year, not the DATE input. Later arithmetic uses the ordinary dependency map.
   The [workspace contract](resources.md#computed-scan-workspace) explains its
   admission and the [execution guide](execution.md#scalar-expression-evaluation)
   explains when scalar demand can avoid work.
4. Read the literal expectations in the
   [public calendar tests](../tests/catalog_lifecycle/date_year.rs), then the
   [forced replay cases](../src/execution/aggregation/grouping/tests/replay.rs).
   They separate calendar answers from the mechanisms that transport those
   answers through sorting, grouping and replay.

The [language reference](language.md#current-public-query-manifest) owns the
accepted syntax and exclusions. YEAR means the Gregorian calendar year;
ISO week-numbering years and other extraction parts are outside this profile.

## Remove the example database

After the program exits, remove the directory created for this lesson:

```sh
rm -r -- "$pipesql_calendar_dir"
unset pipesql_calendar_dir
```
