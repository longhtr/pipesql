//! Compare parsing with refusal armed, then sweep native argument capture.
//!
//! These comparisons check allocation independence. Parser semantics have their
//! own literal expectations beside the production parser.
//!
//! The stock and observed executables receive the same native argument bytes,
//! including invalid UTF-8 and empty values. A healthy census determines the capture
//! prefixes to refuse; each run must report the corresponding allocation outcome
//! without changing the parser's output outside that refused capture.

use super::{Campaign, Result, census, options, status};
use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::Path};

const TOKEN: &str = "000102030405060708090a0b0c0d0e0f0200000000000000";

fn words(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn cases(work: &Path) -> Vec<Vec<OsString>> {
    let invalid = OsString::from_vec(vec![0xff]);
    let mut cases = vec![
        vec![],
        words(&["wrong"]),
        words(&["create"]),
        vec!["create".into(), invalid.clone(), "x".into()],
        words(&[""]),
        vec!["x".repeat(4097).into()],
    ];
    for flag in [
        "--database",
        "--input",
        "--query-file",
        "--schema-file",
        "--table",
        "--transaction",
        "--memory-limit-bytes",
        "--temp-limit-bytes",
        "--unknown",
        &"é".repeat(2048),
    ] {
        cases.push(words(&["create", flag]));
    }
    for operation in [
        "create",
        "create-declared",
        "declare",
        "schema",
        "open",
        "load",
        "query",
        "export",
        "explain",
        "resolve",
    ] {
        let mut base = words(&[operation]);
        base.extend(options(&work.join("absent")));
        let extra: &[&str] = match operation {
            "load" => &["--input", "input.tbl"],
            "query" | "explain" => &["--query-file", "query.sql"],
            "export" => &[
                "--query-file",
                "query.sql",
                "--row-limit",
                "10",
                "--output-limit-bytes",
                "10000",
            ],
            "declare" => &["--schema-file", "events.schema"],
            "schema" => &["--table", "events"],
            "resolve" => &["--transaction", TOKEN],
            _ => &[],
        };
        base.extend(words(extra));
        cases.push(base.clone());
        for (flag, value) in [
            ("--database", "/duplicate"),
            ("--input", "/input"),
            ("--query-file", "/query"),
            ("--schema-file", "/schema"),
            ("--table", "events"),
            ("--transaction", TOKEN),
            ("--memory-limit-bytes", "1"),
            ("--temp-limit-bytes", "1"),
            ("--output-limit-bytes", "10"),
            ("--row-limit", "10"),
            ("--unknown", "x"),
        ] {
            let mut duplicate = base.clone();
            duplicate.extend(words(&[flag, value]));
            cases.push(duplicate);
        }
    }
    let mut resolve = words(&["resolve"]);
    resolve.extend(options(&work.join("absent")));
    cases.push(resolve.clone());
    for token in [
        TOKEN.to_uppercase().into(),
        "".into(),
        TOKEN[..TOKEN.len() - 1].into(),
        format!("{TOKEN}0").into(),
        "g".repeat(48).into(),
        OsString::from_vec(vec![0xff; 48]),
        "0".repeat(48).into(),
        format!("{}01{}", "0".repeat(32), "0".repeat(14)).into(),
        format!("{}{}", &TOKEN[..32], "0".repeat(16)).into(),
        format!("0x{TOKEN}").into(),
        format!(" {TOKEN}").into(),
        format!("{TOKEN} ").into(),
    ] {
        let mut command = resolve.clone();
        command.push("--transaction".into());
        command.push(token);
        cases.push(command);
    }
    for flag in ["--memory-limit-bytes", "--temp-limit-bytes"] {
        for value in [
            "".into(),
            "-1".into(),
            "bad".into(),
            "18446744073709551616".into(),
            invalid.clone(),
            "x".repeat(4097).into(),
        ] {
            cases.push(vec!["create".into(), flag.into(), value]);
        }
        for value in ["0", "18446744073709551615"] {
            let mut command = words(&["create"]);
            command.extend(options(&work.join("absent")));
            let index = command.iter().position(|item| item == flag).unwrap();
            command[index + 1] = value.into();
            cases.push(command);
        }
    }
    let csv = words(&[
        "--table",
        "events",
        "--input",
        "events.csv",
        "--input-limit-bytes",
        "10000",
        "--row-limit",
        "10",
        "--record-limit-bytes",
        "1024",
        "--field-limit-bytes",
        "128",
        "--batch-rows",
        "2",
        "--batch-text-bytes",
        "1024",
        "--batch-limit",
        "4",
        "--encoded-limit-bytes",
        "100000",
    ]);
    let mut import = words(&["import"]);
    import.extend(options(&work.join("absent")));
    import.extend(csv.clone());
    cases.push(import.clone());
    for flag in csv.iter().skip(4).step_by(2) {
        let mut duplicate = import.clone();
        duplicate.extend([flag.clone(), "1".into()]);
        cases.push(duplicate);
        let mut zero = import.clone();
        let index = zero.iter().position(|item| item == flag).unwrap();
        zero[index + 1] = "0".into();
        cases.push(zero);
    }
    for (verb, source) in [
        (
            "import",
            words(&[
                "--format",
                "parquet",
                "--table",
                "facts",
                "--input",
                "input.parquet",
                "--input-limit-bytes",
                "100000",
                "--row-limit",
                "8",
                "--metadata-limit-bytes",
                "16384",
                "--row-group-limit",
                "8",
                "--row-group-rows",
                "3",
                "--row-group-limit-bytes",
                "70000",
                "--page-limit-bytes",
                "70000",
                "--batch-limit",
                "8",
                "--encoded-limit-bytes",
                "200000",
            ]),
        ),
        (
            "export",
            words(&[
                "--format",
                "parquet",
                "--query-file",
                "query.sql",
                "--row-limit",
                "8",
                "--output-limit-bytes",
                "100000",
                "--metadata-limit-bytes",
                "16384",
                "--row-group-limit",
                "8",
                "--row-group-rows",
                "3",
                "--row-group-text-bytes",
                "65536",
            ]),
        ),
    ] {
        let mut valid = words(&[verb]);
        valid.extend(options(&work.join("absent")));
        valid.extend(source.clone());
        cases.push(valid.clone());
        for pair in source.as_chunks::<2>().0 {
            let mut duplicate = valid.clone();
            duplicate.extend(pair.clone());
            cases.push(duplicate);
            let mut missing = valid.clone();
            let index = missing.iter().position(|item| item == &pair[0]).unwrap();
            missing.drain(index..index + 2);
            cases.push(missing);
        }
        for format in ["unknown", "csv", "jsonl"] {
            let mut wrong = valid.clone();
            let index = wrong.iter().position(|item| item == "--format").unwrap();
            wrong[index + 1] = format.into();
            cases.push(wrong);
        }
    }
    cases
}

pub(super) fn run(campaign: &mut Campaign) -> Result<()> {
    let cases = cases(&campaign.run.directory);
    for args in &cases {
        let control = campaign.execute(args, Some("parse-control"), None)?;
        let denied = campaign.execute(args, Some("parse-deny"), None)?;
        status(&control, 0)?;
        status(&denied, 0)?;
        if control.stdout.bytes != denied.stdout.bytes
            || control.stderr.bytes != denied.stderr.bytes
        {
            return Err("CLI parsing changed when allocations were refused".into());
        }
        let text = std::str::from_utf8(&denied.stdout.bytes)?;
        if text
            .lines()
            .filter(|line| *line == "cli allocations=0 refusals=0")
            .count()
            != 1
        {
            return Err("parser allocated or census was incomplete".into());
        }
        if !matches!(text.lines().last(), Some("parsed=true" | "parsed=false")) {
            return Err("parser did not complete".into());
        }
    }
    println!(
        "CLI parser: {} control/refusal pairs, zero allocations",
        cases.len()
    );
    let native = [
        (vec![], "".to_owned()),
        (words(&["", ""]), "".to_owned()),
        (words(&["create", "--database"]), "x".repeat(5000)),
        (
            vec!["create".into(), OsString::from_vec(vec![0xff]), "x".into()],
            "".to_owned(),
        ),
        (
            vec!["create".into(), "é".repeat(2048).into()],
            "".to_owned(),
        ),
        (vec!["x".repeat(4097).into()], "".to_owned()),
        (vec!["".into(); 11], "".to_owned()),
        (vec!["".into(); 34], "".to_owned()),
    ];
    let mut prefixes = 0;
    for (args, argv0) in native {
        let control = campaign.execute(&args, Some("entry-control"), Some(argv0.as_ref()))?;
        let stock = campaign.execute(&args, None, Some(argv0.as_ref()))?;
        status(&control, 2)?;
        status(&stock, 2)?;
        if control.stderr.bytes != stock.stderr.bytes {
            return Err("instrumented CLI capture differs from stock".into());
        }
        let (calls, refused) = census(&control)?;
        if refused != 0 {
            return Err("control refused an allocation".into());
        }
        for prefix in std::iter::once(calls).chain(0..calls) {
            let result = campaign.execute(
                &args,
                Some(&format!("entry-after-{prefix}")),
                Some(argv0.as_ref()),
            )?;
            status(&result, 2)?;
            let (_, refused) = census(&result)?;
            if (refused > 0) != (prefix < calls)
                || (prefix == calls && result.stderr.bytes != control.stderr.bytes)
            {
                return Err("CLI capture refusal disagrees with census".into());
            }
            prefixes += 1;
        }
    }
    println!("CLI native argument capture: {prefixes} allocation prefixes passed");
    Ok(())
}
