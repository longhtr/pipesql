//! Check CSV allocation refusal and cleanup through the public decoder.
//!
//! Construction needs four bounded buffers. Refuse each allocation in turn,
//! keeping refusal active during error formatting and destruction. With all four
//! buffers admitted, decode valid and invalid input while later allocations are
//! forbidden. Literal values and exact error offsets are independent answers.

use super::workload::{arm, finish, format_error};
use super::{CALLS, PEAK_REQUESTED, PEAK_USABLE, REFUSED};
use pipesql::{
    CancellationToken, ColumnDeclaration, CsvDecoder, CsvLimits, DataType, Error, Value,
};
use std::sync::atomic::Ordering;

pub(super) fn run() {
    let schema = [
        ColumnDeclaration {
            name: "id",
            data_type: DataType::Int64,
            nullable: false,
        },
        ColumnDeclaration {
            name: "txt",
            data_type: DataType::String,
            nullable: true,
        },
        ColumnDeclaration {
            name: "v",
            data_type: DataType::Double,
            nullable: false,
        },
    ];
    cached_buffers(&schema);
    let limits = CsvLimits {
        input_bytes: 1024,
        rows: 10,
        record_bytes: 512,
        field_bytes: 128,
        batch_rows: 2,
        batch_text_bytes: 128,
    };
    let cancel = CancellationToken::new();
    let required = CsvDecoder::<&[u8]>::required_memory(&schema, limits).unwrap();
    for invalid in [false, true] {
        let input = if invalid {
            &b"id,txt,v\n1,one,1.5\n-2,\\N,bad\n"[..]
        } else {
            &b"id,txt,v\n1,\"one\",1.5\n-2,\\N,NaN\n3,\"\",-0\n"[..]
        };
        for prefix in 0..=4 {
            let baseline = arm(Some(prefix), 4);
            let outcome = (|| -> Result<usize, Error> {
                let mut decoder = CsvDecoder::new(input, &schema, limits, required, &cancel)?;
                let mut rows = 0;
                while let Some(batch) = decoder.next_batch(&cancel)? {
                    assert!(
                        !invalid,
                        "invalid second row must not expose the first partial batch"
                    );
                    for row in 0..batch.row_count() {
                        assert_eq!(batch.value(row, 0), Some(Value::Int64([1, -2, 3][rows])));
                        match rows {
                            0 => {
                                assert!(
                                    matches!(batch.value(row, 1), Some(Value::String(value)) if value.as_str() == "one")
                                );
                                assert_eq!(batch.value(row, 2), Some(Value::Double(1.5)));
                            }
                            1 => {
                                assert_eq!(batch.value(row, 1), Some(Value::Null));
                                assert!(
                                    matches!(batch.value(row, 2), Some(Value::Double(value)) if value.is_nan())
                                );
                            }
                            2 => {
                                assert!(
                                    matches!(batch.value(row, 1), Some(Value::String(value)) if value.as_str().is_empty())
                                );
                                assert!(
                                    matches!(batch.value(row, 2), Some(Value::Double(value)) if value.to_bits() == (-0.0f64).to_bits())
                                );
                            }
                            _ => panic!("unexpected row"),
                        }
                        rows += 1;
                    }
                }
                Ok(rows)
            })();
            assert!(format_error(outcome.as_ref().err()));
            if prefix < 4 {
                assert!(matches!(outcome, Err(Error::Resource { .. })));
                assert_eq!(CALLS.load(Ordering::Relaxed), prefix + 1);
                assert_eq!(REFUSED.load(Ordering::Relaxed), 1);
            } else {
                assert_eq!(CALLS.load(Ordering::Relaxed), 4);
                assert_eq!(REFUSED.load(Ordering::Relaxed), 0);
                if invalid {
                    assert!(matches!(
                        outcome,
                        Err(Error::Input {
                            message: "invalid CSV DOUBLE",
                            byte_offset: 25
                        })
                    ));
                } else {
                    assert_eq!(outcome.unwrap(), 3);
                }
            }
            assert!(
                PEAK_REQUESTED.load(Ordering::Relaxed) - baseline.requested <= required as usize
            );
            assert!(PEAK_USABLE.load(Ordering::Relaxed) - baseline.usable <= required as usize);
            finish("CSV", baseline);
        }
    }
    // Exercise the long-number conversion path with refusal still armed. Prepare
    // the input outside accounting; its storage belongs to the caller.
    let text = "雪".repeat(21_845);
    let number = "0".repeat(65_535) + "1";
    let input = format!("id,txt,v\n1,\"{text}\",{number}\n");
    let wide = CsvLimits {
        input_bytes: 200_000,
        record_bytes: 200_000,
        field_bytes: 65_536,
        batch_text_bytes: 65_536,
        ..limits
    };
    let required = CsvDecoder::<&[u8]>::required_memory(&schema, wide).unwrap();
    let baseline = arm(Some(4), 4);
    let mut decoder = CsvDecoder::new(input.as_bytes(), &schema, wide, required, &cancel).unwrap();
    let batch = decoder.next_batch(&cancel).unwrap().unwrap();
    assert_eq!(batch.row_count(), 1);
    assert_eq!(batch.value(0, 0), Some(Value::Int64(1)));
    assert!(matches!(batch.value(0, 1), Some(Value::String(value)) if value.as_str() == text));
    assert_eq!(batch.value(0, 2), Some(Value::Double(1.0)));
    assert!(decoder.next_batch(&cancel).unwrap().is_none());
    drop(decoder);
    assert_eq!(CALLS.load(Ordering::Relaxed), 4);
    assert_eq!(REFUSED.load(Ordering::Relaxed), 0);
    assert!(PEAK_REQUESTED.load(Ordering::Relaxed) - baseline.requested <= required as usize);
    assert!(PEAK_USABLE.load(Ordering::Relaxed) - baseline.usable <= required as usize);
    finish("CSV wide fields", baseline);
    for (record_bytes, batch_text_bytes, batch_rows) in [
        (128, 1, 1),
        (16_383, 16_383, 256),
        (16_384, 16_384, 256),
        (16_385, 16_385, 256),
        (65_536, 65_536, 256),
        (8_388_801, 4_194_304, 256),
    ] {
        let shape = CsvLimits {
            record_bytes,
            batch_text_bytes,
            batch_rows,
            ..limits
        };
        let required = CsvDecoder::<&[u8]>::required_memory(&schema, shape).unwrap();
        let baseline = arm(Some(4), 4);
        let decoder = CsvDecoder::new(&b""[..], &schema, shape, required, &cancel).unwrap();
        assert_eq!(CALLS.load(Ordering::Relaxed), 4);
        assert!(PEAK_REQUESTED.load(Ordering::Relaxed) - baseline.requested <= required as usize);
        assert!(PEAK_USABLE.load(Ordering::Relaxed) - baseline.usable <= required as usize);
        drop(decoder);
        finish("CSV buffer geometry", baseline);
    }
    println!(
        "CSV allocation checks passed: four construction allocations, all refusal prefixes, no decoding allocations"
    );
}

