//! Check command parsing with explicit argument lists and expected command fields.
//!
//! Valid cases check operation selection, paths and resource limits. Invalid
//! cases cover missing values, duplicate or incompatible options, malformed
//! numbers and oversized arguments. Transaction tests compare decoded bytes with
//! a separately constructed array and reject malformed token representations.
//!
//! These tests call the parser directly. They neither capture native arguments
//! nor open a database, and cannot establish that a token was ever issued. CLI
//! process tests cover startup, output and exit behavior.

use super::{
    ExportFormat, ImportFormat, MAX_ARGUMENT_BYTES, Operation, check_argument, parse,
    parse_transaction,
};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

fn args(values: &[&str]) -> impl Iterator<Item = OsString> {
    values
        .iter()
        .map(|value| OsString::from(*value))
        .collect::<Vec<_>>()
        .into_iter()
}

#[test]
fn parser_accepts_complete_create_and_open() {
    for operation in ["create", "create-declared", "open"] {
        let command = parse(args(&[
            "pipesql",
            operation,
            "--database",
            "/tmp/example",
            "--memory-limit-bytes",
            "1048576",
            "--temp-limit-bytes",
            "2097152",
        ]))
        .expect("command");
        assert!(matches!(
            (operation, &command.operation),
            ("create", Operation::Create)
                | ("create-declared", Operation::CreateDeclared)
                | ("open", Operation::Open)
        ));
        assert_eq!(command.database, PathBuf::from("/tmp/example"));
        assert_eq!(command.config.memory_limit_bytes(), 1_048_576);
        assert_eq!(command.config.temp_limit_bytes(), 2_097_152);
    }
}

#[test]
fn parser_accepts_load_only_with_input() {
    let command = parse(args(&[
        "pipesql",
        "load",
        "--database",
        "/tmp/example",
        "--input",
        "/tmp/lineitem.tbl",
        "--memory-limit-bytes",
        "2000000",
        "--temp-limit-bytes",
        "1000000",
    ]))
    .expect("load command");
    assert!(
        matches!(command.operation, Operation::Load(input) if input == Path::new("/tmp/lineitem.tbl"))
    );
}

#[test]
fn parser_accepts_query_only_with_query_file() {
    for operation in ["query", "explain"] {
        let command = parse(args(&[
            "pipesql",
            operation,
            "--database",
            "/tmp/example",
            "--query-file",
            "/tmp/q6.pipe.sql",
            "--memory-limit-bytes",
            "2000000",
            "--temp-limit-bytes",
            "1000000",
        ]))
        .expect("query command");
        assert!(
            matches!((operation, command.operation), ("query", Operation::Query(query_file)) | ("explain", Operation::Explain(query_file)) if query_file == Path::new("/tmp/q6.pipe.sql"))
        );
    }
}

#[test]
fn declaration_requires_its_own_source_option() {
    let common = [
        "--database",
        "/tmp/example",
        "--memory-limit-bytes",
        "2000000",
        "--temp-limit-bytes",
        "1000000",
    ];
    for (operation, options, accepted) in [
        ("declare", vec!["--schema-file", "/tmp/events.schema"], true),
        ("declare", vec![], false),
        ("declare", vec!["--query-file", "/tmp/events.schema"], false),
        (
            "declare",
            vec!["--schema-file", "/tmp/a", "--schema-file", "/tmp/b"],
            false,
        ),
        ("query", vec!["--schema-file", "/tmp/events.schema"], false),
    ] {
        let parsed = parse(
            args(&["pipesql", operation])
                .chain(args(&common))
                .chain(args(&options)),
        );
        assert_eq!(parsed.is_ok(), accepted, "{operation} {options:?}");
        if let Ok(command) = parsed {
            assert!(
                matches!(command.operation, Operation::Declare(path) if path == Path::new("/tmp/events.schema"))
            );
        }
    }
}

