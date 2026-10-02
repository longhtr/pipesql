//! Check CLI database operations across allocation refusal and uncertain publication.
//!
//! Each operation uses controlled database copies and a successful allocation census
//! to select refusal prefixes. Stock CLI commands reopen or inspect the result and
//! resolve transaction tokens, distinguishing an aborted attempt from a durable
//! write despite an earlier error. Healthy receipt checks require complete reports
//! and fixture-specific generations. Queries and inspections require complete
//! reports or correct failed prefixes. File comparisons establish when a refusal
//! must leave stored state unchanged; child case modules reuse setup without deriving
//! their answers from the operation being tested.

use super::{Campaign, Result, census, options, status};
use crate::{
    process::{Completion, Output},
    workspace::{copy_tree, root, tree_contents},
};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

const TOKEN: &str = "000102030405060708090a0b0c0d0e0f0200000000000000";
const SCHEMA: &str = "table=events\ngeneration=1\ncolumn_count=4\ncolumn[0]=id:int64:required\ncolumn[1]=value:double:nullable\ncolumn[2]=label:string:nullable\ncolumn[3]=day:date:required\nstatus=inspected\n";

// The logical source has four declared columns, identified from c1 through c4.
// Selecting nullable value / 0 as bad computes c5 from the second column, c2.
const EXPLAIN: &str = concat!(
    "logical plan\n",
    "r0 = source occurrence=0 columns=[c1, c2, c3, c4]\n",
    "r1 = select input=r0 columns=[c5]\n",
    "c5 = numeric [c2, 0, divide] input=r0 type=DOUBLE nullable\n",
    "result = r1\n",
    "output[0] = c5 name=Some(\"bad\") type=DOUBLE nullable\n",
    "status=explained\n",
);

pub(super) struct Inputs {
    empty: PathBuf,
    loaded: PathBuf,
    history: PathBuf,
    declared_empty: PathBuf,
    pub(super) declared: PathBuf,
    input: PathBuf,
    schema: PathBuf,
    explain: PathBuf,
}

pub(super) fn command(verb: &str, database: &Path) -> Vec<OsString> {
    let mut args = vec![verb.into()];
    args.extend(options(database));
    args
}

pub(super) fn add(args: &mut Vec<OsString>, flag: &str, value: impl Into<OsString>) {
    args.extend([flag.into(), value.into()]);
}

pub(super) fn stock(campaign: &mut Campaign, args: &[OsString]) -> Result<Output> {
    let output = campaign.execute(args, None, None)?;
    status(&output, 0)?;
    Ok(output)
}

fn database_header(database: &Path, phase: &str) -> String {
    format!(
        "status={phase}\ndatabase={}\nmemory_limit_bytes=2000000\ntemp_limit_bytes=1000000\n",
        database.display()
    )
}

fn stock_database(campaign: &mut Campaign, verb: &str, database: &Path) -> Result<Output> {
    let output = stock(campaign, &command(verb, database))?;
    database_report(&output, &output.stdout.bytes, database, verb)?;
    Ok(output)
}

// Load and declaration fixtures start at generation 0, including retries after
// an aborted attempt. The attempt number does not determine generation 1.
fn publication_report<'a>(
    output: &Output,
    body: &'a [u8],
    database: &Path,
    verb: &str,
    expected_stderr: &[u8],
) -> Result<&'a str> {
    if verb == "import" {
        return import_completion(output, body, expected_stderr);
    }
    let phase = match verb {
        "load" => "loaded",
        "declare" => "declared",
        _ => return Err("unknown CLI publication fixture".into()),
    };
    status(output, 0)?;
    let prefix = format!(
        "{}generation=1\ntransaction=",
        database_header(database, phase)
    );
    let receipt = std::str::from_utf8(body)?
        .strip_prefix(&prefix)
        .and_then(|tail| tail.strip_suffix('\n'))
        .ok_or("successful publication header differs")?;
    if !valid_token(receipt) || output.stderr.bytes != expected_stderr {
        return Err("complete successful publication report differs".into());
    }
    Ok(receipt)
}

// Both transfer fixtures declare their table at generation 1 before importing.
// The caller removes allocation census output or supplies the exact native
// observer diagnostic. Neither belongs to the successful CLI import report.
pub(super) fn import_completion<'a>(
    output: &Output,
    body: &'a [u8],
    expected_stderr: &[u8],
) -> Result<&'a str> {
    status(output, 0)?;
    if output.stderr.bytes != expected_stderr {
        return Err("successful import diagnostic differs".into());
    }
    let (token, tail) = std::str::from_utf8(body)?
        .strip_prefix("transaction=")
        .and_then(|text| text.split_once('\n'))
        .ok_or("successful import lacks its initial receipt")?;
    if !valid_token(token) || tail != "status=imported\ngeneration=2\n" {
        return Err("complete successful import report differs".into());
    }
    Ok(token)
}

// Each fixture supplies its committed generation independently of the token's
// attempt number. The caller removes any validated allocation census from body.
pub(super) fn resolution(
    output: &Output,
    body: &[u8],
    database: &Path,
    token: &str,
    limits: [u64; 2],
    generation: u64,
) -> Result<bool> {
    status(output, 0)?;
    let header = format!(
        "status=resolved\ndatabase={}\nmemory_limit_bytes={}\ntemp_limit_bytes={}\ntransaction={token}\n",
        database.display(),
        limits[0],
        limits[1],
    );
    let tail = std::str::from_utf8(body)?
        .strip_prefix(&header)
        .ok_or("healthy resolution header differs")?;
    if !output.stderr.bytes.is_empty() {
        return Err("healthy resolution printed stderr".into());
    }
    if tail == "resolution=aborted\n" {
        Ok(false)
    } else if tail == format!("resolution=durable\ngeneration={generation}\n") {
        Ok(true)
    } else {
        Err("healthy resolution outcome differs".into())
    }
}

fn not_found(output: &Output, body: &[u8]) -> Result<()> {
    status(output, 1)?;
    if !body.is_empty()
        || output.stderr.bytes
            != b"database error: requested database, table or transaction was not found\n"
    {
        return Err("missing object did not preserve its exact failure".into());
    }
    Ok(())
}

