//! Public append contract tests.
use super::*;

#[test]
fn public_append_failure_and_drop_recover_without_publishing() {
    let directory = Directory::new();
    let path = directory.database();
    let db = Database::create_empty(&path, config()).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "facts",
        &[ColumnDeclaration {
            name: "value",
            data_type: DataType::Int64,
            nullable: false,
        }],
        &cancel,
    )
    .unwrap();
    let resident = db.reserved_memory_bytes();
    assert!(db.begin_append("missing", limits(), &cancel).is_err());
    assert_eq!(db.reserved_memory_bytes(), resident);
    let input = [ColumnInput {
        values: ColumnValues::Int64(&[42]),
        validity: &[1],
    }];
    let mut append = db.begin_append("facts", limits(), &cancel).unwrap();
    let rejected = append.transaction();
    append.write(&input, &cancel).unwrap();
    assert!(append.write(&[], &cancel).is_err());
    assert!(append.write(&input, &cancel).is_err());
    append.abort().unwrap();
    assert_eq!(
        db.resolve_commit(rejected).unwrap(),
        CommitResolution::Aborted
    );
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert_eq!(db.reserved_memory_bytes(), resident);
    let mut append = db.begin_append("facts", limits(), &cancel).unwrap();
    let abandoned = append.transaction();
    append.write(&input, &cancel).unwrap();
    drop(append);
    assert!(db.reserved_temp_bytes() > 0);
    assert!(db.begin_append("facts", limits(), &cancel).is_err());
    db.close().unwrap();
    let db = Database::open(&path, config()).unwrap();
    assert_eq!(
        db.resolve_commit(abandoned).unwrap(),
        CommitResolution::Aborted
    );
    assert_eq!(db.reserved_temp_bytes(), 0);
    let query = db.prepare("FROM facts |> SELECT value").unwrap();
    assert!(collect(&mut db.execute(&query, &cancel).unwrap()).is_empty());
    let mut append = db.begin_append("facts", limits(), &cancel).unwrap();
    append.write(&input, &cancel).unwrap();
    append.commit(&cancel).unwrap();
    let query = db.prepare("FROM facts |> SELECT value").unwrap();
    assert_eq!(
        collect(&mut db.execute(&query, &cancel).unwrap()),
        vec![vec![Cell::Integer(42)]]
    );
}

#[test]
fn public_date_input_rejects_outside_the_supported_calendar() {
    for days in [i32::MIN, -719_163, 2_932_897, i32::MAX] {
        assert!(DateValue::from_days_since_unix_epoch(days).is_none());
    }
    for days in [-719_162, 0, 2_932_896] {
        assert_eq!(
            DateValue::from_days_since_unix_epoch(days)
                .unwrap()
                .days_since_unix_epoch(),
            days
        );
    }
}