#[test]
fn transaction_arguments_preserve_bytes_and_reject_invalid_tokens() {
    use std::os::unix::ffi::OsStringExt;
    let text = "000102030405060708090a0b0c0d0e0f0100000000000000";
    // Build the expected bytes without hex decoding, so a decoder mistake
    // cannot be repeated in the expected result.
    let mut expected = [0; 24];
    for (index, byte) in expected[..16].iter_mut().enumerate() {
        *byte = u8::try_from(index).unwrap();
    }
    expected[16] = 1;
    for text in [text.to_owned(), text.to_uppercase()] {
        let command = parse(args(&[
            "pipesql",
            "resolve",
            "--database",
            "/tmp/example",
            "--transaction",
            &text,
            "--memory-limit-bytes",
            "2000000",
            "--temp-limit-bytes",
            "1000000",
        ]))
        .unwrap();
        assert!(
            matches!(command.operation, Operation::Resolve(token) if token.as_bytes() == &expected)
        );
    }
    for bytes in [
        Vec::new(),
        text.as_bytes()[..47].to_vec(),
        [text.as_bytes(), b"0"].concat(),
        vec![b'g'; 48],
        vec![0xff; 48],
        vec![b'0'; 48],
        b"000000000000000000000000000000000100000000000000".to_vec(),
        b"000102030405060708090a0b0c0d0e0f0000000000000000".to_vec(),
    ] {
        assert!(parse_transaction(&OsString::from_vec(bytes)).is_err());
    }
    for operation in [
        "create",
        "create-declared",
        "open",
        "load",
        "query",
        "explain",
    ] {
        assert!(
            parse(args(&[
                "pipesql",
                operation,
                "--database",
                "/tmp/example",
                "--transaction",
                text,
                "--memory-limit-bytes",
                "2000000",
                "--temp-limit-bytes",
                "1000000",
            ]))
            .is_err()
        );
    }
    assert!(
        parse(args(&[
            "pipesql",
            "resolve",
            "--database",
            "/tmp/example",
            "--memory-limit-bytes",
            "2000000",
            "--temp-limit-bytes",
            "1000000",
        ]))
        .is_err()
    );
    for (flag, value) in [
        ("--transaction", text),
        ("--input", "/input"),
        ("--query-file", "/query"),
    ] {
        assert!(
            parse(args(&[
                "pipesql",
                "resolve",
                "--database",
                "/tmp/example",
                "--transaction",
                text,
                "--memory-limit-bytes",
                "2000000",
                "--temp-limit-bytes",
                "1000000",
                flag,
                value,
            ]))
            .is_err()
        );
    }
}

#[test]
fn parser_rejects_missing_duplicate_unknown_and_bad_values() {
    let cases: &[&[&str]] = &[
        &["pipesql"],
        &["pipesql", "query"],
        &["pipesql", "create", "--database"],
        &[
            "pipesql",
            "load",
            "--database",
            "/tmp/a",
            "--memory-limit-bytes",
            "1",
            "--temp-limit-bytes",
            "1",
        ],
        &[
            "pipesql",
            "open",
            "--database",
            "/tmp/a",
            "--input",
            "/tmp/input",
            "--memory-limit-bytes",
            "1",
            "--temp-limit-bytes",
            "1",
        ],
        &[
            "pipesql",
            "query",
            "--database",
            "/tmp/a",
            "--memory-limit-bytes",
            "1",
            "--temp-limit-bytes",
            "1",
        ],
        &[
            "pipesql",
            "create",
            "--database",
            "/tmp/a",
            "--query-file",
            "/tmp/q",
            "--memory-limit-bytes",
            "1",
            "--temp-limit-bytes",
            "1",
        ],
        &[
            "pipesql",
            "create",
            "--database",
            "/tmp/a",
            "--database",
            "/tmp/b",
            "--memory-limit-bytes",
            "1",
            "--temp-limit-bytes",
            "1",
        ],
        &[
            "pipesql",
            "create",
            "--database",
            "/tmp/a",
            "--unknown",
            "1",
            "--memory-limit-bytes",
            "1",
            "--temp-limit-bytes",
            "1",
        ],
        &[
            "pipesql",
            "create",
            "--database",
            "/tmp/a",
            "--memory-limit-bytes",
            "not-a-number",
            "--temp-limit-bytes",
            "1",
        ],
        &[
            "pipesql",
            "create",
            "--database",
            "/tmp/a",
            "--memory-limit-bytes",
            "0",
            "--temp-limit-bytes",
            "1",
        ],
    ];
    for case in cases {
        assert!(parse(args(case)).is_err(), "accepted {case:?}");
    }
}

#[test]
fn arguments_are_byte_bounded() {
    let oversized = OsString::from("x".repeat(MAX_ARGUMENT_BYTES + 1));
    assert!(check_argument(&oversized).is_err());
}

