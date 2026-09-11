//! Public spooling contract tests.
use super::*;

#[test]
fn mixed_typed_rows_survive_memory_and_disk_result_spooling() {
    const GROUPS: usize = 1024;
    const CHUNK: usize = 128;
    let directory = Directory::new();
    let path = directory.database();
    let cancel = CancellationToken::new();
    let db = Database::create_empty(&path, Config::new(32_000_000, 16_000_000).unwrap()).unwrap();
    let names = ["i", "d", "day", "s"];
    let kinds = [
        DataType::Int64,
        DataType::Double,
        DataType::Date,
        DataType::String,
    ];
    let schema = std::array::from_fn::<_, 4, _>(|column| ColumnDeclaration {
        name: names[column],
        data_type: kinds[column],
        nullable: true,
    });
    db.declare_table("typed", &schema, &cancel).unwrap();
    let doubles = [
        f64::NEG_INFINITY,
        -0.0,
        0.0,
        f64::INFINITY,
        f64::from_bits(0x7ff8_0000_0000_0042),
    ];
    let days = [-719_162, 0, 2_932_896];
    let valid = |group: usize, column: usize| match column {
        0 => group != 0,
        1 => !group.is_multiple_of(7),
        2 => !group.is_multiple_of(11),
        3 => !group.is_multiple_of(13),
        _ => unreachable!(),
    };
    let text = |group| format!("row-{group:04}\0{}", "界".repeat(340));
    let mut expected = Vec::new();
    for group in 0..GROUPS {
        let fields = [
            if valid(group, 0) {
                Cell::Integer(group as i64)
            } else {
                Cell::Null
            },
            if valid(group, 1) {
                Cell::Number(doubles[group % doubles.len()].to_bits())
            } else {
                Cell::Null
            },
            if valid(group, 2) {
                Cell::Day(days[group % days.len()])
            } else {
                Cell::Null
            },
            if valid(group, 3) {
                Cell::Text(text(group))
            } else {
                Cell::Null
            },
        ];
        expected.push(vec![
            fields[3].clone(),
            fields[2].clone(),
            fields[1].clone(),
            fields[0].clone(),
            Cell::Integer(2),
            fields[3].clone(),
        ]);
    }
    expected.sort_unstable();
    let mut writer = db
        .begin_append(
            "typed",
            AppendLimits {
                batches: 16,
                encoded_bytes: 4_000_000,
            },
            &cancel,
        )
        .unwrap();
    for first in (0..GROUPS * 2).step_by(CHUNK) {
        let integers: Vec<_> = (first..first + CHUNK).map(|row| (row / 2) as i64).collect();
        let numbers: Vec<_> = integers
            .iter()
            .map(|group| doubles[*group as usize % doubles.len()])
            .collect();
        let dates: Vec<_> = integers
            .iter()
            .map(|group| {
                DateValue::from_days_since_unix_epoch(days[*group as usize % days.len()]).unwrap()
            })
            .collect();
        let texts: Vec<_> = integers.iter().map(|group| text(*group as usize)).collect();
        let borrowed: Vec<_> = texts.iter().map(String::as_str).collect();
        let mut validity = [[0_u8; CHUNK / 8]; 4];
        for (column, mask) in validity.iter_mut().enumerate() {
            for (row, group) in integers.iter().enumerate() {
                if valid(*group as usize, column) {
                    mask[row / 8] |= 1 << (row % 8);
                }
            }
        }
        writer
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&integers),
                        validity: &validity[0],
                    },
                    ColumnInput {
                        values: ColumnValues::Double(&numbers),
                        validity: &validity[1],
                    },
                    ColumnInput {
                        values: ColumnValues::Date(&dates),
                        validity: &validity[2],
                    },
                    ColumnInput {
                        values: ColumnValues::String(&borrowed),
                        validity: &validity[3],
                    },
                ],
                &cancel,
            )
            .unwrap();
    }
    writer.commit(&cancel).unwrap();
    db.close().unwrap();
    for (cap, require_disk) in [(8_000_000, false), (2_500_000, true)] {
        let db = Database::open(&path, Config::new(cap, 16_000_000).unwrap()).unwrap();
        let resident = db.reserved_memory_bytes();
        let query = db
            .prepare(
                "FROM typed |> AGGREGATE COUNT(*) AS n GROUP BY i,d,day,s |> SELECT s,day,d,i,n,s",
            )
            .unwrap();
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut actual: Vec<Vec<Cell>> = Vec::new();
        let mut peak_temp = 0;
        let mut finished = false;
        for _ in 0..200_000 {
            let step = result.step();
            peak_temp = peak_temp.max(db.reserved_temp_bytes());
            assert!(db.reserved_memory_bytes() <= cap);
            match step {
                QueryStep::Progress => (),
                QueryStep::Rows(batch) => {
                    assert_eq!(batch.column_count(), 6);
                    for row in 0..batch.len() {
                        actual.push(
                            (0..batch.column_count())
                                .map(|column| owned_cell(batch.value(row, column).unwrap()))
                                .collect(),
                        );
                    }
                }
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("cap {cap}: {error:?}"),
            }
        }
        assert!(finished, "bounded fixture must complete at cap {cap}");
        assert_eq!(
            peak_temp > 0,
            require_disk,
            "cap {cap}, temporary peak {peak_temp}"
        );
        actual.sort_unstable();
        assert_eq!(actual.len(), expected.len());
        for (row, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
            assert_eq!(actual, expected, "row {row}, cap {cap}");
        }
        println!(
            "mixed row cap={cap} rows={} temporary_peak={peak_temp}",
            actual.len()
        );
        drop(result);
        drop(query);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(db.reserved_memory_bytes(), resident);
        db.close().unwrap();
    }
}
