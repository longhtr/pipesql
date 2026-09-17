//! Check CSV imports and JSON Lines exports through resource and stream failures.
//!
//! Imports retain early transaction tokens and resolve outcomes before checking
//! complete rows. Exports compare literal typed records, schema and completion,
//! including failures after a written prefix. Allocation sweeps use the successful
//! census to select each refusal point. Independent output mutations challenge the
//! checker, while stored-file comparisons catch unintended database changes during
//! read-only export or rejected operations.

use super::{
    Campaign, Result, census,
    database::{Inputs, add, command, line, stock, valid_token},
    payload, status,
};
use crate::{
    process::{Completion, Output},
    workspace::{copy_tree, tree_contents},
};
use std::{ffi::OsString, fs, path::Path};

fn import_args(database: &Path, source: &Path) -> Vec<OsString> {
    let mut args = command("import", database);
    add(&mut args, "--table", "events");
    add(&mut args, "--input", source);
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
    args
}

fn expected_export(durable: bool) -> Vec<serde_json::Value> {
    use serde_json::json;
    let mut records = vec![json!({"format":"pipesql-jsonl", "version":1, "columns":[
        {"name":"id", "type":"int64", "nullable":false},
        {"name":"value", "type":"double", "nullable":true},
        {"name":"label", "type":"string", "nullable":true},
        {"name":"day", "type":"date", "nullable":false}
    ]})];
    if durable {
        records.extend([
            json!({"row":["1", "3ff8000000000000", "one", "1970-01-01"]}),
            json!({"row":["2", null, null, "1970-01-02"]}),
            json!({"row":["3", "8000000000000000", "", "1969-12-31"]}),
        ]);
    }
    records.push(json!({"complete":true, "rows":if durable {3} else {0}}));
    records
}

fn export_args(database: &Path, query: &Path) -> Vec<OsString> {
    let mut args = command("export", database);
    let memory = args
        .iter()
        .position(|value| value == "--memory-limit-bytes")
        .unwrap();
    args[memory + 1] = "4000000".into();
    add(&mut args, "--query-file", query);
    add(&mut args, "--row-limit", "3");
    add(&mut args, "--output-limit-bytes", "10000");
    args
}

fn check_export(bytes: &[u8], durable: bool) -> Result<()> {
    let text = std::str::from_utf8(bytes)?;
    let records: Vec<serde_json::Value> = text
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    if records != expected_export(durable) {
        return Err("CSV import left incorrect typed rows or an incomplete export".into());
    }
    Ok(())
}

fn check_rows(campaign: &mut Campaign, database: &Path, query: &Path, durable: bool) -> Result<()> {
    let result = stock(campaign, &export_args(database, query))?;
    check_export(&result.stdout.bytes, durable)
}

fn import_cell(
    campaign: &mut Campaign,
    seed: &Path,
    source: &Path,
    query: &Path,
    mode: &str,
) -> Result<Output> {
    let database = campaign.run.directory.join(format!("import-{mode}"));
    copy_tree(seed, &database)?;
    let args = import_args(&database, source);
    let result = campaign.execute(&args, Some(mode), None)?;
    census(&result)?;
    stock(campaign, &command("open", &database))?;
    let text = std::str::from_utf8(&result.stdout.bytes)?;
    let receipt = if text.lines().any(|line| line.starts_with("transaction=")) {
        Some(line(text, "transaction=")?)
    } else {
        None
    };
    let mut resolve = command("resolve", &database);
    let durable = if let Some(token) = receipt {
        if !valid_token(token) {
            return Err("invalid CSV import receipt".into());
        }
        add(&mut resolve, "--transaction", token);
        let resolved = stock(campaign, &resolve)?;
        let resolved = std::str::from_utf8(&resolved.stdout.bytes)?;
        if line(resolved, "transaction=")? != token {
            return Err("CSV import resolved the wrong token".into());
        }
        match line(resolved, "resolution=")? {
            "durable" => true,
            "aborted" => false,
            _ => return Err("CSV receipt did not settle".into()),
        }
    } else {
        false
    };
    if matches!(result.completion, Completion::Exited(status) if status.success())
        && (!durable || !text.contains("status=imported\ngeneration=2\n"))
    {
        return Err("successful CSV import lacks a durable receipt".into());
    }
    check_rows(campaign, &database, query, durable)?;
    if !durable {
        stock(campaign, &args)?;
        check_rows(campaign, &database, query, true)?;
        if receipt.is_some() {
            let again = stock(campaign, &resolve)?;
            if line(std::str::from_utf8(&again.stdout.bytes)?, "resolution=")? != "aborted" {
                return Err("import retry changed the old aborted receipt".into());
            }
        }
    }
    Ok(result)
}

