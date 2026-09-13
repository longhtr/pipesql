//! Fresh nullable identities for the right side of a LEFT JOIN.
//!
//! Input facts stay unchanged: sorting and evaluating the right producer precede
//! null extension. Include qualified values hidden by column transforms as well
//! as the visible row, so neither naming path can bypass the join boundary.
use super::{
    ColumnFacts, ColumnId, ColumnSet, Error, MAX_COLUMNS, MAX_QUERY_COLUMNS, MAX_ROW_VALUES,
    Output, Plan, RelationId, SemanticColumn, SourceColumn,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NullExtension {
    inputs: [SemanticColumn; MAX_ROW_VALUES],
    visible: [ColumnId; MAX_COLUMNS],
    first: ColumnId,
    count: u8,
    width: u8,
}

impl NullExtension {
    pub(crate) fn len(&self) -> usize {
        usize::from(self.count)
    }

    pub(crate) fn input_for(&self, output: ColumnId) -> Option<SemanticColumn> {
        if output.value() == 0
            || output.value() > MAX_QUERY_COLUMNS as u32
            || self.first.value() == 0
        {
            return None;
        }
        let position = output.value().checked_sub(self.first.value())? as usize;
        self.inputs
            .get(position)
            .copied()
            .filter(|_| position < self.len())
    }

    pub(crate) fn output_for(&self, input: ColumnId) -> Option<ColumnId> {
        let position = self
            .inputs
            .get(..self.len())?
            .iter()
            .position(|c| c.identity() == input)?;
        let id = self.first.value().checked_add(position as u32)?;
        (id != 0 && id <= MAX_QUERY_COLUMNS as u32).then(|| ColumnId::new(id))
    }

    pub(crate) fn column(&self, id: ColumnId) -> Option<SemanticColumn> {
        let input = self.input_for(id)?;
        Some(SemanticColumn::new(id.value(), input.data_type(), true))
    }

    pub(crate) fn visible(&self, position: usize) -> Option<ColumnId> {
        if position >= usize::from(self.width) {
            return None;
        }
        self.output_for(*self.visible.get(position)?)
    }

    pub(super) fn bind(
        outputs: &[Output],
        mut available: ColumnSet,
        facts: &ColumnFacts<'_>,
        next: u32,
    ) -> Result<Self, Error> {
        for output in outputs {
            available.insert(output.id);
        }
        if outputs.is_empty() || outputs.len() > MAX_COLUMNS || available.len() > MAX_ROW_VALUES {
            return Err(Error::Corrupt("null extension input width"));
        }
        next.checked_add(available.len() as u32)
            .filter(|end| next != 0 && *end <= MAX_QUERY_COLUMNS as u32 + 1)
            .ok_or(Error::Corrupt("null extension identity bound"))?;
        let mut bound = Self {
            inputs: [SourceColumn::QUANTITY.semantic(); MAX_ROW_VALUES],
            visible: [ColumnId::EMPTY; MAX_COLUMNS],
            first: ColumnId::new(next),
            count: available.len() as u8,
            width: outputs.len() as u8,
        };
        for (slot, id) in bound.inputs.iter_mut().zip(available.iter()) {
            *slot = facts
                .column(id)
                .ok_or(Error::Corrupt("null extension input facts"))?;
        }
        for (slot, output) in bound.visible.iter_mut().zip(outputs) {
            *slot = output.id;
        }
        Ok(bound)
    }

    pub(super) fn validate(&self, plan: &Plan, right: RelationId, next: u32) -> Result<u32, Error> {
        let available = plan.available_columns(right)?;
        let visible = plan.relation_columns(right)?;
        if self.len() == 0
            || self.len() > MAX_ROW_VALUES
            || self.len() != available.len()
            || usize::from(self.width) != visible.len()
            || self.first.value() != next
        {
            return Err(Error::Corrupt("null extension shape or identity range"));
        }
        for (position, id) in available.iter().enumerate() {
            if plan.column(id) != Some(self.inputs[position]) {
                return Err(Error::Corrupt("null extension input mapping"));
            }
        }
        for (position, id) in visible.iter().enumerate() {
            if self.visible[position] != id {
                return Err(Error::Corrupt("null extension visible mapping"));
            }
        }
        if self.inputs[self.len()..]
            .iter()
            .any(|c| *c != SourceColumn::QUANTITY.semantic())
            || self.visible[usize::from(self.width)..]
                .iter()
                .any(|id| *id != ColumnId::EMPTY)
        {
            return Err(Error::Corrupt("null extension descriptor tail"));
        }
        next.checked_add(u32::from(self.count))
            .filter(|end| *end <= MAX_QUERY_COLUMNS as u32 + 1)
            .ok_or(Error::Corrupt("null extension identity bound"))
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;
    use crate::frontend::{DataType, Stage, validate};

    #[test]
    fn nullable_join_identities_preserve_inputs_and_reject_corrupt_mappings() {
        let path =
            std::env::temp_dir().join(format!("pipesql-null-extension-{}", std::process::id()));
        let db =
            crate::Database::create_empty(&path, crate::Config::new(4_000_000, 2_000_000).unwrap())
                .unwrap();
        let cancel = crate::CancellationToken::new();
        db.declare_table(
            "t",
            &[
                crate::ColumnDeclaration {
                    name: "k",
                    data_type: DataType::Int64,
                    nullable: false,
                },
                crate::ColumnDeclaration {
                    name: "v",
                    data_type: DataType::String,
                    nullable: false,
                },
            ],
            &cancel,
        )
        .unwrap();
        let baseline = db.reserved_memory_bytes();
        for mutation in 0..13 {
            let mut query = db
                .prepare("FROM t AS l |> LEFT JOIN t AS r ON l.k=r.k")
                .unwrap();
            assert!(!query.result_column(0).unwrap().nullable);
            assert!(!query.result_column(1).unwrap().nullable);
            assert!(query.result_column(2).unwrap().nullable);
            assert!(query.result_column(3).unwrap().nullable);
            let right = query.plan.relation_columns(RelationId(1)).unwrap();
            assert!(!query.plan.column(right.get(0).unwrap()).unwrap().nullable());
            assert!(!query.plan.column(right.get(1).unwrap()).unwrap().nullable());
            let original = right.get(0).unwrap();
            let bound = &mut query.plan.null_extensions[0];
            let output = bound.visible(0).unwrap();
            assert_ne!(original, output);
            match mutation {
                0 => (),
                1 => bound.first = ColumnId::EMPTY,
                2 => bound.count = u8::MAX,
                3 => bound.count = 1,
                4 => bound.width = 1,
                5 => bound.inputs.swap(0, 1),
                6 => bound.inputs[0] = SemanticColumn::new(original.value(), DataType::Int64, true),
                7 => {
                    bound.inputs[0] = SemanticColumn::new(original.value(), DataType::Double, false)
                }
                8 => bound.visible.swap(0, 1),
                9 => bound.visible[MAX_COLUMNS - 1] = original,
                10 => bound.inputs[MAX_ROW_VALUES - 1] = bound.inputs[0],
                11 => query.plan.range_columns[2].insert(original),
                12 => {
                    let Stage::Join { nulls, .. } = &mut query.plan.stages[1].stage else {
                        unreachable!()
                    };
                    *nulls = None;
                }
                _ => unreachable!(),
            }
            assert_eq!(
                validate(&query.plan).is_ok(),
                mutation == 0,
                "mutation {mutation}"
            );
            drop(query);
            assert_eq!(db.reserved_memory_bytes(), baseline);
        }
        db.close().unwrap();
        std::fs::remove_dir_all(path).unwrap();
    }
}
