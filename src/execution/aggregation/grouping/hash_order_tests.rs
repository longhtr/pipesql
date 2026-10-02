//! Check the work and failure boundaries of in-memory group ordering.
//!
//! These fixtures insert distinct encoded keys at the private lookup boundary.
//! They isolate sorting from accumulation; hash_tests checks representative key
//! bits and aggregate cells through actual input batches.

use super::*;
use crate::Database;
use crate::execution::blocking::append_value;
use crate::execution::blocking::test_support::{Directory, schema};
use crate::value::{DataType, StringValue};

fn fixture() -> (Directory, Database) {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(16_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    database
        .declare_table(
            "facts",
            &[crate::ColumnDeclaration {
                name: "n",
                data_type: DataType::Int64,
                nullable: false,
            }],
            &CancellationToken::new(),
        )
        .unwrap();
    (directory, database)
}

fn insert(groups: &mut MemoryGroups<'_>, keys: &RowLayout, values: &[Value<'_>], id: usize) {
    groups.key.clear();
    for (value, column) in values.iter().zip(&keys.columns) {
        append_value(&mut groups.key, *value, column.kind, column.nullable).unwrap();
    }
    assert_eq!(groups.lookup(keys, id as u64).unwrap(), Ok(id));
}

#[test]
fn narrow_keys_batch_moves_across_pairs_and_passes() {
    let (_directory, database) = fixture();
    let query = database
        .prepare("FROM facts |> AGGREGATE COUNT(*) AS n")
        .unwrap();
    let aggregate = AggregateState::new(
        &database.memory,
        &query.plan.aggregates[0],
        query.plan.aggregate_demand(0),
        1,
        query.plan.input_columns(),
    )
    .unwrap();
    let keys = schema(&[(DataType::Int64, false)]);
    let baseline = database.reserved_memory_bytes();
    // Every merge pass moves all IDs, including an unpaired trailing run.
    // Counts include initialization, separately capped at 256 IDs per call.
    for (rows, calls) in [
        (0, 1),
        (1, 1),
        (2, 2),
        (3, 2),
        (31, 2),
        (32, 2),
        (255, 9),
        (256, 9),
        (257, 12),
        (511, 20),
        (512, 20),
        (513, 24),
        (4096, 208),
    ] {
        let mut groups =
            MemoryGroups::new(&database.memory, &aggregate, &keys, rows.max(1), rows * 9).unwrap();
        for id in 0..rows {
            insert(&mut groups, &keys, &[Value::Int64((rows - id) as i64)], id);
        }
        groups.begin_order().unwrap();
        let charged = database.reserved_memory_bytes();
        for call in 1..=calls {
            assert_eq!(
                groups.order_step(&keys, &CancellationToken::new()).unwrap(),
                call == calls,
                "rows={rows}, call={call}"
            );
        }
        assert_eq!(database.reserved_memory_bytes(), charged);
        for position in 0..rows {
            let id = groups.ordered_group(position).unwrap();
            assert_eq!(id, rows - 1 - position);
            assert_eq!(
                groups.key_value(id, 0, &keys).unwrap(),
                Value::Int64((position + 1) as i64)
            );
        }
        assert!(groups.ordered_group(rows).is_err());
        assert!(groups.order_step(&keys, &CancellationToken::new()).unwrap());
        drop(groups);
        assert_eq!(database.reserved_memory_bytes(), baseline);
    }
}

