//! Qualify persistent IDs with complete schema and value checks through both readers.
//!
//! Retained bytes exercise a layout independent of engine encoding. Four-type
//! cases vary IDs and physical order separately, moving each descriptor with its
//! payload. Literal answers stay separate from those byte edits. Readable wrong
//! values and schema names must fail the checker; bad IDs with repaired checksums
//! must fail structural validation without changing stored files.

use super::*;

pub(super) fn run(c: &mut Campaign) -> Result<()> {
    retained_bytes(c)?;
    column_identities(c)
}

fn retained_bytes(c: &mut Campaign) -> Result<()> {
    let genesis = c.run.directory.join("genesis");
    c.driver(&genesis, "genesis")?;
    let empty = catalog::inspect(&genesis)?;
    if empty["issued"] != 0
        || empty["generation"] != 0
        || ["tables", "successes", "reachable"]
            .iter()
            .any(|name| empty[name] != json!([]))
    {
        return Err("wrong empty catalog".into());
    }
    let path = c.run.directory.join("retained-bytes");
    crate::oracle::catalog_vectors::database(&path, &c.run.root.join("test/data"))?;
    let graph = catalog::inspect(&path)?;
    expected_retained(&graph)?;
    c.driver(&path, "retained")?;
    for field in ["id", "name"] {
        let mut wrong = graph.clone();
        wrong["tables"][0][field] = if field == "id" {
            json!(29)
        } else {
            json!("other")
        };
        if expected_retained(&wrong).is_ok() {
            return Err("retained table control accepted".into());
        }
    }
    for column in 0..2 {
        for field in ["id", "name", "type", "nullable"] {
            let mut wrong = graph.clone();
            let value = &mut wrong["tables"][0]["columns"][column][field];
            *value = match field {
                "id" => json!(value.as_u64().unwrap() + 1),
                "type" => json!(1),
                "nullable" => json!(!value.as_bool().unwrap()),
                _ => json!("other"),
            };
            if expected_retained(&wrong).is_ok() {
                return Err("retained schema control accepted".into());
            }
        }
    }
    for row in 0..4 {
        for column in 0..2 {
            let mut wrong = graph.clone();
            wrong["tables"][0]["rows"][row][column] = if column == 0 {
                json!("wrong")
            } else {
                json!({"double_bits":"3ff0000000000000"})
            };
            if expected_retained(&wrong).is_ok() {
                return Err("retained value control accepted".into());
            }
        }
    }
    for cut in 0..4 {
        let mut wrong = graph.clone();
        wrong["tables"][0]["rows"]
            .as_array_mut()
            .unwrap()
            .truncate(cut);
        if expected_retained(&wrong).is_ok() {
            return Err("retained prefix control accepted".into());
        }
    }
    let wrong = c.run.directory.join("retained-wrong-answer");
    copy_tree(&path, &wrong)?;
    Mutation::new(&wrong)?.payload(3, true, |data, at| {
        put(data, at + 1, 1.0_f64.to_bits(), 8);
    })?;
    if expected_retained(&catalog::inspect(&wrong)?).is_ok() {
        return Err("readable wrong retained amount accepted".into());
    }
    c.wrong_answer(&wrong, "retained", "retained amount bits")?;
    let wrong = c.run.directory.join("retained-wrong-name");
    copy_tree(&path, &wrong)?;
    Mutation::new(&wrong)?.change("schema", |data| {
        data[72..76].copy_from_slice(b"nope");
    })?;
    if expected_retained(&catalog::inspect(&wrong)?).is_ok() {
        return Err("readable wrong retained name accepted".into());
    }
    c.wrong_answer(&wrong, "retained", "query result schema")?;
    fs::write(
        c.run.directory.join("retained-identity.json"),
        serde_json::to_vec_pretty(&json!({
            "complete":true, "rows":4, "columns":2,
            "readers":["independent inspector", "engine query"],
            "negative_controls": {
                "table_fields":2, "column_fields":8, "cells":8, "prefixes":4,
                "readable_wrong_amount":true, "readable_wrong_name":true
            }
        }))?,
    )?;
    Ok(())
}

fn expected_retained(graph: &Value) -> Result<()> {
    // These bytes store columns in the opposite order to their schema. Values
    // must follow column identity, including signed zero and the NaN payload.
    if graph["successes"] != json!([3, 5])
        || graph["issued"] != 6
        || graph["generation"] != 2
        || graph["tables"]
            != json!([{
                "id": 0x0102030405060708_u64,
                "name": "facts",
                "columns": [
                    {"id":29, "name":"note", "type":3, "nullable":true},
                    {"id":3, "name":"amount", "type":2, "nullable":false}
                ],
                "rows": [
                    [null, {"double_bits":"8000000000000000"}],
                    ["", {"double_bits":"7ff0000000000001"}],
                    ["雪", {"double_bits":"7ff0000000000000"}],
                    ["é\u{0}🙂", {"double_bits":"ffefffffffffffff"}]
                ]
            }])
    {
        return Err("retained bytes decoded with wrong column identities or values".into());
    }
    Ok(())
}

