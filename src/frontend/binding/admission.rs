//! Size the retained plan storage before binding allocates its descriptors.
//!
//! `BindingBudget::calculate` counts the parsed operations that need owned
//! storage. Direct column projections reuse identities; computed expressions,
//! aggregates and set operations need additional descriptors. The budget includes
//! the plan itself and the allocation allowances required by those owners.
//!
//! The binder reserves `bytes` from the database before calling `allocate`.
//! Allocation can still fail after admission; partially constructed vectors drop
//! before the caller releases its reservation. `Descriptors` transfers completed
//! vectors into the plan. Temporary suspended name scopes have their own charged
//! owner in the parent module because they end before the prepared plan's lifetime.

use super::{
    AggregateEntry, AggregatePlan, Computed, DistinctPlan, Error, JoinKind, NullExtension,
    PREPARED_ALLOCATION_ALLOWANCE, Parsed, ParsedOp, ParsedStage, Plan, PreparedQuery, SetPlan,
};
use std::mem::size_of;

pub(super) struct BindingBudget {
    pub(super) bytes: u64,
    aggregate_count: usize,
    computed_capacity: usize,
    distinct_count: usize,
    set_count: usize,
    null_count: usize,
    computed_bytes: usize,
    distinct_bytes: usize,
    set_bytes: usize,
    null_bytes: usize,
}

// The caller retains the prepared-plan reservation until all descriptors drop.
pub(super) struct Descriptors {
    pub(super) computed: Vec<Computed>,
    pub(super) distinct: Vec<DistinctPlan>,
    pub(super) set_operations: Vec<SetPlan>,
    pub(super) null_extensions: Vec<NullExtension>,
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
        let mut computed_count = 0;
        for stage in &parsed.stages[..usize::from(parsed.len)] {
            let (start, len, fresh) = match *stage {
                ParsedStage::Set { start, len } => (start, len, true),
                ParsedStage::Select { start, len } | ParsedStage::Extend { start, len, .. } => {
                    (start, len, false)
                }
                _ => continue,
            };
            computed_count += parsed.projections
                [usize::from(start)..usize::from(start) + usize::from(len)]
                .iter()
                .filter(|entry| {
                    fresh
                        || !(entry.expression.len == 1
                            && matches!(
                                parsed.expression_ops[usize::from(entry.expression.start)],
                                ParsedOp::Column(_)
                            ))
                })
                .count();
        }
        let computed_capacity = Computed::allocation_capacity(computed_count)
            .ok_or(Error::Corrupt("prepared computation capacity overflow"))?;
        let computed_bytes = if computed_capacity == 0 {
            0
        } else {
            computed_capacity
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
        let set_count = parsed.stages[..usize::from(parsed.len)]
            .iter()
            .filter(|stage| {
                matches!(
                    stage,
                    ParsedStage::UnionAll(_)
                        | ParsedStage::ExceptDistinct(_)
                        | ParsedStage::IntersectDistinct(_)
                        | ParsedStage::ExceptAll(_)
                        | ParsedStage::IntersectAll(_)
                )
            })
            .count();
        let set_bytes = if set_count == 0 {
            0
        } else {
            set_count * size_of::<SetPlan>() + PREPARED_ALLOCATION_ALLOWANCE
        };
        let null_count = parsed.stages[..usize::from(parsed.len)]
            .iter()
            .filter(|stage| {
                matches!(
                    stage,
                    ParsedStage::Join {
                        kind: JoinKind::Left,
                        ..
                    }
                )
            })
            .count();
        let null_bytes = if null_count == 0 {
            0
        } else {
            null_count * size_of::<NullExtension>() + PREPARED_ALLOCATION_ALLOWANCE
        };
        let bytes = size_of::<PreparedQuery<'_>>()
            .checked_add(size_of::<Plan>())
            .and_then(|bytes| bytes.checked_add(PREPARED_ALLOCATION_ALLOWANCE))
            .and_then(|bytes| bytes.checked_add(aggregate_bytes))
            .and_then(|bytes| bytes.checked_add(computed_bytes))
            .and_then(|bytes| bytes.checked_add(distinct_bytes))
            .and_then(|bytes| bytes.checked_add(set_bytes))
            .and_then(|bytes| bytes.checked_add(null_bytes))
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or(Error::Corrupt("prepared plan size overflow"))?;
        Ok(Self {
            bytes,
            aggregate_count,
            computed_capacity,
            distinct_count,
            set_count,
            null_count,
            computed_bytes,
            distinct_bytes,
            set_bytes,
            null_bytes,
        })
    }

    pub(super) fn allocate(&self) -> Result<Descriptors, Error> {
        let Self {
            bytes,
            aggregate_count,
            computed_capacity,
            distinct_count,
            set_count,
            null_count,
            computed_bytes,
            distinct_bytes,
            set_bytes,
            null_bytes,
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
        let mut set_operations = Vec::new();
        set_operations
            .try_reserve_exact(set_count)
            .map_err(|_| Error::Resource {
                owner: "prepared set descriptors",
                required: set_bytes as u64,
                limit: bytes,
            })?;
        if set_operations.capacity() != set_count {
            return Err(Error::Resource {
                owner: "prepared set capacity",
                required: (set_operations.capacity() * size_of::<SetPlan>()) as u64,
                limit: set_bytes as u64,
            });
        }
        let mut null_extensions = Vec::new();
        null_extensions
            .try_reserve_exact(null_count)
            .map_err(|_| Error::Resource {
                owner: "prepared null extension descriptors",
                required: null_bytes as u64,
                limit: bytes,
            })?;
        if null_extensions.capacity() != null_count {
            return Err(Error::Resource {
                owner: "prepared null extension capacity",
                required: (null_extensions.capacity() * size_of::<NullExtension>()) as u64,
                limit: null_bytes as u64,
            });
        }
        let mut computed = Vec::new();
        computed
            .try_reserve_exact(computed_capacity)
            .map_err(|_| Error::Resource {
                owner: "prepared expressions",
                required: bytes,
                limit: bytes,
            })?;
        if computed.capacity() != computed_capacity {
            return Err(Error::Resource {
                owner: "prepared expression capacity",
                required: (computed.capacity() * size_of::<Computed>()) as u64,
                limit: computed_bytes as u64,
            });
        }
        Ok(Descriptors {
            computed,
            distinct,
            set_operations,
            null_extensions,
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
