# Data formats

Import maps an external file to a declared table. Export maps a query result to an
external stream. These are separate contracts: CSV is input, JSON Lines is output,
and Parquet supports both within the profile below.

| Format | Use | Type information |
| --- | --- | --- |
| CSV | Import text rows. | Supplied by the destination declaration. |
| Parquet | Import or export a binary column file. | Carried by the file schema. |
| JSON Lines | Export exact typed results. | Carried by the first record. |

All formats preserve the difference between NULL and an ordinary value. Export
success includes completing the query and flushing output. Import success includes
committing every row in one attempt. See [operations](operations.md) for retry and
incomplete-output handling, and [CLI](cli.md) for command options.

## CSV

The header names each declared column exactly once. Names match ignoring ASCII
case; order may differ from the declaration. Unknown, missing, or repeated names
fail. Decoded batches use declaration order.

A comma separates fields. LF or CRLF ends a record, and the last record may end
at EOF. Bare CR outside quotes fails. Blank records are not skipped. Whitespace
is data: it is never trimmed, and a UTF-8 byte-order mark is not special.

A quoted field starts with `"` as its first byte. Doubled quotes inside it mean
one quote; commas and line endings inside are text. After the closing quote,
only a delimiter, record end, or EOF is valid. Unquoted fields cannot contain quotes.

Unquoted `\N` is NULL. Quoted `"\N"` is text. An empty field is empty text, not NULL,
and cannot be converted to an empty number or date. NULL in a required column fails.
Quoting removes CSV escaping; it does not change the destination type's grammar.

| Destination | Accepted decoded text |
| --- | --- |
| INT64 | Optional sign followed by decimal digits, within signed 64-bit range. Leading zeros are allowed. |
| DOUBLE | Optional sign, decimal mantissa with at least one digit, optional decimal point, and optional `e`/`E` exponent with its digits. Also exactly `NaN`, `Infinity`, or `-Infinity`. |
| DATE | A valid Gregorian `YYYY-MM-DD` in years 0001–9999. |
| STRING | Valid UTF-8, including empty text and NUL. |

DOUBLE conversion uses nearest binary64. Decimal overflow fails; underflow may
produce signed zero. No locale-specific number or date interpretation is used.

### CSV bounds

The input-byte limit includes the header. The row limit counts data rows. A record's
encoded size includes quotes, delimiters, and its line ending; a field's size is
its decoded content. All import bounds are explicit and positive.

| Decoder capacity | Maximum |
| --- | ---: |
| Columns | 64 |
| Encoded record bytes | 8,388,801 |
| Decoded field bytes | 65,536 |
| Rows in a decoder batch | 256 |
| STRING bytes in a decoder batch | 4,194,304 |

One row must fit the batch's text allowance. A wider next row can end the current
batch early and be retained for the following call. To distinguish exact EOF from
excess input, the decoder may probe one byte past the total input limit; it refuses
that byte. Buffered reads may look ahead by at most 4,096 bytes.

The standalone `CsvDecoder` obtains its admitted workspace before reading. It lends
batches until `next_batch` returns `None`; errors are terminal. Earlier batches only
establish a valid prefix. Import rejects a missing header and also rejects a header
with no data, reporting offset zero for the latter.

### CSV error positions

Malformed input and input-limit failures use `Error::Input` with a zero-based byte
offset into the original stream. The location depends on what failed:

| Failure | Offset |
| --- | --- |
| Syntax, UTF-8, or byte limit | Offending byte; EOF if a required byte is missing. |
| Header name or type conversion | Field start, including its opening quote. |
| Field count | Extra field or record end. |
| Row limit or a row too wide for the batch | Row start. |

I/O failures retain their underlying cause. Import cleanup failure is separate
from the input failure; it can require recovery even though the input was rejected.

## Parquet

PipeSQL accepts one seekable `PAR1` file containing 1–64 flat named columns. It
supports uncompressed PLAIN values, not the full range of Parquet encodings.
A reader accepting PipeSQL output does not imply its own default output is accepted
by PipeSQL. For example, an external PyArrow writer should disable compression
and dictionary encoding.

