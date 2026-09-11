//! Optional scan branch state. Row producers use the same physical leaf decisions.
use crate::execution::planning::Pipeline;
use crate::execution::{BATCH_ROWS, COMPUTE_ROWS};
use crate::frontend::{Comparison, FilterControl, FilterLiteral, Predicate};
use crate::resources::allocate;
use crate::{Error, Value};
use std::mem::size_of;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PhysicalFilter<'query> {
    pub(super) column: u8,
    pub(super) predicate: &'query Predicate,
    pub(super) control: FilterControl,
}

impl PhysicalFilter<'_> {
    pub(super) const EMPTY: Self = Self {
        column: 0,
        control: FilterControl::LINEAR,
        predicate: &Predicate::Compare {
            comparison: Comparison::Equal,
            literal: FilterLiteral::Double(0.0_f64.to_bits()),
        },
    };
}

impl PhysicalFilter<'_> {
    pub(super) fn matches(self, value: Value<'_>) -> Result<bool, Error> {
        let (comparison, literal) = match self.predicate {
            Predicate::IsNull { negated } => {
                return Ok((matches!(value, Value::Null) != *negated) != self.control.negated);
            }
            Predicate::Compare {
                comparison,
                literal,
            } => (comparison, literal),
        };
        if value == Value::Null {
            return Ok(false);
        } // UNKNOWN matches neither requested truth.
        Ok(match (value, literal) {
            (Value::Double(value), FilterLiteral::Double(bits)) => {
                comparison.test(value, f64::from_bits(*bits))
            }
            (Value::Int64(value), FilterLiteral::Double(bits)) => {
                comparison.test(value as f64, f64::from_bits(*bits))
            }
            (Value::Int64(value), FilterLiteral::Int64(literal)) => {
                comparison.test_order(value.cmp(literal))
            }
            (Value::Date(value), FilterLiteral::Date(literal)) => {
                comparison.test_order(value.cmp(literal))
            }
            (Value::String(value), FilterLiteral::String(literal)) => {
                comparison.test_order(value.as_str().cmp(literal.as_str()))
            }
            _ => return Err(Error::Corrupt("filter input type disagrees")),
        } != self.control.negated)
    }
}

#[derive(Default)]
pub(super) struct BranchScratch {
    pub(super) next: Vec<u8>,
    pub(super) selected: Vec<u32>,
}

impl BranchScratch {
    pub(super) const MAX_BYTES: u64 = Self::bytes(COMPUTE_ROWS);

    pub(super) fn rows(plan: &Pipeline<'_>) -> usize {
        if plan.filters[..plan.filter_count]
            .iter()
            .any(|filter| filter.control.matched != 1 || filter.control.other != 0)
        {
            if plan.has_computed_work() {
                BATCH_ROWS
            } else {
                COMPUTE_ROWS
            }
        } else {
            0
        }
    }

    pub(super) const fn bytes(rows: usize) -> u64 {
        if rows == 0 {
            0
        } else {
            (rows * (size_of::<u8>() + size_of::<u32>()) + 2 * 4096) as u64
        }
    }

    pub(super) fn new(rows: usize, admitted: u64) -> Result<Self, Error> {
        Ok(Self {
            next: allocate(rows, rows, "scan branch cursors", admitted)?,
            selected: allocate(rows, rows, "scan branch selection", admitted)?,
        })
    }
}
