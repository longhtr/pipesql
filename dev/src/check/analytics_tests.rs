//! Challenge the analytical model and JSON Lines result checker independently.
//!
//! The small model result must match literal peer-inclusive answers, so using a
//! row-by-row running sum cannot pass by agreeing with itself. Synthetic output then
//! changes rows, types, schema or completion to prove the checker rejects them.
//! These tests do not execute the CLI: they establish that successful process exit
//! cannot compensate for malformed, incomplete or incorrect output in a workflow.

use super::*;

#[test]
fn model_matches_literal_peer_inclusive_answers() {
    assert_eq!(model(&data::events(false)), data::small_report());
}

#[test]
fn jsonl_rejects_wrong_rows_types_and_incomplete_results() {
    let directory = std::env::temp_dir().join(format!("pipesql-jsonl-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let path = directory.join("result");
    let expected = vec![json!(["7"])];
    let good = "{\"format\":\"pipesql-jsonl\",\"version\":1,\"columns\":[{\"name\":\"n\",\"type\":\"int64\",\"nullable\":false}]}\n{\"row\":[\"7\"]}\n{\"complete\":true,\"rows\":1}\n";
    const SCHEMA: Schema = &[("n", "int64", false)];
    fs::write(&path, good).unwrap();
    check_jsonl(&path, SCHEMA, &expected).unwrap();
    for bad in [
        good.replace("\"7\"", "\"8\""),
        good.replace("\"7\"", "7"),
        good.replace("\"nullable\":false", "\"nullable\":0"),
        good.replace("\"version\":1", "\"version\":1.0"),
        good.replace("\"complete\":true", "\"complete\":1"),
        good.replace("\"rows\":1", "\"rows\":1.0"),
        good.lines().take(2).collect::<Vec<_>>().join("\n"),
        format!("{good}{{\"row\":[\"7\"]}}\n"),
    ] {
        fs::write(&path, bad).unwrap();
        assert!(check_jsonl(&path, SCHEMA, &expected).is_err());
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn import_abort_requires_exact_receipt_error_and_resolution() {
    use crate::process::{Capture, Output};
    use std::os::unix::process::ExitStatusExt;
    let token = "0123456789abcdef0123456789abcdef0100000000000000";
    let receipt = format!("transaction={token}\n");
    let error = "database error: input error at byte 5260: invalid CSV INT64\n";
    let mut output = Output {
        completion: Completion::Exited(std::process::ExitStatus::from_raw(256)),
        stdout: Capture {
            bytes: receipt.as_bytes().to_vec(),
            omitted: 0,
        },
        stderr: Capture {
            bytes: error.as_bytes().to_vec(),
            omitted: 0,
        },
    };
    assert_eq!(import_receipt(&output, Some(5260)).unwrap(), (token, None));
    for wrong in [
        receipt.replace(token, &token.to_uppercase()),
        receipt.replace(token, ""),
        receipt.trim_end().to_owned(),
        format!("{receipt}{receipt}"),
        format!("{receipt}status=imported\ngeneration=4\n"),
        format!("extra\n{receipt}"),
        String::new(),
    ] {
        output.stdout.bytes = wrong.into_bytes();
        assert!(import_receipt(&output, Some(5260)).is_err());
    }
    output.stdout.bytes = receipt.as_bytes().to_vec();
    for wrong in [
        error.replace("5260", "5259"),
        error.replace("INT64", "DOUBLE"),
        format!("{error}{error}"),
        String::new(),
    ] {
        output.stderr.bytes = wrong.into_bytes();
        assert!(import_receipt(&output, Some(5260)).is_err());
    }
    output.stderr.bytes = error.as_bytes().to_vec();
    for raw in [0, 512, 9] {
        output.completion = Completion::Exited(std::process::ExitStatus::from_raw(raw));
        assert!(import_receipt(&output, Some(5260)).is_err());
    }
    output.completion = Completion::Exited(std::process::ExitStatus::from_raw(256));
    output.stdout.omitted = 1;
    assert!(import_receipt(&output, Some(5260)).is_err());
    output.stdout.omitted = 0;
    output.stderr.omitted = 1;
    assert!(import_receipt(&output, Some(5260)).is_err());
    output.stderr = Capture::default();
    output.completion = Completion::Exited(std::process::ExitStatus::from_raw(0));
    let resolved = format!(
        "status=resolved\ndatabase=/test\nmemory_limit_bytes=8000000\ntemp_limit_bytes=64000000\ntransaction={token}\nresolution=aborted\n"
    );
    output.stdout.bytes = resolved.as_bytes().to_vec();
    verify_resolution(&output, Path::new("/test"), token, None).unwrap();
    for wrong in [
        resolved.replace("aborted", "durable"),
        resolved.replace(token, &"f".repeat(48)),
        format!("{resolved}resolution=durable\n"),
        resolved.replace("/test", "/other"),
        resolved.trim_end().to_owned(),
    ] {
        output.stdout.bytes = wrong.into_bytes();
        assert!(verify_resolution(&output, Path::new("/test"), token, None).is_err());
    }
    output.stdout.bytes = format!("{receipt}status=imported\ngeneration=7\n").into_bytes();
    assert_eq!(import_receipt(&output, None).unwrap(), (token, Some(7)));
    output.stdout.bytes = resolved
        .replace("resolution=aborted", "resolution=durable\ngeneration=7")
        .into_bytes();
    verify_resolution(&output, Path::new("/test"), token, Some(7)).unwrap();
    assert!(verify_resolution(&output, Path::new("/test"), token, Some(8)).is_err());
}
