//! Decode typed CSV with explicit input and memory limits.
//!
//! The header chooses input order; output uses declaration order. Only reaching
//! EOF confirms the entire input. This example makes no database writes.
//!
//! The in-memory sample distinguishes an unquoted NULL marker from quoted empty
//! text and checks every typed row before accepting EOF. Batch values borrow decoder
//! storage, so the example inspects them before requesting the next batch. Reader,
//! limit or conversion errors propagate from main instead of accepting a prefix.

use pipesql::{CancellationToken, ColumnDeclaration, CsvDecoder, CsvLimits, DataType, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema = [
        ColumnDeclaration {
            name: "id",
            data_type: DataType::Int64,
            nullable: false,
        },
        ColumnDeclaration {
            name: "label",
            data_type: DataType::String,
            nullable: true,
        },
    ];
    let limits = CsvLimits {
        input_bytes: 1_024,
        rows: 10,
        record_bytes: 128,
        field_bytes: 64,
        batch_rows: 2,
        batch_text_bytes: 128,
    };
    let input = b"label,id\n\"hello, world\",1\n\\N,2\n\"\",3\n";
    let cancel = CancellationToken::new();
    let mut decoder = CsvDecoder::new(&input[..], &schema, limits, 131_072, &cancel)?;
    let mut rows = 0;
    while let Some(batch) = decoder.next_batch(&cancel)? {
        for row in 0..batch.row_count() {
            assert_eq!(batch.value(row, 0), Some(Value::Int64(rows + 1)));
            match rows {
                0 => assert!(
                    matches!(batch.value(row, 1), Some(Value::String(value)) if value.as_str() == "hello, world")
                ),
                1 => assert_eq!(batch.value(row, 1), Some(Value::Null)),
                2 => assert!(
                    matches!(batch.value(row, 1), Some(Value::String(value)) if value.as_str().is_empty())
                ),
                _ => panic!("unexpected row"),
            }
            rows += 1;
        }
    }
    assert_eq!(rows, 3);
    println!("Decoded all {rows} rows; NULL and empty text remain distinct.");
    Ok(())
}
