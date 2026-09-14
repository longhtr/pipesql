//! Caller-owned inputs for the event report. Expected answers belong to the caller.
use pipesql::{
    Append, AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues,
    DataType, Database, DateValue, Error,
};

#[derive(Clone, Copy)]
pub struct Event {
    pub id: i64,
    pub dimension: Option<i64>,
    pub day: Option<i32>,
    pub amount: Option<i64>,
    pub measurement: Option<f64>,
}

// Days since 1970-01-01: 1999-12-31, 2000-02-29, 2000-12-31, 2001-01-01.
pub const EVENTS: [Event; 16] = [
    Event {
        id: 0,
        dimension: Some(1),
        day: Some(10_956),
        amount: Some(10),
        measurement: Some(0.5),
    },
    Event {
        id: 1,
        dimension: Some(2),
        day: Some(11_016),
        amount: Some(20),
        measurement: Some(1.5),
    },
    Event {
        id: 2,
        dimension: None,
        day: Some(11_016),
        amount: Some(30),
        measurement: None,
    },
    Event {
        id: 3,
        dimension: Some(99),
        day: Some(11_016),
        amount: None,
        measurement: Some(-0.0),
    },
    Event {
        id: 4,
        dimension: Some(3),
        day: None,
        amount: Some(5),
        measurement: Some(2.0),
    },
    Event {
        id: 5,
        dimension: Some(1),
        day: Some(11_323),
        amount: None,
        measurement: Some(2.5),
    },
    Event {
        id: 6,
        dimension: Some(2),
        day: Some(11_322),
        amount: Some(-5),
        measurement: Some(3.0),
    },
    Event {
        id: 7,
        dimension: Some(1),
        day: None,
        amount: Some(7),
        measurement: Some(-1.0),
    },
    Event {
        id: 8,
        dimension: Some(1),
        day: Some(10_956),
        amount: Some(-2),
        measurement: Some(4.0),
    },
    Event {
        id: 9,
        dimension: Some(2),
        day: Some(11_323),
        amount: Some(40),
        measurement: Some(4.5),
    },
    Event {
        id: 10,
        dimension: Some(99),
        day: Some(11_016),
        amount: Some(3),
        measurement: Some(5.0),
    },
    Event {
        id: 11,
        dimension: None,
        day: None,
        amount: Some(11),
        measurement: None,
    },
    Event {
        id: 12,
        dimension: Some(3),
        day: Some(11_016),
        amount: Some(0),
        measurement: Some(6.0),
    },
    Event {
        id: 13,
        dimension: Some(1),
        day: Some(11_323),
        amount: Some(13),
        measurement: Some(6.5),
    },
    Event {
        id: 14,
        dimension: Some(2),
        day: None,
        amount: None,
        measurement: Some(7.0),
    },
    Event {
        id: 15,
        dimension: Some(3),
        day: Some(11_322),
        amount: Some(9),
        measurement: Some(-2.0),
    },
];

pub fn declare(db: &Database, cancel: &CancellationToken) -> Result<(), Error> {
    db.declare_table(
        "events",
        &[
            ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "dimension_id",
                data_type: DataType::Int64,
                nullable: true,
            },
            ColumnDeclaration {
                name: "happened",
                data_type: DataType::Date,
                nullable: true,
            },
            ColumnDeclaration {
                name: "amount",
                data_type: DataType::Int64,
                nullable: true,
            },
            ColumnDeclaration {
                name: "measurement",
                data_type: DataType::Double,
                nullable: true,
            },
        ],
        cancel,
    )?;
    db.declare_table(
        "dimensions",
        &[
            ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "label",
                data_type: DataType::String,
                nullable: false,
            },
        ],
        cancel,
    )?;
    let mut append = db.begin_append(
        "dimensions",
        AppendLimits {
            batches: 1,
            encoded_bytes: 1024,
        },
        cancel,
    )?;
    append.write(
        &[
            ColumnInput {
                values: ColumnValues::Int64(&[1, 2, 2, 3]),
                validity: &[0b1111],
            },
            ColumnInput {
                values: ColumnValues::String(&["north", "south", "南", ""]),
                validity: &[0b1111],
            },
        ],
        cancel,
    )?;
    append.commit(cancel)?;
    Ok(())
}

/// Write bounded batches into a transaction owned by the caller. Each write
/// borrows these buffers only until it returns; the next chunk reuses them.
/// The caller supplies valid DATE offsets and reserves enough batches/bytes.
pub fn write_events(
    append: &mut Append<'_>,
    events: &[Event],
    cancel: &CancellationToken,
) -> Result<(), Error> {
    const ROWS: usize = 256;
    let mut ids = [0; ROWS];
    let mut dimensions = [0; ROWS];
    let mut dates = [DateValue::from_days_since_unix_epoch(0).expect("epoch"); ROWS];
    let mut amounts = [0; ROWS];
    let mut measurements = [0.0; ROWS];
    for chunk in events.chunks(ROWS) {
        let mut valid = [[0u8; ROWS / 8]; 5];
        for (row, event) in chunk.iter().enumerate() {
            ids[row] = event.id;
            dimensions[row] = event.dimension.unwrap_or(0);
            dates[row] = DateValue::from_days_since_unix_epoch(event.day.unwrap_or(0))
                .expect("fixture date range");
            amounts[row] = event.amount.unwrap_or(0);
            measurements[row] = event.measurement.unwrap_or(0.0);
            for (column, present) in [
                true,
                event.dimension.is_some(),
                event.day.is_some(),
                event.amount.is_some(),
                event.measurement.is_some(),
            ]
            .into_iter()
            .enumerate()
            {
                if present {
                    valid[column][row / 8] |= 1 << (row % 8);
                }
            }
        }
        let rows = chunk.len();
        let bytes = rows.div_ceil(8);
        append.write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&ids[..rows]),
                    validity: &valid[0][..bytes],
                },
                ColumnInput {
                    values: ColumnValues::Int64(&dimensions[..rows]),
                    validity: &valid[1][..bytes],
                },
                ColumnInput {
                    values: ColumnValues::Date(&dates[..rows]),
                    validity: &valid[2][..bytes],
                },
                ColumnInput {
                    values: ColumnValues::Int64(&amounts[..rows]),
                    validity: &valid[3][..bytes],
                },
                ColumnInput {
                    values: ColumnValues::Double(&measurements[..rows]),
                    validity: &valid[4][..bytes],
                },
            ],
            cancel,
        )?;
    }
    Ok(())
}
