mod cli;

fn main() -> std::process::ExitCode {
    cli::main()
}

#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod test_support;
