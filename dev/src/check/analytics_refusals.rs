//! Check byte-budget failures against independently specified output prefixes.
//!
//! JSONL expectations come from literal schema and input rows. The small Parquet
//! prefix contains three Compact Protocol DATA_PAGE headers and twelve INT64
//! values, including independently calculated IEEE CRC-32 values. Healthy output is
//! also imported and compared as typed JSONL; that round trip is not an external
//! interoperability claim.

use super::*;
use crate::process::Output;
use std::fmt::Write as _;

const PARQUET_OPTIONS: &[&str] = &[
    "--format",
    "parquet",
    "--metadata-limit-bytes",
    "262144",
    "--row-group-limit",
    "256",
    "--row-group-rows",
    "4",
    "--row-group-text-bytes",
    "8192",
];

const ID_SCHEMA: Schema = &[("id", "int64", false)];
const PAD: &str = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
const PAD_SCHEMA: Schema = &[
    ("id", "int64", false),
    ("a", "string", false),
    ("b", "string", false),
    ("c", "string", false),
    ("d", "string", false),
];
const JSON_HEADER: &str = "{\"format\":\"pipesql-jsonl\",\"version\":1,\"columns\":[{\"name\":\"id\",\"type\":\"int64\",\"nullable\":false},{\"name\":\"a\",\"type\":\"string\",\"nullable\":false},{\"name\":\"b\",\"type\":\"string\",\"nullable\":false},{\"name\":\"c\",\"type\":\"string\",\"nullable\":false},{\"name\":\"d\",\"type\":\"string\",\"nullable\":false}]}\n";
// Compact DATA_PAGE headers for required INT64 groups [0, 1, 2, 3],
// [4, 5, 6, 7] and [8, 9, 10, 11]. Sizes=32, rows=4, PLAIN values,
// RLE levels, two STOPs. IEEE CRC-32 values were calculated independently
// with Python zlib over the little-endian inputs: 9100120a, d9f7d402, 00ef9e1a.
const PAGE_HEADERS: [&[u8]; 3] = [
    &[
        21, 0, 21, 64, 21, 64, 21, 235, 183, 255, 239, 13, 28, 21, 8, 21, 0, 21, 6, 21, 6, 0, 0,
    ],
    &[
        21, 0, 21, 64, 21, 64, 21, 251, 175, 193, 224, 4, 28, 21, 8, 21, 0, 21, 6, 21, 6, 0, 0,
    ],
    &[
        21, 0, 21, 64, 21, 64, 21, 180, 248, 252, 14, 28, 21, 8, 21, 0, 21, 6, 21, 6, 0, 0,
    ],
];

fn verify(
    result: &Output,
    actual: &[u8],
    expected: &[u8],
    owner: &str,
    required: usize,
    limit: usize,
) -> Result<()> {
    let diagnostic = format!(
        "database error: resource refusal for {owner}: required={required}, limit={limit}\n"
    );
    if !matches!(result.completion, Completion::Exited(status) if status.code() == Some(1))
        || result.stdout.omitted != 0
        || result.stderr.omitted != 0
        || result.stderr.bytes != diagnostic.as_bytes()
        || actual.len() > limit
        || actual != expected
    {
        return Err("analytical export refusal or exact file prefix differs".into());
    }
    Ok(())
}

