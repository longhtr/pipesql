//! Start one command-line invocation and return its process exit status.
//!
//! The CLI module owns argument handling and terminal output. Keeping that work
//! in the executable lets embedded applications use the library's typed results
//! and errors without adopting this program's process behavior.

mod cli;

fn main() -> std::process::ExitCode {
    cli::main()
}

#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod test_support;
