# PipeSQL

PipeSQL is an embedded analytical database written in Rust. Queries start with
`FROM` and compose transformations with `|>`. It is also a learning project:
the code should make query execution, resource ownership and recovery understandable.

GNU/Linux is the primary platform; macOS is supported. Windows is deferred.
The project is experimental: APIs and stored formats are unstable, and no release
is planned. Production-grade correctness is the engineering standard, not a claim
that platform and durability qualification are complete.

## Try it

Install the pinned Rust toolchain, then run from the repository root:

```sh
cargo build --release --offline --locked -j 1
pipesql_example=$(mktemp -d)
cargo run --release --offline --locked -j 1 --example declared -- "$pipesql_example/sales"
```

The example appends four sales rows, reopens the database and prints:

```text
north total=15 rows=3 present=2
south total=20 rows=1 present=1
```

Its query is:

```sql
FROM sales
|> AGGREGATE SUM(amount) AS total, COUNT(*) AS n, COUNT(amount) AS present
   GROUP AND ORDER BY region
```

Read [the example](examples/declared.rs), or follow the complete
[CLI import/query/export tutorial](docs/start.md). Check successful process exit:
printed rows can be a prefix followed by an error. When finished, remove only the
example directory you created:

```sh
rm -r -- "$pipesql_example"
```

Dependencies are vendored; builds run offline after toolchain installation.
[DEVELOPMENT.md](DEVELOPMENT.md) covers setup, focused checks and Linux containers.

## What it does

The library and local CLI create typed tables, append batches, query committed
snapshots and export results. Supported values are INT64, DOUBLE, STRING, DATE
and NULL. Queries include projections, expressions, filters, equality joins,
grouping, sorting, set operations, searched CASE, partitioned count and ordered
INT64 running SUM. Blocking operators use temporary files for work beyond memory.

CSV and bounded Parquet imports publish all rows in one transaction. JSON Lines
and Parquet exports preserve typed results and report incomplete output. One writer
can publish while existing readers retain older immutable snapshots. Transaction
tokens let callers resolve uncertain writes after reopen. The legacy fixed-schema
`lineitem` loader supports Q1/Q6 through the same query engine.

The [query reference](docs/sql/README.md) defines the accepted subset and limits;
this is not general GoogleSQL or TPC-H support. The [format references](docs/formats.md)
define CSV, JSON Lines and the uncompressed flat Parquet profile.

## Product scope

PipeSQL is a single-node library for scans, joins, aggregation, bulk import/export
and analytical transformations. It has no server, replication, distributed
scheduler or row-at-a-time multiwriter transaction system. Traditional SQL query
blocks, remote transactional storage, triggers, stored procedures, foreign keys
and unrestricted native extensions are outside the present scope.

Correctness and truthful commit outcomes come first, followed by bounded resources
and failure containment, analytical usefulness, measured performance, simplicity,
dependency cost and extensibility. A later priority cannot excuse violating an
earlier one. Unsupported operations fail explicitly. Limits apply to engine-owned
reservations, not whole-process memory; durability depends on the storage stack.

## Read the system

The [documentation](docs/README.md) leads from a first report to guides, language
and format references, and the storage design. Use
[DEVELOPMENT.md](DEVELOPMENT.md) to build, test and investigate failures.

The development guide tracks [remaining work](DEVELOPMENT.md#remaining-work).
The Rust API reference lives beside the public types and methods and can be built with
`cargo dev test documentation`. Third-party sources and retained test inputs are
attributed in [THIRD_PARTY.md](THIRD_PARTY.md).