pub(super) fn run(workflow: &mut Workflow<'_>, db: &Path, work: &Path) -> Result<()> {
    // Keep refusal coverage on the full joined/windowed report as well as the
    // small streams below, whose late prefixes have literal independent answers.
    for (parquet, limit, required, owner) in [
        (false, 32, 49, "result export bytes"),
        (true, 3, 4, "Parquet output bytes"),
    ] {
        let path = work.join(format!("report-refused-{parquet}"));
        let mut args = strings(&[
            "--query-file",
            &workflow
                .run
                .root
                .join("examples/analytics.sql")
                .display()
                .to_string(),
            "--row-limit",
            "100",
            "--output-limit-bytes",
            &limit.to_string(),
        ]);
        if parquet {
            args.extend(strings(PARQUET_OPTIONS));
        }
        let result = workflow.command(db, "export", &args, 8_000_000, Some(&path), false)?;
        verify(&result, &fs::read(path)?, b"", owner, required, limit)?;
    }
    let json_query = work.join("refusal-json.sql");
    fs::write(
        &json_query,
        format!(
            "FROM events |> WHERE id < 16 |> SELECT id, '{PAD}' AS a, '{PAD}' AS b, '{PAD}' AS c, '{PAD}' AS d |> ORDER BY id"
        ),
    )?;
    let parquet_query = work.join("refusal-parquet.sql");
    fs::write(
        &parquet_query,
        "FROM events |> WHERE id < 16 |> SELECT id |> ORDER BY id",
    )?;
    let mut json_prefix = JSON_HEADER.to_owned();
    for id in 0..8 {
        writeln!(
            json_prefix,
            "{{\"row\":[\"{id}\",\"{PAD}\",\"{PAD}\",\"{PAD}\",\"{PAD}\"]}}"
        )?;
    }
    let late_limit = json_prefix.len();
    // The next row starts with eight bytes. Only full 1024-byte buffers have
    // reached the file; refusing it must discard the remaining buffered tail.
    let json_written = late_limit / 1024 * 1024;
    // CLI stdout also buffers bytes and drops its tail on failure. INT64 10
    // contains a newline byte, flushing all three complete groups. Refusing the
    // fourth header then leaves this precise file prefix, without a footer.
    let mut parquet_prefix = b"PAR1".to_vec();
    for (group, header) in PAGE_HEADERS.iter().enumerate() {
        parquet_prefix.extend_from_slice(header);
        for id in (group as i64 * 4)..(group as i64 * 4 + 4) {
            parquet_prefix.extend_from_slice(&id.to_le_bytes());
        }
    }
    for (label, parquet, limit, required, expected) in [
        ("json-early", false, 32, 49, &b""[..]),
        (
            "json-late",
            false,
            late_limit,
            late_limit + 8,
            &json_prefix.as_bytes()[..json_written],
        ),
        ("parquet-magic", true, 3, 4, &b""[..]),
        ("parquet-payload", true, 32, 59, &b""[..]),
        ("parquet-late", true, 168, 191, parquet_prefix.as_slice()),
    ] {
        let query = if parquet { &parquet_query } else { &json_query };
        let path = work.join(format!("{label}.refused"));
        let mut args = strings(&[
            "--query-file",
            &query.display().to_string(),
            "--row-limit",
            "16",
            "--output-limit-bytes",
            &limit.to_string(),
        ]);
        if parquet {
            args.extend(strings(PARQUET_OPTIONS));
        }
        let result = workflow.command(db, "export", &args, 8_000_000, Some(&path), false)?;
        verify(
            &result,
            &fs::read(&path)?,
            expected,
            if parquet {
                "Parquet output bytes"
            } else {
                "result export bytes"
            },
            required,
            limit,
        )?;
        // Reuse the identical query and group shape after each refusal.
        args[5] = "32000000".into();
        let healthy = work.join(format!("{label}.healthy"));
        workflow.command(db, "export", &args, 8_000_000, Some(&healthy), true)?;
        if parquet {
            let copy = work.join(format!("{label}-db"));
            workflow.command(&copy, "create-declared", &[], 8_000_000, None, true)?;
            let schema = work.join("refusal-ids.schema");
            fs::write(&schema, "table ids\nid int64 required\n")?;
            workflow.command(
                &copy,
                "declare",
                &strings(&["--schema-file", &schema.display().to_string()]),
                8_000_000,
                None,
                true,
            )?;
            workflow.import(&copy, "ids", &healthy, 16, true, None)?;
            let query = work.join("refusal-ids.sql");
            fs::write(&query, "FROM ids |> ORDER BY id")?;
            let decoded = work.join(format!("{label}.jsonl"));
            workflow.export(
                &copy,
                &query,
                &decoded,
                Export {
                    rows: 16,
                    parquet: false,
                    memory: 8_000_000,
                    bytes: 32_000_000,
                },
            )?;
            check_jsonl(
                &decoded,
                ID_SCHEMA,
                &(0..16)
                    .map(|id| json!([id.to_string()]))
                    .collect::<Vec<_>>(),
            )?;
        } else {
            check_jsonl(
                &healthy,
                PAD_SCHEMA,
                &(0..16)
                    .map(|id| json!([id.to_string(), PAD, PAD, PAD, PAD]))
                    .collect::<Vec<_>>(),
            )?;
        }
    }
    println!("analytical export refusals: 7 exact prefixes and 5 healthy retries passed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Capture;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn refusal_rejects_wrong_prefix_completion_diagnostic_and_outcome() {
        let mut prefix = b"PAR1".to_vec();
        prefix.extend_from_slice(PAGE_HEADERS[0]);
        for id in 0_i64..4 {
            prefix.extend_from_slice(&id.to_le_bytes());
        }
        let diagnostic =
            b"database error: resource refusal for Parquet output bytes: required=82, limit=59\n";
        let mut result = Output {
            completion: Completion::Exited(std::process::ExitStatus::from_raw(256)),
            stdout: Capture::default(),
            stderr: Capture {
                bytes: diagnostic.to_vec(),
                omitted: 0,
            },
        };
        let check = |result: &Output, actual: &[u8]| {
            verify(result, actual, &prefix, "Parquet output bytes", 82, 59)
        };
        check(&result, &prefix).unwrap();
        let mut wrong_value = prefix.clone();
        *wrong_value.last_mut().unwrap() = 1;
        let mut wrong_crc = prefix.clone();
        wrong_crc[12] ^= 1;
        for wrong in [
            Vec::new(),
            prefix[..prefix.len() - 1].to_vec(),
            wrong_value,
            wrong_crc,
            [prefix.as_slice(), b"PAR1"].concat(),
            [prefix.as_slice(), b"{\"complete\":true,\"rows\":1}\n"].concat(),
        ] {
            assert!(check(&result, &wrong).is_err());
        }
        for wrong in [
            String::from_utf8_lossy(diagnostic).replace("82", "83"),
            String::from_utf8_lossy(diagnostic).replace("59", "58"),
            String::from_utf8_lossy(diagnostic)
                .replace("Parquet output bytes", "result export bytes"),
            format!("{}extra\n", String::from_utf8_lossy(diagnostic)),
            String::new(),
        ] {
            result.stderr.bytes = wrong.into_bytes();
            assert!(check(&result, &prefix).is_err());
        }
        result.stderr.bytes = diagnostic.to_vec();
        for raw in [0, 512, 9] {
            result.completion = Completion::Exited(std::process::ExitStatus::from_raw(raw));
            assert!(check(&result, &prefix).is_err());
        }
        result.completion = Completion::TimedOut;
        assert!(check(&result, &prefix).is_err());
        result.completion = Completion::Exited(std::process::ExitStatus::from_raw(256));
        result.stdout.omitted = 1;
        assert!(check(&result, &prefix).is_err());
        result.stdout.omitted = 0;
        result.stderr.omitted = 1;
        assert!(check(&result, &prefix).is_err());
    }
}
