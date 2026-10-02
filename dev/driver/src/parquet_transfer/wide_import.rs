//! Import independent maximum-width files and expose complete reopened answers.
//!
//! The supervisor owns expected cells. This driver checks publication, receipts,
//! resource release and retry after a final source failure, using the same reader
//! fault boundary as the smaller transfer profiles.

use super::*;

fn configuration() -> Config {
    Config::new(64_000_000, 8_000_000).unwrap()
}

fn seed(path: &Path, names: &[String]) -> Result<Database> {
    let db = Database::create_empty(path, configuration())?;
    let cancel = CancellationToken::new();
    let schema: Vec<_> = names
        .iter()
        .enumerate()
        .rev()
        .map(|(column, name)| ColumnDeclaration {
            name,
            data_type: [
                DataType::Int64,
                DataType::Double,
                DataType::Date,
                DataType::String,
            ][column % 4],
            nullable: column % 3 != 0,
        })
        .collect();
    db.declare_table("facts", &schema, &cancel)?;
    let mut append = db.begin_append(
        "facts",
        AppendLimits {
            batches: 1,
            encoded_bytes: 16_384,
        },
        &cancel,
    )?;
    let epoch = [DateValue::from_days_since_unix_epoch(0).unwrap()];
    let inputs: Vec<_> = (0..64)
        .rev()
        .map(|column| ColumnInput {
            values: match column % 4 {
                0 if column == 0 => ColumnValues::Int64(&[-1]),
                0 => ColumnValues::Int64(&[0]),
                1 => ColumnValues::Double(&[0.0]),
                2 => ColumnValues::Date(&epoch),
                _ => ColumnValues::String(&["existing"]),
            },
            validity: &[1],
        })
        .collect();
    append.write(&inputs, &cancel)?;
    append.commit(&cancel)?;
    Ok(db)
}

fn export(db: &Database, path: &Path, names: &[String], rows: u64) -> Result<()> {
    let memory = db.reserved_memory_bytes();
    {
        let query = db.prepare(&format!(
            "FROM facts |> ORDER BY c00 |> SELECT {}",
            names.join(", ")
        ))?;
        let mut output = File::create_new(path)?;
        assert_eq!(
            db.export_jsonl(
                &query,
                &mut output,
                ExportLimits {
                    rows,
                    bytes: 2_000_000
                },
                &CancellationToken::new(),
            )?,
            rows
        );
    }
    assert_eq!(db.reserved_memory_bytes(), memory);
    assert_eq!(db.reserved_temp_bytes(), 0);
    Ok(())
}

pub(super) fn run(root: &Path, input: &Path) -> Result<()> {
    if !root.is_absolute() || !input.is_absolute() {
        return Err("wide import requires absolute paths and a new output directory".into());
    }
    let skip = match std::env::var("PIPESQL_IMPORT_CONTROL").ok().as_deref() {
        None => false,
        Some("skip-source-fault") => true,
        Some(_) => return Err("unknown wide import control".into()),
    };
    let length = fs::metadata(input)?.len();
    if !(12..=2_000_000).contains(&length) {
        return Err("wide import input size differs".into());
    }
    fs::create_dir(root)?;
    let names: Vec<_> = (0..64).map(|column| format!("c{column:02}")).collect();
    let path = root.join("database");
    let mut db = seed(&path, &names)?;
    let limits = ParquetImportLimits {
        parquet: ParquetReadLimits {
            input_bytes: 2_000_000,
            metadata_bytes: 65_536,
            row_groups: 2,
            row_group_rows: 513,
            row_group_bytes: 1_048_576,
            page_bytes: 65_536,
            rows: 513,
        },
        append: AppendLimits {
            batches: 2,
            encoded_bytes: 2_000_000,
        },
    };
    let generation = db.generation();
    let mut aborted = None;
    for (name, fault) in [("source", Fault::Source), ("complete", Fault::None)] {
        let cancel = CancellationToken::new();
        let observed = Cell::new(Observed::default());
        let issued = Cell::new(None);
        let memory = db.reserved_memory_bytes();
        let handles = descriptors()?;
        let objects = units(&path)?;
        let result = db.import_parquet(
            "facts",
            Input {
                file: File::open(input)?,
                fragment: 127,
                fault: if skip { Fault::None } else { fault },
                cancel: &cancel,
                observed: &observed,
            },
            limits,
            &cancel,
            |token| {
                assert!(issued.replace(Some(token)).is_none());
                Ok(())
            },
        );
        let token = issued.get().expect("wide import issued a token");
        assert_eq!(db.reserved_memory_bytes(), memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(descriptors()?, handles);
        let observation = observed.get();
        assert_eq!(observation.bytes, length);
        assert_eq!(observation.end_seeks, 2);
        assert!(observation.calls >= length.div_ceil(127));
        let resolution = if fault == Fault::Source {
            if skip && result.is_ok() {
                export(&db, &root.join("unexpected-success.jsonl"), &names, 514)?;
            }
            assert!(matches!(
                result.expect_err("wide import fault must fail"),
                Error::Io { operation: "seek Parquet input", source }
                    if source.kind() == io::ErrorKind::BrokenPipe
            ));
            assert!(observation.faulted);
            assert_eq!(db.generation(), generation);
            assert_eq!(units(&path)?, objects);
            aborted = Some(token);
            CommitResolution::Aborted
        } else {
            let commit = result?;
            assert_eq!(token, commit.transaction());
            assert_eq!(commit.generation(), generation + 1);
            CommitResolution::Durable(commit)
        };
        db.close()?;
        db = Database::open(&path, configuration())?;
        assert_eq!(db.resolve_commit(token)?, resolution);
        assert_eq!(
            db.resolve_commit(aborted.unwrap())?,
            CommitResolution::Aborted
        );
        export(
            &db,
            &root.join(format!("{name}.jsonl")),
            &names,
            if fault == Fault::Source { 1 } else { 514 },
        )?;
    }
    db.close()?;
    println!(
        "wide-import rows=513 columns=64 aborted=true durable=true released=true reopened=true status=finished"
    );
    Ok(())
}
