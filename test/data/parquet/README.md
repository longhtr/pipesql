# Independent Parquet inputs

These files encode first-party literal values using PyArrow 22.0.0. They exercise
an implementation independent of PipeSQL's metadata and page codecs. The inputs
include NULL, extreme INT64 and DATE values, exact exceptional DOUBLE bits,
Unicode, controls, empty text and a 65,536-byte string.

[interop.py](interop.py) owns the values and
writer options. In a development environment containing exactly PyArrow 22.0.0,
run it without arguments to reproduce and compare every byte. `--write` replaces
only its eight named outputs. The script reads each flat output back with PyArrow
and compares full values, including DOUBLE bits, before retaining it.

| File | Purpose |
| --- | --- |
| `plain-v1.parquet` | Uncompressed PLAIN values, v1 pages, CRC-32, three row groups. |
| `plain-v2.parquet` | The same complete values in v2 pages with CRC-32. |
| `pipesql-v1.parquet` | PipeSQL output independently read and checked by PyArrow; reproduced by the library export test. |
| `plain-batches.parquet` | 600 consecutive INT64 values in one group with several pages, lent across three import batches. |
| `typed-boundaries-v1.parquet` | 777 typed rows in groups of 389 and 388, independent NULL periods and unequal column page boundaries. |
| `typed-boundaries-v2.parquet` | The same complete typed rows with v2 pages and CRC-32. |
| `pipesql-typed-boundaries.parquet` | PipeSQL export of all 777 typed rows, independently checked with PyArrow, grouped by both row and text limits. |
| `unsupported-snappy.parquet` | Explicit refusal of a compressed file. |
| `unsupported-dictionary.parquet` | Explicit refusal of dictionary-encoded input. |
| `unsupported-nested.parquet` | Explicit refusal of a nested/repeated schema. |

The decoder lends the two groups as batches of 389 and 388 rows. Integer and
DOUBLE pages hold up to 148 rows, DATE pages up to 296, and text pages mostly 37.
Their two 65,536-byte strings cross different batch positions. The private
`parquet::boundary_tests` checks every cell against independently authored values
after mapping columns into a different target order. Its literal mutations affect
the last text page of the second group: invalid UTF-8 with an updated IEEE CRC,
and a v2 NULL count that differs from the definition levels. Each demands the
specific error and file offset, no exposed failed group, an aborted import into a
nonempty target and a complete retry after release. Separate footer mutations
change the file, group or column count without changing field sizes; these must
fail before transaction issuance and preserve the existing table.

`pipesql-v1.parquet` is produced by
`parquet::export::tests::complete_scalar_export_matches_external_reader_fixture`.
That test imports `plain-v2.parquet`, orders by `id`, then compares every output
byte. Its source owns the exact export limits. The generator's default mode also
reads this retained output with PyArrow and checks the independent literal values;
`--read-export FILE` applies that check to another freshly produced output. Updating
expected output requires this independent check, not merely accepting bytes from
the production encoder.

`pipesql-typed-boundaries.parquet` is reproduced by
`parquet::boundary_tests::typed_export_preserves_values_across_row_text_and_cursor_boundaries`.
The table declaration reverses the input columns; the export projects them back
to `id`, `amount`, `number`, `day`, `note`, ordered by `id`. Limits of 311 rows
and 65,536 text bytes per group produce groups of 255, 1, 311, 78, 2 and 130 rows.
The test compares complete output from different query batch shapes and checks
exact resource refusals and output prefixes before a healthy retry. Run
`--read-boundary-export FILE` to independently check a freshly produced file's
schema, all values and group sizes. Default mode also checks the retained output.

Fresh transfer profiles use `--read-profile DIRECTORY QUERY PROFILE ROWS
GROUP_ROWS GROUP_TEXT`; `--read-profile-file` takes a single file with the same
selection. The supported inputs are those in `export_cost`. These modes check
complete schema and values, raw DOUBLE buffers, row identities and row/text group
cuts. Directory mode also rejects unfinished failure files and runs deliberate
corruptions on the 257-row control. JSON receipts include file hashes and separate
validation times. `--describe` identifies the installed reader; Python's `-O`
mode is rejected because it disables assertions.

Use the optional image and supervised command in
[development](../../../DEVELOPMENT.md#measure-analytical-work) to run these checks.
The supervisor requires every receipt, matches files to producer API outcomes and
owns deadlines and cleanup. Reading a file successfully does not establish that
the export API or final writer flush succeeded.

`--read-wide-measure DIRECTORY ROWS GROUP_ROWS` checks six complete 64-column
files from a measured export process. It accepts 513 or 8,192 rows, checks each
permuted cell and raw DOUBLE bits, and derives exact group extents from the chosen
row bound. The small control rejects a substituted same-type column, a changed
zero sign bit and a readable prefix. The Rust supervisor separately checks sample
order, API completion, byte counts, resource release and file hashes.

`--write-imports NEW_DIRECTORY` creates eight fresh v1/v2 inputs for
`cargo dev linux test parquet-transfer`. It extends the typed boundary pattern to
777 or 8,192 rows and writes them in descending ID order. Row groups contain up to
255, 256, 257, 389 or 513 rows; page targets and write batches vary independently of the driver's
7-byte and 127-byte reads of every input. Page sizes are writer targets: a 65,536-byte value may exceed them.
The v1 8,192-row inputs compare groups of 255, 256 and 257 with identical page
targets and write batches. The generator reads every value back, verifies raw
DOUBLE bits and group counts,
and returns file hashes and generation settings as JSON. The directory must not
already exist. Two qualification-only inputs place maximum-length strings at IDs
768 through 776 in 513-row groups, crossing both the lending and native column
byte boundaries. Their page/group bound is 1 MiB; the original inputs retain their
smaller bounds. Retained fixtures and default reproduction stay unchanged.

The Rust supervisor checks the imported database after close/reopen through JSON
Lines with its own literal expectations. Import and Parquet export cannot conceal
matching format mistakes in this check.

`--write-wide-imports NEW_DIRECTORY` creates two fresh 64-column inputs for the
same transfer qualification. They contain 513 descending row identities and
repeated INT64, DOUBLE, DATE and STRING columns with independent NULL positions.
Physical columns follow a permutation; the target declaration reverses their
names, and JSON Lines restores canonical order for the supervisor's separate
expected values. V1 uses one 513-row group, crossing the decoder's 512-row lend;
v2 uses groups of 257 and 256, with different page targets and writer batches.
The generator reads both files back and checks every value and raw DOUBLE bit.
A late source fault must preserve an existing row and release private units;
reopen resolves its aborted receipt, and a healthy retry returns all 514 rows.
Wrong same-type columns, changed zero bits, shifted NULLs, incomplete output and
a disabled source fault must fail their controls. These generated files do not
change retained fixtures or their default reproduction.

This small optional generator stays in Python because PyArrow supplies the
independent reader/writer that produced these bytes. It is outside Cargo and the
ordinary offline suite; routine development tooling is Rust. Run it explicitly
with `python3 test/data/parquet/interop.py` in the provisioned environment.

PyArrow is a development oracle; its source and binaries are not vendored or
linked into PipeSQL. The [profile](../../../docs/formats.md#parquet) defines the supported
subset and its limits. These fixtures alone do not establish complete format
interoperability, resource bounds or persistence behavior.
