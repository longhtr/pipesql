# Examples

These programs demonstrate the public Rust API and check the results they produce.
Start with [declared.rs](declared.rs) to create a typed table, append rows and query
it after reopen. The [root README](../README.md#try-it) gives a complete command.

## Structure

| Examples | What to explore |
| --- | --- |
| [logical_plan.rs](logical_plan.rs) | Inspect a prepared plan and follow column identities through a query. |
| [query_results.rs](query_results.rs), [snapshots.rs](snapshots.rs) | Handle late errors and cancellation; retain an older snapshot across writes. |
| [decode_csv.rs](decode_csv.rs), [import_csv.rs](import_csv.rs), [parquet.rs](parquet.rs) | Decode bounded input and import or export complete typed results. |
| [equality.rs](equality.rs), [left_join.rs](left_join.rs), [calendar_year.rs](calendar_year.rs) | Follow individual query operations with checked answers. |
| [event_report.rs](event_report.rs), [scaled_report.rs](scaled_report.rs) | Compose an analytical report, then exercise larger inputs, spill and cancellation. |
| [grouping.rs](grouping.rs), [string_grouping.rs](string_grouping.rs), [composed.rs](composed.rs) | Explore grouping and join costs with controlled inputs and memory budgets. |
| [projection_cost.rs](projection_cost.rs), [text_cost.rs](text_cost.rs) | Compare equivalent projections or byte and character counting on the same data. |
| [support/event_data.rs](support/event_data.rs) | Construct the shared event and dimension inputs. Expected answers stay with their checks. |
| [support/analytics_tests.rs](support/analytics_tests.rs) | Check report snapshots and cleanup after failed imports or exports. |

The `.sql` files are query inputs, not standalone programs. For example,
[query-flow.sql](query-flow.sql) uses `sales`, while [analytics.sql](analytics.sql)
uses `events` and `dimensions`. The corresponding declarations are in
[events.schema](events.schema) and [dimensions.schema](dimensions.schema).
The [CLI tutorial](../docs/start.md) explains how to supply schemas and query files.

## Run an example

Run Rust examples from the repository root with `cargo run --release --offline
--locked -j 1 --example NAME -- ARGUMENTS`. Each source introduction states its
arguments and required input database. Most create a new database; `logical_plan`
instead opens the one created by `declared`. The caller owns the remaining files.

Require successful process exit, including when rows or timings were printed.
Performance examples check answers alongside measurements; their reported engine
reservations are not whole-process memory. Use the stated input and timing
conditions when comparing runs.
