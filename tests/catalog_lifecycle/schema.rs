//! Check schema inspection against literal declarations through the public API.
//!
//! A SQL keyword as a table name keeps lookup independent of query parsing.
//! Publication inside the callback challenges locking and snapshot stability;
//! callback errors and panic unwinding must release the borrowed view's resources.

use super::*;

#[test]
fn inspection_borrows_declared_metadata_without_holding_the_writer_lock() {
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config()).unwrap();
    let cancel = CancellationToken::new();
    let columns = [
        ("id", DataType::Int64, false),
        ("value", DataType::Double, true),
        ("label", DataType::String, true),
        ("day", DataType::Date, false),
    ];
    db.declare_table(
        "SELECT",
        &columns.map(|(name, data_type, nullable)| ColumnDeclaration {
            name,
            data_type,
            nullable,
        }),
        &cancel,
    )
    .unwrap();
    let resident = db.reserved_memory_bytes();
    db.inspect_table("select", &cancel, |schema| {
        assert_eq!(schema.name(), "SELECT");
        assert_eq!(schema.generation(), 1);
        assert_eq!(schema.column_count(), 4);
        for (index, expected) in columns.into_iter().enumerate() {
            let column = schema.column(index).unwrap();
            assert_eq!((column.name, column.data_type, column.nullable), expected);
        }
        assert!(schema.column(4).is_none());
        assert!(schema.column(usize::MAX).is_none());
        db.declare_table(
            "other",
            &[ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            }],
            &cancel,
        )?;
        assert_eq!(db.generation(), 2);
        assert_eq!(schema.generation(), 1);
        assert_eq!(schema.column(3).unwrap().name, "day");
        Ok(())
    })
    .unwrap();
    assert_eq!(db.reserved_memory_bytes(), resident);
    for name in ["missing", "bad name"] {
        let result = db.inspect_table(name, &cancel, |_| panic!("invalid lookup callback"));
        if name == "missing" {
            assert!(matches!(result, Err(Error::NotFound)));
        } else {
            assert!(matches!(
                result,
                Err(Error::Unsupported("invalid table name"))
            ));
        }
        assert_eq!(db.reserved_memory_bytes(), resident);
    }
    assert!(matches!(
        db.inspect_table("select", &cancel, |_| Err(Error::Unsupported(
            "caller failure"
        ))),
        Err(Error::Unsupported("caller failure"))
    ));
    assert_eq!(db.reserved_memory_bytes(), resident);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = db.inspect_table("select", &cancel, |_| panic!("caller panic"));
    }));
    assert!(panic.is_err());
    assert_eq!(db.reserved_memory_bytes(), resident);
    db.inspect_table("select", &cancel, |schema| {
        assert_eq!(schema.generation(), 2);
        Ok(())
    })
    .unwrap();
    cancel.cancel();
    assert!(matches!(
        db.inspect_table("select", &cancel, |_| panic!("cancelled callback")),
        Err(Error::Cancelled)
    ));
    assert_eq!(db.reserved_memory_bytes(), resident);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();

    let legacy = Database::create(&directory.0.join("legacy"), config()).unwrap();
    assert!(matches!(
        legacy.inspect_table("lineitem", &CancellationToken::new(), |_| panic!(
            "legacy callback"
        )),
        Err(Error::Unsupported("catalog database required"))
    ));
    legacy.close().unwrap();
}
