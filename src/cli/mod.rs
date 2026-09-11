// Process entry and command lifetime. Parsing, native capture, and output have
// separate owners; every opened database is closed after its operation returns.
use command::{Command, Operation};
use diagnostic::print_error;
use output::{output_error, write_database_status};
use pipesql::{CancellationToken, CommitResolution, Database, Error};
use query::execute_query_file;
use std::io::Write;
use std::process::ExitCode;

#[allow(unsafe_code)]
mod arguments;
mod command;
mod diagnostic;
mod output;
mod query;

#[allow(unsafe_code)]
mod sink;

pub(super) fn main() -> ExitCode {
    // Capture before any engine descriptor can reuse a closed inherited slot.
    // No diagnostic sink is an explicit allowed state, not a fallback to fd 2.
    let mut errors = sink::stderr().ok();
    let command =
        match arguments::capture().and_then(|arguments| command::parse(arguments.into_iter())) {
            Ok(command) => command,
            Err(message) => {
                print_error(&mut errors, "usage error", &message);
                return ExitCode::from(2);
            }
        };
    let mut output = match sink::Output::stdout() {
        Ok(output) => output,
        Err(source) => {
            print_error(
                &mut errors,
                "database error",
                &output_error("capture command output", source),
            );
            return ExitCode::from(1);
        }
    };
    let outcome = run(command, &mut output);
    let outcome = outcome.and_then(|()| {
        output
            .flush()
            .map_err(|source| output_error("flush command output", source))
    });
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            print_error(&mut errors, "database error", &error);
            ExitCode::from(1)
        }
    }
}

fn run(command: Command, output: &mut impl Write) -> Result<(), Error> {
    let mut database = if matches!(command.operation, Operation::Create) {
        Database::create(&command.database, command.config)?
    } else {
        Database::open(&command.database, command.config)?
    };
    let outcome = (|| match command.operation {
        Operation::Create => write_database_status(output, "created", &database),
        Operation::Open => write_database_status(output, "opened", &database),
        Operation::Load(input) => {
            let commit = database.load_lineitem(&input, &CancellationToken::new())?;
            write_database_status(output, "loaded", &database)?;
            writeln!(output, "generation={}", commit.generation())
                .and_then(|()| writeln!(output, "transaction={}", commit.transaction()))
                .map_err(|source| output_error("write commit status", source))
        }
        Operation::Query(query_file) => execute_query_file(&database, &query_file, output),
        Operation::Resolve(transaction) => {
            let resolution = database.resolve_commit(transaction)?;
            write_database_status(output, "resolved", &database)?;
            writeln!(output, "transaction={transaction}")
                .and_then(|()| match resolution {
                    CommitResolution::Durable(commit) => {
                        writeln!(
                            output,
                            "resolution=durable\ngeneration={}",
                            commit.generation()
                        )
                    }
                    CommitResolution::Aborted => writeln!(output, "resolution=aborted"),
                })
                .map_err(|source| output_error("write resolution status", source))
        }
    })();
    let close = database.close();
    outcome.and(close)
}