#[test]
fn comparison_bytes_count_both_keys_and_allow_one_oversized_pair() {
    let (_directory, database) = fixture();
    let query = database
        .prepare("FROM facts |> AGGREGATE COUNT(*) AS n")
        .unwrap();
    let aggregate = AggregateState::new(
        &database.memory,
        &query.plan.aggregates[0],
        query.plan.aggregate_demand(0),
        1,
        query.plan.input_columns(),
    )
    .unwrap();
    let keys = schema(&[(DataType::String, true), (DataType::Int64, false)]);
    let baseline = database.reserved_memory_bytes();
    // The second key prevents equal strings from collapsing into one group.
    // Encoded lengths are text bytes + 14, or 10 for NULL plus the integer.
    // Each first-pass pair compares once, then moves its tail without charge.
    for (widths, rows, moves) in [
        (&[Some(1024)][..], 512, 62),           // 31 * (2 * 1,038) = 64,356.
        (&[Some(0), Some(16_000)][..], 128, 8), // 4 * 16,028 = 64,112.
        (&[Some(16_000), Some(0)][..], 128, 8),
        (&[Some(0), Some(16_000), Some(1024), Some(7)][..], 128, 12),
        (&[Some(16_370)][..], 8, 4), // Two pairs exactly consume 65,536 bytes.
        (&[Some(65_536)][..], 8, 2),
        (&[None, Some(65_536)][..], 8, 2),
        (&[Some(65_536), None][..], 8, 2),
        (&[None, Some(0), Some(7)][..], 512, 256),
    ] {
        let values: Vec<_> = (0..rows).map(|id| widths[id % widths.len()]).collect();
        let bytes = values
            .iter()
            .map(|width| width.map_or(10, |width| 14 + width))
            .sum();
        let mut groups =
            MemoryGroups::new(&database.memory, &aggregate, &keys, rows, bytes).unwrap();
        for (id, width) in values.iter().enumerate() {
            let text = width.map(|width| "x".repeat(width));
            let value = text
                .as_deref()
                .map_or(Value::Null, |text| Value::String(StringValue::new(text)));
            insert(
                &mut groups,
                &keys,
                &[value, Value::Int64((rows - id) as i64)],
                id,
            );
        }
        let charged = database.reserved_memory_bytes();
        groups.begin_order().unwrap();
        while matches!(groups.phase, Phase::OrderInit(_)) {
            assert!(!groups.order_step(&keys, &CancellationToken::new()).unwrap());
        }
        assert!(!groups.order_step(&keys, &CancellationToken::new()).unwrap());
        let Phase::Order(cursor) = groups.phase else {
            panic!("unfinished first pass");
        };
        assert_eq!(cursor.width, 1);
        assert_eq!(cursor.output, moves, "widths={widths:?}");
        for _ in 0..4096 {
            if groups.order_step(&keys, &CancellationToken::new()).unwrap() {
                break;
            }
        }
        assert_eq!(groups.phase, Phase::Ordered);
        let mut expected: Vec<_> = values.iter().copied().enumerate().collect();
        expected.sort_by_key(|&(id, width)| (width, rows - id));
        for (position, (id, width)) in expected.into_iter().enumerate() {
            assert_eq!(groups.ordered_group(position).unwrap(), id);
            assert_eq!(
                groups.key_value(id, 1, &keys).unwrap(),
                Value::Int64((rows - id) as i64)
            );
            match (groups.key_value(id, 0, &keys).unwrap(), width) {
                (Value::Null, None) => (),
                (Value::String(text), Some(width)) => assert_eq!(text.as_str(), "x".repeat(width)),
                mismatch => panic!("typed key mismatch: {mismatch:?}"),
            }
        }
        assert_eq!(database.reserved_memory_bytes(), charged);
        drop(groups);
        assert_eq!(database.reserved_memory_bytes(), baseline);
    }
}

#[test]
fn partially_ordered_groups_fail_permanently_on_cancellation_or_invalid_keys() {
    let (_directory, database) = fixture();
    let query = database
        .prepare("FROM facts |> AGGREGATE COUNT(*) AS n")
        .unwrap();
    let aggregate = AggregateState::new(
        &database.memory,
        &query.plan.aggregates[0],
        query.plan.aggregate_demand(0),
        1,
        query.plan.input_columns(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    for fault in 0..3 {
        let mut keys = schema(&[(DataType::Int64, false)]);
        let mut groups =
            MemoryGroups::new(&database.memory, &aggregate, &keys, 512, 512 * 9).unwrap();
        for id in 0..512 {
            insert(&mut groups, &keys, &[Value::Int64((512 - id) as i64)], id);
        }
        groups.begin_order().unwrap();
        for _ in 0..3 {
            assert!(!groups.order_step(&keys, &CancellationToken::new()).unwrap());
        }
        let Phase::Order(cursor) = groups.phase else {
            panic!("unfinished sort");
        };
        assert_eq!(cursor.output, 256);
        let before = groups.buckets.clone();
        let cancel = CancellationToken::new();
        match fault {
            0 => cancel.cancel(),
            1 => keys.layout ^= 1,
            2 => groups.arena[groups.entries[300].start] = 255,
            _ => unreachable!(),
        }
        let failure = groups.order_step(&keys, &cancel);
        if fault == 0 {
            assert!(matches!(failure, Err(Error::Cancelled)));
        } else {
            assert!(matches!(failure, Err(Error::Corrupt(_))));
        }
        assert_eq!(groups.phase, Phase::Failed);
        if fault == 2 {
            assert_ne!(
                groups.buckets, before,
                "malformed key follows earlier moves in this call"
            );
        } else {
            assert_eq!(groups.buckets, before);
        }
        let after = groups.buckets.clone();
        keys.layout = groups.key_layout;
        assert!(groups.order_step(&keys, &CancellationToken::new()).is_err());
        assert!(groups.begin_order().is_err());
        assert!(groups.ordered_group(0).is_err());
        assert_eq!(groups.buckets, after);
        drop(groups);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
}
