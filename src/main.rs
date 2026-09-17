//! Entry point for one PipeSQL command-line invocation.
//!
//! `cli::main` captures process arguments and output streams, runs the requested
//! library operation, and chooses an exit status. Process behavior stays here in
//! the executable; embedded callers use the library's results and errors directly.
//!
//! Argument, database and output failures become process failures in cli, including
//! errors after rows have already been printed. This entry point returns that status
//! unchanged. Test-only modules supply shared process and directory cleanup helpers.

mod cli;

fn main() -> std::process::ExitCode {
    cli::main()
}

#[cfg(test)]
#[path = "../test/support/mod.rs"]
mod test_support;
