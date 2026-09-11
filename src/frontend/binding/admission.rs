//! Prepared-plan capacity calculation and fallible descriptor allocation.
use super::{
    AggregateEntry, AggregatePlan, Computed, DistinctPlan, Error, PREPARED_ALLOCATION_ALLOWANCE,
    Parsed, ParsedOp, ParsedStage, Plan, PreparedQuery,
};
use std::mem::size_of;

pub(super) struct BindingBudget {
    pub(super) bytes: u64,
    aggregate_count: usize,
    computed_count: usize,
    distinct_count: usize,
    computed_bytes: usize,
    distinct_bytes: usize,
}

// The caller retains the prepared-plan reservation until all descriptors drop.
pub(super) struct Descriptors {
    pub(super) computed: Vec<Computed>,
    pub(super) distinct: Vec<DistinctPlan>,
    pub(super) aggregates: Vec<AggregatePlan>,
}

impl BindingBudget {
    pub(super) fn calculate(parsed: &Parsed) -> Result<Self, Error> {
        let entry_count = usize::from(parsed.aggregate_entry_count);
        let aggregate_count = parsed.stages[..usize::from(parsed.len)]
            .iter()
            .filter(|stage| matches!(stage, ParsedStage::Aggregate(_)))
            .count();
        let aggregate_bytes = if entry_count == 0 {
            0
        } else {
            entry_count
                .checked_mul(size_of::<AggregateEntry>())
                .and_then(|bytes| {
                    bytes.checked_add(aggregate_count.checked_mul(size_of::<AggregatePlan>())?)
                })
                .and_then(|bytes| {
                    bytes.checked_add(
                        (aggregate_count + 1).checked_mul(PREPARED_ALLOCATION_ALLOWANCE)?,
                    )
                })
                .ok_or(Error::Corrupt("prepared aggregate size overflow"))?
        };
        let computed_count = parsed.projections[..usize::from(parsed.projection_count)]
            .iter()
            .filter(|entry| {
                !(entry.expression.len == 1
                    && matches!(
                        parsed.numeric_ops[usize::from(entry.expression.start)],
                        ParsedOp::Column(_)
                    ))
            })
            .count();
        let computed_bytes = if computed_count == 0 {
            0
        } else {
            computed_count
                .checked_mul(size_of::<Computed>())
                .and_then(|bytes| bytes.checked_add(PREPARED_ALLOCATION_ALLOWANCE))
                .ok_or(Error::Corrupt("prepared computation size overflow"))?
        };
        let distinct_count = parsed.stages[..usize::from(parsed.len)]
            .iter()
            .filter(|stage| matches!(stage, ParsedStage::Distinct(_)))
            .count();
        let distinct_bytes = if distinct_count == 0 {
            0
        } else {
            distinct_count * size_of::<DistinctPlan>() + PREPARED_ALLOCATION_ALLOWANCE
        };
        let bytes = size_of::<PreparedQuery<'_>>()
            .checked_add(size_of::<Plan>())
            .and_then(|bytes| bytes.checked_add(PREPARED_ALLOCATION_ALLOWANCE))
            .and_then(|bytes| bytes.checked_add(aggregate_bytes))
            .and_then(|bytes| bytes.checked_add(computed_bytes))
            .and_then(|bytes| bytes.checked_add(distinct_bytes))
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or(Error::Corrupt("prepared plan size overflow"))?;
        Ok(Self {
            bytes,
            aggregate_count,
            computed_count,
            distinct_count,
            computed_bytes,
            distinct_bytes,
        })
    }

    pub(super) fn allocate(&self) -> Result<Descriptors, Error> {
        let Self {
            bytes,
            aggregate_count,
            computed_count,
            distinct_count,
            computed_bytes,
            distinct_bytes,
        } = *self;
        let mut aggregates = Vec::new();
        aggregates
            .try_reserve_exact(aggregate_count)
            .map_err(|_| Error::Resource {
                owner: "prepared aggregate plans",
                required: bytes,
                limit: bytes,
            })?;
        if aggregates.capacity() != aggregate_count {
            return Err(Error::Resource {
                owner: "prepared aggregate plan capacity",
                required: (aggregates.capacity() * size_of::<AggregatePlan>()) as u64,
                limit: bytes,
            });
        }
        let mut distinct = Vec::new();
        distinct
            .try_reserve_exact(distinct_count)
            .map_err(|_| Error::Resource {
                owner: "prepared DISTINCT descriptors",
                required: distinct_bytes as u64,
                limit: bytes,
            })?;
        if distinct.capacity() != distinct_count {
            return Err(Error::Resource {
                owner: "prepared DISTINCT capacity",
                required: (distinct.capacity() * size_of::<DistinctPlan>()) as u64,
                limit: distinct_bytes as u64,
            });
        }
        let mut computed = Vec::new();
        computed
            .try_reserve_exact(computed_count)
            .map_err(|_| Error::Resource {
                owner: "prepared expressions",
                required: bytes,
                limit: bytes,
            })?;
        if computed.capacity() != computed_count {
            return Err(Error::Resource {
                owner: "prepared expression capacity",
                required: (computed.capacity() * size_of::<Computed>()) as u64,
                limit: computed_bytes as u64,
            });
        }
        Ok(Descriptors {
            computed,
            distinct,
            aggregates,
        })
    }
}

pub(super) fn aggregate_entries(
    count: usize,
    prepared_bytes: u64,
) -> Result<Vec<AggregateEntry>, Error> {
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(count)
        .map_err(|_| Error::Resource {
            owner: "prepared aggregate entries",
            required: prepared_bytes,
            limit: prepared_bytes,
        })?;
    if entries.capacity() != count {
        return Err(Error::Resource {
            owner: "prepared aggregate capacity",
            required: entries
                .capacity()
                .checked_mul(size_of::<AggregateEntry>())
                .and_then(|bytes| u64::try_from(bytes).ok())
                .ok_or(Error::Corrupt("prepared capacity size overflow"))?,
            limit: prepared_bytes,
        });
    }
    Ok(entries)
}
