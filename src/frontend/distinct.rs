//! Fresh identities and complete-row grouping, independent of aggregate limits.
use super::{
    ColumnFacts, ColumnId, Error, MAX_COLUMNS, MAX_QUERY_COLUMNS, Output, Plan, RelationColumns,
    SemanticColumn, SourceColumn,
};

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct DistinctPlan {
    inputs: [SemanticColumn; MAX_COLUMNS],
    outputs: [u8; MAX_COLUMNS],
    first: ColumnId,
    count: u8,
    unique: u8,
}

impl DistinctPlan {
    pub(crate) fn inputs(&self) -> &[SemanticColumn] {
        self.inputs.get(..usize::from(self.unique)).unwrap_or(&[])
    }

    pub(crate) fn output(&self, position: usize) -> Option<ColumnId> {
        if position >= usize::from(self.count) {
            return None;
        }
        self.first
            .value()
            .checked_add(u32::from(*self.outputs.get(position)?))
            .filter(|identity| *identity > 0 && *identity <= MAX_QUERY_COLUMNS as u32)
            .map(ColumnId::new)
    }

    pub(crate) fn input_for(&self, output: ColumnId) -> Option<SemanticColumn> {
        let position = output.value().checked_sub(self.first.value())? as usize;
        self.inputs().get(position).copied()
    }

    pub(crate) fn output_for(&self, input: ColumnId) -> Option<ColumnId> {
        self.inputs()
            .iter()
            .position(|column| column.identity() == input)
            .and_then(|position| self.first.value().checked_add(position as u32))
            .filter(|identity| *identity > 0 && *identity <= MAX_QUERY_COLUMNS as u32)
            .map(ColumnId::new)
    }

    pub(super) fn bind(
        outputs: &mut [Output],
        facts: &ColumnFacts<'_>,
        next: &mut u32,
    ) -> Result<Self, Error> {
        let mut bound = Self {
            inputs: [SourceColumn::QUANTITY.semantic(); MAX_COLUMNS],
            outputs: [0; MAX_COLUMNS],
            first: ColumnId::new(*next),
            count: outputs.len() as u8,
            unique: 0,
        };
        for (index, output) in outputs.iter_mut().enumerate() {
            let position = match bound
                .inputs()
                .iter()
                .position(|column| column.identity() == output.id)
            {
                Some(position) => position,
                None => {
                    let position = usize::from(bound.unique);
                    bound.inputs[position] = facts
                        .column(output.id)
                        .ok_or(Error::Corrupt("DISTINCT input facts absent"))?;
                    bound.unique += 1;
                    position
                }
            };
            bound.outputs[index] = position as u8;
            output.id = ColumnId::new(bound.first.value() + position as u32);
        }
        *next = next
            .checked_add(u32::from(bound.unique))
            .filter(|next| *next <= MAX_QUERY_COLUMNS as u32 + 1)
            .ok_or(Error::Corrupt("DISTINCT identity bound"))?;
        Ok(bound)
    }

