//! Positional union mappings. Output identities belong to positions, not names
//! or repeated input identities; either branch may supply each output value.
use super::{
    ColumnFacts, ColumnId, Error, MAX_COLUMNS, MAX_QUERY_COLUMNS, Output, Plan, RelationColumns,
    SemanticColumn, SourceColumn, SourceSpan, bind_error,
};

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct UnionPlan {
    inputs: [[SemanticColumn; MAX_COLUMNS]; 2],
    first: ColumnId,
    count: u8,
}

impl UnionPlan {
    pub(crate) fn inputs(&self, position: usize) -> Option<[SemanticColumn; 2]> {
        if position >= usize::from(self.count) {
            return None;
        }
        Some([
            *self.inputs[0].get(position)?,
            *self.inputs[1].get(position)?,
        ])
    }

    pub(crate) fn output(&self, position: usize) -> Option<SemanticColumn> {
        let [left, right] = self.inputs(position)?;
        let identity = self.first.value().checked_add(position as u32)?;
        if identity == 0 || identity > MAX_QUERY_COLUMNS as u32 {
            return None;
        }
        Some(SemanticColumn::new(
            identity,
            left.data_type(),
            left.nullable() || right.nullable(),
        ))
    }

    pub(crate) fn column(&self, id: ColumnId) -> Option<SemanticColumn> {
        self.output(id.value().checked_sub(self.first.value())? as usize)
    }

    pub(super) fn bind(
        left: &[Output],
        right: &[Output],
        facts: &ColumnFacts<'_>,
        next: u32,
        span: SourceSpan,
    ) -> Result<Self, Error> {
        if left.len() != right.len() {
            return Err(bind_error(
                "UNION ALL inputs require equal column counts",
                span,
            ));
        }
        if left.is_empty() || left.len() > MAX_COLUMNS {
            return Err(Error::Corrupt("union input width"));
        }
        next.checked_add(left.len() as u32)
            .filter(|end| next != 0 && *end <= MAX_QUERY_COLUMNS as u32 + 1)
            .ok_or(Error::Corrupt("union identity bound"))?;
        let mut bound = Self {
            inputs: [[SourceColumn::QUANTITY.semantic(); MAX_COLUMNS]; 2],
            first: ColumnId::new(next),
            count: left.len() as u8,
        };
        for (position, (left, right)) in left.iter().zip(right).enumerate() {
            let left = facts
                .column(left.id)
                .ok_or(Error::Corrupt("union left facts"))?;
            let right = facts
                .column(right.id)
                .ok_or(Error::Corrupt("union right facts"))?;
            if left.data_type() != right.data_type() {
                return Err(bind_error(
                    "UNION ALL column coercion is not implemented",
                    span,
                ));
            }
            bound.inputs[0][position] = left;
            bound.inputs[1][position] = right;
        }
        Ok(bound)
    }

