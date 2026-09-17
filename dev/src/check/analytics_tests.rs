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
