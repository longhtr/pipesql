//! Judge recovered report storage against independently stated rows and receipts.
//!
//! The graph comes from the independent catalog reader. Literal event and dimension
//! rows preserve NULLs, dates and DOUBLE bits instead of rebuilding answers through
//! the workload's input generator. The expected state and retry flag select which
//! transaction history and rows may be present. Schema, receipts and complete data
//! must agree; a structurally valid catalog with the wrong report is still a failure.

use super::*;

pub(super) fn check_graph(graph: &Value, state: u32, retried: bool) -> Result<()> {
    let events = json!([
        [0, 1, 10956, 10, {"double_bits": "3fe0000000000000"}],
        [1, 2, 11016, 20, {"double_bits": "3ff8000000000000"}],
        [2, null, 11016, 30, null],
        [3, 99, 11016, null, {"double_bits": "8000000000000000"}],
        [4, 3, null, 5, {"double_bits": "4000000000000000"}],
        [5, 1, 11323, null, {"double_bits": "4004000000000000"}],
        [6, 2, 11322, -5, {"double_bits": "4008000000000000"}],
        [7, 1, null, 7, {"double_bits": "bff0000000000000"}],
        [8, 1, 10956, -2, {"double_bits": "4010000000000000"}],
        [9, 2, 11323, 40, {"double_bits": "4012000000000000"}],
        [10, 99, 11016, 3, {"double_bits": "4014000000000000"}],
        [11, null, null, 11, null],
        [12, 3, 11016, 0, {"double_bits": "4018000000000000"}],
        [13, 1, 11323, 13, {"double_bits": "401a000000000000"}],
        [14, 2, null, null, {"double_bits": "401c000000000000"}],
        [15, 3, 11322, 9, {"double_bits": "c000000000000000"}]
    ]);
    let mut rows = events.as_array().unwrap()[..if state == 2 { 16 } else { 8 }].to_vec();
    let mut issued = if state == 0 { 5 } else { 6 };
    let mut successes = vec![1, 2, 3, 4];
    if state == 2 {
        successes.push(6);
    }
    if retried {
        issued += 1;
        successes.push(issued);
        rows.push(events[8].clone());
    }
    if graph["issued"] != issued
        || graph["generation"] != successes.len()
        || graph["successes"] != json!(successes)
    {
        return Err("independent report receipt history differs".into());
    }
    let tables = json!([
        {"id": 1, "name": "events", "rows": rows, "columns": [
            {"id": 1, "name": "id", "type": 1, "nullable": false},
            {"id": 2, "name": "dimension_id", "type": 1, "nullable": true},
            {"id": 3, "name": "happened", "type": 4, "nullable": true},
            {"id": 4, "name": "amount", "type": 1, "nullable": true},
            {"id": 5, "name": "measurement", "type": 2, "nullable": true}
        ]},
        {"id": 2, "name": "dimensions", "rows": [[1, "north"], [2, "south"], [2, "南"], [3, ""]], "columns": [
            {"id": 1, "name": "id", "type": 1, "nullable": false},
            {"id": 2, "name": "label", "type": 3, "nullable": false}
        ]}
    ]);
    if graph["tables"] != tables {
        return Err("independent report rows or schema differ".into());
    }
    Ok(())
}
