//! Compare scans, selective filters, sorting, joins and analytic replay on controlled inputs.
//!
//! Pass a fresh absolute database path, operation (`scan`, `order`, `order-text`,
//! `filter-none`, `filter-sparse`, `filter-half`, `filter-all`, `distinct`, `join`,
//! `join-many`, `window-count` or `window-sum`), rows (8192 or 65536), keys (32 or
//! 4096), dimension text bytes (8 or 1024), memory bytes, and optional fact batch rows
//! (64, 512 or 4096; default 64).
//! Facts arrive in descending ID order. Each key has one dimension row, or eight
//! for `join-many`; the text identifies each distinct right-side match. Every
//! result value, identity, multiplicity and schema is checked, without assuming
//! join or DISTINCT order.
//! `order-text` sorts the dimension payloads in descending text order. Windows
//! count each key's partition or sum amounts through the current key's peer group.
//! Filters select no IDs, zero amounts (about 1%), negative amounts (about half),
//! or all IDs. Expected subsets and totals come from input identities before timing.
//!
//! `scan-payload`, `order-payload`, `window-count-payload` and `window-sum-payload`
//! retain fact text with every result. Here text width describes the fact payload,
//! and fact batches must contain 64 rows. Wide text includes UTF-8 and NUL bytes.
//! These profiles allow 256 MB of temporary space because merge can retain two
//! roughly 70 MB files for 65,536 rows; ordinary profiles keep their 128 MB limit.
//!
//! `order-many-text` sorts the same eight-match dimension input as `join-many`.
//! Joins and dimension ordering also accept 2,048-byte text. Eight matches then
//! cross the join's initial 16 KiB replay hint; the following key is needed too.
//! This new width allows 256 MB of temporary space because two roughly 69 MB
//! dimension files can overlap during merge. Existing widths retain their limits.
//!
//! Computed cases filter a derived amount after sorting or joining, then emit it
//! and its double. Identity-derived expectations check both values and the subset.
//!
//! Fact batch size controls immutable storage unit rows. Dimensions retain
//! 64-row input batches. Output batch sizes depend on the operator and text
//! capacity; samples report their observed count. Input construction uses a
//! fixed memory allowance.
//! One checked warm-up precedes five measured executions of one prepared query.
//! Timings include validation and cursor disposal, excluding setup, preparation,
//! the warm-up spill probe and a later cancellation/replay check. No cache eviction
//! is attempted. Reservation samples are not process memory or guaranteed peaks.
//! On Linux the warm-up also requires nonempty, unlinked scratch files for the
//! blocking operations and checks that dropping the cursor closes those files.
//!
//! Run `cargo run --release --offline --locked -j 1 --example operator_cost --
//! /absolute/new-operators distinct 65536 4096 8 8000000`. The caller owns the
//! remaining database. Require successful exit and the final completion line.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, PreparedQuery, QueryStep, Value,
};
use std::path::Path;
use std::time::Instant;

#[path = "support/query.rs"]
mod query_support;
use query_support::{cancel, released, scratch_usage};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const BATCH: usize = 64;
const TEMP: u64 = 128_000_000;
const PAYLOAD_TEMP: u64 = 256_000_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Scan,
    FilterNone,
    FilterSparse,
    FilterHalf,
    FilterAll,
    Order,
    OrderComputed,
    TextOrder,
    TextOrderMany,
    Distinct,
    Join,
    JoinMany,
    JoinComputed,
    JoinManyComputed,
    WindowCount,
    WindowSum,
    ScanPayload,
    OrderPayload,
    WindowCountPayload,
    WindowSumPayload,
}

impl Operation {
    fn scanning(self) -> bool {
        matches!(
            self,
            Self::Scan
                | Self::ScanPayload
                | Self::FilterNone
                | Self::FilterSparse
                | Self::FilterHalf
                | Self::FilterAll
        )
    }

    fn payload(self) -> bool {
        matches!(
            self,
            Self::ScanPayload
                | Self::OrderPayload
                | Self::WindowCountPayload
                | Self::WindowSumPayload
        )
    }

    fn text_ordering(self) -> bool {
        matches!(self, Self::TextOrder | Self::TextOrderMany)
    }

    fn computed(self) -> bool {
        matches!(
            self,
            Self::OrderComputed | Self::JoinComputed | Self::JoinManyComputed
        )
    }

    fn joining(self) -> bool {
        matches!(
            self,
            Self::Join | Self::JoinMany | Self::JoinComputed | Self::JoinManyComputed
        )
    }

    // Input identities define the predicate answers without consulting a query.
    fn selected(self, id: usize) -> bool {
        match self {
            Self::FilterNone => false,
            Self::FilterSparse => id % 101 == 50,
            Self::FilterHalf => id % 101 < 50,
            Self::OrderComputed | Self::JoinComputed | Self::JoinManyComputed => id % 101 >= 49,
            _ => true,
        }
    }

    fn right_matches(self) -> usize {
        if matches!(
            self,
            Self::JoinMany | Self::JoinManyComputed | Self::TextOrderMany
        ) {
            8
        } else {
            1
        }
    }

