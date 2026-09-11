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
    for operation in ["create", "open"] {
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
    let command = parse(args(&[
        "pipesql",
        "query",
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
        matches!(command.operation, Operation::Query(query_file) if query_file == Path::new("/tmp/q6.pipe.sql"))
    );
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
    for operation in ["create", "open", "load", "query"] {
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
