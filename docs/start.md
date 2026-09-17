# Build a daily sales report

This tutorial imports six sales records and produces daily regional totals with
a running total for each region. You will see how a missing amount affects the
report, export the typed answer, and verify the import's transaction outcome.

Run the shell blocks in order from the repository root, in the same shell. Install
the toolchain named in `rust-toolchain.toml` first; dependencies are vendored.

## Prepare the workspace

```sh
set -eu
cargo build --release --offline --locked -j 1
pipesql_work=$(mktemp -d)
printf 'Tutorial directory: %s\n' "$pipesql_work"
pipesql() {
  target/release/pipesql "$@" --database "$pipesql_work/database" \
    --memory-limit-bytes 16000000 --temp-limit-bytes 32000000
}
```

The shell stops on a failed command. Keep the printed directory if something fails;
cleanup comes at the end. The helper supplies a database path and budgets for this
small workload. They bound engine reservations, not whole-process memory.

## Give the data a schema

```sh
cat > "$pipesql_work/sales.schema" <<'SCHEMA'
table sales
day date required
region string required
amount int64 nullable
SCHEMA
pipesql create-declared
pipesql declare --schema-file "$pipesql_work/sales.schema"
```

Dates and regions are required. Amounts are exact signed integers and may be NULL.
The declaration creates an empty table and publishes generation 1. No type is
inferred from the CSV that follows.

## Import the records

```sh
cat > "$pipesql_work/sales.csv" <<'CSV'
day,region,amount
2026-01-01,north,10
2026-01-01,north,20
2026-01-02,north,\N
2026-01-02,south,7
2026-01-03,north,5
2026-01-03,south,8
CSV
pipesql import --table sales --input "$pipesql_work/sales.csv" \
  --input-limit-bytes 1024 --row-limit 10 \
  --record-limit-bytes 128 --field-limit-bytes 64 \
  --batch-rows 4 --batch-text-bytes 256 \
  --batch-limit 8 --encoded-limit-bytes 1000000 > "$pipesql_work/import.receipt"
cat "$pipesql_work/import.receipt"
```

Unquoted `\N` means NULL. The north record on January 2 exists, but its amount is
unknown. An empty numeric field would be invalid rather than another spelling of
NULL.

The bounds cover input parsing and the native units produced by the append. They
are explicit so oversized input fails instead of silently expanding the work.
[CLI options](cli.md#import-options) and [CSV bounds](formats.md#csv-bounds) give
their units.

All six rows commit together as generation 2. The receipt includes a token written
before input processing proceeds. Save it: if the process stops before the final
outcome, resolution can tell you whether retrying would duplicate the import.

## Group first, then calculate the running total

```sh
cat > "$pipesql_work/report.sql" <<'SQL'
FROM sales
|> AGGREGATE SUM(amount) AS total, COUNT(*) AS n GROUP BY region, day
|> EXTEND SUM(total) OVER (PARTITION BY region ORDER BY day) AS running
|> ORDER BY region, day
SQL
pipesql query --query-file "$pipesql_work/report.sql"
```

AGGREGATE makes one row per region and day. The window then adds a cumulative SUM
of those daily totals. The final ORDER BY controls presentation; the window's
order controls its calculation.

| region | day | total | n | running |
| --- | --- | ---: | ---: | ---: |
| north | 2026-01-01 | 30 | 2 | 30 |
| north | 2026-01-02 | NULL | 1 | 30 |
| north | 2026-01-03 | 5 | 1 | 35 |
| south | 2026-01-02 | 7 | 1 | 7 |
| south | 2026-01-03 | 8 | 1 | 15 |

COUNT(*) includes the January 2 north record. Its SUM is NULL because it has no
present amounts. Running SUM ignores that NULL and retains 30. Partitioning by
region makes south start its own total at 7.

The CLI prints typed diagnostic values, including hexadecimal STRING bytes. It
ends with `row_count=5` and `status=queried`. Successful exit is also required:
rows printed before a later failure are an incomplete answer.

## Export and verify the write

```sh
pipesql export --query-file "$pipesql_work/report.sql" \
  --row-limit 10 --output-limit-bytes 4096 > "$pipesql_work/report.jsonl"
cat "$pipesql_work/report.jsonl"
pipesql_token=$(sed -n 's/^transaction=//p' "$pipesql_work/import.receipt")
pipesql resolve --transaction "$pipesql_token"
```

The export begins with a schema, then five positional rows, and ends with
`{"complete":true,"rows":5}`. Integer cells are strings so JSON consumers cannot
round large INT64 values. A NULL cell remains JSON `null`. See [JSON Lines](formats.md#json-lines)
for the complete encoding.

Resolution reports `resolution=durable`, generation 2, and `status=resolved`.
It would report aborted for an issued attempt that did not commit. A missing
success receipt alone does not establish an abort; read
[write resolution](operations.md#resolve-a-write) before implementing retries.

When finished, remove the directory this tutorial created:

```sh
rm -r -- "$pipesql_work"
```

## Continue

Read [architecture](architecture.md) to understand why a prepared query keeps one
version of the data and how a report can exceed memory. Use [embedding](embedding.md)
to build the same workflow into a Rust application. The [SQL reference](sql/README.md)
explains the available stages; [windows](sql/windows.md) shows why rows with equal
order keys share a running total.
