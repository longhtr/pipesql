# Handle partial query results

Run this example to see rows followed by an arithmetic failure, cancel another
execution after receiving rows, then reuse its prepared query successfully.
The program checks every value and terminal outcome. It also checks that each
execution releases its logical memory and temporary reservations.

Complete [the first declared-table tutorial](getting-started.md) first. Use the
same [build prerequisites](testing.md#prerequisites) and run commands from the
repository root on macOS or Linux. The complete program is
[query_results.rs](../examples/query_results.rs).

## Create the example database

Create a temporary directory and run the example with a new database path:

```sh
pipesql_result_dir=$(mktemp -d)
cargo run --release --offline --locked --example query_results -- "$pipesql_result_dir/facts"
```

The table contains nonnullable INT64 amounts 0 through 255, followed by
9,223,372,036,854,775,807 (`i64::MAX`). Every query explicitly orders by amount.
With the current producers, a successful example run prints:

```text
overflow: prefix=256 values=1..256; addition bytes=40..50; owners released
cancelled: prefix=1; execution released; plan retained
finished: rows=257 last=9223372036854775807; owners released
```

Require all three lines and successful process exit. The example deliberately
handles two failed queries; its successful exit means those expected outcomes
were verified. The cancelled query's first batch currently contains one row.
The program accepts any nonempty partial batch there: a batch can contain fewer
than the API's maximum 256 rows.

## Follow the failure after rows

The first query computes `amount + 1` after ordering. Its first 256 values are
1 through 256. The final input cannot be increased within INT64, so a later step
returns `Failed` with an addition overflow. The earlier rows are an incomplete
result, even though each value was valid.

In `overflow_after_rows`, inspect each borrowed batch inside its match arm.
Once that borrow ends, the loop can call `step` again. On failure, another step
must still report failure. `into_error` consumes the result and returns an owned
error. The main function releases the prepared query before checking that the
error still identifies addition at source bytes 40 through 49. The original SQL
string was already dropped after preparation.

The source span identifies `amount + 1`; it does not retain the SQL text. Keep
your own source if you want to display the expression alongside a diagnostic.
The [stepping contract](interfaces.md#stepping-a-query) owns the precise public
outcomes and borrowing rules.

## Cancel one execution and reuse the plan

The second query selects the ordered amounts without adding one. In
`cancel_after_rows`, validate the first borrowed batch, request cancellation,
and keep stepping until the operation reports its terminal outcome. The example
requires `Cancelled`, moves that error out, and checks reservation release while
the prepared query stays alive.

A cancellation request is one-way; it does not reset the plan or undo database
writes. `finish_ordered` creates a fresh token and executes the same immutable
prepared query again. It checks all 257 values, observes `Finished`, and verifies
that another step remains finished. Only then does the main function release
the plan and close the database.

The three runs separate these facts:

| Observation | What it establishes |
| --- | --- |
| A `Rows` batch | Those rows are available now; later work can still fail. |
| `Failed` or `Finished` | The execution is terminal and its runtime buffers have been released. |
| Consuming or dropping the result | The remaining result handle is released; the prepared plan has its own lifetime. |

The [result ownership trace](execution.md#trace-a-terminal-query-result) follows
these transitions through the implementation. Reservation equality in this
example concerns logical engine ownership, not whole-process or physical-memory
bounds. The run does not qualify arbitrary concurrent cancellation schedules.

## Clean up

The program leaves its database for inspection. Remove only the temporary
directory created above when you finish:

```sh
rm -r -- "$pipesql_result_dir"
```
