# Release notes

## 0.1.0

The first experimental release establishes an embedded library and CLI for
GNU/Linux and macOS. It includes typed tables, atomic CSV and flat uncompressed
Parquet imports, analytical pipe SQL with spill beyond memory, and typed JSON Lines
and Parquet exports. The [product scope](../README.md#product-scope),
[SQL reference](sql/README.md) and [format reference](formats.md) define the subset
and its limits.

Build from source with the pinned Rust 1.98.1 toolchain. Dependencies are vendored;
the [development guide](../DEVELOPMENT.md) describes offline builds and checks.
`pipesql --version` reports `pipesql 0.1.0` without opening a database.

Joins free unused merge buffers and the staging record after the first input's
sort finishes, before collecting the second input. This leaves more workspace
available to the second input while preserving initial memory admission and
duplicate-group replay. Measurements do not establish a general query speedup.

Development checks validate complete spilled INT64 running SUM and equality-join
answers under native allocation and I/O faults. Join fixtures cover INT64, DOUBLE,
DATE and STRING keys, unmatched rows, duplicate-group replay, cancellation, retry
and reopen. Checked window and join measurements include matching scan and ORDER
controls and separate native transfer observations.

APIs and stored formats remain unstable. This release makes no compatibility or
migration promise for future experimental versions. Release numbering does not
change the stored format or add an automatic upgrade path.

Callers must observe complete query/export outcomes and resolve uncertain commits
with their transaction tokens. Resource limits govern engine reservations, while
callers own their input/output buffers and file publication. See
[embedding](embedding.md) and [operations](operations.md) for these contracts.

Stronger power-loss qualification remains deferred. Database synchronization and
fail-closed checks remain required; process interruption and Linux container checks
do not establish storage behavior during power loss.
