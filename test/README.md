# Integration tests

These tests exercise the database through its public Rust API and CLI. They check
complete results, transaction outcomes and resource lifetimes across components.
Private implementation tests remain beside their owners in [src/](../src/README.md).

## Structure

| Area | Responsibility |
| --- | --- |
| [public.rs](public.rs), [public/](public/) | Declared-table queries, append, import, export and SQL behavior. |
| [execution.rs](execution.rs), [execution/](execution/) | Queries over the legacy `lineitem` schema, result ownership, CLI output and stack checks. |
| [lifecycle.rs](lifecycle.rs) | Database creation, exclusive opening, basic CLI operations and cleanup failures. |
| [load.rs](load.rs), [load/](load/) | Legacy loading through commit, close, reopen and transaction resolution. |
| [support/](support/) | Shared directory, child-process and input-writing helpers. |
| [data/](data/) | Stored inputs, SQL and expected bytes, including rejected formats and external Parquet samples. |

The root `.rs` files are the four integration test targets. Their child directories
group related cases. [Cargo.toml](../Cargo.toml) registers each target explicitly
because Cargo does not discover `test/` automatically.

Expected answers belong to the cases. Support code shares setup and cleanup, not
the calculation being checked. Stored samples retain their attribution; see
[THIRD_PARTY.md](../THIRD_PARTY.md).

## Find the right check

For declared-table behavior, start in `public/`. For a process-lifetime failure,
start in `lifecycle.rs`: its lease test waits until a child has opened the database,
checks that competing opens fail, then checks that a later open succeeds after
the child closes or stops.

Use the [development guide](../DEVELOPMENT.md#test-the-changed-behavior) for test
commands and selection. Native fault campaigns, allocation observations and
independent format checks are supervised by [dev/](../dev/README.md).
