use super::numeric::{average_add, sum_add};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::*;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::frontend::DataType;
use crate::storage_format;

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn shared_scaled_average_matches_independent_intervals() {
    let path = std::env::temp_dir().join(format!("pipesql-shared-mean-{}", std::process::id()));
    let database = Database::create(&path, crate::Config::new(2_000_000, 1).unwrap()).unwrap();
    let query = database
        .prepare("FROM lineitem |> AGGREGATE SUM(l_quantity) AS s,AVG(l_quantity) AS a")
        .unwrap();
    let semantic = query.plan.aggregates.first().unwrap();
    let mut checked = 0;
    for line in include_str!("../../../tests/fixtures/aggregate-semantics/rounding.txt").lines() {
        let fields: Vec<_> = line.split(';').collect();
        if fields[1] != "overflow" {
            continue;
        }
        let rows: Vec<_> = fields[0]
            .split(',')
            .map(|raw| {
                let mut row = [Value::Null; MAX_COLUMNS];
                row[0] = Value::Double(f64::from_bits(u64::from_str_radix(raw, 16).unwrap()));
                row
            })
            .collect();
        let mut groups = Groups::new(
            &database,
            semantic,
            query.plan.aggregate_demand(0),
            query.plan.input_columns(),
        )
        .unwrap();
        let mut batch = Batch::new(&[DataType::Double], 1_000_000).unwrap();
        for rows in rows.chunks(BATCH_ROWS) {
            for (index, row) in rows.iter().enumerate() {
                batch.set(index, 0, row[0]).unwrap();
            }
            batch.publish_rows(rows.len());
            groups.consume(&batch).unwrap();
        }
        assert!(matches!(
            groups.value(0, 0),
            Err(Error::ArithmeticOverflow {
                operation: "SUM",
                ..
            })
        ));
        let Value::Double(average) = groups.value(0, 1).unwrap() else {
            panic!("mean type");
        };
        let (low, high) = fields[2].split_once(',').unwrap();
        let low = f64::from_bits(u64::from_str_radix(low, 16).unwrap());
        let high = f64::from_bits(u64::from_str_radix(high, 16).unwrap());
        assert!(average.is_finite() && average >= low && average <= high);
        checked += 1;
    }
    assert_eq!(checked, 150);
    drop(query);
    assert_eq!(
        database.reserved_memory_bytes(),
        database.path_memory_bytes()
    );
    database.close().unwrap();
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn aggregate_states_match_independent_rational_vectors() {
    let vectors = include_str!("../../../tests/fixtures/aggregate-semantics/rounding.txt");
    let mut scaled_seen = 0;
    let mut resumed = 0;
    let mut mean_seen = 0;
    for (line_number, line) in vectors.lines().enumerate() {
        let mut fields = line.split(';');
        let inputs = fields.next().unwrap();
        let expected_sum = fields.next().unwrap();
        let expected_average = fields.next().unwrap();
        assert!(fields.next().is_none());
        let values: Vec<_> = inputs
            .split(',')
            .map(|raw| f64::from_bits(u64::from_str_radix(raw, 16).unwrap()))
            .collect();
        let mut sum = values[0];
        let mut scaled = false;
        let mut average = values[0];
        let mut is_mean = false;
        for (index, input) in values.iter().enumerate().skip(1) {
            let previous = scaled;
            (sum, scaled) = sum_add(sum, scaled, *input);
            scaled_seen += usize::from(scaled);
            resumed += usize::from(previous && !scaled);
            (average, is_mean) =
                average_add(average, is_mean, *input, u32::try_from(index + 1).unwrap());
            mean_seen += usize::from(is_mean);
        }
        match expected_sum {
            "overflow" => assert!(scaled, "line {line_number}: expected SUM overflow"),
            "nan" => assert!(sum.is_nan() && !scaled, "line {line_number}: expected NaN"),
            raw => {
                assert!(!scaled, "line {line_number}: unexpected SUM overflow");
                assert_eq!(
                    sum.to_bits(),
                    u64::from_str_radix(raw, 16).unwrap(),
                    "line {line_number}"
                );
            }
        }
        if !is_mean {
            average /= f64::from(u32::try_from(values.len()).unwrap());
        }
        let (low, high) = expected_average.split_once(',').unwrap();
        if low == "nan" {
            assert!(average.is_nan(), "line {line_number}");
        } else {
            let low_bits = u64::from_str_radix(low, 16).unwrap();
            let high_bits = u64::from_str_radix(high, 16).unwrap();
            if low_bits == high_bits {
                assert_eq!(average.to_bits(), low_bits, "mean line {line_number}");
            } else {
                assert!(
                    average.is_finite()
                        && average >= f64::from_bits(low_bits)
                        && average <= f64::from_bits(high_bits),
                    "mean line {line_number}"
                );
            }
        }
    }
    assert!(scaled_seen > 0 && resumed > 0 && mean_seen > 0);
    eprintln!(
        "aggregate vectors={} scaled={scaled_seen} resumed={resumed} mean={mean_seen}",
        vectors.lines().count()
    );

    let mut sum = f64::MAX;
    let mut scaled = false;
    let mut average = f64::MAX;
    let mut is_mean = false;
    for count in 2..=u32::try_from(storage_format::MAX_ROWS).unwrap() {
        (sum, scaled) = sum_add(sum, scaled, f64::MAX);
        (average, is_mean) = average_add(average, is_mean, f64::MAX, count);
    }
    assert!(scaled && sum.is_finite());
    assert!(is_mean);
    assert_eq!(average, f64::MAX);
}