    pub(super) fn validate(
        &self,
        plan: &Plan,
        input: RelationColumns<'_>,
        next: u32,
    ) -> Result<u32, Error> {
        if self.first.value() != next
            || usize::from(self.count) != input.len()
            || self.unique == 0
            || self.unique > self.count
        {
            return Err(Error::Corrupt("DISTINCT shape or identity range"));
        }
        let mut unique = 0;
        for (position, id) in input.iter().enumerate() {
            let prior = (0..position).find(|prior| input.get(*prior) == Some(id));
            let expected = if let Some(prior) = prior {
                self.outputs[prior]
            } else {
                let expected = unique;
                unique += 1;
                expected
            };
            if self.outputs[position] != expected
                || expected >= self.unique
                || plan.column(id) != Some(self.inputs[usize::from(expected)])
            {
                return Err(Error::Corrupt("DISTINCT input mapping"));
            }
        }
        if unique != self.unique
            || self.outputs[usize::from(self.count)..]
                .iter()
                .any(|position| *position != 0)
            || self.inputs[usize::from(self.unique)..]
                .iter()
                .any(|column| *column != SourceColumn::QUANTITY.semantic())
        {
            return Err(Error::Corrupt("DISTINCT descriptor tail"));
        }
        next.checked_add(u32::from(unique))
            .filter(|next| *next <= MAX_QUERY_COLUMNS as u32 + 1)
            .ok_or(Error::Corrupt("DISTINCT identity range"))
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::super::{DataType, PreparedQuery, validate};
    use super::*;
    use crate::Database;
    use std::mem::size_of;

    #[test]
    fn distinct_identity_order_and_descriptor_mutations() {
        let path =
            std::env::temp_dir().join(format!("pipesql-distinct-binding-{}", std::process::id()));
        let db = Database::create_empty(&path, crate::Config::new(4_000_000, 2_000_000).unwrap())
            .unwrap();
        let cancel = crate::CancellationToken::new();
        db.declare_table(
            "facts",
            &["a", "b"].map(|name| crate::ColumnDeclaration {
                name,
                data_type: DataType::Int64,
                nullable: true,
            }),
            &cancel,
        )
        .unwrap();
        let sql = "FROM facts |> ORDER BY b |> SELECT a,a AS duplicate |> DISTINCT |> AS d |> SELECT d.duplicate,d.a";
        let baseline = db.reserved_memory_bytes();
        for mutation in 0..11 {
            let mut query = db.prepare(sql).unwrap();
            let descriptor = &query.plan.distinct[0];
            assert_eq!(descriptor.inputs().len(), 1);
            assert_eq!(descriptor.output(0), descriptor.output(1));
            assert_ne!(descriptor.output(0), Some(ColumnId::new(1)));
            assert_eq!(
                query
                    .plan
                    .order_key(query.plan.final_relation(), 0)
                    .unwrap(),
                None
            );
            let output = query.plan.outputs().collect::<Vec<_>>();
            assert_eq!(output[0], output[1]);
            assert_eq!(query.result_column(0).unwrap().name, Some("duplicate"));
            let descriptor = &mut query.plan.distinct[0];
            match mutation {
                0 => (),
                1 => descriptor.first = ColumnId::EMPTY,
                2 => descriptor.first = ColumnId::new(u32::MAX),
                3 => descriptor.unique = 65,
                4 => descriptor.count = 65,
                5 => descriptor.outputs[1] = 1,
                6 => descriptor.inputs[0] = SemanticColumn::new(2, DataType::Int64, true),
                7 => descriptor.inputs[0] = SemanticColumn::new(1, DataType::Double, true),
                8 => descriptor.inputs[0] = SemanticColumn::new(1, DataType::Int64, false),
                9 => descriptor.outputs[63] = 1,
                10 => descriptor.inputs[63] = SemanticColumn::new(2, DataType::Int64, true),
                _ => unreachable!(),
            }
            assert_eq!(
                validate(&query.plan).is_ok(),
                mutation == 0,
                "mutation {mutation}"
            );
        }
        assert_eq!(db.reserved_memory_bytes(), baseline);
        for sql in [
            "FROM facts |> DISTINCT a",
            "FROM facts |> DISTINCT BY a",
            "FROM facts |> SELECT a AS x,a AS x |> DISTINCT |> SELECT x",
            "FROM facts |> SELECT a |> DISTINCT |> SELECT b",
        ] {
            assert!(db.prepare(sql).is_err(), "{sql}");
            assert_eq!(db.reserved_memory_bytes(), baseline);
        }
        eprintln!(
            "DISTINCT descriptor bytes={} prepared ceiling={}",
            size_of::<DistinctPlan>(),
            PreparedQuery::memory_requirement_bytes()
        );
        db.close().unwrap();
        std::fs::remove_dir_all(path).unwrap();
    }
}