pub(super) fn import(campaign: &mut Campaign) -> Result<()> {
    let inputs = Inputs::create(campaign)?;
    let source = campaign.run.directory.join("events.csv");
    fs::write(
        &source,
        b"id,value,label,day\n1,1.5,one,1970-01-01\n2,\\N,\\N,1970-01-02\n3,-0,\"\",1969-12-31\n",
    )?;
    let query = campaign.run.directory.join("import-count.sql");
    fs::write(&query, "FROM events |> ORDER BY id")?;
    let control = import_cell(campaign, &inputs.declared, &source, &query, "entry-control")?;
    status(&control, 0)?;
    let (count, refused) = census(&control)?;
    if refused != 0 || count == 0 {
        return Err("invalid CLI import census".into());
    }
    for prefix in std::iter::once(count).chain(0..count) {
        let result = import_cell(
            campaign,
            &inputs.declared,
            &source,
            &query,
            &format!("entry-after-{prefix}"),
        )?;
        let (calls, refused) = census(&result)?;
        if (refused > 0) != (prefix < count) || (prefix < count && calls <= prefix) {
            return Err("CLI import refusal disagrees with census".into());
        }
        if prefix == count {
            status(&result, 0)?;
        } else if status(&result, 1).is_err() {
            status(&result, 2)?;
        }
    }
    let before = tree_contents(&inputs.declared)?;
    let args = import_args(&inputs.declared, Path::new("-"));
    let closed = campaign.execute(&args, Some("entry-closed-stdin"), None)?;
    status(&closed, 1)?;
    if !std::str::from_utf8(&closed.stderr.bytes)?.contains("capture CSV stdin")
        || tree_contents(&inputs.declared)? != before
    {
        return Err("closed CSV stdin changed database state or missed its diagnostic".into());
    }
    println!(
        "CLI CSV import: {} prefixes, complete rows, receipt resolution, retry and closed stdin passed",
        count + 1
    );
    Ok(())
}

fn incomplete(bytes: &[u8]) -> Result<()> {
    for line in bytes.split(|byte| *byte == b'\n') {
        if let Ok(record) = serde_json::from_slice::<serde_json::Value>(line)
            && record.get("complete") == Some(&serde_json::Value::Bool(true))
        {
            return Err("failed export reported completion".into());
        }
    }
    Ok(())
}

// Decode the actual exported values using standard JSON and IEEE conversion.
// Quoting every non-NULL CSV field preserves empty strings and literal NULL text.
fn export_csv(bytes: &[u8]) -> Result<String> {
    check_export(bytes, true)?;
    let records: Vec<serde_json::Value> = std::str::from_utf8(bytes)?
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    let mut csv = String::from("id,value,label,day\n");
    for record in &records[1..records.len() - 1] {
        for (column, value) in record["row"]
            .as_array()
            .ok_or("missing export row")?
            .iter()
            .enumerate()
        {
            if column != 0 {
                csv.push(',');
            }
            if value.is_null() {
                csv.push_str("\\N");
                continue;
            }
            let text = value.as_str().ok_or("non-text export value")?;
            let text = if column == 1 {
                let number = f64::from_bits(u64::from_str_radix(text, 16)?);
                if !number.is_finite() {
                    return Err("CSV round-trip requires finite DOUBLE values".into());
                }
                number.to_string()
            } else {
                text.to_owned()
            };
            csv.push('"');
            csv.push_str(&text.replace('"', "\"\""));
            csv.push('"');
        }
        csv.push('\n');
    }
    Ok(csv)
}

