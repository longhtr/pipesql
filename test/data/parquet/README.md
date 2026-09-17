# Independent Parquet inputs

These files encode first-party literal values using PyArrow 22.0.0. They exercise
an implementation independent of PipeSQL's metadata and page codecs. The inputs
include NULL, extreme INT64 and DATE values, exact exceptional DOUBLE bits,
Unicode, controls, empty text and a 65,536-byte string.

[interop.py](interop.py) owns the values and
writer options. In a development environment containing exactly PyArrow 22.0.0,
run it without arguments to reproduce and compare every byte. `--write` replaces
only its six named outputs. The script reads each flat output back with PyArrow
and compares full values, including DOUBLE bits, before retaining it.

| File | Purpose |
| --- | --- |
| `plain-v1.parquet` | Uncompressed PLAIN values, v1 pages, CRC-32, three row groups. |
| `plain-v2.parquet` | The same complete values in v2 pages with CRC-32. |
| `pipesql-v1.parquet` | PipeSQL output independently read and checked by PyArrow; reproduced by the library export test. |
| `plain-batches.parquet` | 600 consecutive INT64 values in one group with several pages, lent across three import batches. |
| `unsupported-snappy.parquet` | Explicit refusal of a compressed file. |
| `unsupported-dictionary.parquet` | Explicit refusal of dictionary-encoded input. |
| `unsupported-nested.parquet` | Explicit refusal of a nested/repeated schema. |

`pipesql-v1.parquet` is produced by
`parquet::export::tests::complete_scalar_export_matches_external_reader_fixture`.
That test imports `plain-v2.parquet`, orders by `id`, then compares every output
byte. Its source owns the exact export limits. The generator's default mode also
reads this retained output with PyArrow and checks the independent literal values;
`--read-export FILE` applies that check to another freshly produced output. Updating
expected output requires this independent check, not merely accepting bytes from
the production encoder.

This small optional generator stays in Python because PyArrow supplies the
independent reader/writer that produced these bytes. It is outside Cargo and the
ordinary offline suite; routine development tooling is Rust. Run it explicitly
with `python3 test/data/parquet/interop.py` in the provisioned environment.

PyArrow is a development oracle; its source and binaries are not vendored or
linked into PipeSQL. The [profile](../../../docs/formats.md#parquet) defines the supported
subset and its limits. These fixtures alone do not establish complete format
interoperability, resource bounds or persistence behavior.