// These allocation fixtures use the default limits and retain one publication
// at generation 1. A failure can occur after some or all report bytes were sent.
fn failed_known_resolution(
    output: &Output,
    body: &[u8],
    database: &Path,
    receipt: &str,
    durable: bool,
) -> Result<()> {
    if status(output, 1).is_err() {
        status(output, 2)?;
    }
    let outcome = if durable {
        "resolution=durable\ngeneration=1\n"
    } else {
        "resolution=aborted\n"
    };
    let expected = format!(
        "{}transaction={receipt}\n{outcome}",
        database_header(database, "resolved")
    );
    if !expected.as_bytes().starts_with(body) {
        return Err("failed resolution differs from its known report prefix".into());
    }
    Ok(())
}

fn resolve_receipt(
    campaign: &mut Campaign,
    database: &Path,
    token: &str,
    generation: u64,
) -> Result<bool> {
    if !valid_token(token) {
        return Err("invalid transaction token".into());
    }
    let mut args = command("resolve", database);
    add(&mut args, "--transaction", token);
    let output = stock(campaign, &args)?;
    resolution(
        &output,
        &output.stdout.bytes,
        database,
        token,
        [2_000_000, 1_000_000],
        generation,
    )
}

impl Inputs {
    pub(super) fn create(campaign: &mut Campaign) -> Result<Self> {
        let work = campaign.run.directory.join("inputs");
        fs::create_dir(&work)?;
        let input = work.join("input.tbl");
        fs::write(
            &input,
            b"1|2|3|4|1|100|0.08|8|R|F|1994-01-01|12|13|14|15|16|\n",
        )?;
        let empty = work.join("empty");
        stock_database(campaign, "create", &empty)?;
        let loaded = work.join("loaded");
        copy_tree(&empty, &loaded)?;
        let mut load = command("load", &loaded);
        add(&mut load, "--input", &input);
        let loaded_report = stock(campaign, &load)?;
        let receipt = publication_report(
            &loaded_report,
            &loaded_report.stdout.bytes,
            &loaded,
            "load",
            b"",
        )?;
        if !resolve_receipt(campaign, &loaded, receipt, 1)? {
            return Err("fixture load did not become durable".into());
        }
        let history = work.join("history");
        fs::create_dir_all(history.join("units"))?;
        fs::create_dir(history.join("private"))?;
        fs::write(history.join("LOCK"), b"")?;
        let stored = root()?.join("test/data/current-single-table-format");
        for name in ["CONTROL", "ROOT.A", "ROOT.B", "WAL"] {
            // Frozen inputs are read-only. Working database files must permit
            // recovery; write their exact bytes into newly owned files.
            fs::write(history.join(name), fs::read(stored.join(name))?)?;
        }
        fs::write(
            history.join("units/0000000000000001.unit"),
            fs::read(stored.join("UNIT"))?,
        )?;
        let schema = work.join("events.schema");
        fs::write(
            &schema,
            "table events\nid int64 required\nvalue double nullable\nlabel string nullable\nday date required\n",
        )?;
        let declared_empty = work.join("declared-empty");
        stock_database(campaign, "create-declared", &declared_empty)?;
        let declared = work.join("declared");
        copy_tree(&declared_empty, &declared)?;
        let mut declare = command("declare", &declared);
        add(&mut declare, "--schema-file", &schema);
        let declared_report = stock(campaign, &declare)?;
        let receipt = publication_report(
            &declared_report,
            &declared_report.stdout.bytes,
            &declared,
            "declare",
            b"",
        )?;
        if !resolve_receipt(campaign, &declared, receipt, 1)? {
            return Err("fixture declaration did not become durable".into());
        }
        let explain = work.join("explain.sql");
        fs::write(&explain, "FROM events |> SELECT value / 0 AS bad")?;
        Ok(Self {
            empty,
            loaded,
            history,
            declared_empty,
            declared,
            input,
            schema,
            explain,
        })
    }

    fn args(&self, operation: &str, database: &Path) -> Result<Vec<OsString>> {
        let verb = if operation.starts_with("resolve-") {
            "resolve"
        } else if operation.starts_with('q') {
            "query"
        } else if operation == "schema-missing" {
            "schema"
        } else {
            operation
        };
        let mut args = command(verb, database);
        match operation {
            "load" => add(&mut args, "--input", &self.input),
            "declare" => add(&mut args, "--schema-file", &self.schema),
            "schema" | "schema-missing" => add(
                &mut args,
                "--table",
                if operation == "schema" {
                    "events"
                } else {
                    "missing"
                },
            ),
            "explain" => add(&mut args, "--query-file", &self.explain),
            "q1" | "q6" => add(
                &mut args,
                "--query-file",
                root()?.join(if operation == "q1" {
                    "test/data/upstream/q1-upstream.pipe.sql"
                } else {
                    "test/data/q6.pipe.sql"
                }),
            ),
            operation if operation.starts_with("resolve-") => {
                add(&mut args, "--transaction", token(operation))
            }
            _ => (),
        }
        Ok(args)
    }
}

fn token(operation: &str) -> String {
    match operation {
        "resolve-aborted" => format!("{}0100000000000000", &TOKEN[..32]),
        "resolve-unknown" => format!("{}0300000000000000", &TOKEN[..32]),
        _ => TOKEN.to_owned(),
    }
}

pub(super) fn line<'a>(text: &'a str, prefix: &str) -> Result<&'a str> {
    let mut matches = text.lines().filter_map(|line| line.strip_prefix(prefix));
    let value = matches.next().ok_or("missing CLI outcome field")?;
    if matches.next().is_some() {
        return Err("duplicate CLI outcome field".into());
    }
    Ok(value)
}

