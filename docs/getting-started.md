# Create and query a declared table

This walkthrough creates a table, appends three rows, closes and reopens the
database, and computes a total for each region. It uses the ordinary Rust
library on macOS or Linux. Use the pinned toolchain and [build
prerequisites](testing.md#prerequisites); the [platform
matrix](testing.md#platform-status) distinguishes exercised behavior from the
remaining qualification requirements.

The complete program lives in [examples/declared.rs](../examples/declared.rs).
Read it alongside this walkthrough: it declares `sales(region, amount)`, writes
a single typed batch, commits, closes, and reopens before preparing the query.

## Create and query the table

From the repository root, create a temporary parent directory and run the
example. Keep this shell open so the directory variable remains available:

```sh
pipesql_example_dir=$(mktemp -d)
cargo run --release --offline --locked --example declared -- "$pipesql_example_dir/sales"
```

Successful completion prints:

```text
north 15
south 20
```

## Follow the operation

Each validity bitmap marks the three supplied values as present. A successful
`write` creates private data; `commit` publishes it. Reopen checks that the
table and rows survived closing the first handle. `GROUP AND ORDER BY`
establishes the displayed order. `Progress` means more work remains, and output
is complete only after `Finished` and successful process exit.

The example leaves the database in that directory. Running the command again
with the same path fails because creation requires a new database. Existing
databases use `Database::open`. The memory and temporary limits account for
engine-owned resources, not the caller or whole-process RSS. For typed outcomes,
abort/reopen requirements and retained query lifetimes, see
[interfaces.md](interfaces.md). For supported syntax and bounds, see
[language.md](language.md#current-public-query-manifest).

The CLI can open and query this database. Table declaration and typed append
currently require the library; the CLI's `create` and `load` commands use the
legacy `lineitem` schema.

## Finish and clean up

If the program reports an error, do not treat any printed rows as a complete
result. Dropping a failed query releases its buffers. An unfinished append can
leave recovery work for a later open; drop alone does not roll it back. The
[transaction outcomes](transactions.md#outcomes) explain when to reopen and
resolve a retained token.

When finished, remove only the temporary example directory created above:

```sh
rm -r -- "$pipesql_example_dir"
```

Continue with [the preparation
walkthrough](frontend.md#trace-a-query-through-preparation) to follow a query
from names to typed column identities. Keep the example source open when tracing
the append, commit, and query lifetimes.