    fn sql(self) -> &'static str {
        match self {
            Self::Scan => "FROM facts |> SELECT id, amount",
            Self::ScanPayload => "FROM facts |> SELECT id, amount, payload",
            Self::FilterNone => "FROM facts |> WHERE amount < -50 |> SELECT id, amount",
            Self::FilterSparse => "FROM facts |> WHERE amount = 0 |> SELECT id, amount",
            Self::FilterHalf => "FROM facts |> WHERE amount < 0 |> SELECT id, amount",
            Self::FilterAll => "FROM facts |> WHERE amount <= 50 |> SELECT id, amount",
            Self::Order => "FROM facts |> SELECT id, amount |> ORDER BY id",
            Self::OrderPayload => "FROM facts |> SELECT id, amount, payload |> ORDER BY id",
            Self::OrderComputed => {
                "FROM facts |> ORDER BY id |> EXTEND amount + 1 AS adjusted |> WHERE adjusted >= 0 |> SELECT id, adjusted, adjusted * 2 AS doubled"
            }
            Self::JoinComputed | Self::JoinManyComputed => {
                "FROM facts AS f |> JOIN dimensions AS d ON f.k = d.k |> EXTEND f.amount + 1 AS adjusted |> WHERE adjusted >= 0 |> SELECT f.id AS id, d.payload AS payload, adjusted, adjusted * 2 AS doubled"
            }
            Self::TextOrder | Self::TextOrderMany => {
                "FROM dimensions |> SELECT payload |> ORDER BY payload DESC"
            }
            Self::Distinct => "FROM facts |> SELECT k |> DISTINCT",
            Self::WindowCount => {
                "FROM facts |> EXTEND COUNT(*) OVER (PARTITION BY k) AS count |> SELECT id, count"
            }
            Self::WindowSum => {
                "FROM facts |> EXTEND SUM(amount) OVER (ORDER BY k) AS total |> SELECT id, total"
            }
            Self::WindowCountPayload => {
                "FROM facts |> EXTEND COUNT(*) OVER (PARTITION BY k) AS count |> SELECT id, count, payload"
            }
            Self::WindowSumPayload => {
                "FROM facts |> EXTEND SUM(amount) OVER (ORDER BY k) AS total |> SELECT id, total, payload"
            }
            Self::Join | Self::JoinMany => {
                "FROM facts AS f |> JOIN dimensions AS d ON f.k = d.k |> SELECT f.id AS id, d.payload AS payload"
            }
        }
    }
}

struct Input {
    rows: usize,
    keys: usize,
    width: usize,
}

impl Input {
    fn temporary(&self, payload: bool) -> u64 {
        if payload || self.width == 2048 {
            PAYLOAD_TEMP
        } else {
            TEMP
        }
    }
}

fn create(
    path: &Path,
    input: &Input,
    matches: usize,
    fact_batch: usize,
    payload: bool,
) -> Result<()> {
    let cancel = CancellationToken::new();
    let db = Database::create_empty(path, Config::new(128_000_000, input.temporary(payload))?)?;
    let mut columns: Vec<_> = ["id", "k", "amount"]
        .map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: false,
        })
        .into_iter()
        .collect();
    if payload {
        columns.push(ColumnDeclaration {
            name: "payload",
            data_type: DataType::String,
            nullable: false,
        });
    }
    db.declare_table("facts", &columns, &cancel)?;
    db.declare_table(
        "dimensions",
        &[
            ColumnDeclaration {
                name: "k",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "payload",
                data_type: DataType::String,
                nullable: false,
            },
        ],
        &cancel,
    )?;
    let mut append = db.begin_append(
        "facts",
        AppendLimits {
            batches: input.rows.div_ceil(fact_batch) as u32,
            // A 64-row unit has fixed values, string offsets and headers in
            // addition to text. This fixture's 64-byte allowance per row bounds
            // those bytes; it is an append limit, not an engine memory charge.
            encoded_bytes: if payload {
                input.rows as u64 * (input.width as u64 + 64)
            } else {
                8_000_000
            },
        },
        &cancel,
    )?;
    for start in (0..input.rows).step_by(fact_batch) {
        let count = fact_batch.min(input.rows - start);
        let ids: Vec<_> = (0..count)
            .map(|i| (input.rows - 1 - start - i) as i64)
            .collect();
        let keys: Vec<_> = ids.iter().map(|id| id % input.keys as i64).collect();
        let amounts: Vec<_> = ids.iter().map(|id| id % 101 - 50).collect();
        let validity = vec![255; count.div_ceil(8)];
        let texts: Vec<_> = if payload {
            ids.iter()
                .map(|id| {
                    if input.width == 8 {
                        format!("{id:08x}")
                    } else {
                        format!("{id:08x}雪\0{}", "x".repeat(input.width - 12))
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        let borrowed: Vec<_> = texts.iter().map(String::as_str).collect();
        let mut columns: Vec<_> = [&ids, &keys, &amounts]
            .map(|values| ColumnInput {
                values: ColumnValues::Int64(values),
                validity: &validity,
            })
            .into_iter()
            .collect();
        if payload {
            columns.push(ColumnInput {
                values: ColumnValues::String(&borrowed),
                validity: &validity,
            });
        }
        append.write(&columns, &cancel)?;
    }
    append.commit(&cancel)?;
    let dimension_rows = input.keys * matches;
    let mut append = db.begin_append(
        "dimensions",
        AppendLimits {
            batches: dimension_rows.div_ceil(BATCH) as u32,
            // New wide dimensions can exceed the old fixed 8 MB per match.
            // The 64-byte allowance covers keys, offsets and unit headers.
            encoded_bytes: if input.width == 2048 {
                dimension_rows as u64 * (input.width as u64 + 64)
            } else {
                8_000_000 * matches as u64
            },
        },
        &cancel,
    )?;
    for start in (0..dimension_rows).step_by(BATCH) {
        let count = BATCH.min(dimension_rows - start);
        let keys: Vec<_> = (start..start + count)
            .map(|id| (id / matches) as i64)
            .collect();
        let texts: Vec<_> = (start..start + count)
            .map(|id| format!("{id:08x}{}", "x".repeat(input.width - 8)))
            .collect();
        let borrowed: Vec<_> = texts.iter().map(String::as_str).collect();
        let validity = &([255_u8; BATCH / 8])[..count / 8];
        append.write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&keys),
                    validity,
                },
                ColumnInput {
                    values: ColumnValues::String(&borrowed),
                    validity,
                },
            ],
            &cancel,
        )?;
    }
    append.commit(&cancel)?;
    db.close()?;
    Ok(())
}