| PipeSQL type | Physical and logical representation |
| --- | --- |
| INT64 | INT64, either unannotated or annotated as a signed 64-bit integer. |
| DOUBLE | DOUBLE, preserving its IEEE-754 bits. |
| DATE | INT32 with DATE annotation; days since 1970-01-01, restricted to years 0001–9999. |
| STRING | BYTE_ARRAY with STRING/UTF8 annotation and valid UTF-8. |
| NULL | Missing optional value, encoded through definition levels. |

Old converted-type annotations and newer logical-type annotations must agree.
Column names must be unique and match the declared schema ignoring ASCII case;
types must match exactly. NULL still fails in a required destination column.
STRING cells have a 65,536-byte maximum. Export requires unique named result
columns following declaration name rules, so alias unnamed or duplicate outputs.

The reader accepts data pages v1 and v2 with RLE/bit-packed hybrid definition
levels; the writer emits v1. Compression, dictionary, delta, byte-stream-split,
and deprecated BIT_PACKED level encoding are unsupported. So are nested or repeated
fields, external column files, encryption, and unlisted physical or logical types.

Chunks must lie within the file and appear without overlap in metadata order.
Every group in a nonempty file must have a positive row count. Page CRC-32 is checked
when present. Statistics or indexes never permit skipping validation. Invalid
ranges, truncation, and missing, duplicate, or inconsistent required metadata fail.
Unknown optional metadata is skipped only within the parser's bounds.

### Parquet bounds

| Capacity | Import | Export |
| --- | ---: | ---: |
| Footer metadata | 16 MiB | 16 MiB |
| Row groups | 4,096 | 4,096 |
| Rows per group | 65,536 | 65,536 |
| Encoded bytes per group | 64 MiB | Derived from admitted columns and group bounds. |
| Page payload | 16 MiB | One page per column/group within the reader bound. |
| Retained STRING bytes per group | Covered by encoded group bound. | 8 MiB |

Input page headers are limited to 65,536 bytes and nested metadata to 16 containers.
A whole input group must fit its admitted workspace. Export accumulates groups
across query batches, and one row must fit the group's text allowance. Neither
path retains the complete file or result in memory.

Import requires a seekable input and therefore does not accept stdin. Its limits
are positive, and the total input allowance must be at least 12 bytes. An empty
file is rejected before transaction issuance. Export permits an empty result,
writing schema and footer with zero groups. Its metadata, group, row-group-row,
and text capacities must still be positive.

[parquet.rs](../examples/parquet.rs) demonstrates a round trip.
[External test files](../test/data/parquet/README.md) retain interoperability sources.
The format references are [Parquet 2.10.0](https://github.com/apache/parquet-format/tree/apache-parquet-format-2.10.0)
and the [Thrift compact protocol](https://github.com/apache/thrift/blob/v0.22.0/doc/specs/thrift-compact-protocol.md).

## JSON Lines

The first line describes positional columns. Each following row has exactly one
cell per column. The last line reports successful query completion:

```json
{"format":"pipesql-jsonl","version":1,"columns":[{"name":"amount","type":"int64","nullable":false}]}
{"row":["9223372036854775807"]}
{"complete":true,"rows":1}
```

The schema is present even for an empty result. Column names may repeat; an unnamed
expression has a JSON `null` name. Positional rows prevent a duplicate name from
overwriting another value as it would in a JSON object keyed by column name.

| Value | JSON cell |
| --- | --- |
| NULL | `null` |
| INT64 | Decimal string. |
| DOUBLE | String of 16 lowercase hexadecimal digits containing raw bits, most significant digit first. |
| DATE | `YYYY-MM-DD` string. |
| STRING | JSON string with controls escaped, including NUL; Unicode text is preserved. |

INT64 is a string so readers using floating-point JSON numbers do not round it.
DOUBLE uses bits so nonfinite values and signed zeros survive exactly. Consumers
must interpret cells using the schema. CSV import does not read this format.

## Result completion

Export row and byte limits include the entire encoded result: schema, values,
escaping, line endings or binary metadata, and completion record or footer. A
zero-row limit permits an empty result. A zero-byte limit cannot contain a complete
result. No write may exceed the byte allowance, but refusal can leave a partial
record or file.

Accept output only when the export call or process succeeds. Seeing its final
record is insufficient if a later flush or database close fails. The caller owns
incomplete bytes and any replacement of a destination file. Cancellation does not
interrupt a blocked writer, and drop does not flush buffered output.