pub(super) fn export(campaign: &mut Campaign) -> Result<()> {
    let inputs = Inputs::create(campaign)?;
    let database = campaign.run.directory.join("export-db");
    copy_tree(&inputs.declared, &database)?;
    let source = campaign.run.directory.join("export-input.csv");
    fs::write(
        &source,
        b"id,value,label,day\n1,1.5,one,1970-01-01\n2,\\N,\\N,1970-01-02\n3,-0,\"\",1969-12-31\n",
    )?;
    stock(campaign, &import_args(&database, &source))?;
    let query = campaign.run.directory.join("export.sql");
    fs::write(&query, "FROM events |> ORDER BY id")?;
    let args = export_args(&database, &query);
    let before = tree_contents(&database)?;
    let control = campaign.execute(&args, Some("entry-control"), None)?;
    status(&control, 0)?;
    let complete = payload(&control)?;
    check_export(complete, true)?;
    let ordinary = stock(campaign, &args)?;
    if ordinary.stdout.bytes != complete {
        return Err("stock and instrumented exports differ".into());
    }
    let (count, refused) = census(&control)?;
    if count == 0 || refused != 0 {
        return Err("invalid export allocation census".into());
    }
    for prefix in std::iter::once(count).chain(0..count) {
        let result = campaign.execute(&args, Some(&format!("entry-after-{prefix}")), None)?;
        let (calls, refused) = census(&result)?;
        let payload = payload(&result)?;
        if prefix == count {
            status(&result, 0)?;
            if payload != complete || refused != 0 || calls != count {
                return Err("full allocation prefix changed export".into());
            }
        } else {
            if status(&result, 1).is_err() {
                status(&result, 2)?;
            }
            if refused == 0 || calls <= prefix {
                return Err("export allocation refusal was not observed".into());
            }
            incomplete(payload)?;
        }
        if tree_contents(&database)? != before {
            return Err(format!("export prefix {prefix} changed database files").into());
        }
    }
    for (flag, limit, succeeds) in [
        ("--output-limit-bytes", complete.len(), true),
        ("--output-limit-bytes", complete.len() - 1, false),
        ("--row-limit", 2, false),
    ] {
        let mut bounded = args.clone();
        let position = bounded.iter().position(|arg| arg == flag).unwrap();
        bounded[position + 1] = limit.to_string().into();
        let result = campaign.execute(&bounded, None, None)?;
        status(&result, if succeeds { 0 } else { 1 })?;
        if succeeds {
            if result.stdout.bytes != complete {
                return Err("exact output bound changed export".into());
            }
        } else {
            incomplete(&result.stdout.bytes)?;
        }
    }
    let restored = campaign.run.directory.join("export-restored");
    copy_tree(&inputs.declared, &restored)?;
    fs::write(&source, export_csv(complete)?)?;
    stock(campaign, &import_args(&restored, &source))?;
    let round_trip = stock(campaign, &export_args(&restored, &query))?;
    if round_trip.stdout.bytes != complete {
        return Err("JSONL to CSV round-trip changed typed values".into());
    }
    for (mode, code) in [("entry-closed-stdout", 1), ("entry-closed-stderr", 0)] {
        status(&campaign.execute(&args, Some(mode), None)?, code)?;
    }
    super::broken_output(campaign, &args)?;
    if tree_contents(&database)? != before {
        return Err("export limits or stream failures changed database files".into());
    }
    println!(
        "CLI JSONL export: {} prefixes, typed rows, exact limits, CSV round-trip and failed output streams passed",
        count + 1
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(records: &[serde_json::Value]) -> Vec<u8> {
        records
            .iter()
            .map(|record| format!("{record}\n"))
            .collect::<String>()
            .into_bytes()
    }

    #[test]
    fn failed_export_must_not_claim_completion() {
        incomplete(b"{\"row\":[\"1\"]}\n{\"ro").unwrap();
        for bytes in [
            b"{\"complete\":true,\"rows\":3}\n".as_slice(),
            b"{ \"rows\": 3, \"complete\": true }\n".as_slice(),
        ] {
            assert!(incomplete(bytes).is_err());
        }
    }

    #[test]
    fn csv_conversion_preserves_null_empty_and_negative_zero() {
        assert_eq!(
            export_csv(&encode(&expected_export(true))).unwrap(),
            "id,value,label,day\n\"1\",\"1.5\",\"one\",\"1970-01-01\"\n\"2\",\\N,\\N,\"1970-01-02\"\n\"3\",\"-0\",\"\",\"1969-12-31\"\n"
        );
    }

    #[test]
    fn import_rows_require_exact_types_bits_nulls_and_completion() {
        let records = expected_export(true);
        check_export(&encode(&records), true).unwrap();
        check_export(&encode(&expected_export(false)), false).unwrap();
        for (row, column, wrong) in [
            (1, 0, serde_json::json!(1)),
            (1, 1, serde_json::json!("3ff0000000000000")),
            (2, 2, serde_json::json!("")),
            (3, 1, serde_json::json!("0000000000000000")),
            (3, 3, serde_json::json!("1970-01-01")),
        ] {
            let mut altered = records.clone();
            altered[row]["row"][column] = wrong;
            assert!(check_export(&encode(&altered), true).is_err());
        }
        for omitted in 0..records.len() {
            let mut altered = records.clone();
            altered.remove(omitted);
            assert!(check_export(&encode(&altered), true).is_err());
        }
        assert!(check_export(&encode(&records), false).is_err());
    }
}