pub(super) fn valid_token(token: &str) -> bool {
    token.len() == 48
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn heal(
    campaign: &mut Campaign,
    database: &Path,
    retry: &[OsString],
    result: &Output,
    verb: &str,
    generation: u64,
) -> Result<Option<&'static str>> {
    let text = std::str::from_utf8(&result.stderr.bytes)?;
    let Some((_, rest)) = text.split_once("commit outcome is ambiguous for transaction ") else {
        return Ok(None);
    };
    let token = rest
        .split_once(':')
        .ok_or("missing ambiguous token terminator")?
        .0;
    if resolve_receipt(campaign, database, token, generation)? {
        return Ok(Some("durable"));
    }
    let retried = stock(campaign, retry)?;
    let retry_token = publication_report(&retried, &retried.stdout.bytes, database, verb, b"")?;
    if !resolve_receipt(campaign, database, retry_token, generation)? {
        return Err("healthy publication retry did not become durable".into());
    }
    if resolve_receipt(campaign, database, token, generation)? {
        return Err("retry changed the original aborted outcome".into());
    }
    Ok(Some("aborted"))
}

// Inputs::create writes one row: quantity 1, price 100, discount 0.08, tax 8,
// keys R/F and date 1994-01-01. Both retained queries include it. Q1 therefore
// has discounted price 92 and charge 828; Q6 has revenue 8. The DOUBLE bits below
// are independent literal IEEE-754 answers, including the input discount's bits.
const Q1_ANSWER: &str = concat!(
    "column_count=10\n",
    "columns=l_returnflag:string:required|l_linestatus:string:required|",
    "sum_qty:double:nullable|sum_base_price:double:nullable|",
    "sum_disc_price:double:nullable|sum_charge:double:nullable|",
    "avg_qty:double:nullable|avg_price:double:nullable|",
    "avg_disc:double:nullable|count_order:int64:required\n",
    "row=string:52|string:46|double:1:3ff0000000000000|",
    "double:100:4059000000000000|double:92:4057000000000000|",
    "double:828:4089e00000000000|double:1:3ff0000000000000|",
    "double:100:4059000000000000|double:0.08:3fb47ae147ae147b|int64:1\n",
    "row_count=1\nstatus=queried\n",
);
const Q6_ANSWER: &str = concat!(
    "column_count=1\ncolumns=revenue:double:nullable\n",
    "row=double:8:4020000000000000\nrow_count=1\nstatus=queried\n",
);

fn database_report(output: &Output, body: &[u8], database: &Path, operation: &str) -> Result<()> {
    let (phase, answer) = match operation {
        "q1" => ("querying", Q1_ANSWER),
        "q6" => ("querying", Q6_ANSWER),
        "schema" => ("inspecting", SCHEMA),
        "explain" => ("explaining", EXPLAIN),
        "create" | "create-declared" => ("created", ""),
        "open" => ("opened", ""),
        _ => return Err("unknown CLI database report fixture".into()),
    };
    let expected = format!("{}{answer}", database_header(database, phase));
    if matches!(output.completion, Completion::Exited(status) if status.success()) {
        status(output, 0)?;
        if body != expected.as_bytes() || !output.stderr.bytes.is_empty() {
            return Err("complete CLI report differs".into());
        }
    } else {
        if status(output, 1).is_err() {
            status(output, 2)?;
        }
        // A close or flush error may follow the final marker. Correct bytes do
        // not turn that failed process into a successful command.
        if !expected.as_bytes().starts_with(body) {
            return Err("failed CLI report differs from an expected output prefix".into());
        }
    }
    Ok(())
}

fn cell(campaign: &mut Campaign, inputs: &Inputs, operation: &str, mode: &str) -> Result<Output> {
    let database = campaign.run.directory.join(format!("{operation}-{mode}"));
    if !matches!(operation, "create" | "create-declared") {
        let source = match operation {
            "declare" => &inputs.declared_empty,
            "schema" | "schema-missing" | "explain" => &inputs.declared,
            "load" => &inputs.empty,
            operation if operation.starts_with("resolve-") => &inputs.history,
            _ => &inputs.loaded,
        };
        copy_tree(source, &database)?;
    }
    let read_only = matches!(
        operation,
        "open" | "q1" | "q6" | "schema" | "schema-missing" | "explain"
    ) || operation.starts_with("resolve-");
    let before = if read_only {
        Some(tree_contents(&database)?)
    } else {
        None
    };
    if operation == "resolve-repair" {
        fs::remove_file(database.join("ROOT.B"))?;
    }
    let args = inputs.args(operation, &database)?;
    let result = campaign.execute(&args, Some(mode), None)?;
    let (_, refused) = census(&result)?;
    let text = std::str::from_utf8(&result.stdout.bytes)?;
    let body = text
        .rsplit_once("cli allocations=")
        .ok_or("missing census separator")?
        .0;
    let succeeded = matches!(result.completion, Completion::Exited(status) if status.success());
    if read_only && operation != "resolve-repair" && Some(tree_contents(&database)?) != before {
        return Err("read-only CLI operation changed database contents".into());
    }
    if matches!(
        operation,
        "q1" | "q6" | "schema" | "explain" | "create" | "create-declared" | "open"
    ) {
        database_report(&result, body.as_bytes(), &database, operation)?;
    }
    if operation == "schema-missing" {
        if !body.is_empty() {
            return Err("missing table printed a schema report".into());
        }
        if refused == 0 {
            not_found(&result, body.as_bytes())?;
        }
    }
    if operation.starts_with("resolve-") {
        let receipt = token(operation);
        let durable = operation != "resolve-aborted";
        // The retained legacy history has one publication by attempt 2;
        // attempt 1 is aborted and attempt 3 has never been issued.
        if succeeded
            && (operation == "resolve-unknown"
                || resolution(
                    &result,
                    body.as_bytes(),
                    &database,
                    &receipt,
                    [2_000_000, 1_000_000],
                    1,
                )? != durable)
        {
            return Err("receipt resolution differs from retained fixture".into());
        }
        if !succeeded && operation != "resolve-unknown" {
            failed_known_resolution(&result, body.as_bytes(), &database, &receipt, durable)?;
        }
        let healed = campaign.execute(&args, None, None)?;
        if operation == "resolve-unknown" {
            not_found(&healed, &healed.stdout.bytes)?;
            if !body.is_empty() {
                return Err("unknown receipt printed an outcome".into());
            }
            if refused == 0 {
                not_found(&result, body.as_bytes())?;
            }
        } else if resolution(
            &healed,
            &healed.stdout.bytes,
            &database,
            &receipt,
            [2_000_000, 1_000_000],
            1,
        )? != durable
        {
            return Err("healthy receipt repair changed its outcome".into());
        }
        if Some(tree_contents(&database)?) != before {
            return Err("receipt repair changed retained fixture".into());
        }
    }
    if matches!(operation, "load" | "declare") {
        if succeeded {
            let receipt = publication_report(&result, body.as_bytes(), &database, operation, b"")?;
            if !resolve_receipt(campaign, &database, receipt, 1)? {
                return Err("successful publication is not durable".into());
            }
        }
        heal(campaign, &database, &args, &result, operation, 1)?;
    }
    if operation == "declare" {
        stock_database(campaign, "open", &database)?;
        let mut inspect = command("schema", &database);
        add(&mut inspect, "--table", "events");
        let settled = campaign.execute(&inspect, None, None)?;
        if status(&settled, 0).is_ok() {
            database_report(&settled, &settled.stdout.bytes, &database, "schema")?;
        } else {
            not_found(&settled, &settled.stdout.bytes)?;
        }
        if succeeded {
            status(&settled, 0)?;
        }
    }
    Ok(result)
}

