mod cli;

fn main() -> std::process::ExitCode {
    cli::main()
}

#[cfg(test)]
#[path = "../tests/support/cleanup.rs"]
mod test_cleanup;
