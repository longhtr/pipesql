use super::*;

#[test]
fn projection_ranges_are_complete_disjoint_and_bounded() {
    let path =
        std::env::temp_dir().join(format!("pipesql-projection-ranges-{}", std::process::id()));
    let database =
        Database::create_empty(&path, crate::Config::new(4_000_000, 1_000_000).unwrap()).unwrap();
    let names: Vec<_> = (0..10).map(|i| format!("c{i}")).collect();
    let schema: Vec<_> = names
        .iter()
        .map(|name| crate::ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: false,
        })
        .collect();
    database
        .declare_table("wide", &schema, &crate::CancellationToken::new())
        .unwrap();
    let stage = format!(" |> SELECT {}", names.join(","));
    let query = format!("FROM wide{}", stage.repeat(7));
    let prepared = database.prepare(&query).unwrap();
    assert_eq!(prepared.result_column_count(), 10);
    assert_eq!(prepared.plan.projection_count, 70);
    drop(prepared);
    assert!(matches!(
        database.prepare(&format!("{query}{stage}")),
        Err(Error::Parse { .. })
    ));
    for mutation in 0..7 {
        let mut prepared = database.prepare(&query).unwrap();
        match mutation {
            0 => prepared.plan.projection_count = u8::MAX,
            1 => prepared.plan.projection_count -= 1,
            2 => prepared.plan.projection_count += 1,
            3 => prepared.plan.stages[1].stage = Stage::Select { start: 0, len: 10 },
            4 => prepared.plan.stages[1].stage = Stage::Select { start: 11, len: 10 },
            5 => {
                prepared.plan.stages[1].stage = Stage::Select {
                    start: u8::MAX,
                    len: u8::MAX,
                }
            }
            6 => prepared.plan.projections[MAX_PROJECTIONS - 1] = ColumnId::new(8),
            _ => unreachable!(),
        }
        assert!(
            validate(&prepared.plan).is_err(),
            "range mutation {mutation}"
        );
    }
    database.close().unwrap();
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn semantic_plan_mutations_refuse() {
    let path = std::env::temp_dir().join(format!("pipesql-stream-bind-{}", std::process::id()));
    let database =
        Database::create(&path, crate::Config::new(2_000_000, 1_000_000).unwrap()).unwrap();
    for mutation in 0..9 {
        let mut query = database
            .prepare("FROM lineitem |> SELECT l_quantity AS q |> WHERE q < 25.0")
            .unwrap();
        match mutation {
            0 => query.plan.count = 17,
            1 => query.plan.output_count = 11,
            2 => query.plan.outputs[0].id = SourceColumn::PRICE.identity,
            3 => query.plan.outputs[0].name = Name::new("SELECT"),
            4 => query.plan.stages[0].stage = Stage::Empty,
            5 => query.plan.generation = 2,
            6..=8 => {
                let Stage::Where(filter) = &mut query.plan.stages[1].stage else {
                    unreachable!()
                };
                match mutation {
                    6 => filter.column = SourceColumn::PRICE.identity,
                    7 => {
                        let Predicate::Compare { literal, .. } = &mut filter.predicate else {
                            unreachable!()
                        };
                        *literal = FilterLiteral::Double(f64::NAN.to_bits());
                    }
                    8 => filter.span.end = u16::MAX,
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        }
        assert!(validate(&query.plan).is_err(), "mutation {mutation}");
    }
    for mutation in 0..14 {
        let mut query = database.prepare("FROM lineitem |> AGGREGATE SUM(l_quantity) AS s,AVG(l_quantity) AS a,COUNT(*) AS n |> WHERE n > 0 |> SELECT a AS n").unwrap();
        assert_eq!(
            query.result_column(0),
            Some(ResultColumn {
                name: Some("n"),
                data_type: DataType::Double,
                nullable: true
            })
        );
        assert_eq!(query.plan.aggregate_demand(0), 0b110);
        match mutation {
            0 => query.plan.outputs[0].id = ColumnId::new(8),
            1 => query.plan.outputs[0].id = SourceColumn::QUANTITY.identity,
            2 => query.plan.stages[2].stage = Stage::Aggregate(0),
            3 => query.plan.aggregates.first_mut().unwrap().entries[1].kind = AggregateKind::Count,
            4..=6 => {
                let Stage::Where(filter) = &mut query.plan.stages[1].stage else {
                    unreachable!()
                };
                match mutation {
                    4 => filter.column = ColumnId::new(18),
                    5 => filter.column = SourceColumn::QUANTITY.identity,
                    6 => {
                        let Predicate::Compare { literal, .. } = &mut filter.predicate else {
                            unreachable!()
                        };
                        *literal = FilterLiteral::Date(DateValue::from_days(0).unwrap());
                    }
                    _ => unreachable!(),
                }
            }
            7 => {
                let Stage::Select { start, .. } = query.plan.stages[2].stage else {
                    unreachable!()
                };
                query.plan.projections[usize::from(start)] = ColumnId::new(18);
            }
            8 => query.plan.aggregates.first_mut().unwrap().first_output = ColumnId::new(1),
            9 => {
                let aggregate = query.plan.aggregates.first_mut().unwrap();
                aggregate.first_output = ColumnId::EMPTY;
                assert!(aggregate.output(1).is_none());
            }
            10 => {
                let aggregate = query.plan.aggregates.first_mut().unwrap();
                aggregate.first_output = ColumnId::new(u32::MAX);
                assert!(aggregate.output(1).is_none());
            }
            11..=13 => {
                let source_bytes = query.plan.source_bytes;
                let span = &mut query.plan.aggregates.first_mut().unwrap().entries[1].span;
                match mutation {
                    11 => span.end = span.start,
                    12 => span.start = span.end + 1,
                    13 => span.end = source_bytes + 1,
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        }
        assert!(
            validate(&query.plan).is_err(),
            "aggregate mutation {mutation}"
        );
    }
    assert_eq!(
        database.reserved_memory_bytes(),
        database.path_memory_bytes()
    );
    database.close().unwrap();
    std::fs::remove_dir_all(path).unwrap();
}