pub(super) fn run(campaign: &mut Campaign, selection: &str) -> Result<()> {
    let inputs = Inputs::create(campaign)?;
    let mut prefixes = 0;
    for operation in [
        "create",
        "create-declared",
        "declare",
        "schema",
        "schema-missing",
        "explain",
        "open",
        "load",
        "q1",
        "q6",
        "resolve-durable",
        "resolve-repair",
        "resolve-aborted",
        "resolve-unknown",
    ] {
        if selection == "receipts" && !operation.starts_with("resolve-") {
            continue;
        }
        let expected = if matches!(operation, "resolve-unknown" | "schema-missing") {
            1
        } else {
            0
        };
        let control = cell(campaign, &inputs, operation, "entry-control")?;
        status(&control, expected)?;
        let (count, refusals) = census(&control)?;
        if refusals != 0 {
            return Err("CLI census refused allocation".into());
        }
        for prefix in std::iter::once(count).chain(0..count) {
            let result = cell(
                campaign,
                &inputs,
                operation,
                &format!("entry-after-{prefix}"),
            )?;
            let (calls, refused) = census(&result)?;
            if (refused > 0) != (prefix < count) || (prefix < count && calls <= prefix) {
                return Err("CLI refusal missed its allocation prefix".into());
            }
            if refused == 0 {
                status(&result, expected)?;
            } else if status(&result, 1).is_err() {
                status(&result, 2)?;
            }
            prefixes += 1;
        }
        println!(
            "CLI {operation}: census={count}, every refusal prefix and full-prefix control passed"
        );
    }
    println!("CLI database operations: {prefixes} allocation prefixes passed");
    Ok(())
}

pub(super) fn streams(campaign: &mut Campaign) -> Result<()> {
    let inputs = Inputs::create(campaign)?;
    let database = campaign.run.directory.join("closed-output");
    let create = command("create", &database);
    status(
        &campaign.execute(&create, Some("entry-closed-stdout"), None)?,
        1,
    )?;
    if database.exists() {
        return Err("closed stdout allowed database creation".into());
    }
    status(
        &campaign.execute(&create, Some("entry-closed-stderr"), None)?,
        0,
    )?;
    let before = tree_contents(&database)?;
    status(
        &campaign.execute(&create, Some("entry-closed-stderr"), None)?,
        1,
    )?;
    if tree_contents(&database)? != before {
        return Err("closed stderr redirected diagnostics into database files".into());
    }
    fs::remove_dir_all(&database)?;

    // The pinned Rust runtime replaces inherited closed descriptors with
    // /dev/null. These stock runs distinguish startup repair from CLI handling
    // when the instrumented driver closes a descriptor after startup.
    status(
        &closed_before_start(campaign, &create, libc::STDOUT_FILENO)?,
        0,
    )?;
    stock_database(campaign, "open", &database)?;
    fs::remove_dir_all(&database)?;
    let result = closed_before_start(campaign, &create, libc::STDERR_FILENO)?;
    status(&result, 0)?;
    database_report(&result, &result.stdout.bytes, &database, "create")?;
    let before = tree_contents(&database)?;
    status(
        &closed_before_start(campaign, &create, libc::STDERR_FILENO)?,
        1,
    )?;
    if tree_contents(&database)? != before {
        return Err("closed inherited stderr changed existing database".into());
    }
    for args in [
        command("open", &inputs.loaded),
        inputs.args("resolve-durable", &inputs.history)?,
    ] {
        super::broken_output(campaign, &args)?;
    }
    println!(
        "CLI database streams: closed descriptors before/after startup and broken open/receipt output passed"
    );
    Ok(())
}