    pub(super) fn validate(
        &self,
        plan: &Plan,
        left: RelationColumns<'_>,
        right: RelationColumns<'_>,
        next: u32,
    ) -> Result<u32, Error> {
        let width = usize::from(self.count);
        if width == 0
            || width > MAX_COLUMNS
            || left.len() != width
            || right.len() != width
            || self.first.value() != next
        {
            return Err(Error::Corrupt("union shape or identity range"));
        }
        for position in 0..width {
            let left = plan
                .column(
                    left.get(position)
                        .ok_or(Error::Corrupt("union left output"))?,
                )
                .ok_or(Error::Corrupt("union left facts"))?;
            let right = plan
                .column(
                    right
                        .get(position)
                        .ok_or(Error::Corrupt("union right output"))?,
                )
                .ok_or(Error::Corrupt("union right facts"))?;
            if left.data_type() != right.data_type()
                || self.inputs[0][position] != left
                || self.inputs[1][position] != right
            {
                return Err(Error::Corrupt("union positional input mapping"));
            }
        }
        if self.inputs.iter().any(|side| {
            side[width..]
                .iter()
                .any(|column| *column != SourceColumn::QUANTITY.semantic())
        }) {
            return Err(Error::Corrupt("union descriptor tail"));
        }
        next.checked_add(u32::from(self.count))
            .filter(|end| next != 0 && *end <= MAX_QUERY_COLUMNS as u32 + 1)
            .ok_or(Error::Corrupt("union identity range"))
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;
    use crate::frontend::{DataType, Stage, validate};

    #[test]
    fn union_binding_preserves_positions_and_rejects_corrupt_mappings() {
        let path =
            std::env::temp_dir().join(format!("pipesql-union-binding-{}", std::process::id()));
        let db =
            crate::Database::create_empty(&path, crate::Config::new(4_000_000, 2_000_000).unwrap())
                .unwrap();
        let cancel = crate::CancellationToken::new();
        db.declare_table(
            "l",
            &[crate::ColumnDeclaration {
                name: "a",
                data_type: DataType::Int64,
                nullable: false,
            }],
            &cancel,
        )
        .unwrap();
        db.declare_table(
            "r",
            &[
                crate::ColumnDeclaration {
                    name: "b",
                    data_type: DataType::Int64,
                    nullable: true,
                },
                crate::ColumnDeclaration {
                    name: "c",
                    data_type: DataType::Int64,
                    nullable: false,
                },
                crate::ColumnDeclaration {
                    name: "d",
                    data_type: DataType::Double,
                    nullable: false,
                },
            ],
            &cancel,
        )
        .unwrap();
        let baseline = db.reserved_memory_bytes();
        for mode in ["ALL", "DISTINCT"] {
            let sql =
                format!("FROM l |> SELECT a AS x, a AS y |> UNION {mode} (FROM r |> SELECT b, c)");
            for mutation in 0..12 {
                let mut query = db.prepare(&sql).unwrap();
                let union = &query.plan.unions[0];
                let first = union.output(0).unwrap();
                let second = union.output(1).unwrap();
                assert_ne!(first.identity(), second.identity());
                assert!(first.nullable());
                assert!(!second.nullable());
                assert_eq!(union.inputs(0).unwrap()[0], union.inputs(1).unwrap()[0]);
                assert_ne!(union.inputs(0).unwrap()[1], union.inputs(1).unwrap()[1]);
                assert_eq!(query.result_column(0).unwrap().name, Some("x"));
                assert_eq!(query.result_column(1).unwrap().name, Some("y"));
                assert_eq!(
                    query
                        .plan
                        .order_key(query.plan.final_relation(), 0)
                        .unwrap(),
                    None
                );
                let last = usize::from(query.plan.count) - 1 - usize::from(mode == "DISTINCT");
                let union = &mut query.plan.unions[0];
                match mutation {
                    0 => (),
                    1 => union.first = ColumnId::EMPTY,
                    2 => union.count = 65,
                    3 => union.count = 1,
                    4 => union.inputs[1].swap(0, 1),
                    5 => union.inputs[0][0] = first,
                    6 => {
                        union.inputs[1][0] = SemanticColumn::new(
                            union.inputs[1][0].identity().value(),
                            DataType::Double,
                            true,
                        )
                    }
                    7 => {
                        union.inputs[1][0] = SemanticColumn::new(
                            union.inputs[1][0].identity().value(),
                            DataType::Int64,
                            false,
                        )
                    }
                    8 => union.inputs[1][63] = first,
                    9 => query.plan.range_columns[last + 1].insert(first.identity()),
                    10 => {
                        query.plan.stages[last].stage = Stage::UnionAll {
                            right: query.plan.final_relation(),
                            descriptor: 0,
                        }
                    }
                    11 => {
                        query.plan.stages[last].stage = Stage::UnionAll {
                            right: query.plan.stages[last].input,
                            descriptor: 0,
                        }
                    }
                    _ => unreachable!(),
                }
                assert_eq!(
                    validate(&query.plan).is_ok(),
                    mutation == 0,
                    "{mode}: mutation {mutation}"
                );
            }
        }
        for sql in [
            "FROM l |> UNION ALL (FROM r |> SELECT b) |> SELECT l.a",
            "FROM l |> UNION ALL (FROM r |> SELECT l.a)",
            "FROM l |> UNION ALL (FROM r)",
            "FROM l |> UNION ALL (FROM r |> SELECT d)",
        ] {
            assert!(matches!(db.prepare(sql), Err(Error::Bind { .. })), "{sql}");
        }
        for sql in [
            "FROM l |> UNION ALL (FROM r |> SELECT b) |> AS u |> SELECT u.a",
            "FROM l |> UNION ALL (FROM r |> SELECT b), (FROM l) |> SELECT a",
            "FROM l |> UNION ALL (FROM r |> SELECT b |> UNION ALL (FROM l))",
            "FROM l |> UNION ALL (FROM l) |> AS u |> JOIN (FROM l) AS r ON u.a=r.a",
        ] {
            let query = db.prepare(sql).unwrap();
            validate(&query.plan).unwrap();
        }
        assert_eq!(db.reserved_memory_bytes(), baseline);
        db.close().unwrap();
        std::fs::remove_dir_all(path).unwrap();
    }
}
