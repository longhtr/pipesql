# PipeSQL

PipeSQL is an embedded analytical database: write typed data, compose a report
with pipe SQL, and pull the answer in batches. These chapters explain how to use
it and why the engine works the way it does.

## Start with a report

[Build a daily sales report](start.md) imports CSV, groups records, calculates
regional running totals, and exports the answer. It uses six rows so every result
can be checked by hand.

## Understand the design

Read these chapters in order:

1. [Architecture](architecture.md): the design choices connecting snapshots,
   column storage, batch execution, bounded resources, and commit outcomes.
2. [Execution](execution.md): one report from SQL names to operators, replay,
   spill, and a completed result.
3. [Storage](storage.md): which files establish authority and how publication,
   recovery, and reclamation preserve it.

## Use the engine

[Embedding](embedding.md) explains API lifetimes and resource ownership.
[Operations](operations.md) covers uncertain writes, incomplete output, cancellation,
and filesystem requirements. [CLI](cli.md) lists commands and their options.

## Look up a rule

The [SQL reference](sql/README.md) defines stages, expressions, aggregation, and
windows. [Data formats](formats.md) defines CSV, Parquet, and typed JSON Lines.
The Rust API documents individual types and methods beside their implementation;
build it with `cargo dev test documentation`.

For repository work, use [DEVELOPMENT.md](../DEVELOPMENT.md). For the experimental
status and intended scope, read the [project README](../README.md#product-scope).