struct Sample {
    ns: u128,
    rows: usize,
    batches: u64,
    progress: u64,
    memory: u64,
    temporary: u64,
    scratch: u64,
}

fn text_identity(text: &str, input: &Input) -> Result<usize> {
    let key = text
        .get(..8)
        .filter(|s| {
            s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .and_then(|s| usize::from_str_radix(s, 16).ok());
    if text.len() != input.width
        || key.is_none()
        || !text.as_bytes()[8..].iter().all(|b| *b == b'x')
    {
        return Err("incorrect dimension text".into());
    }
    Ok(key.expect("checked text identity"))
}

fn payload_identity(text: &str, width: usize) -> Result<usize> {
    let head = text.get(..8).ok_or("incorrect fact text")?;
    if text.len() != width
        || !head
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("incorrect fact text".into());
    }
    let tail = &text.as_bytes()[8..];
    if width != 8 && (!tail.starts_with("雪\0".as_bytes()) || !tail[4..].iter().all(|b| *b == b'x'))
    {
        return Err("incorrect fact text".into());
    }
    Ok(usize::from_str_radix(head, 16)?)
}

// The fixture assigns id % keys to a key and id % 101 - 50 to its amount.
// Aggregate those identities directly, then include every equal-key peer before
// moving to the next key. This answer does not depend on engine output order.
fn frame_totals(input: &Input) -> Vec<i64> {
    let mut totals = vec![0; input.keys];
    for id in 0..input.rows {
        totals[id % input.keys] += id as i64 % 101 - 50;
    }
    for key in 1..input.keys {
        totals[key] += totals[key - 1];
    }
    totals
}

fn execute(
    db: &Database,
    query: &PreparedQuery<'_>,
    operation: Operation,
    input: &Input,
    path: &Path,
    observe: bool,
) -> Result<Sample> {
    let columns: &[(&str, DataType, bool)] = match operation {
        Operation::Distinct => &[("k", DataType::Int64, false)],
        Operation::TextOrder | Operation::TextOrderMany => &[("payload", DataType::String, false)],
        Operation::Join | Operation::JoinMany => &[
            ("id", DataType::Int64, false),
            ("payload", DataType::String, false),
        ],
        Operation::OrderComputed => &[
            ("id", DataType::Int64, false),
            ("adjusted", DataType::Int64, false),
            ("doubled", DataType::Int64, false),
        ],
        Operation::JoinComputed | Operation::JoinManyComputed => &[
            ("id", DataType::Int64, false),
            ("payload", DataType::String, false),
            ("adjusted", DataType::Int64, false),
            ("doubled", DataType::Int64, false),
        ],
        Operation::WindowCount => &[
            ("id", DataType::Int64, false),
            ("count", DataType::Int64, false),
        ],
        Operation::WindowSum => &[
            ("id", DataType::Int64, false),
            ("total", DataType::Int64, true),
        ],
        Operation::WindowCountPayload => &[
            ("id", DataType::Int64, false),
            ("count", DataType::Int64, false),
            ("payload", DataType::String, false),
        ],
        Operation::WindowSumPayload => &[
            ("id", DataType::Int64, false),
            ("total", DataType::Int64, true),
            ("payload", DataType::String, false),
        ],
        Operation::ScanPayload | Operation::OrderPayload => &[
            ("id", DataType::Int64, false),
            ("amount", DataType::Int64, false),
            ("payload", DataType::String, false),
        ],
        _ => &[
            ("id", DataType::Int64, false),
            ("amount", DataType::Int64, false),
        ],
    };
    if query.result_column_count() != columns.len()
        || columns
            .iter()
            .enumerate()
            .any(|(i, (name, kind, nullable))| {
                query.result_column(i).is_none_or(|column| {
                    column.name != Some(*name)
                        || column.data_type != *kind
                        || column.nullable != *nullable
                })
            })
    {
        return Err("unexpected operator result schema".into());
    }
    let totals = if matches!(
        operation,
        Operation::WindowSum | Operation::WindowSumPayload
    ) {
        frame_totals(input)
    } else {
        Vec::new()
    };
    let expected_rows = if operation == Operation::Distinct || operation.text_ordering() {
        input.keys * operation.right_matches()
    } else if operation.scanning() || operation.computed() {
        (0..input.rows).filter(|&id| operation.selected(id)).count() * operation.right_matches()
    } else {
        input.rows * operation.right_matches()
    };
    let partition_rows = input.rows / input.keys;
    let extra_keys = input.rows % input.keys;
    let mut seen = vec![
        false;
        if operation.scanning() || operation.computed() {
            input.rows * operation.right_matches()
        } else {
            expected_rows
        }
    ];
    let mut previous_id = None;
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let start = Instant::now();
    let mut result = db.execute(query, &cancel)?;
    let mut sample = Sample {
        ns: 0,
        rows: 0,
        batches: 0,
        progress: 0,
        memory: db.reserved_memory_bytes(),
        temporary: db.reserved_temp_bytes(),
        scratch: 0,
    };
    let mut finished = false;
    for _ in 0..20_000_000 {
        match result.step() {
            QueryStep::Progress => sample.progress += 1,
            QueryStep::Rows(batch) => {
                sample.batches += 1;
                if batch.column_count() != columns.len() {
                    return Err("unexpected result width".into());
                }
                for row in 0..batch.len() {
                    let id = match (operation, batch.value(row, 0)) {
                        (_, Some(Value::String(text))) if operation.text_ordering() => {
                            text_identity(text.as_str(), input)?
                        }
                        (_, Some(Value::Int64(id))) if !operation.text_ordering() => {
                            usize::try_from(id)?
                        }
                        _ => return Err("missing typed identity".into()),
                    };
                    let joins = operation.joining();
                    if id
                        >= if joins || operation.scanning() || operation.computed() {
                            input.rows
                        } else {
                            expected_rows
                        }
                    {
                        return Err("extra or duplicate identity".into());
                    }
                    if operation.computed() {
                        if !operation.selected(id) {
                            return Err("incorrect computed predicate subset".into());
                        }
                        let adjusted = id as i64 % 101 - 49;
                        let column = if joins { 2 } else { 1 };
                        if batch.value(row, column) != Some(Value::Int64(adjusted))
                            || batch.value(row, column + 1) != Some(Value::Int64(adjusted * 2))
                        {
                            return Err("incorrect computed output".into());
                        }
                    }
                    let mut identity = id;
                    match operation {
                        Operation::Scan
                        | Operation::FilterNone
                        | Operation::FilterSparse
                        | Operation::FilterHalf
                        | Operation::FilterAll
                        | Operation::Order
                        | Operation::ScanPayload
                        | Operation::OrderPayload => {
                            if !operation.selected(id) {
                                return Err("incorrect predicate subset".into());
                            }
                            if (matches!(operation, Operation::Order | Operation::OrderPayload)
                                && id != sample.rows)
                                || batch.value(row, 1) != Some(Value::Int64(id as i64 % 101 - 50))
                            {
                                return Err("incorrect ordered row or amount".into());
                            }
                        }
                        Operation::OrderComputed => {
                            if previous_id.is_some_and(|previous| previous >= id) {
                                return Err("incorrect computed row order".into());
                            }
                            previous_id = Some(id);
                        }
                        Operation::Join
                        | Operation::JoinMany
                        | Operation::JoinComputed
                        | Operation::JoinManyComputed => {
                            let Some(Value::String(text)) = batch.value(row, 1) else {
                                return Err("missing dimension text".into());
                            };
                            let right = text_identity(text.as_str(), input)?;
                            let matches = operation.right_matches();
                            if right / matches != id % input.keys {
                                return Err("wrong dimension text for fact identity".into());
                            }
                            identity = id * matches + right % matches;
                        }
                        Operation::TextOrder | Operation::TextOrderMany => {
                            if Some(id) != expected_rows.checked_sub(sample.rows + 1) {
                                return Err("incorrect descending text order".into());
                            }
                        }
                        Operation::WindowCount | Operation::WindowCountPayload => {
                            if batch.value(row, 1)
                                != Some(Value::Int64(
                                    (partition_rows
                                        + usize::from(
                                            extra_keys != 0 && id % input.keys < extra_keys,
                                        )) as i64,
                                ))
                            {
                                return Err("incorrect window partition count".into());
                            }
                        }
                        Operation::WindowSum | Operation::WindowSumPayload => {
                            if batch.value(row, 1) != Some(Value::Int64(totals[id % input.keys])) {
                                return Err("incorrect peer-inclusive window sum".into());
                            }
                        }
                        Operation::Distinct => (),
                    }
                    if operation.payload() {
                        let Some(Value::String(text)) = batch.value(row, 2) else {
                            return Err("missing fact text".into());
                        };
                        if payload_identity(text.as_str(), input.width)? != id {
                            return Err("wrong fact text for identity".into());
                        }
                    }
                    if seen[identity] {
                        return Err("extra or duplicate identity".into());
                    }
                    seen[identity] = true;
                    sample.rows += 1;
                }
            }
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(_) => {
                return Err(result.into_error().ok_or("missing query error")?.into());
            }
        }
        sample.memory = sample.memory.max(db.reserved_memory_bytes());
        sample.temporary = sample.temporary.max(db.reserved_temp_bytes());
        if operation.scanning() && sample.temporary != 0 {
            return Err("scan reserved temporary storage".into());
        }
        if observe && operation.scanning() && scratch_usage(path)?.0 != 0 {
            return Err("scan retained scratch".into());
        }
        if observe && sample.temporary > 0 && sample.scratch == 0 {
            sample.scratch = scratch_usage(path)?.1;
        }
    }
    if !finished
        || sample.rows != expected_rows
        || seen.iter().enumerate().any(|(identity, &seen)| {
            seen != operation.selected(identity / operation.right_matches())
        })
    {
        return Err("incomplete operator result".into());
    }
    drop(result);
    sample.ns = start.elapsed().as_nanos();
    released(db, baseline, path)?;
    if !operation.scanning()
        && (sample.temporary == 0 || (observe && cfg!(target_os = "linux") && sample.scratch == 0))
    {
        return Err("blocking operator did not write scratch".into());
    }
    Ok(sample)
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !matches!(args.len(), 6 | 7) {
        return Err("expected fresh path, operation, rows, keys, text bytes, memory bytes, optional fact batch rows".into());
    }
    let path = Path::new(&args[0]);
    if !path.is_absolute() || path.try_exists()? {
        return Err("expected a fresh absolute database path".into());
    }
    let operation = match args[1].as_str() {
        "scan" => Operation::Scan,
        "filter-none" => Operation::FilterNone,
        "filter-sparse" => Operation::FilterSparse,
        "filter-half" => Operation::FilterHalf,
        "filter-all" => Operation::FilterAll,
        "order" => Operation::Order,
        "order-computed" => Operation::OrderComputed,
        "order-text" => Operation::TextOrder,
        "order-many-text" => Operation::TextOrderMany,
        "distinct" => Operation::Distinct,
        "join" => Operation::Join,
        "join-many" => Operation::JoinMany,
        "join-computed" => Operation::JoinComputed,
        "join-many-computed" => Operation::JoinManyComputed,
        "window-count" => Operation::WindowCount,
        "window-sum" => Operation::WindowSum,
        "scan-payload" => Operation::ScanPayload,
        "order-payload" => Operation::OrderPayload,
        "window-count-payload" => Operation::WindowCountPayload,
        "window-sum-payload" => Operation::WindowSumPayload,
        _ => {
            return Err(
                "choose scan, filter-none, filter-sparse, filter-half, filter-all, order, order-computed, order-text, order-many-text, distinct, join, join-many, join-computed, join-many-computed, window-count, window-sum, scan-payload, order-payload, window-count-payload or window-sum-payload".into(),
            );
        }
    };
    let input = Input {
        rows: args[2].parse()?,
        keys: args[3].parse()?,
        width: args[4].parse()?,
    };
    let memory = args[5].parse()?;
    let fact_batch = args
        .get(6)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(BATCH);
    if !matches!(fact_batch, 64 | 512 | 4096) {
        return Err("unsupported fact batch rows".into());
    }
    if operation.payload() && fact_batch != BATCH {
        return Err("retained fact text requires 64-row input batches".into());
    }
    let supported = if operation.payload() {
        matches!(input.rows, 8192 | 8320 | 12288 | 65536) && matches!(input.keys, 32 | 256 | 4096)
    } else {
        matches!(input.rows, 8192 | 65536) && matches!(input.keys, 32 | 4096)
    };
    let supported_width = matches!(input.width, 8 | 1024)
        || (input.width == 2048 && (operation.joining() || operation.text_ordering()));
    if !supported || !supported_width {
        return Err("unsupported input dimensions".into());
    }
    let config = Config::new(memory, input.temporary(operation.payload()))?;
    create(
        path,
        &input,
        operation.right_matches(),
        fact_batch,
        operation.payload(),
    )?;
    let db = Database::open(path, config)?;
    let resident = db.reserved_memory_bytes();
    let query = db.prepare(operation.sql())?;
    let warmup = execute(&db, &query, operation, &input, path, true)?;
    println!(
        "input operation={} rows={} keys={} text_bytes={} memory_limit={} dimension_batch_rows={BATCH} fact_batch_rows={fact_batch}",
        args[1], input.rows, input.keys, input.width, memory
    );
    println!(
        "warmup scratch_bytes={} file_probe={}",
        warmup.scratch,
        cfg!(target_os = "linux")
    );
    for sample in 0..5 {
        let value = execute(&db, &query, operation, &input, path, false)?;
        println!(
            "sample={sample} elapsed_ns={} rows={} batches={} progress={} memory={} temporary={}",
            value.ns, value.rows, value.batches, value.progress, value.memory, value.temporary
        );
    }
    if !operation.scanning() {
        cancel(&db, &query, path)?;
        execute(&db, &query, operation, &input, path, false)?;
    }
    drop(query);
    released(&db, resident, path)?;
    db.close()?;
    println!("status=finished");
    Ok(())
}

// The native census driver supplies observation callbacks. Share this fixture
// and checked execution with timing so call/byte measurements use the same rows;
// observation remains outside the ordinary example's measured samples.
#[allow(dead_code)] // Called by the native driver that includes this example.
pub(crate) fn census(
    path: &Path,
    arguments: &[String],
    begin: impl FnOnce(),
    end: impl FnOnce(),
) -> Result<u64> {
    let [name, rows, keys, width, memory] = arguments else {
        return Err("expected census operation, rows, keys, width and memory".into());
    };
    let operation = match name.as_str() {
        "window-count-payload" => Operation::WindowCountPayload,
        "window-sum-payload" => Operation::WindowSumPayload,
        "scan-payload" => Operation::ScanPayload,
        "order-payload" => Operation::OrderPayload,
        "join" => Operation::Join,
        "join-many" => Operation::JoinMany,
        "order-text" => Operation::TextOrder,
        "order-many-text" => Operation::TextOrderMany,
        "scan" => Operation::Scan,
        _ => return Err("unknown checked census operation".into()),
    };
    let input = Input {
        rows: rows.parse()?,
        keys: keys.parse()?,
        width: width.parse()?,
    };
    let supported = if operation.payload() {
        matches!(input.rows, 8192 | 8320 | 12288)
            && matches!(input.keys, 32 | 256 | 4096)
            && matches!(input.width, 8 | 1024)
            && matches!(memory.as_str(), "4000000" | "12000000")
    } else {
        input.rows == 8192
            && matches!(input.keys, 32 | 4096)
            && matches!(input.width, 8 | 1024 | 2048)
            && matches!(memory.as_str(), "2600000" | "12000000")
            && (operation != Operation::Scan
                || (input.width == 8 && input.keys == 32 && memory == "12000000"))
    };
    if !supported {
        return Err("unsupported census dimensions".into());
    }
    create(
        path,
        &input,
        operation.right_matches(),
        BATCH,
        operation.payload(),
    )?;
    let db = Database::open(
        path,
        Config::new(memory.parse()?, input.temporary(operation.payload()))?,
    )?;
    let query = db.prepare(operation.sql())?;
    begin();
    let sample = execute(&db, &query, operation, &input, path, true);
    end();
    let sample = sample?;
    drop(query);
    db.close()?;
    Ok(sample.rows as u64)
}

#[cfg(test)]
#[path = "../test/support/mod.rs"]
pub(crate) mod support;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_join_replay_and_matching_order_reject_wrong_complete_answers() {
        for matches in [1, 8] {
            let directory = support::Directory::new();
            let path = directory.0.join("wide");
            let input = Input {
                rows: 8192,
                keys: 32,
                width: 2048,
            };
            create(&path, &input, matches, BATCH, false).unwrap();
            let db = Database::open(&path, Config::new(12_000_000, PAYLOAD_TEMP).unwrap()).unwrap();
            let join = if matches == 1 {
                Operation::Join
            } else {
                Operation::JoinMany
            };
            let order = if matches == 1 {
                Operation::TextOrder
            } else {
                Operation::TextOrderMany
            };
            for operation in [join, order] {
                let query = db.prepare(operation.sql()).unwrap();
                assert_eq!(
                    execute(&db, &query, operation, &input, &path, true)
                        .unwrap()
                        .rows,
                    if operation.text_ordering() {
                        32 * matches
                    } else {
                        8192 * matches
                    }
                );
                cancel(&db, &query, &path).unwrap();
                execute(&db, &query, operation, &input, &path, false).unwrap();
                let mut wrong = vec![format!("{} |> LIMIT 1", operation.sql())];
                if operation.text_ordering() {
                    wrong.push(operation.sql().replace("DESC", "ASC"));
                } else {
                    wrong.push(operation.sql().replace("f.id AS id", "f.id + 1 AS id"));
                    wrong.push(operation.sql().replace("f.id AS id", "f.k AS id"));
                }
                for sql in wrong {
                    let query = db.prepare(&sql).unwrap();
                    let before = db.reserved_memory_bytes();
                    assert!(execute(&db, &query, operation, &input, &path, false).is_err());
                    released(&db, before, &path).unwrap();
                }
            }
            let correct = format!("00000001{}", "x".repeat(2040));
            assert_eq!(text_identity(&correct, &input).unwrap(), 1);
            let mut wrong = correct.into_bytes();
            wrong[2047] = b'y';
            assert!(text_identity(std::str::from_utf8(&wrong).unwrap(), &input).is_err());
            db.close().unwrap();
        }
    }

    #[test]
    fn retained_text_windows_require_complete_values_and_release_failed_checks() {
        let directory = support::Directory::new();
        let path = directory.0.join("payloads");
        let input = Input {
            rows: 8192,
            keys: 32,
            width: 1024,
        };
        create(&path, &input, 1, BATCH, true).unwrap();
        let db = Database::open(&path, Config::new(4_000_000, PAYLOAD_TEMP).unwrap()).unwrap();
        for operation in [
            Operation::ScanPayload,
            Operation::OrderPayload,
            Operation::WindowCountPayload,
            Operation::WindowSumPayload,
        ] {
            let query = db.prepare(operation.sql()).unwrap();
            execute(&db, &query, operation, &input, &path, true).unwrap();
            if !operation.scanning() {
                cancel(&db, &query, &path).unwrap();
                execute(&db, &query, operation, &input, &path, false).unwrap();
            }
            for (wrong, diagnostic) in [
                (
                    operation.sql().replace(", payload", ", 'wrong' AS payload"),
                    "incorrect fact text",
                ),
                (
                    operation.sql().replace("SELECT id,", "SELECT k AS id,"),
                    if matches!(operation, Operation::ScanPayload | Operation::OrderPayload) {
                        "incorrect ordered row or amount"
                    } else {
                        "wrong fact text for identity"
                    },
                ),
                (
                    format!("{} |> LIMIT 8191", operation.sql()),
                    "incomplete operator result",
                ),
            ] {
                let query = db.prepare(&wrong).unwrap();
                let baseline = db.reserved_memory_bytes();
                assert_eq!(
                    execute(&db, &query, operation, &input, &path, false)
                        .err()
                        .unwrap()
                        .to_string(),
                    diagnostic
                );
                released(&db, baseline, &path).unwrap();
            }
            if matches!(
                operation,
                Operation::WindowCountPayload | Operation::WindowSumPayload
            ) {
                let name = if operation == Operation::WindowCountPayload {
                    "count"
                } else {
                    "total"
                };
                let wrong = operation.sql().replace(
                    &format!("SELECT id, {name},"),
                    &format!("SELECT id, {name} + 1 AS {name},"),
                );
                let query = db.prepare(&wrong).unwrap();
                let baseline = db.reserved_memory_bytes();
                let expected = if name == "count" {
                    "incorrect window partition count"
                } else {
                    "incorrect peer-inclusive window sum"
                };
                assert_eq!(
                    execute(&db, &query, operation, &input, &path, false)
                        .err()
                        .unwrap()
                        .to_string(),
                    expected
                );
                released(&db, baseline, &path).unwrap();
            }
        }
        let correct = format!("00000001雪\0{}", "x".repeat(1012));
        assert_eq!(payload_identity(&correct, 1024).unwrap(), 1);
        let mut wrong = correct.into_bytes();
        wrong[1023] = b'y';
        assert!(payload_identity(std::str::from_utf8(&wrong).unwrap(), 1024).is_err());
        wrong[1023] = b'x';
        wrong[11] = b'x';
        assert!(payload_identity(std::str::from_utf8(&wrong).unwrap(), 1024).is_err());
        db.close().unwrap();
    }

    #[test]
    fn computed_answers_require_selected_values_and_all_join_pairs() {
        for matches in [1, 8] {
            let directory = support::Directory::new();
            let path = directory.0.join("db");
            let input = Input {
                rows: 8192,
                keys: 32,
                width: 8,
            };
            create(&path, &input, matches, BATCH, false).unwrap();
            let db = Database::open(&path, Config::new(4_000_000, TEMP).unwrap()).unwrap();
            let join = if matches == 1 {
                Operation::JoinComputed
            } else {
                Operation::JoinManyComputed
            };
            for operation in [Operation::OrderComputed, join] {
                let query = db.prepare(operation.sql()).unwrap();
                execute(&db, &query, operation, &input, &path, true).unwrap();
                cancel(&db, &query, &path).unwrap();
                execute(&db, &query, operation, &input, &path, false).unwrap();
                for (wrong, diagnostic) in [
                    (
                        operation.sql().replace("adjusted * 2", "adjusted * 3"),
                        "incorrect computed output",
                    ),
                    (
                        operation.sql().replace("adjusted >= 0", "adjusted > 0"),
                        "incomplete operator result",
                    ),
                    (
                        operation.sql().replace("adjusted >= 0", "adjusted >= -1"),
                        "incorrect computed predicate subset",
                    ),
                    (
                        format!("{} |> LIMIT 1", operation.sql()),
                        "incomplete operator result",
                    ),
                ] {
                    let query = db.prepare(&wrong).unwrap();
                    let baseline = db.reserved_memory_bytes();
                    assert_eq!(
                        execute(&db, &query, operation, &input, &path, false)
                            .err()
                            .unwrap()
                            .to_string(),
                        diagnostic
                    );
                    released(&db, baseline, &path).unwrap();
                }
                if operation == Operation::OrderComputed {
                    let query = db
                        .prepare(&operation.sql().replace("ORDER BY id", "ORDER BY id DESC"))
                        .unwrap();
                    let baseline = db.reserved_memory_bytes();
                    assert_eq!(
                        execute(&db, &query, operation, &input, &path, false)
                            .err()
                            .unwrap()
                            .to_string(),
                        "incorrect computed row order"
                    );
                    released(&db, baseline, &path).unwrap();
                }
            }
            db.close().unwrap();
        }
    }

    #[test]
    fn scan_answers_cross_fact_unit_boundaries() {
        for fact_batch in [64, 512, 4096] {
            let directory = support::Directory::new();
            let path = directory.0.join("db");
            let input = Input {
                rows: 8192,
                keys: 32,
                width: 8,
            };
            create(&path, &input, 1, fact_batch, false).unwrap();
            let db = Database::open(&path, Config::new(8_000_000, TEMP).unwrap()).unwrap();
            for operation in [
                Operation::Scan,
                Operation::FilterNone,
                Operation::FilterSparse,
                Operation::FilterHalf,
                Operation::FilterAll,
            ] {
                let query = db.prepare(operation.sql()).unwrap();
                execute(&db, &query, operation, &input, &path, true).unwrap();
            }
            // Omit either side of a physical unit boundary while preserving the
            // schema and every remaining value. Completion must still fail.
            for id in [input.rows - fact_batch - 1, input.rows - fact_batch] {
                let query = db
                    .prepare(&format!(
                        "FROM facts |> WHERE id != {id} |> SELECT id, amount"
                    ))
                    .unwrap();
                let baseline = db.reserved_memory_bytes();
                assert_eq!(
                    execute(&db, &query, Operation::Scan, &input, &path, false)
                        .err()
                        .unwrap()
                        .to_string(),
                    "incomplete operator result"
                );
                released(&db, baseline, &path).unwrap();
            }
            db.close().unwrap();
        }
    }

    #[test]
    fn complete_typed_answers_and_spill_cleanup() {
        let directory = support::Directory::new();
        let path = directory.0.join("db");
        let input = Input {
            rows: 8192,
            keys: 32,
            width: 8,
        };
        assert_eq!(text_identity("0000000a", &input).unwrap(), 10);
        for wrong in ["0000000A", "+0000000", "0000000", "000000000"] {
            assert!(text_identity(wrong, &input).is_err());
        }
        create(&path, &input, 1, BATCH, false).unwrap();
        let db = Database::open(&path, Config::new(32_000_000, TEMP).unwrap()).unwrap();
        #[cfg(target_os = "linux")]
        {
            use std::io::Write;
            let name = path.join("spill-observer-control");
            let mut file = std::fs::File::create_new(&name).unwrap();
            std::fs::remove_file(name).unwrap();
            assert_eq!(scratch_usage(&path).unwrap(), (1, 0));
            assert!(released(&db, db.reserved_memory_bytes(), &path).is_err());
            file.write_all(&[7; 64]).unwrap();
            assert_eq!(scratch_usage(&path).unwrap(), (1, 64));
            let scan = db.prepare(Operation::FilterAll.sql()).unwrap();
            assert_eq!(
                execute(&db, &scan, Operation::FilterAll, &input, &path, true)
                    .err()
                    .unwrap()
                    .to_string(),
                "scan retained scratch"
            );
            drop(scan);
            drop(file);
            released(&db, db.reserved_memory_bytes(), &path).unwrap();
        }
        for operation in [
            Operation::Scan,
            Operation::FilterNone,
            Operation::FilterSparse,
            Operation::FilterHalf,
            Operation::FilterAll,
            Operation::Order,
            Operation::TextOrder,
            Operation::Distinct,
            Operation::Join,
            Operation::WindowCount,
            Operation::WindowSum,
        ] {
            let query = db.prepare(operation.sql()).unwrap();
            execute(&db, &query, operation, &input, &path, true).unwrap();
            if !operation.scanning() {
                cancel(&db, &query, &path).unwrap();
                execute(&db, &query, operation, &input, &path, false).unwrap();
            }
        }
        // Reaching ordinary completion without spill is not a cancellation
        // check, whether rows were returned along the way or not.
        for sql in [
            "FROM facts |> SELECT id",
            "FROM dimensions |> ORDER BY payload |> LIMIT 0",
        ] {
            let query = db.prepare(sql).unwrap();
            let baseline = db.reserved_memory_bytes();
            assert!(cancel(&db, &query, &path).is_err());
            released(&db, baseline, &path).unwrap();
        }
        for (sql, operation, diagnostic) in [
            (
                "FROM facts |> WHERE amount = 1 |> SELECT id, amount",
                Operation::FilterSparse,
                "incorrect predicate subset",
            ),
            (
                "FROM facts |> WHERE amount = 0 |> SELECT id, amount |> LIMIT 1",
                Operation::FilterSparse,
                "incomplete operator result",
            ),
            (
                "FROM facts |> WHERE amount = 0 |> SELECT id, amount + 1 AS amount",
                Operation::FilterSparse,
                "incorrect ordered row or amount",
            ),
            (
                "FROM facts |> SELECT id, amount",
                Operation::FilterNone,
                "incorrect predicate subset",
            ),
            (
                "FROM facts |> WHERE id < 0 |> SELECT id, amount",
                Operation::FilterAll,
                "incomplete operator result",
            ),
        ] {
            let query = db.prepare(sql).unwrap();
            let baseline = db.reserved_memory_bytes();
            assert_eq!(
                execute(&db, &query, operation, &input, &path, false)
                    .err()
                    .unwrap()
                    .to_string(),
                diagnostic
            );
            released(&db, baseline, &path).unwrap();
        }
        // These controls retain the expected schema and reach answer validation.
        // In particular, sorting by (k, id) computes row-by-row sums instead of
        // including the complete equal-k peer group.
        for (sql, operation, diagnostic) in [
            (
                "FROM facts |> EXTEND COUNT(*) OVER () AS count |> SELECT id, count",
                Operation::WindowCount,
                "incorrect window partition count",
            ),
            (
                "FROM facts |> EXTEND SUM(amount) OVER (ORDER BY k, id) AS total |> SELECT id, total",
                Operation::WindowSum,
                "incorrect peer-inclusive window sum",
            ),
            (
                "FROM facts |> EXTEND SUM(amount) OVER (ORDER BY k) AS total |> SELECT id, total + 1 AS total",
                Operation::WindowSum,
                "incorrect peer-inclusive window sum",
            ),
            (
                "FROM facts |> EXTEND SUM(amount) OVER (ORDER BY k) AS total |> SELECT id, total |> LIMIT 8191",
                Operation::WindowSum,
                "incomplete operator result",
            ),
        ] {
            let query = db.prepare(sql).unwrap();
            let baseline = db.reserved_memory_bytes();
            let error = execute(&db, &query, operation, &input, &path, false)
                .err()
                .expect("control must be rejected");
            assert_eq!(error.to_string(), diagnostic);
            released(&db, baseline, &path).unwrap();
        }
        // These successful queries must also be rejected by the consumer.
        for (sql, operation) in [
            (
                "FROM dimensions |> SELECT payload |> ORDER BY payload",
                Operation::TextOrder,
            ),
            (
                "FROM dimensions |> SELECT '00000000' AS payload |> ORDER BY payload DESC",
                Operation::TextOrder,
            ),
            (
                "FROM facts |> SELECT k |> DISTINCT |> LIMIT 31",
                Operation::Distinct,
            ),
            (
                "FROM facts |> SELECT id, amount + 1 AS amount |> ORDER BY id",
                Operation::Order,
            ),
            (
                "FROM facts AS f |> JOIN dimensions AS d ON f.id = d.k |> SELECT f.id, d.payload",
                Operation::Join,
            ),
            (
                "FROM facts |> SELECT amount AS id, id AS amount",
                Operation::Scan,
            ),
            (
                "FROM facts AS f |> JOIN dimensions AS d ON f.k = d.k |> SELECT f.id, 'xxxxxxxx' AS payload",
                Operation::Join,
            ),
            (
                "FROM facts |> SELECT id AS changed, amount",
                Operation::Scan,
            ),
        ] {
            let query = db.prepare(sql).unwrap();
            let baseline = db.reserved_memory_bytes();
            assert!(execute(&db, &query, operation, &input, &path, false).is_err());
            released(&db, baseline, &path).unwrap();
        }
        db.close().unwrap();
        many_matches(&directory);
    }

    fn many_matches(directory: &support::Directory) {
        let path = directory.0.join("many");
        let input = Input {
            rows: 8192,
            keys: 32,
            width: 8,
        };
        create(&path, &input, 8, BATCH, false).unwrap();
        let db = Database::open(&path, Config::new(2_600_000, TEMP).unwrap()).unwrap();
        let operation = Operation::JoinMany;
        let query = db.prepare(operation.sql()).unwrap();
        let result = execute(&db, &query, operation, &input, &path, true).unwrap();
        assert_eq!(result.rows, 65_536);
        cancel(&db, &query, &path).unwrap();
        execute(&db, &query, operation, &input, &path, false).unwrap();
        drop(query);
        db.close().unwrap();
        // LIMIT adds another output owner. Admit the deliberately wrong queries
        // fully so each fails answer validation rather than resource admission.
        let db = Database::open(&path, Config::new(3_000_000, TEMP).unwrap()).unwrap();
        for (sql, diagnostic) in [
            (
                "FROM facts AS f |> JOIN dimensions AS d ON f.k = d.k |> SELECT f.id + 1 AS id, d.payload AS payload",
                "wrong dimension text for fact identity",
            ),
            (
                "FROM facts AS f |> JOIN dimensions AS d ON f.k = d.k |> SELECT f.k AS id, d.payload AS payload",
                "extra or duplicate identity",
            ),
            (
                "FROM facts AS f |> JOIN dimensions AS d ON f.k = d.k |> SELECT f.id AS id, d.payload AS payload |> LIMIT 65535",
                "incomplete operator result",
            ),
        ] {
            let query = db.prepare(sql).unwrap();
            let before = db.reserved_memory_bytes();
            let error = execute(&db, &query, operation, &input, &path, false)
                .err()
                .expect("reject incorrect pair stream");
            assert_eq!(error.to_string(), diagnostic);
            released(&db, before, &path).unwrap();
        }
        db.close().unwrap();
    }
}
