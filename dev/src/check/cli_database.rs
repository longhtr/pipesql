//! Check CLI database operations across allocation refusal and uncertain publication.
//!
//! Each operation uses controlled database copies and a successful allocation census
//! to select refusal prefixes. Stock CLI commands reopen or inspect the result and
//! resolve transaction tokens, distinguishing an aborted attempt from a durable
//! write despite an earlier error. Helpers require unambiguous output fields and
//! valid token syntax. File comparisons establish when a refusal must leave stored
//! state unchanged; child case modules reuse setup without deriving their answers
//! from the operation being tested.

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
        stock(campaign, &command("create", &empty))?;
        let loaded = work.join("loaded");
        copy_tree(&empty, &loaded)?;
        let mut load = command("load", &loaded);
        add(&mut load, "--input", &input);
        stock(campaign, &load)?;
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
        stock(campaign, &command("create-declared", &declared_empty))?;
        let declared = work.join("declared");
        copy_tree(&declared_empty, &declared)?;
        let mut declare = command("declare", &declared);
        add(&mut declare, "--schema-file", &schema);
        stock(campaign, &declare)?;
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
) -> Result<Option<&'static str>> {
    let text = std::str::from_utf8(&result.stderr.bytes)?;
    let Some((_, rest)) = text.split_once("commit outcome is ambiguous for transaction ") else {
        return Ok(None);
    };
    let token = rest
        .split_once(':')
        .ok_or("missing ambiguous token terminator")?
        .0;
    if !valid_token(token) {
        return Err("invalid ambiguous transaction token".into());
    }
    let mut resolve = command("resolve", database);
    add(&mut resolve, "--transaction", token);
    let resolved = stock(campaign, &resolve)?;
    let text = std::str::from_utf8(&resolved.stdout.bytes)?;
    if line(text, "transaction=")? != token {
        return Err("resolution returned a different token".into());
    }
    match line(text, "resolution=")? {
        "durable" => return Ok(Some("durable")),
        "aborted" => {
            stock(campaign, retry)?;
            let again = stock(campaign, &resolve)?;
            if line(std::str::from_utf8(&again.stdout.bytes)?, "resolution=")? != "aborted" {
                return Err("retry changed the original aborted outcome".into());
            }
        }
        _ => return Err("ambiguous token did not settle".into()),
    }
    Ok(Some("aborted"))
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
    census(&result)?;
    let text = std::str::from_utf8(&result.stdout.bytes)?;
    let body = text
        .rsplit_once("cli allocations=")
        .ok_or("missing census separator")?
        .0;
    let succeeded = matches!(result.completion, Completion::Exited(status) if status.success());
    if read_only && operation != "resolve-repair" && Some(tree_contents(&database)?) != before {
        return Err("read-only CLI operation changed database contents".into());
    }
    if operation.starts_with("resolve-") {
        if succeeded
            && (line(body, "transaction=")? != token(operation)
                || line(body, "resolution=")?
                    != if operation == "resolve-aborted" {
                        "aborted"
                    } else {
                        "durable"
                    })
        {
            return Err("receipt resolution differs from retained fixture".into());
        }
        let healed = campaign.execute(&args, None, None)?;
        status(&healed, if operation == "resolve-unknown" { 1 } else { 0 })?;
        if Some(tree_contents(&database)?) != before {
            return Err("receipt repair changed retained fixture".into());
        }
        if operation == "resolve-unknown"
            && (body.contains("resolution=")
                || healed
                    .stdout
                    .bytes
                    .windows(11)
                    .any(|bytes| bytes == b"resolution=")
                || !std::str::from_utf8(&healed.stderr.bytes)?
                    .contains("transaction was not found"))
        {
            return Err("unknown receipt acquired an outcome".into());
        }
    }
    if matches!(operation, "load" | "declare") {
        heal(campaign, &database, &args, &result)?;
    }
    if operation == "declare" {
        stock(campaign, &command("open", &database))?;
        let mut inspect = command("schema", &database);
        add(&mut inspect, "--table", "events");
        let settled = campaign.execute(&inspect, None, None)?;
        if status(&settled, 0).is_ok() {
            if !std::str::from_utf8(&settled.stdout.bytes)?.ends_with(SCHEMA) {
                return Err("settled declaration schema differs".into());
            }
        } else {
            status(&settled, 1)?;
            if !settled.stdout.bytes.is_empty()
                || !std::str::from_utf8(&settled.stderr.bytes)?.contains("was not found")
            {
                return Err("failed declaration left unexpected schema".into());
            }
        }
        if succeeded {
            status(&settled, 0)?;
            let token = line(body, "transaction=")?;
            if !valid_token(token) {
                return Err("invalid declaration token".into());
            }
            let mut resolve = command("resolve", &database);
            add(&mut resolve, "--transaction", token);
            let resolved = stock(campaign, &resolve)?;
            if !resolved
                .stdout
                .bytes
                .ends_with(b"resolution=durable\ngeneration=1\n")
            {
                return Err("successful declaration is not durable".into());
            }
        }
    }
    if succeeded && operation == "schema" && !body.ends_with(SCHEMA) {
        return Err("schema output differs".into());
    }
    if succeeded
        && operation == "explain"
        && (!body.contains("logical plan\n")
            || !body.ends_with("status=explained\n")
            || body.contains("row="))
    {
        return Err("explain did not produce a complete plan".into());
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
    stock(campaign, &command("open", &database))?;
    fs::remove_dir_all(&database)?;
    let result = closed_before_start(campaign, &create, libc::STDERR_FILENO)?;
    status(&result, 0)?;
    if !std::str::from_utf8(&result.stdout.bytes)?.contains("status=created") {
        return Err("closed inherited stderr lost creation output".into());
    }
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
    for (verb, seed) in [
        ("load", &inputs.empty),
        ("declare", &inputs.declared_empty),
        ("import", &inputs.declared),
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
            let settled = heal(campaign, &database, &args, &result)?;
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
