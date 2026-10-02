//! Keep the classified report's snapshot across imports and failed exports.
//!
//! Literal report rows distinguish peer-inclusive totals from row accumulation.
//! CSV publication, retained queries and output failure share one database handle;
//! every failed or completed export must release its execution reservations.
//!
//! A prepared report stays on its original snapshot after another import commits;
//! a newly prepared report sees the added data. Invalid CSV must leave that state
//! unchanged. Failed JSON Lines and Parquet outputs are followed by owner checks
//! and another report, exercising reuse rather than cleanup only at process exit.

use super::event_data;
use super::support::Directory;
use pipesql::{
    AppendLimits, CancellationToken, Config, CsvLimits, Database, ExportLimits, ImportLimits,
    ParquetExportLimits,
};

const FIRST: &[u8] = b"id,dimension_id,happened,amount,measurement\n0,1,1999-12-31,10,0.5\n1,2,2000-02-29,20,1.5\n2,\\N,2000-02-29,30,\\N\n3,99,2000-02-29,\\N,-0\n4,3,\\N,5,2\n5,1,2001-01-01,\\N,2.5\n6,2,2000-12-31,-5,3\n7,1,\\N,7,-1\n";
const SECOND: &[u8] = b"id,dimension_id,happened,amount,measurement\n8,1,1999-12-31,-2,4\n9,2,2001-01-01,40,4.5\n10,99,2000-02-29,3,5\n11,\\N,\\N,11,\\N\n12,3,2000-02-29,0,6\n13,1,2001-01-01,13,6.5\n14,2,\\N,\\N,7\n15,3,2000-12-31,9,-2\n";
const EXPORT: ExportLimits = ExportLimits {
    rows: 100,
    bytes: 100_000,
};

fn import(db: &Database, input: &[u8]) -> Result<pipesql::Commit, pipesql::Error> {
    db.import_csv(
        "events",
        input,
        ImportLimits {
            csv: CsvLimits {
                input_bytes: 10_000,
                rows: 100,
                record_bytes: 1024,
                field_bytes: 256,
                batch_rows: 2,
                batch_text_bytes: 1024,
            },
            append: AppendLimits {
                batches: 16,
                encoded_bytes: 100_000,
            },
        },
        &CancellationToken::new(),
        |_| Ok(()),
    )
}

fn check(db: &Database, query: &pipesql::PreparedQuery<'_>, expected: &[&str]) {
    let baseline = db.reserved_memory_bytes();
    let mut output = Vec::new();
    let count = db
        .export_jsonl(query, &mut output, EXPORT, &CancellationToken::new())
        .unwrap();
    let text = std::str::from_utf8(&output).unwrap();
    let rows: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("{\"row\":"))
        .collect();
    assert_eq!(rows, expected);
    assert_eq!(count as usize, expected.len());
    assert_eq!(
        text.lines().last().unwrap(),
        format!("{{\"complete\":true,\"rows\":{count}}}")
    );
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}

struct FailedOutput;

impl std::io::Write for FailedOutput {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("injected report output failure"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn imported_report_keeps_old_snapshot_and_releases_failed_output() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("database"),
        Config::new(16_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    event_data::declare(&db, &cancel).unwrap();
    import(&db, FIRST).unwrap();
    let old = db.prepare(include_str!("../analytics.sql")).unwrap();
    check(&db, &old, OLD);
    import(&db, SECOND).unwrap();
    let new = db.prepare(include_str!("../analytics.sql")).unwrap();
    check(&db, &old, OLD);
    check(&db, &new, NEW);
    let baseline = db.reserved_memory_bytes();
    assert!(matches!(
        db.export_jsonl(&new, &mut FailedOutput, EXPORT, &cancel),
        Err(pipesql::Error::Io { .. })
    ));
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert!(matches!(
        db.export_parquet(
            &new,
            &mut FailedOutput,
            ParquetExportLimits {
                rows: 100,
                bytes: 100_000,
                row_group_rows: 3,
                row_group_text_bytes: 1024,
                row_groups: 32,
                metadata_bytes: 16_384,
            },
            &cancel
        ),
        Err(pipesql::Error::Io { .. })
    ));
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    let bad = b"id,dimension_id,happened,amount,measurement\n20,1,2000-01-01,5,1\n21,1,2000-01-01,6,1\nbad,1,2000-01-01,7,1\n";
    assert!(import(&db, bad).is_err());
    assert_eq!(db.reserved_memory_bytes(), baseline);
    check(&db, &old, OLD);
    check(&db, &new, NEW);
    drop(new);
    drop(old);
    db.close().unwrap();
    let db = Database::open(
        &directory.0.join("database"),
        Config::new(16_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let query = db.prepare(include_str!("../analytics.sql")).unwrap();
    check(&db, &query, NEW);
    drop(query);
    db.close().unwrap();
}

const OLD: &[&str] = &[
    r#"{"row":[null,"","1","1","1","5","1","5"]}"#,
    r#"{"row":[null,"north","1","1","1","7","3","7"]}"#,
    r#"{"row":["1999","north","1","1","1","10","3","17"]}"#,
    r#"{"row":["2000",null,"0","1","0",null,"2","30"]}"#,
    r#"{"row":["2000",null,"1","1","1","30","2","30"]}"#,
    r#"{"row":["2000","south","-1","1","1","-5","2","15"]}"#,
    r#"{"row":["2000","south","1","1","1","20","2","15"]}"#,
    r#"{"row":["2000","南","-1","1","1","-5","2","15"]}"#,
    r#"{"row":["2000","南","1","1","1","20","2","15"]}"#,
    r#"{"row":["2001","north","0","1","0",null,"3","17"]}"#,
];

const NEW: &[&str] = &[
    r#"{"row":[null,null,"1","1","1","11","3","11"]}"#,
    r#"{"row":[null,"","1","1","1","5","2","5"]}"#,
    r#"{"row":[null,"north","1","1","1","7","5","7"]}"#,
    r#"{"row":[null,"south","0","1","0",null,"4",null]}"#,
    r#"{"row":[null,"南","0","1","0",null,"4",null]}"#,
    r#"{"row":["1999","north","-1","1","1","-2","5","15"]}"#,
    r#"{"row":["1999","north","1","1","1","10","5","15"]}"#,
    r#"{"row":["2000",null,"0","1","0",null,"3","44"]}"#,
    r#"{"row":["2000",null,"1","2","2","33","3","44"]}"#,
    r#"{"row":["2000","","1","2","2","9","2","14"]}"#,
    r#"{"row":["2000","south","-1","1","1","-5","4","15"]}"#,
    r#"{"row":["2000","south","1","1","1","20","4","15"]}"#,
    r#"{"row":["2000","南","-1","1","1","-5","4","15"]}"#,
    r#"{"row":["2000","南","1","1","1","20","4","15"]}"#,
    r#"{"row":["2001","north","0","1","0",null,"5","28"]}"#,
    r#"{"row":["2001","north","1","1","1","13","5","28"]}"#,
    r#"{"row":["2001","south","1","1","1","40","4","55"]}"#,
    r#"{"row":["2001","南","1","1","1","40","4","55"]}"#,
];
