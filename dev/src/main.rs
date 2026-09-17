//! Dispatch cargo dev commands and report their final process outcome.
//!
//! Public commands select focused checks, full suites, Linux execution or read-only
//! inspection. The same executable also has internal entry points for supervised
//! controls; those are launched by their owning campaigns. Unknown or empty
//! selections return an error. Interrupt handlers record cancellation before work
//! starts, allowing the process owners to finish cleanup before main reports failure.
//! Cargo compilation and artifact recording remain in workspace.

mod check;
mod linux;
mod oracle;
mod process;
mod tidy;
mod workspace;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    process::install_interrupt_handlers()?;
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    dispatch(&arguments)
}

fn dispatch(arguments: &[String]) -> Result<()> {
    match arguments {
        [verb] if verb == "check" => check::fast(),
        [verb] if verb == "tidy" => tidy::run(),
        [verb, path] if verb == "identity" => {
            check::identity::run(Some(std::path::Path::new(path)))
        }
        [verb, name] if verb == "test" && name == "identity" => check::identity::run(None),
        [verb, path, options @ ..] if verb == "inspect" => {
            oracle::catalog::command(std::path::Path::new(path), options)
        }
        [role] if role == "suite-inner" => check::suite::inner(),
        [role, mode] if role == "linux-probe" => linux::probe(mode),
        [role, path] if role == "catalog-locked" => {
            check::catalog::locked(std::path::Path::new(path))
        }
        [verb, rest @ ..] if verb == "linux" => linux::run(rest),
        [verb, rest @ ..] if verb == "linux-inner" => {
            dispatch(rest)?;
            linux::completed(rest)
        }
        [] => Err("usage: cargo dev test process | --help".into()),
        [argument] if argument == "--help" => {
            println!(
                "cargo dev check\ncargo dev tidy\ncargo dev identity /absolute/new-directory\ncargo dev test identity\ncargo dev inspect /absolute/database [--max-objects N] [--max-read-bytes N] [--max-values N]\ncargo dev test all\ncargo dev test process\ncargo dev test linux-runner\ncargo dev test rust\ncargo dev test documentation\ncargo dev test platform\ncargo dev test sanitizer --case controls|mutex|pathname|memory|concurrency --toolchain NIGHTLY\ncargo dev test initialization\ncargo dev test sync\ncargo dev test io --case controls|base|repeated|derived\ncargo dev test models\ncargo dev test aggregate\ncargo dev test codec --case catalog|snapshot|rejected|rounding\ncargo dev test composition --case grouping|expressions|dates|numeric|derived|filters|columns|repeated|demand|storage|unions|report\ncargo dev test allocation --case foundation|csv|import|parquet|catalog|recovery|legacy|concurrent|shapes|reports|joins\ncargo dev test cli --case arguments|database|receipts|streams|publication|import|export|parquet\ncargo dev test catalog\ncargo dev test analytics --case small|scaled\ncargo dev test recovery --case append-boundaries|all-events|report|images|report-images\ncargo dev linux test ...\n\nUse --list instead of --case NAME to list campaign cases. The full suite runs sequentially on frozen source. Nightly sanitizers and Docker-owner controls have separate commands."
            );
            Ok(())
        }
        [verb, name] if verb == "test" && name == "process" => check::process_tests(),
        [verb, name] if verb == "test" && name == "platform" => check::platform::run(),
        [verb, name] if verb == "test" && name == "rust" => check::rust_tests(),
        [verb, name] if verb == "test" && name == "all" => check::suite::run(),
        [verb, name] if verb == "test" && name == "documentation" => check::documentation(),
        [verb, name] if verb == "test" && name == "linux-runner" => {
            let mut run = workspace::Run::new(workspace::root()?, "linux-runner-controls")?;
            let mut command = std::process::Command::new("cargo");
            command.args([
                "test",
                "--offline",
                "--locked",
                "-p",
                "pipesql-dev",
                "-j",
                "1",
                "process::tests::linux::container_outcomes_and_interruption",
                "--",
                "--exact",
                "--ignored",
                "--nocapture",
            ]);
            let output = run.command(&mut command, None, std::time::Duration::from_secs(600))?;
            print!("{}", String::from_utf8_lossy(&output.stdout.bytes));
            eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
            output.require_success()?;
            if !std::str::from_utf8(&output.stdout.bytes)?
                .lines()
                .any(|line| line.starts_with("test result: ok. 1 passed; 0 failed; 0 ignored;"))
            {
                return Err("Linux owner control was not executed".into());
            }
            run.finish()
        }
        [verb, name] if verb == "test" && name == "models" => check::model::run(),
        [verb, name, option, case, flag, toolchain]
            if verb == "test"
                && name == "sanitizer"
                && option == "--case"
                && flag == "--toolchain" =>
        {
            check::sanitizer::run(case, toolchain, None)
        }
        [
            verb,
            name,
            option,
            case,
            flag,
            toolchain,
            vendor_flag,
            vendor,
        ] if verb == "test"
            && name == "sanitizer"
            && option == "--case"
            && flag == "--toolchain"
            && vendor_flag == "--standard-library-vendor" =>
        {
            check::sanitizer::run(case, toolchain, Some(std::path::Path::new(vendor)))
        }
        [verb, name, option] if verb == "test" && name == "codec" && option == "--list" => {
            println!("catalog\nsnapshot\nrejected\nrounding");
            Ok(())
        }
        [verb, name, option, case] if verb == "test" && name == "codec" && option == "--case" => {
            check::codec::run(case)
        }
        [verb, name, option] if verb == "test" && name == "composition" && option == "--list" => {
            println!(
                "grouping\nexpressions\ndates\nnumeric\nderived\nfilters\ncolumns\nrepeated\ndemand\nstorage\nunions\nreport"
            );
            Ok(())
        }
        [verb, name, option, case]
            if verb == "test" && name == "composition" && option == "--case" =>
        {
            check::composition::run(case)
        }
        [verb, name] if verb == "test" && name == "aggregate" => check::aggregate::run(),
        [verb, name] if verb == "test" && name == "sync" => check::sync::run(false),
        [verb, name, option, case]
            if verb == "test" && name == "sync" && option == "--case" && case == "controls" =>
        {
            check::sync::run(true)
        }
        [verb, name, option] if verb == "test" && name == "io" && option == "--list" => {
            println!("controls\nbase\nrepeated\nderived");
            Ok(())
        }
        [verb, name, option, case] if verb == "test" && name == "io" && option == "--case" => {
            check::io::run(case)
        }
        [verb, name] if verb == "test" && name == "initialization" => check::initialization::run(),
        [verb, name] if verb == "test" && name == "catalog" => check::catalog::run(),
        [verb, name, option] if verb == "test" && name == "cli" && option == "--list" => {
            println!(
                "arguments\ndatabase\nreceipts\nstreams\npublication\nimport\nexport\nparquet"
            );
            Ok(())
        }
        [verb, name, option, case]
            if verb == "test"
                && name == "cli"
                && option == "--case"
                && matches!(
                    case.as_str(),
                    "arguments"
                        | "database"
                        | "receipts"
                        | "streams"
                        | "publication"
                        | "import"
                        | "export"
                        | "parquet"
                ) =>
        {
            check::cli::run(case)
        }
        [verb, name, option] if verb == "test" && name == "allocation" && option == "--list" => {
            println!(
                "foundation\ncsv\nimport\nparquet\ncatalog\nrecovery\nlegacy\nconcurrent\nshapes\nreports\njoins"
            );
            Ok(())
        }
        [verb, name, option, case]
            if verb == "test"
                && name == "allocation"
                && option == "--case"
                && matches!(
                    case.as_str(),
                    "foundation"
                        | "csv"
                        | "import"
                        | "parquet"
                        | "catalog"
                        | "recovery"
                        | "legacy"
                        | "concurrent"
                        | "shapes"
                        | "reports"
                        | "joins"
                ) =>
        {
            check::allocation::run(case)
        }
        [verb, name, option] if verb == "test" && name == "analytics" && option == "--list" => {
            println!("small\nscaled");
            Ok(())
        }
        [verb, name, option, case]
            if verb == "test"
                && name == "analytics"
                && option == "--case"
                && matches!(case.as_str(), "small" | "scaled") =>
        {
            check::analytics::run(case)
        }
        [verb, name, option] if verb == "test" && name == "recovery" && option == "--list" => {
            println!("append-boundaries\nall-events\nreport\nimages\nreport-images");
            Ok(())
        }
        [verb, name, option, case]
            if verb == "test"
                && name == "recovery"
                && option == "--case"
                && matches!(
                    case.as_str(),
                    "append-boundaries" | "all-events" | "report" | "images" | "report-images"
                ) =>
        {
            check::recovery::run(case)
        }
        _ => Err(format!("unknown or empty selection: {}", arguments.join(" ")).into()),
    }
}
