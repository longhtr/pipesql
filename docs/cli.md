# Command-line reference

The CLI opens one database, performs one command, and closes it. Build the executable
with `cargo build --release --offline --locked -j 1`; it appears at
`target/release/pipesql`. The [tutorial](start.md) supplies complete commands.

Every invocation requires an absolute database path and both resource limits:

```text
pipesql COMMAND --database ABSOLUTE_PATH
  --memory-limit-bytes N --temp-limit-bytes N
```

Options and values are separate arguments. Unknown, repeated, missing, or
command-inappropriate options are errors. There is no default database.

## Commands

| Command | Required command-specific options | Purpose |
| --- | --- | --- |
| `create-declared` | None. | Create a database for declared tables. |
| `declare` | `--schema-file PATH` | Publish an empty table's schema. |
| `schema` | `--table NAME` | Inspect its current declaration. |
| `import` | `--table NAME --input PATH` and format limits. | Append one complete input. |
| `query` | `--query-file PATH` | Print diagnostic rows. |
| `export` | `--query-file PATH` and format limits. | Write a typed result to stdout. |
| `explain` | `--query-file PATH` | Prepare SQL and print its logical plan. |
| `resolve` | `--transaction TOKEN` | Recover on open, then look up an attempt. |
| `open` | None. | Recover and check an existing database. |
| `create` | None. | Create a legacy fixed-schema database. |
| `load` | `--input PATH` | Load legacy lineitem input once. |

PATH inputs must be absolute regular files, not final symlinks. Keep them unchanged
during the command; identity, size, and timestamp checks cannot freeze another
process's writes. CSV import alone accepts `--input -` for stdin.

## Declare a schema

A schema file is UTF-8 and at most 4,096 bytes:

```text
table sales
day date required
region string required
amount int64 nullable
```

The first line names the table. Each of the next 1–64 lines gives a column name,
type, and `required` or `nullable`. Keywords and type names are lowercase; accepted
types are `int64`, `double`, `string`, and `date`. Separate fields with ASCII
whitespace. LF and CRLF are accepted and the last newline is optional. Blank lines,
comments, quotes, or extra fields are errors.

Names contain 1–32 ASCII letters, digits, or underscores and begin with a letter
or underscore. Table names and column names within a table must be unique ignoring
ASCII case. Declaration does not reject SQL reserved words, so choose
[queryable identifiers](sql/README.md#names-positions-and-identities) when SQL must
address the table or column.

Parsing finishes before publication. Syntax errors report the line's starting byte
offset; invalid names retain the library failure. Success prints `status=declared`,
generation, and transaction. Printing can fail after the declaration committed.

## Import options

CSV is the default (`--format csv`). In addition to table and input, supply:

| Option | Bounds |
| --- | --- |
| `--input-limit-bytes` | Complete CSV stream, including header. |
| `--row-limit` | Data rows. |
| `--record-limit-bytes` | Encoded record. |
| `--field-limit-bytes` | Decoded field. |
| `--batch-rows` | Rows in a decoder batch. |
| `--batch-text-bytes` | STRING bytes in a decoder batch. |
| `--batch-limit` | Native units appended. |
| `--encoded-limit-bytes` | Encoded native append data. |

These are positive limits. Decoder batches and native units differ: a batch may
produce several units as columns reach their stored-byte limit. At most 4,096 units
can be appended. [CSV bounds](formats.md#csv-bounds) gives decoder maxima.

For `--format parquet`, keep input, row, batch-limit, and encoded-limit-bytes;
replace CSV decoding options with:

```text
--metadata-limit-bytes N --row-group-limit N --row-group-rows N
--row-group-limit-bytes N --page-limit-bytes N
```

[Parquet bounds](formats.md#parquet-bounds) defines each maximum. Import validates
the entire input and commits once. Retain its early token before interpreting
any later failure; [operations](operations.md#resolve-a-write) explains why.

## Export options

Export defaults to `--format jsonl`. Supply `--row-limit` and
`--output-limit-bytes` along with the query file. For `--format parquet`, also give:

```text
--metadata-limit-bytes N --row-group-limit N --row-group-rows N
--row-group-text-bytes N
```

Parquet export writes binary stdout; JSON Lines writes UTF-8. Redirect stdout to
a new file and accept it only after successful process exit. Limits and encoded
values are defined in [formats](formats.md). An output prefix can remain on failure.

## Inspect query output

`query` begins with `status=querying`, database and budget fields, then
`column_count` and `columns`. Descriptors give name, type, and required/nullable;
an unnamed column has an empty name. Row values carry type tags. STRING is
hexadecimal UTF-8, DATE is ISO calendar text, and DOUBLE includes decimal display
plus its 16 hexadecimal bit digits, such as `double:1001:408f480000000000`.

`row_count` and `status=queried` appear after query completion. Exit must still be
zero: final flush or close may fail. Use export rather than diagnostic text for
a typed interchange file.

`schema` prints `status=inspecting`, table and pinned generation, column count,
indexed declarations such as `column[0]=day:date:required`, then `status=inspected`.
`explain` prints `status=explaining`, the logical plan, then `status=explained`.
Explanation does not execute rows, so runtime arithmetic or payload errors can
still occur when that query runs. Opening may recover before either command.

Query files have the same 4,096-byte, file-type, and stability limits as schema
files. Text output labels and public interfaces remain experimental.

## Legacy input

`load` accepts an absolute path of at most 4,096 bytes, without dot components or
NUL, to a regular non-symlink file at most 1 GiB. This is the fixed lineitem path,
not declared-table CSV. Each row has 16 pipe-delimited fields, a required final
`|`, and LF termination. CR is forbidden. A row including LF is at most 512 bytes;
at most 6,500,000 rows are allowed independently of the file-byte bound.

The loader projects source positions 5–11: quantity, extended price, discount, tax,
return flag, line status, and ship date. All seven are required. The two flags are
one printable ASCII byte `0x20`–`0x7e` except `|`. Unprojected fields may be empty
but must obey the delimiter and row grammar.

The four numbers allow an optional minus, integer digits, and an optional decimal
point followed by digits; or exactly `NaN`, `inf`, or `-inf`. Each has at most
32 ASCII bytes. Plus signs, exponents, and missing integer or fractional digits
are rejected. Date is a valid ten-byte `YYYY-MM-DD` in years 0001–9999. Keep the
input unchanged across both loader passes. The library entry is `load_lineitem`;
[storage](storage.md#formats-and-bounds) explains its distinct construction path.