#[test]
fn schema_requires_one_table_and_rejects_other_source_options() {
    let base = [
        "pipesql",
        "schema",
        "--database",
        "/tmp/db",
        "--memory-limit-bytes",
        "1000000",
        "--temp-limit-bytes",
        "1000000",
    ];
    // A table argument is a name, not SQL source. A keyword-shaped name must
    // reach schema inspection unchanged.
    let valid: Vec<_> = base.into_iter().chain(["--table", "SELECT"]).collect();
    let command = parse(args(&valid)).unwrap();
    assert!(matches!(command.operation, Operation::Schema(name) if name == "SELECT"));
    for suffix in [
        vec![],
        vec!["--table", "a", "--table", "b"],
        vec!["--table", "a", "--schema-file", "/tmp/schema"],
        vec!["--table", "a", "--query-file", "/tmp/query"],
    ] {
        let invalid: Vec<_> = base.into_iter().chain(suffix).collect();
        assert!(parse(args(&invalid)).is_err(), "accepted {invalid:?}");
    }
    let mut wrong_operation = valid;
    wrong_operation[1] = "open";
    assert!(parse(args(&wrong_operation)).is_err());
}

#[test]
fn import_requires_every_bound_and_rejects_duplicates_or_invalid_ranges() {
    let base = [
        "pipesql",
        "import",
        "--database",
        "/tmp/db",
        "--input",
        "-",
        "--table",
        "facts",
        "--memory-limit-bytes",
        "8000000",
        "--temp-limit-bytes",
        "8000000",
    ];
    let values = [
        "1000000", "100", "4096", "1024", "2", "4096", "8", "1000000",
    ];
    let mut complete = base.to_vec();
    for (option, value) in super::IMPORT_OPTIONS.iter().zip(values) {
        complete.extend([*option, value]);
    }
    let command = parse(args(&complete)).unwrap();
    let Operation::Import {
        input,
        table,
        limits,
    } = command.operation
    else {
        panic!("wrong operation")
    };
    assert_eq!(input, Path::new("-"));
    assert_eq!(table, "facts");
    let ImportFormat::Csv(limits) = limits else {
        panic!("expected CSV limits")
    };
    assert_eq!(limits.csv.input_bytes, 1_000_000);
    assert_eq!(limits.csv.rows, 100);
    assert_eq!(limits.csv.record_bytes, 4096);
    assert_eq!(limits.csv.field_bytes, 1024);
    assert_eq!(limits.csv.batch_rows, 2);
    assert_eq!(limits.csv.batch_text_bytes, 4096);
    assert_eq!(limits.append.batches, 8);
    assert_eq!(limits.append.encoded_bytes, 1_000_000);
    for (index, value) in values.iter().enumerate() {
        let start = base.len() + index * 2;
        let mut missing = complete.clone();
        missing.drain(start..start + 2);
        assert!(parse(args(&missing)).is_err());
        let mut duplicate = complete.clone();
        duplicate.extend([super::IMPORT_OPTIONS[index], *value]);
        assert!(parse(args(&duplicate)).is_err());
        for bad in ["0", "-1", "x", "18446744073709551616"] {
            let mut invalid = complete.clone();
            invalid[start + 1] = bad;
            assert!(parse(args(&invalid)).is_err());
        }
    }
    for (index, excessive) in [
        (2, "8388802"),
        (3, "65537"),
        (4, "257"),
        (5, "4194305"),
        (6, "4097"),
    ] {
        let mut invalid = complete.clone();
        invalid[base.len() + index * 2 + 1] = excessive;
        assert!(parse(args(&invalid)).is_err());
    }
    complete[1] = "load";
    assert!(parse(args(&complete)).is_err());
}

#[test]
fn export_requires_its_limits_and_rejects_import_options() {
    let base = [
        "pipesql",
        "export",
        "--database",
        "/tmp/db",
        "--query-file",
        "/tmp/query",
        "--memory-limit-bytes",
        "2000000",
        "--temp-limit-bytes",
        "1000000",
    ];
    assert!(parse(args(&base)).is_err());
    let valid = [
        base.as_slice(),
        &["--row-limit", "0", "--output-limit-bytes", "1000"],
    ]
    .concat();
    let command = parse(args(&valid)).unwrap();
    assert!(
        matches!(command.operation, Operation::Export { limits: ExportFormat::Jsonl(limits), .. } if limits.rows == 0 && limits.bytes == 1000)
    );
    for extra in [
        ["--row-limit", "1"],
        ["--output-limit-bytes", "100"],
        ["--batch-rows", "1"],
        ["--encoded-limit-bytes", "100"],
    ] {
        assert!(parse(args(&[valid.as_slice(), &extra].concat())).is_err());
    }
    for operation in ["query", "explain", "open", "import"] {
        let mut invalid = valid.clone();
        invalid[1] = operation;
        assert!(parse(args(&invalid)).is_err());
    }
}