// The raw/decoded pair and text owner can each request large native allocations.
// Check admission and every refusal prefix after a larger block has been freed.
fn cached_buffers(schema: &[ColumnDeclaration<'_>]) {
    let cancel = CancellationToken::new();
    for (record_bytes, batch_text_bytes) in [(3_817_440, 128), (128, 3_817_440)] {
        let limits = CsvLimits {
            input_bytes: 10_000_000,
            rows: 10,
            record_bytes,
            field_bytes: 128,
            batch_rows: 2,
            batch_text_bytes,
        };
        let required = CsvDecoder::<&[u8]>::required_memory(schema, limits).unwrap();
        for prefix in 0..=4 {
            super::cache_allocation(7_503_872);
            let baseline = arm(Some(prefix), 4);
            let outcome = CsvDecoder::new(&b""[..], schema, limits, required, &cancel);
            if prefix < 4 {
                assert!(matches!(outcome, Err(Error::Resource { .. })));
                assert!(format_error(outcome.as_ref().err()));
                assert_eq!(REFUSED.load(Ordering::Relaxed), 1);
            } else {
                assert!(outcome.is_ok());
                assert_eq!(REFUSED.load(Ordering::Relaxed), 0);
            }
            assert_eq!(CALLS.load(Ordering::Relaxed), (prefix + 1).min(4));
            let requested = PEAK_REQUESTED.load(Ordering::Relaxed) - baseline.requested;
            let usable = PEAK_USABLE.load(Ordering::Relaxed) - baseline.usable;
            drop(outcome);
            finish("CSV cached buffers", baseline);
            assert!(requested <= required as usize);
            assert!(
                usable <= required as usize,
                "CSV cached buffers exceeded admission"
            );
        }
        let baseline = arm(Some(0), 4);
        let refused = CsvDecoder::new(&b""[..], schema, limits, required - 1, &cancel);
        assert!(matches!(
            refused,
            Err(Error::Resource {
                owner: "CSV decoder",
                ..
            })
        ));
        assert_eq!(CALLS.load(Ordering::Relaxed), 0);
        drop(refused);
        finish("CSV memory refusal", baseline);
    }
}