fn closed_before_start(
    campaign: &mut Campaign,
    args: &[OsString],
    descriptor: i32,
) -> Result<Output> {
    use std::{os::unix::process::CommandExt, time::Duration};
    let mut command = campaign.command(args, None, None);
    // SAFETY: the supervisor installs both output pipes before this callback.
    // close is synchronous and does not allocate or take a process-local lock.
    unsafe {
        command.pre_exec(move || {
            if libc::close(descriptor) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    campaign
        .run
        .command(&mut command, None, Duration::from_secs(20))
}

pub(super) fn publication(campaign: &mut Campaign) -> Result<()> {
    use std::time::Duration;
    let observer = campaign
        .run
        .build("pipesql-native", "--lib", "pipesql_native")?;
    let inputs = Inputs::create(campaign)?;
    let csv = campaign.run.directory.join("events.csv");
    fs::write(
        &csv,
        b"id,value,label,day\n1,1.5,one,1970-01-01\n2,\\N,\\N,1970-01-02\n3,-0,\"\",1969-12-31\n",
    )?;
    for (verb, seed, generation) in [
        ("load", &inputs.empty, 1),
        ("declare", &inputs.declared_empty, 1),
        ("import", &inputs.declared, 2),
    ] {
        for cut in [0, 3, 4] {
            let database = campaign.run.directory.join(format!("{verb}-rename-{cut}"));
            copy_tree(seed, &database)?;
            let mut args = inputs.args(verb, &database)?;
            if verb == "import" {
                add(&mut args, "--table", "events");
                add(&mut args, "--input", &csv);
                for (flag, value) in [
                    ("--input-limit-bytes", "10000"),
                    ("--row-limit", "10"),
                    ("--record-limit-bytes", "1024"),
                    ("--field-limit-bytes", "128"),
                    ("--batch-rows", "2"),
                    ("--batch-text-bytes", "1024"),
                    ("--batch-limit", "4"),
                    ("--encoded-limit-bytes", "100000"),
                ] {
                    add(&mut args, flag, value);
                }
            }
            let mut command = campaign.command(&args, Some(&format!("publication-{cut}")), None);
            command
                .env_remove("LD_PRELOAD")
                .env_remove("DYLD_INSERT_LIBRARIES");
            command.env(
                if cfg!(target_os = "macos") {
                    "DYLD_INSERT_LIBRARIES"
                } else {
                    "LD_PRELOAD"
                },
                &observer,
            );
            let result = campaign
                .run
                .command(&mut command, None, Duration::from_secs(20))?;
            status(&result, if cut == 0 { 0 } else { 1 })?;
            let errors = std::str::from_utf8(&result.stderr.bytes)?;
            if line(errors, "observed_renames=")?
                != if cut == 0 {
                    "4"
                } else if cut == 3 {
                    "3"
                } else {
                    "4"
                }
            {
                return Err("publication observer missed a rename".into());
            }
            if cut == 0 {
                let receipt = publication_report(
                    &result,
                    &result.stdout.bytes,
                    &database,
                    verb,
                    b"observed_renames=4\n",
                )?;
                if !resolve_receipt(campaign, &database, receipt, generation)? {
                    return Err("successful native publication did not become durable".into());
                }
            }
            let settled = heal(campaign, &database, &args, &result, verb, generation)?;
            let expected = match cut {
                3 => Some("aborted"),
                4 => Some("durable"),
                _ => None,
            };
            if settled != expected {
                return Err("CLI publication outcome differs from native cut".into());
            }
            println!("CLI {verb}: rename refusal={cut}, outcome={settled:?} passed");
        }
    }
    // The fixture must not silently run healthy when its observer is absent.
    let args = command("open", &inputs.empty);
    let before = tree_contents(&inputs.empty)?;
    let mut missing = campaign.command(&args, Some("publication-0"), None);
    missing
        .env_remove("LD_PRELOAD")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env("RUST_BACKTRACE", "0");
    let result = campaign
        .run
        .command(&mut missing, None, Duration::from_secs(20))?;
    status(&result, 101)?;
    if !std::str::from_utf8(&result.stderr.bytes)?.contains("native publication observer missing")
        || before != tree_contents(&inputs.empty)?
    {
        return Err("missing publication observer control failed".into());
    }
    println!("CLI publication: nine native rename cases and missing-observer control passed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Capture;
    use std::os::unix::process::ExitStatusExt;

    fn output(text: &str) -> Output {
        Output {
            completion: Completion::Exited(std::process::ExitStatus::from_raw(0)),
            stdout: Capture {
                bytes: text.as_bytes().to_vec(),
                omitted: 0,
            },
            stderr: Capture::default(),
        }
    }

    #[test]
    fn analytical_answers_require_complete_typed_results_and_success() {
        let path = Path::new("/analytical");
        for (operation, answer, decimal, bits) in [
            ("q1", Q1_ANSWER, "double:828:", "4089e00000000000"),
            ("q6", Q6_ANSWER, "double:8:", "4020000000000000"),
        ] {
            let text = format!(
                "status=querying\ndatabase=/analytical\nmemory_limit_bytes=2000000\ntemp_limit_bytes=1000000\n{answer}"
            );
            let valid = output(&text);
            database_report(&valid, &valid.stdout.bytes, path, operation).unwrap();
            for bad in [
                String::new(),
                text.replace(decimal, "double:9:"),
                text.replace(bits, "0000000000000000"),
                text.replacen(":double:", ":int64:", 1),
                text.replacen(":nullable", ":required", 1),
                text.replace("database=/analytical", "database=/wrong"),
                text.replace("memory_limit_bytes=2000000", "memory_limit_bytes=1"),
                text.replace("row_count=1", "row_count=0"),
                text.replace("status=queried\n", ""),
                format!("{text}extra\n"),
                text.trim_end().to_owned(),
            ] {
                let bad = output(&bad);
                assert!(database_report(&bad, &bad.stdout.bytes, path, operation).is_err());
            }
            for fault in 0..5 {
                let mut bad = output(&text);
                match fault {
                    0 => bad.stdout.omitted = 1,
                    1 => bad.stderr.omitted = 1,
                    2 => bad.stderr.bytes = b"unexpected diagnostic\n".to_vec(),
                    3 => bad.completion = Completion::TimedOut,
                    4 => bad.completion = Completion::Exited(std::process::ExitStatus::from_raw(9)),
                    _ => unreachable!(),
                }
                assert!(database_report(&bad, &bad.stdout.bytes, path, operation).is_err());
            }
            for code in [1, 2] {
                for end in [0, 1, text.len() / 2, text.len()] {
                    let mut failed = output(&text[..end]);
                    failed.completion =
                        Completion::Exited(std::process::ExitStatus::from_raw(code << 8));
                    database_report(&failed, &failed.stdout.bytes, path, operation).unwrap();
                    // Accepted failure bytes still fail the ordinary success gate.
                    assert!(status(&failed, 0).is_err());
                }
                let mut wrong = output(&text.replace(decimal, "double:9:"));
                wrong.completion =
                    Completion::Exited(std::process::ExitStatus::from_raw(code << 8));
                assert!(database_report(&wrong, &wrong.stdout.bytes, path, operation).is_err());
            }
            let observed = output(&format!("{text}cli allocations=12 refusals=0\n"));
            database_report(
                &observed,
                super::super::payload(&observed).unwrap(),
                path,
                operation,
            )
            .unwrap();
            assert!(database_report(&observed, &observed.stdout.bytes, path, operation).is_err());
        }
        assert!(database_report(&output(""), b"", path, "other").is_err());
    }

    #[test]
    fn inspection_reports_require_complete_schema_and_plan() {
        let path = Path::new("/inspection");
        for (operation, phase, answer) in [
            ("schema", "inspecting", SCHEMA),
            ("explain", "explaining", EXPLAIN),
        ] {
            let text = format!(
                "status={phase}\ndatabase=/inspection\nmemory_limit_bytes=2000000\ntemp_limit_bytes=1000000\n{answer}"
            );
            let valid = output(&text);
            database_report(&valid, &valid.stdout.bytes, path, operation).unwrap();
            let mut wrong = vec![
                String::new(),
                answer.to_owned(),
                text.replace("database=/inspection", "database=/other"),
                text.replace("temp_limit_bytes=1000000", "temp_limit_bytes=1"),
                text.replace(&format!("status={phase}"), "status=querying"),
                text.trim_end().to_owned(),
                format!("{text}extra\n"),
            ];
            if operation == "schema" {
                wrong.extend([
                    text.replace("generation=1", "generation=2"),
                    text.replace("table=events", "table=other"),
                    text.replace("column_count=4", "column_count=3"),
                    text.replace("id:int64:required", "id:double:nullable"),
                    text.replace("column[3]=day:date:required\n", ""),
                    text.replace("status=inspected\n", ""),
                ]);
            } else {
                wrong.extend([
                    text.replace("occurrence=0", "occurrence=1"),
                    text.replace("[c2, 0, divide]", "[c2, 1, multiply]"),
                    text.replace("type=DOUBLE nullable", "type=INT64 required"),
                    text.replace("result = r1", "result = r0"),
                    text.replace("name=Some(\"bad\")", "name=Some(\"other\")"),
                    text.replace("r1 = select input=r0 columns=[c5]\n", ""),
                    text.replace("status=explained\n", ""),
                ]);
            }
            for text in wrong {
                let mut bad = output(&text);
                assert!(database_report(&bad, &bad.stdout.bytes, path, operation).is_err());
                bad.completion = Completion::Exited(std::process::ExitStatus::from_raw(256));
                if !valid.stdout.bytes.starts_with(&bad.stdout.bytes) {
                    assert!(database_report(&bad, &bad.stdout.bytes, path, operation).is_err());
                }
            }
        }
    }

    #[test]
    fn lifecycle_reports_require_complete_headers_and_receipts() {
        let path = Path::new("/lifecycle");
        for (verb, phase) in [
            ("create", "created"),
            ("create-declared", "created"),
            ("open", "opened"),
        ] {
            let text = format!(
                "status={phase}\ndatabase=/lifecycle\nmemory_limit_bytes=2000000\ntemp_limit_bytes=1000000\n"
            );
            let valid = output(&text);
            database_report(&valid, &valid.stdout.bytes, path, verb).unwrap();
            for wrong in [
                text.replace(phase, "wrong"),
                text.replace("database=/lifecycle", "database=/other"),
                text.replace("memory_limit_bytes=2000000", "memory_limit_bytes=1"),
                text.replace("temp_limit_bytes=1000000", "temp_limit_bytes=1"),
                text.trim_end().to_owned(),
                format!("{text}extra\n"),
            ] {
                let bad = output(&wrong);
                assert!(database_report(&bad, &bad.stdout.bytes, path, verb).is_err());
            }
        }
        for (verb, phase) in [("load", "loaded"), ("declare", "declared")] {
            let text = format!(
                "status={phase}\ndatabase=/lifecycle\nmemory_limit_bytes=2000000\ntemp_limit_bytes=1000000\ngeneration=1\ntransaction={TOKEN}\n"
            );
            let valid = output(&text);
            assert_eq!(
                publication_report(&valid, &valid.stdout.bytes, path, verb, b"").unwrap(),
                TOKEN
            );
            for wrong in [
                String::new(),
                text.replace(phase, "wrong"),
                text.replace("database=/lifecycle", "database=/other"),
                text.replace("memory_limit_bytes=2000000", "memory_limit_bytes=1"),
                text.replace("generation=1", "generation=2"),
                text.replace("generation=1\n", ""),
                text.replace(TOKEN, &TOKEN.to_uppercase()),
                text.replace(TOKEN, &TOKEN[..47]),
                text.replace(TOKEN, &format!("{TOKEN}0")),
                text.replace(TOKEN, &"x".repeat(48)),
                format!("{text}transaction={TOKEN}\n"),
                text.trim_end().to_owned(),
            ] {
                let bad = output(&wrong);
                assert!(publication_report(&bad, &bad.stdout.bytes, path, verb, b"").is_err());
            }
            for fault in 0..5 {
                let mut bad = output(&text);
                match fault {
                    0 => bad.stderr.bytes = b"unexpected diagnostic\n".to_vec(),
                    1 => {
                        bad.completion = Completion::Exited(std::process::ExitStatus::from_raw(256))
                    }
                    2 => bad.stdout.omitted = 1,
                    3 => bad.stderr.omitted = 1,
                    4 => bad.completion = Completion::TimedOut,
                    _ => unreachable!(),
                }
                assert!(publication_report(&bad, &bad.stdout.bytes, path, verb, b"").is_err());
            }
            let mut observed = output(&text);
            observed.stderr.bytes = b"observed_renames=4\n".to_vec();
            publication_report(
                &observed,
                &observed.stdout.bytes,
                path,
                verb,
                b"observed_renames=4\n",
            )
            .unwrap();
            assert!(
                publication_report(&observed, &observed.stdout.bytes, path, verb, b"").is_err()
            );
            let observed = output(&format!("{text}cli allocations=12 refusals=0\n"));
            publication_report(
                &observed,
                super::super::payload(&observed).unwrap(),
                path,
                verb,
                b"",
            )
            .unwrap();
            assert!(
                publication_report(&observed, &observed.stdout.bytes, path, verb, b"").is_err()
            );
        }
    }

    #[test]
    fn failed_known_receipts_preserve_only_their_expected_prefix() {
        let path = Path::new("/resolution");
        for durable in [false, true] {
            let outcome = if durable {
                "resolution=durable\ngeneration=1\n"
            } else {
                "resolution=aborted\n"
            };
            let text = format!(
                "status=resolved\ndatabase=/resolution\nmemory_limit_bytes=2000000\ntemp_limit_bytes=1000000\ntransaction={TOKEN}\n{outcome}"
            );
            for code in [1, 2] {
                for end in 0..=text.len() {
                    let mut failed = output(&text[..end]);
                    failed.completion =
                        Completion::Exited(std::process::ExitStatus::from_raw(code << 8));
                    failed_known_resolution(&failed, &failed.stdout.bytes, path, TOKEN, durable)
                        .unwrap();
                    assert!(status(&failed, 0).is_err());
                }
            }
            for wrong in [
                text.replace("status=resolved", "status=opened"),
                text.replace("database=/resolution", "database=/other"),
                text.replace("memory_limit_bytes=2000000", "memory_limit_bytes=1"),
                text.replace("temp_limit_bytes=1000000", "temp_limit_bytes=1"),
                text.replace(TOKEN, &"0".repeat(48)),
                text.replace(
                    outcome,
                    if durable {
                        "resolution=aborted\n"
                    } else {
                        "resolution=durable\ngeneration=1\n"
                    },
                ),
                if durable {
                    text.replace("generation=1", "generation=2")
                } else {
                    format!("{text}generation=1\n")
                },
                format!("{text}extra\n"),
            ] {
                let mut failed = output(&wrong);
                failed.completion = Completion::Exited(std::process::ExitStatus::from_raw(256));
                assert!(
                    failed_known_resolution(&failed, &failed.stdout.bytes, path, TOKEN, durable)
                        .is_err()
                );
            }
            for fault in 0..5 {
                let mut bad = output(&text);
                bad.completion = Completion::Exited(std::process::ExitStatus::from_raw(256));
                match fault {
                    0 => bad.stdout.omitted = 1,
                    1 => bad.stderr.omitted = 1,
                    2 => bad.completion = Completion::TimedOut,
                    3 => bad.completion = Completion::Exited(std::process::ExitStatus::from_raw(0)),
                    4 => bad.completion = Completion::Exited(std::process::ExitStatus::from_raw(9)),
                    _ => unreachable!(),
                }
                assert!(
                    failed_known_resolution(&bad, &bad.stdout.bytes, path, TOKEN, durable).is_err()
                );
            }
        }
    }

    #[test]
    fn receipt_generation_does_not_follow_its_attempt_number() {
        let path = Path::new("/history");
        let text = format!(
            "status=resolved\ndatabase=/history\nmemory_limit_bytes=2000000\ntemp_limit_bytes=1000000\ntransaction={TOKEN}\nresolution=durable\ngeneration=1\n"
        );
        let value = output(&text);
        assert!(
            resolution(
                &value,
                &value.stdout.bytes,
                path,
                TOKEN,
                [2_000_000, 1_000_000],
                1
            )
            .unwrap()
        );
        assert!(
            resolution(
                &value,
                &value.stdout.bytes,
                path,
                TOKEN,
                [2_000_000, 1_000_000],
                2
            )
            .is_err()
        );
        let wrong = output(&text.replace("generation=1", "generation=2"));
        assert!(
            resolution(
                &wrong,
                &wrong.stdout.bytes,
                path,
                TOKEN,
                [2_000_000, 1_000_000],
                1
            )
            .is_err()
        );
        let observed = output(&format!("{text}cli allocations=12 refusals=0\n"));
        let body = super::super::payload(&observed).unwrap();
        assert!(resolution(&observed, body, path, TOKEN, [2_000_000, 1_000_000], 1).unwrap());
        assert!(
            resolution(
                &observed,
                &observed.stdout.bytes,
                path,
                TOKEN,
                [2_000_000, 1_000_000],
                1
            )
            .is_err()
        );
    }

    #[test]
    fn missing_object_requires_ordinary_failure_and_no_report() {
        let diagnostic =
            b"database error: requested database, table or transaction was not found\n";
        let make = || {
            let mut value = output("");
            value.completion = Completion::Exited(std::process::ExitStatus::from_raw(256));
            value.stderr.bytes = diagnostic.to_vec();
            value
        };
        let valid = make();
        not_found(&valid, &valid.stdout.bytes).unwrap();
        let mut observed = make();
        observed.stdout.bytes = b"cli allocations=12 refusals=0\n".to_vec();
        not_found(&observed, super::super::payload(&observed).unwrap()).unwrap();
        assert!(not_found(&observed, &observed.stdout.bytes).is_err());
        for text in [
            "status=resolved\n",
            "transaction=anything\n",
            "resolution=aborted\n",
            "extra",
        ] {
            let mut bad = make();
            bad.stdout.bytes = text.as_bytes().to_vec();
            assert!(not_found(&bad, &bad.stdout.bytes).is_err());
        }
        for text in [
            b"".as_slice(),
            b"transaction was not found\n",
            &diagnostic[..diagnostic.len() - 1],
        ] {
            let mut bad = make();
            bad.stderr.bytes = text.to_vec();
            assert!(not_found(&bad, &bad.stdout.bytes).is_err());
        }
        for fault in 0..5 {
            let mut bad = make();
            match fault {
                0 => bad.stdout.omitted = 1,
                1 => bad.stderr.omitted = 1,
                2 => bad.completion = Completion::TimedOut,
                3 => bad.completion = Completion::Exited(std::process::ExitStatus::from_raw(0)),
                4 => bad.stderr.bytes.extend_from_slice(b"extra\n"),
                _ => unreachable!(),
            }
            assert!(not_found(&bad, &bad.stdout.bytes).is_err());
        }
    }

    #[test]
    fn successful_import_requires_one_canonical_receipt_and_complete_report() {
        let report = format!("transaction={TOKEN}\nstatus=imported\ngeneration=2\n");
        let healthy = output(&report);
        assert_eq!(
            import_completion(&healthy, &healthy.stdout.bytes, b"").unwrap(),
            TOKEN
        );
        let measured = output(&format!("{report}cli allocations=12 refusals=0\n"));
        let body = super::super::payload(&measured).unwrap();
        assert_eq!(import_completion(&measured, body, b"").unwrap(), TOKEN);
        assert!(import_completion(&measured, &measured.stdout.bytes, b"").is_err());
        let mut observed = output(&report);
        observed.stderr.bytes = b"observed_renames=4\n".to_vec();
        assert_eq!(
            import_completion(&observed, &observed.stdout.bytes, b"observed_renames=4\n").unwrap(),
            TOKEN
        );
        assert!(import_completion(&observed, &observed.stdout.bytes, b"").is_err());
        observed
            .stderr
            .bytes
            .extend_from_slice(b"unexpected error\n");
        assert!(
            import_completion(&observed, &observed.stdout.bytes, b"observed_renames=4\n").is_err()
        );
        for bad in [
            String::new(),
            format!("prefix\n{report}"),
            format!("{report}extra\n"),
            format!("{report}transaction={TOKEN}\n"),
            format!("{report}generation=3\n"),
            report.replace(TOKEN, &TOKEN.to_uppercase()),
            report.replace(TOKEN, &TOKEN[..47]),
            report.replace(TOKEN, &"z".repeat(48)),
            report.replace("status=imported\n", ""),
            report.replace("status=imported", "status=aborted"),
            report.replace("generation=2", "generation=3"),
            report.replace("generation=2", "generation=02"),
            report.replace("generation=2\n", ""),
            report.trim_end().to_owned(),
        ] {
            let bad = output(&bad);
            assert!(import_completion(&bad, &bad.stdout.bytes, b"").is_err());
        }
        for fault in 0..5 {
            let mut bad = output(&report);
            match fault {
                0 => bad.stdout.omitted = 1,
                1 => bad.stderr.omitted = 1,
                2 => bad.stderr.bytes = b"unexpected diagnostic\n".to_vec(),
                3 => bad.completion = Completion::TimedOut,
                4 => bad.completion = Completion::Exited(std::process::ExitStatus::from_raw(256)),
                _ => unreachable!(),
            }
            assert!(import_completion(&bad, &bad.stdout.bytes, b"").is_err());
        }
    }

    #[test]
    fn import_resolution_requires_complete_identity_generation_and_outcome() {
        let path = Path::new("/independent/import");
        for (limits, fields) in [
            (
                [2_000_000, 1_000_000],
                "memory_limit_bytes=2000000\ntemp_limit_bytes=1000000\n",
            ),
            (
                [8_000_000, 8_000_000],
                "memory_limit_bytes=8000000\ntemp_limit_bytes=8000000\n",
            ),
        ] {
            let check =
                |output: &Output| resolution(output, &output.stdout.bytes, path, TOKEN, limits, 2);
            let header = format!(
                "status=resolved\ndatabase=/independent/import\n{fields}transaction={TOKEN}\n"
            );
            let durable = format!("{header}resolution=durable\ngeneration=2\n");
            let aborted = format!("{header}resolution=aborted\n");
            assert!(check(&output(&durable)).unwrap());
            assert!(!check(&output(&aborted)).unwrap());
            for text in [&durable, &aborted] {
                for bad in [
                    text.replace("status=resolved", "status=imported"),
                    text.replace("/independent/import", "/another/import"),
                    text.replace("memory_limit_bytes=", "memory_limit_bytes=9"),
                    text.replace("temp_limit_bytes=", "temp_limit_bytes=9"),
                    text.replace(TOKEN, &"f".repeat(48)),
                    text.replace("transaction=", "transaction=0"),
                    format!("prefix\n{text}"),
                    format!("{text}extra\n"),
                    format!("{text}resolution=aborted\n"),
                    format!("{text}cli allocations=12 refusals=0\n"),
                    text.trim_end().to_owned(),
                ] {
                    assert!(check(&output(&bad)).is_err(), "{bad}");
                }
                for fault in 0..5 {
                    let mut bad = output(text);
                    match fault {
                        0 => bad.stdout.omitted = 1,
                        1 => bad.stderr.omitted = 1,
                        2 => bad.stderr.bytes = b"unexpected warning\n".to_vec(),
                        3 => bad.completion = Completion::TimedOut,
                        4 => {
                            bad.completion =
                                Completion::Exited(std::process::ExitStatus::from_raw(256))
                        }
                        _ => unreachable!(),
                    }
                    assert!(check(&bad).is_err());
                }
            }
            for bad in [
                durable.replace("generation=2\n", "generation=3\n"),
                durable.replace("generation=2\n", "generation=02\n"),
                durable.replace("generation=2\n", ""),
                format!("{aborted}generation=2\n"),
                format!("{header}resolution=unknown\n"),
            ] {
                assert!(check(&output(&bad)).is_err(), "{bad}");
            }
        }
    }

    #[test]
    fn receipt_fields_require_one_value_and_exact_token_bytes() {
        assert!(valid_token(TOKEN));
        for token in [
            TOKEN.to_uppercase(),
            TOKEN[..47].to_owned(),
            format!("{TOKEN}0"),
            "g".repeat(48),
            format!(" {TOKEN}"),
        ] {
            assert!(!valid_token(&token));
        }
        assert_eq!(
            line("resolution=aborted\ngeneration=1\n", "resolution=").unwrap(),
            "aborted"
        );
        assert!(line("generation=1\n", "resolution=").is_err());
        assert!(line("resolution=aborted\nresolution=durable\n", "resolution=").is_err());
    }
}