#[test]
fn parquet_import_requires_its_own_bounds_and_preserves_typed_limits() {
    let valid = [
        "pipesql",
        "import",
        "--format",
        "parquet",
        "--database",
        "/tmp/db",
        "--input",
        "/tmp/data.parquet",
        "--table",
        "facts",
        "--memory-limit-bytes",
        "8000000",
        "--temp-limit-bytes",
        "8000000",
        "--input-limit-bytes",
        "100000",
        "--row-limit",
        "600",
        "--metadata-limit-bytes",
        "2048",
        "--row-group-limit",
        "2",
        "--row-group-rows",
        "300",
        "--row-group-limit-bytes",
        "6000",
        "--page-limit-bytes",
        "3000",
        "--batch-limit",
        "4",
        "--encoded-limit-bytes",
        "100000",
    ];
    let command = parse(args(&valid)).unwrap();
    let Operation::Import {
        limits: ImportFormat::Parquet(limits),
        ..
    } = command.operation
    else {
        panic!("wrong format")
    };
    assert_eq!(limits.parquet.input_bytes, 100000);
    assert_eq!(limits.parquet.rows, 600);
    assert_eq!(limits.parquet.metadata_bytes, 2048);
    assert_eq!(limits.parquet.row_groups, 2);
    assert_eq!(limits.parquet.row_group_rows, 300);
    assert_eq!(limits.parquet.row_group_bytes, 6000);
    assert_eq!(limits.parquet.page_bytes, 3000);
    assert_eq!(limits.append.batches, 4);
    assert_eq!(limits.append.encoded_bytes, 100000);
    for index in (14..valid.len()).step_by(2) {
        let mut missing = valid.to_vec();
        missing.drain(index..index + 2);
        assert!(parse(args(&missing)).is_err(), "{}", valid[index]);
        for value in ["0", "-1", "18446744073709551616"] {
            let mut invalid = valid;
            invalid[index + 1] = value;
            assert!(parse(args(&invalid)).is_err(), "{}={value}", valid[index]);
        }
        assert!(parse(args(&[&valid[..], &valid[index..index + 2]].concat())).is_err());
    }
    for extra in [
        ["--batch-rows", "1"],
        ["--field-limit-bytes", "64"],
        ["--record-limit-bytes", "100"],
        ["--batch-text-bytes", "64"],
        ["--row-group-text-bytes", "64"],
        ["--format", "parquet"],
    ] {
        assert!(parse(args(&[&valid[..], &extra].concat())).is_err());
    }
    for format in ["csv", "jsonl", "unknown"] {
        let mut invalid = valid;
        invalid[3] = format;
        assert!(parse(args(&invalid)).is_err());
    }
}

#[test]
fn parquet_export_checks_format_specific_options_and_zero_rows() {
    let valid = [
        "pipesql",
        "export",
        "--format",
        "parquet",
        "--database",
        "/tmp/db",
        "--query-file",
        "/tmp/q.sql",
        "--memory-limit-bytes",
        "8000000",
        "--temp-limit-bytes",
        "8000000",
        "--row-limit",
        "0",
        "--output-limit-bytes",
        "10000",
        "--metadata-limit-bytes",
        "2048",
        "--row-group-limit",
        "2",
        "--row-group-rows",
        "300",
        "--row-group-text-bytes",
        "65536",
    ];
    let command = parse(args(&valid)).unwrap();
    let Operation::Export {
        limits: ExportFormat::Parquet(limits),
        ..
    } = command.operation
    else {
        panic!("wrong format")
    };
    assert_eq!(limits.rows, 0);
    assert_eq!(limits.bytes, 10000);
    assert_eq!(limits.row_groups, 2);
    assert_eq!(limits.row_group_rows, 300);
    assert_eq!(limits.row_group_text_bytes, 65536);
    assert_eq!(limits.metadata_bytes, 2048);
    for index in (12..valid.len()).step_by(2) {
        let mut missing = valid.to_vec();
        missing.drain(index..index + 2);
        assert!(parse(args(&missing)).is_err());
        assert!(parse(args(&[&valid[..], &valid[index..index + 2]].concat())).is_err());
    }
    for extra in [
        ["--row-group-limit-bytes", "100"],
        ["--page-limit-bytes", "100"],
        ["--input-limit-bytes", "100"],
        ["--batch-limit", "1"],
    ] {
        assert!(parse(args(&[&valid[..], &extra].concat())).is_err());
    }
    for format in ["csv", "jsonl", "unknown"] {
        let mut invalid = valid;
        invalid[3] = format;
        assert!(parse(args(&invalid)).is_err());
    }
}
