//! Run one command and translate its outcome into output and an exit status.
//!
//! `arguments` captures bounded native arguments; `command` parses an operation
//! and its options. Capture stderr and stdout before opening a database, so a
//! closed inherited descriptor cannot be reused by a database file and mistaken
//! for an output stream. Imports also capture stdin before database open.
//! `stdio` owns the captured descriptors and output buffer.
//!
//! `run` opens the database, calls the public library, then closes the database
//! even if the command failed. The command's error takes precedence over a close
//! error. Success also requires flushing output; a completed database operation
//! can therefore still produce a failing process exit status.
//!
//! Argument errors exit with status 2. Database and output errors exit with
//! status 1, even if stderr is unavailable or writing the diagnostic fails.

use command::{Command, Operation};
use diagnostic::print_error;
use output::{output_error, write_database_status};
use pipesql::{CancellationToken, CommitResolution, Database, Error};
use query::{execute_query_file, explain_query_file};
use std::io::Write;
use std::process::ExitCode;

#[allow(unsafe_code)]
mod arguments;
mod command;
mod declaration;
mod diagnostic;
mod import;
mod output;
mod query;
mod source;

#[allow(unsafe_code)]
mod stdio;

pub(super) fn main() -> ExitCode {
    // Save stderr first, before argument capture can open /proc on Linux.
    // If it is unavailable, keep None: never try descriptor 2 again later.
    let mut errors = stdio::stderr().ok();
    let command =
        match arguments::capture().and_then(|arguments| command::parse(arguments.into_iter())) {
            Ok(command) => command,
            Err(message) => {
                print_error(&mut errors, "usage error", &message);
                return ExitCode::from(2);
            }
        };
    let mut output = match stdio::Output::stdout() {
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
    // Capture stdin before a database file can occupy a closed descriptor 0.
    let input = match &command.operation {
        Operation::Import { input, limits, .. } => Some(match limits {
            command::ImportFormat::Csv(limits) => {
                import::Input::open(input, limits.csv.input_bytes)?
            }
            command::ImportFormat::Parquet(limits) => {
                import::Input::open_parquet(input, limits.parquet.input_bytes)?
            }
        }),
        _ => None,
    };
    let mut database = match &command.operation {
        Operation::Create => Database::create(&command.database, command.config)?,
        Operation::CreateDeclared => Database::create_empty(&command.database, command.config)?,
        _ => Database::open(&command.database, command.config)?,
    };
    let outcome = (|| match &command.operation {
        Operation::Create | Operation::CreateDeclared => {
            write_database_status(output, "created", &database)
        }
        Operation::Declare(schema) => declaration::declare_file(&database, schema, output),
        Operation::Schema(name) => {
            database.inspect_table(name, &CancellationToken::new(), |schema| {
                output::write_table_schema(output, &database, &schema)
            })
        }
        Operation::Open => write_database_status(output, "opened", &database),
        Operation::Load(input) => {
            let commit = database.load_lineitem(input, &CancellationToken::new())?;
            write_database_status(output, "loaded", &database)?;
            writeln!(output, "generation={}", commit.generation())
                .and_then(|()| writeln!(output, "transaction={}", commit.transaction()))
                .map_err(|source| output_error("write commit status", source))
        }
        Operation::Import { table, limits, .. } => import::execute(
            &database,
            table,
            input.expect("import source was captured"),
            *limits,
            output,
        ),
        Operation::Query(query_file) => execute_query_file(&database, query_file, output),
        Operation::Export { query, limits } => {
            query::export_query_file(&database, query, *limits, output)
        }
        Operation::Explain(query_file) => explain_query_file(&database, query_file, output),
        Operation::Resolve(transaction) => {
            let resolution = database.resolve_commit(*transaction)?;
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
    // Close on both paths, but preserve the original command failure if close
    // also fails. A successful command must report a close failure instead.
    let close = database.close();
    outcome.and(close)
}
