//! Check CLI grammar using literal argument lists and expected command fields.
//!
//! Complete operations establish required options; malformed lists challenge
//! duplicate, missing, incompatible and oversized inputs. Token bytes are literal
//! expectations, not produced by the parser under test. These tests do not open
//! databases; process and allocation behavior belong to the CLI campaigns.

use super::{MAX_ARGUMENT_BYTES, Operation, check_argument, parse, parse_transaction};
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
