# Development tools

This directory builds the `cargo dev` runner and the programs it supervises.
The runner asks Cargo to build artifacts, runs checks with deadlines, and keeps
diagnostics for failed runs. Start with [src/main.rs](src/main.rs) for command
dispatch and [src/check/](src/check/) for individual verification campaigns.

## Structure

| Area | Responsibility |
| --- | --- |
| [src/check/](src/check/) | Set up cases, run programs and judge complete results; also run small abstract models. |
| [src/oracle/](src/oracle/) | Independently decode stored formats and calculate expected answers. |
| [src/process.rs](src/process.rs) | Own child processes and descendants, capture output, enforce deadlines and finish cleanup. |
| [src/workspace.rs](src/workspace.rs) | Select Cargo artifacts, freeze source inputs and record commands and results. |
| [src/linux.rs](src/linux.rs) | Run frozen inputs in Linux containers, collect results and remove owned containers. |
| [src/tidy.rs](src/tidy.rs) | Check local documentation links. |
| [driver/](driver/) | Build small programs that exercise the stock database library. |
| [native/](native/) | Observe OS calls and inject I/O failures or process interruption. |
| [bindings/](bindings/) | Supply small C bridges for native ABI details. |
| [sanitizer.Dockerfile](sanitizer.Dockerfile) | Provision the separate Linux image for sanitizer diagnostics. |

The runner has no dependency on the database library. Drivers produce actual
results; independent checks determine whether those results are correct.
Native observers control failure points without deciding the expected SQL answer.

## Follow a check

For an import/report/export workflow, read
[src/check/analytics.rs](src/check/analytics.rs). It creates inputs, runs the CLI
and checks complete typed output. An unexpected command failure stops the workflow
and keeps its working files for investigation. Process success alone is insufficient: the
expected schema, rows and completion must also match.

Run `cargo dev --help` from the repository root to see commands. The
[development guide](../DEVELOPMENT.md) explains focused checks, Linux execution
and failure replay. [Platform limitations](../docs/operations.md#choose-storage-that-meets-the-engines-assumptions) define what
native and instrumented checks can establish.