fn column_identities(c: &mut Campaign) -> Result<()> {
    let id_cases = [
        [u32::MAX, 19, 2, 7],
        [3, 1, u32::MAX, 42],
        [19, u32::MAX, 42, 2],
        [7, 42, 1, u32::MAX],
    ];
    let orders = [[0, 1, 2, 3], [3, 2, 1, 0], [2, 0, 3, 1], [1, 3, 0, 2]];
    fs::write(
        c.run.directory.join("column-identity-selection.json"),
        serde_json::to_vec_pretty(&json!({
            "ids": id_cases, "physical_orders": orders,
            "refusals": ["unknown column ID", "duplicate column ID"],
            "wrong_values": ["INT64", "DOUBLE", "STRING", "DATE"],
            "answers": "Complete literal schema and typed rows through independent inspector and public query driver"
        }))?,
    )?;
    let mut cases = Vec::new();
    for (id_case, ids) in id_cases.into_iter().enumerate() {
        for (order_case, order) in orders.into_iter().enumerate() {
            let label = format!("column-identity-{id_case}-{order_case}");
            let path = c.run.directory.join(&label);
            copy_tree(&c.seed, &path)?;
            Mutation::new(&path)?.column_layout(ids, order)?;
            let graph = catalog::inspect(&path)?;
            expected_ids(&graph, ids)?;
            // Observe the bytes after mutation: unchanged physical order can
            // return correct rows while silently failing to exercise this case.
            let mut unit_orders = Vec::new();
            for name in graph["reachable"].as_array().unwrap() {
                let name = name.as_str().unwrap();
                let data = fs::read(path.join("units").join(name))?;
                if data.starts_with(b"PSQLDATA") {
                    let actual: Vec<_> =
                        (64..192).step_by(32).map(|at| get(&data, at, 4)).collect();
                    if actual != order.map(|position| u64::from(ids[position])) {
                        return Err(
                            format!("{label}: physical column order was not exercised").into()
                        );
                    }
                    unit_orders.push(json!({"object":name, "ids":actual}));
                }
            }
            if unit_orders.len() != 4 {
                return Err(format!("{label}: expected four native units").into());
            }
            c.driver(&path, "check")?;
            cases.push(
                json!({"case": label, "ids": ids, "physical_order": order, "units":unit_orders, "complete": true}),
            );
        }
    }
    let seed = c.run.directory.join("column-identity-0-2");
    for (label, duplicate, reason) in [
        ("unknown-column-id", false, "unit column identity"),
        (
            "duplicate-column-id",
            true,
            "duplicate unit column identity",
        ),
    ] {
        let path = c.run.directory.join(label);
        copy_tree(&seed, &path)?;
        Mutation::new(&path)?.change("unit", |data| {
            let value = if duplicate { get(data, 64, 4) } else { 1 };
            put(data, if duplicate { 96 } else { 64 }, value, 4);
        })?;
        let before = workspace::tree_contents(&path)?;
        let error = catalog::inspect(&path).expect_err("accepted malformed column identity");
        if error.to_string() != reason {
            return Err(format!("{label}: expected {reason}, got {error}").into());
        }
        c.driver(&path, "reject")?;
        if workspace::tree_contents(&path)? != before {
            return Err(format!("{label}: refusal changed stored files").into());
        }
        cases.push(json!({"case":label, "expected_error":reason, "complete":true}));
    }
    for (id, reason) in [
        (u32::MAX, "known row key"),
        (19, "stock DOUBLE bits"),
        (2, "stock STRING value"),
        (7, "stock DATE value"),
    ] {
        let label = format!("column-identity-wrong-{id}");
        let wrong = c.run.directory.join(&label);
        copy_tree(&seed, &wrong)?;
        Mutation::new(&wrong)?.payload(u64::from(id), true, |data, at| {
            match id {
                u32::MAX => put(data, at + 1, (i64::MIN + 1) as u64, 8),
                19 => put(data, at + 1, 1.0_f64.to_bits(), 8),
                // Eight rows: one bitmap byte and nine u32 string offsets.
                // Replace the first three-byte character with another valid one.
                2 => data[at + 37..at + 40].copy_from_slice("雨".as_bytes()),
                7 => put(data, at + 1, (-719161_i32) as u32 as u64, 4),
                _ => unreachable!(),
            }
        })?;
        if expected_ids(&catalog::inspect(&wrong)?, id_cases[0]).is_ok() {
            return Err(format!("readable wrong column {id} accepted").into());
        }
        c.wrong_answer(&wrong, "check", reason)?;
        cases.push(json!({"case": label, "expected_error":reason, "complete":true}));
    }
    fs::write(
        c.run.directory.join("column-identities.json"),
        serde_json::to_vec_pretty(&cases)?,
    )?;
    Ok(())
}
