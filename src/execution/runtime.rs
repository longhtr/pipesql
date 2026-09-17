//! Drive a query's operators and pass batches from each producer to its consumer.
//!
//! The runtime stores operators as nodes in input-first order. Every intermediate
//! node has exactly one consumer, recorded in `parents`; the final node produces
//! the query result. `active` selects the node to step, replacing recursive calls
//! between operators with a walk through this array.
//!
//! For example, an aggregate requests input and the runtime switches to its scan.
//! When the scan has rows, control returns to the aggregate. The scan keeps that
//! batch until the aggregate asks for another, so a consumer can pause midway
//! through input. Intermediate batches stay inside the runtime; only the final
//! node's rows are returned by the public query cursor.
//!
//! Construction reserves operator storage before opening sources. Each aggregate
//! can use spare memory only after later aggregates' minimum budgets are protected.
//! During execution, `step` leaves the runtime failed if an operator returns an
//! error. The enclosing `QueryResult` then releases its buffers and controllers.

use crate::batch::{Batch, OwnedBatch};
use crate::effects::Effects;
use crate::execution::aggregation::Aggregation;
use crate::execution::planning::{MAX_PIPELINES, PhysicalPlan, Pipeline, Producer, WindowFunction};
use crate::execution::scan::{AdmittedScan, ScanCursor, declared};
use crate::execution::{
    Advance, ConsumerInput, ConsumerStep, SetStep, blocking, count, limit, union,
};
use crate::query::{self, MAX_AGGREGATE_COLUMNS, MAX_ROW_VALUES, PreparedQuery};
use crate::resources::{Reservation, allocate};
use crate::value::DataType;
use crate::{CancellationToken, Database, Error};
use std::mem::size_of;

// Allow for allocator overhead on the separate vector of aggregate controllers.
const AGGREGATE_ALLOCATION_ALLOWANCE: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputState {
    Ready,
    Rows,
    Finished,
}

enum Owner<'db> {
    Pending(declared::Admission<'db>),
    Scan {
        cursor: ScanCursor<'db>,
        output: OwnedBatch<'db>,
    },
    Count {
        count: count::Count,
        output: OwnedBatch<'db>,
    },
    Limit {
        limit: limit::Limit,
        output: OwnedBatch<'db>,
    },
    Union {
        union: union::Union,
        output: OwnedBatch<'db>,
    },
    SortedSet {
        sorted_set: Vec<blocking::sorted_set::SortedSet<'db>>,
        output: OwnedBatch<'db>,
    },
    Aggregate(OwnedBatch<'db>),
    Order {
        order: Vec<blocking::order::Order<'db>>,
        output: OwnedBatch<'db>,
    },
    Join {
        join: Vec<blocking::join::Join<'db>>,
        output: OwnedBatch<'db>,
    },
    // A placeholder during construction or while an owner moves into its node.
    // Construction must fill every placeholder before the runtime can execute.
    Vacant,
}

impl<'db> Owner<'db> {
    // Allocate operator state without source I/O. Declared scans remain Pending
    // until all mandatory workspaces and aggregate minima have been admitted.
    fn admit_native(
        database: &'db Database,
        query: &'db PreparedQuery<'db>,
        plan: &PhysicalPlan<'_>,
        pipeline: &Pipeline<'_>,
        reservation: &mut Reservation<'db>,
    ) -> Result<Self, Error> {
        Ok(match pipeline.producer {
            Producer::Scan(_) => {
                Self::Pending(declared::admit(database, query, plan, pipeline, None)?)
            }
            Producer::Aggregate { .. } => {
                Self::Aggregate(producer_output(database, query, pipeline)?)
            }
            Producer::Join {
                kind,
                left,
                right,
                left_key,
                right_key,
            } => {
                let output = producer_output(database, query, pipeline)?;
                let join = blocking::join::Join::new(
                    database,
                    plan.pipelines()[left.index()].output_columns(&query.plan),
                    plan.pipelines()[right.index()].output_columns(&query.plan),
                    (left_key, right_key),
                    kind,
                    reservation,
                )?;
                Self::Join { join, output }
            }
            Producer::Order { input, start, len } => {
                let output = producer_output(database, query, pipeline)?;
                let order = blocking::order::Order::new(
                    database,
                    plan.pipelines()[input.index()].output_columns(&query.plan),
                    plan.order_columns(start, len),
                    reservation,
                )?;
                Self::Order { order, output }
            }
            Producer::Analytic {
                input,
                partition,
                function,
            } => {
                let output = producer_output(database, query, pipeline)?;
                Self::analytic(
                    database,
                    query,
                    &plan.pipelines()[input.index()],
                    &partition,
                    &function,
                    output,
                    reservation,
                )?
            }
            Producer::Distinct { input, .. } => {
                let output = producer_output(database, query, pipeline)?;
                let order = blocking::order::Order::distinct(
                    database,
                    plan.pipelines()[input.index()].output_columns(&query.plan),
                    reservation,
                )?;
                Self::Order { order, output }
            }
            Producer::SetOperation {
                left,
                right,
                descriptor,
            } => {
                let output = producer_output(database, query, pipeline)?;
                let bound = &query.plan.set_operations[usize::from(descriptor)];
                let inputs = [
                    &plan.pipelines()[left.index()],
                    &plan.pipelines()[right.index()],
                ];
                match bound.kind() {
                    query::SetKind::UnionAll => Self::Union {
                        union: union::Union::new(bound, inputs),
                        output,
                    },
                    query::SetKind::ExceptDistinct
                    | query::SetKind::IntersectDistinct
                    | query::SetKind::ExceptAll
                    | query::SetKind::IntersectAll => Self::SortedSet {
                        sorted_set: blocking::sorted_set::SortedSet::new(
                            database,
                            bound,
                            inputs,
                            reservation,
                        )?,
                        output,
                    },
                }
            }
            Producer::Limit { bounds, .. } => {
                let output = producer_output(database, query, pipeline)?;
                Self::Limit {
                    limit: limit::Limit::new(bounds),
                    output,
                }
            }
        })
    }

    fn analytic(
        database: &'db Database,
        query: &PreparedQuery<'_>,
        input: &Pipeline<'_>,
        partition: &[Option<u8>; query::MAX_PARTITION_KEYS],
        function: &WindowFunction,
        output: OwnedBatch<'db>,
        reservation: &mut Reservation<'db>,
    ) -> Result<Self, Error> {
        if input.column_count == 0 {
            if partition.iter().any(Option::is_some) || !matches!(function, WindowFunction::Count) {
                return Err(Error::Corrupt("analytic function requires input values"));
            }
            Ok(Self::Count {
                count: count::Count::new(input.column_count)?,
                output,
            })
        } else {
            let order = blocking::order::Order::window(
                database,
                input.output_columns(&query.plan),
                partition,
                function,
                reservation,
            )?;
            Ok(Self::Order { order, output })
        }
    }

    fn output(&self) -> &OwnedBatch<'db> {
        match self {
            Self::Scan { output, .. }
            | Self::Join { output, .. }
            | Self::Order { output, .. }
            | Self::Count { output, .. }
            | Self::Limit { output, .. }
            | Self::Union { output, .. }
            | Self::SortedSet { output, .. }
            | Self::Aggregate(output) => output,
            Self::Pending(_) | Self::Vacant => {
                unreachable!("runtime construction must finish before execution")
            }
        }
    }

    fn output_mut(&mut self) -> &mut OwnedBatch<'db> {
        match self {
            Self::Scan { output, .. }
            | Self::Join { output, .. }
            | Self::Order { output, .. }
            | Self::Count { output, .. }
            | Self::Limit { output, .. }
            | Self::Union { output, .. }
            | Self::SortedSet { output, .. }
            | Self::Aggregate(output) => output,
            Self::Pending(_) | Self::Vacant => {
                unreachable!("runtime construction must finish before execution")
            }
        }
    }

    fn memory_bytes(&self) -> u64 {
        self.output().memory_bytes()
            + match self {
                Self::Scan { cursor, .. } => cursor.memory_bytes(),
                Self::Join { join, .. } => join[0].memory_bytes(),
                Self::SortedSet { sorted_set, .. } => sorted_set[0].memory_bytes(),
                Self::Order { order, .. } => order[0].memory_bytes(),
                Self::Aggregate(_)
                | Self::Count { .. }
                | Self::Limit { .. }
                | Self::Union { .. } => 0,
                Self::Pending(_) | Self::Vacant => unreachable!("runtime admission is private"),
            }
    }

    fn replay(&mut self, cancel: &CancellationToken) -> Result<(), Error> {
        match self {
            Self::Scan { cursor, .. } => cursor.restart(cancel),
            Self::Count { count, .. } => count.replay(cancel),
            Self::Limit { limit, .. } => limit.replay(),
            Self::Union { union, .. } => union.replay(),
            Self::SortedSet { sorted_set, .. } => sorted_set[0].replay(cancel),
            Self::Join { join, .. } => join[0].replay(cancel),
            Self::Order { order, .. } => order[0].replay(cancel),
            _ => Err(Error::Corrupt("invalid replay producer")),
        }
    }
}

struct Node<'db> {
    owner: Owner<'db>,
    state: OutputState,
}

enum Phase {
    Run,
    Replay(usize),
    Done,
    Failed,
}

pub(super) struct Runtime<'db> {
    nodes: Vec<Node<'db>>,
    pub(super) aggregates: Vec<Aggregation<'db>>,
    parents: [Option<u8>; MAX_PIPELINES],
    active: usize,
    phase: Phase,
    // Free node and aggregate vectors before releasing this budget. It also
    // covers the heap storage holding blocking controllers: dropping a field
    // must not release the budget for the still-live object containing it.
    reservation: Reservation<'db>,
}

#[cfg(test)]
pub(super) struct WorkspaceView<'a, 'db> {
    pub(super) scan: &'a mut ScanCursor<'db>,
    pub(super) output: &'a mut OwnedBatch<'db>,
}

impl<'db> Runtime<'db> {
    pub(super) fn required_bytes(plan: &PhysicalPlan<'_>) -> u64 {
        let aggregates = plan
            .pipelines()
            .iter()
            .filter(|pipeline| matches!(pipeline.producer, Producer::Aggregate { .. }))
            .count();
        (plan.pipelines().len() * size_of::<Node<'_>>()
            + aggregates * size_of::<Aggregation<'_>>()
            + if aggregates == 0 {
                0
            } else {
                AGGREGATE_ALLOCATION_ALLOWANCE
            }) as u64
    }

    pub(super) const fn maximum_bytes() -> u64 {
        (MAX_PIPELINES * size_of::<Node<'_>>()
            + query::MAX_AGGREGATE_COLUMNS * size_of::<Aggregation<'_>>()
            + AGGREGATE_ALLOCATION_ALLOWANCE) as u64
    }
    // Reserve the node arrays and verify the parent links before installing
    // operators. Input-first order and one consumer per node prevent cycles and
    // competing consumers from sharing a mutable output batch.
    pub(super) fn admit(database: &'db Database, plan: &PhysicalPlan<'_>) -> Result<Self, Error> {
        let count = plan.pipelines().len();
        if count == 0 || count > MAX_PIPELINES {
            return Err(Error::Corrupt("runtime pipeline count"));
        }
        let reservation = database.reserve_memory(Self::required_bytes(plan), "runtime nodes")?;
        let nodes = allocate(count, count, "runtime node vector", reservation.bytes())?;
        let aggregate_count = plan
            .pipelines()
            .iter()
            .filter(|pipeline| matches!(pipeline.producer, Producer::Aggregate { .. }))
            .count();
        if aggregate_count > query::MAX_AGGREGATE_COLUMNS {
            return Err(Error::Corrupt("runtime aggregate count"));
        }
        let aggregates = allocate(
            aggregate_count,
            aggregate_count,
            "runtime aggregate vector",
            reservation.bytes(),
        )?;
        let mut parents = [None; MAX_PIPELINES];
        for (index, pipeline) in plan.pipelines().iter().enumerate() {
            let inputs = match pipeline.producer {
                Producer::Scan(_) => [None, None],
                Producer::Aggregate { input, .. }
                | Producer::Analytic { input, .. }
                | Producer::Order { input, .. }
                | Producer::Distinct { input, .. }
                | Producer::Limit { input, .. } => [Some(input), None],
                Producer::Join { left, right, .. } | Producer::SetOperation { left, right, .. } => {
                    [Some(left), Some(right)]
                }
            };
            for input in inputs.into_iter().flatten() {
                let child = input.index();
                if child >= index || parents[child].replace(index as u8).is_some() {
                    return Err(Error::Corrupt("runtime requires distinct earlier inputs"));
                }
            }
        }
        if parents[..count - 1].iter().any(Option::is_none) || parents[count - 1].is_some() {
            return Err(Error::Corrupt("runtime producer reachability"));
        }
        Ok(Self {
            nodes,
            aggregates,
            parents,
            active: count - 1,
            phase: Phase::Run,
            reservation,
        })
    }
    // The legacy workspace will supply the scan and first aggregate batches.
    // Reserve every additional output now, before optional grouping growth can
    // consume memory needed to pass rows between operators.
    pub(super) fn admit_legacy_outputs(
        mut self,
        database: &'db Database,
        query: &PreparedQuery<'_>,
        plan: &PhysicalPlan<'_>,
    ) -> Result<Self, Error> {
        if !self.nodes.is_empty() {
            return Err(Error::Corrupt("runtime already installed"));
        }
        for pipeline in plan.pipelines() {
            let owner = match pipeline.producer {
                Producer::Limit { bounds, .. } => Owner::Limit {
                    limit: limit::Limit::new(bounds),
                    output: producer_output(database, query, pipeline)?,
                },
                Producer::Analytic {
                    input,
                    partition,
                    function,
                } => Owner::analytic(
                    database,
                    query,
                    &plan.pipelines()[input.index()],
                    &partition,
                    &function,
                    producer_output(database, query, pipeline)?,
                    &mut self.reservation,
                )?,
                Producer::Scan(0) | Producer::Aggregate { aggregate: 0, .. } => Owner::Vacant,
                Producer::Aggregate { .. } => {
                    Owner::Aggregate(producer_output(database, query, pipeline)?)
                }
                _ => return Err(Error::Corrupt("invalid legacy producer")),
            };
            self.nodes.push(Node {
                owner,
                state: OutputState::Ready,
            });
        }
        Ok(self)
    }

    pub(super) fn install_workspace(
        mut self,
        workspace: AdmittedScan<'db>,
        plan: &PhysicalPlan<'_>,
    ) -> Result<Self, Error> {
        if self.nodes.len() != plan.pipelines().len() {
            return Err(Error::Corrupt("legacy runtime shape"));
        }
        let AdmittedScan {
            scan,
            input,
            output,
        } = workspace;
        let mut source = Some(Owner::Scan {
            cursor: scan,
            output: input,
        });
        let mut aggregate = Some(output);
        for (node, pipeline) in self.nodes.iter_mut().zip(plan.pipelines()) {
            match pipeline.producer {
                Producer::Scan(0) if matches!(node.owner, Owner::Vacant) => {
                    node.owner = source
                        .take()
                        .ok_or(Error::Corrupt("duplicate legacy scan"))?;
                }
                Producer::Aggregate { .. } if matches!(node.owner, Owner::Vacant) => {
                    node.owner = Owner::Aggregate(
                        aggregate
                            .take()
                            .ok_or(Error::Corrupt("duplicate legacy aggregate"))?,
                    );
                }
                Producer::Aggregate { .. } if matches!(node.owner, Owner::Aggregate(_)) => (),
                Producer::Limit { .. } if matches!(node.owner, Owner::Limit { .. }) => (),
                Producer::Analytic { .. }
                    if matches!(node.owner, Owner::Order { .. } | Owner::Count { .. }) => {}
                _ => return Err(Error::Corrupt("legacy owners disagree with producers")),
            }
        }
        if source.is_some() {
            return Err(Error::Corrupt("legacy source not installed"));
        }
        Ok(self)
    }

    pub(super) fn open_native(
        mut self,
        database: &'db Database,
        query: &'db PreparedQuery<'db>,
        plan: &PhysicalPlan<'_>,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Self, Error> {
        if !self.nodes.is_empty() {
            return Err(Error::Corrupt("runtime already installed"));
        }
        let mut scratch = crate::storage::catalog::Scratch::sized(
            &database.memory,
            crate::storage::catalog::MAX_BYTES,
            "native scan catalog admission",
        )?;
        for pipeline in plan.pipelines() {
            let owner =
                Owner::admit_native(database, query, plan, pipeline, &mut self.reservation)?;
            self.nodes.push(Node {
                owner,
                state: OutputState::Ready,
            });
        }
        self.open_aggregates(database, query, plan)?;
        for node in &mut self.nodes {
            if matches!(node.owner, Owner::Pending(_)) {
                let Owner::Pending(admission) = std::mem::replace(&mut node.owner, Owner::Vacant)
                else {
                    unreachable!("pending admission was matched");
                };
                let AdmittedScan {
                    scan,
                    input,
                    output,
                } = admission.open(&mut scratch, cancel, effects)?;
                drop(output);
                node.owner = Owner::Scan {
                    cursor: scan,
                    output: input,
                };
            }
        }
        Ok(self)
    }

    pub(super) fn open_aggregates(
        &mut self,
        database: &'db Database,
        query: &'db PreparedQuery<'db>,
        plan: &PhysicalPlan<'_>,
    ) -> Result<(), Error> {
        if !self.aggregates.is_empty() {
            return Err(Error::Corrupt("aggregate owners already installed"));
        }
        let utf8 = super::output_text_capacity(query).is_some();
        let mut minima = [0; MAX_AGGREGATE_COLUMNS];
        let mut count = 0;
        let mut total = 0_u64;
        for pipeline in plan.pipelines() {
            if let Producer::Aggregate {
                aggregate,
                input,
                demand,
            } = pipeline.producer
            {
                if usize::from(aggregate) != count || count == minima.len() {
                    return Err(Error::Corrupt("runtime aggregate owner index"));
                }
                minima[count] = Aggregation::minimum_bytes(
                    utf8,
                    query
                        .plan
                        .aggregates
                        .get(count)
                        .ok_or(Error::Corrupt("aggregate plan absent"))?,
                    demand,
                    plan.pipelines()[input.index()].output_columns(&query.plan),
                    pipeline.output_columns(&query.plan),
                )?;
                total = total
                    .checked_add(minima[count])
                    .ok_or(Error::Corrupt("combined aggregate minimum"))?;
                count += 1;
            }
        }
        let mut remaining = database.reserve_memory(total, "aggregate minima")?;
        for pipeline in plan.pipelines() {
            if let Producer::Aggregate {
                aggregate,
                input,
                demand,
            } = pipeline.producer
            {
                let index = usize::from(aggregate);
                // Release only this aggregate's share so it can reserve its own
                // storage. Keep later minima charged: optional growth here must
                // not make a later aggregate impossible to construct.
                let next = remaining
                    .bytes()
                    .checked_sub(minima[index])
                    .ok_or(Error::Corrupt("aggregate minimum reservation"))?;
                remaining.shrink_to(next);
                if self.aggregates.len() == self.aggregates.capacity() {
                    return Err(Error::Corrupt("aggregate vector capacity"));
                }
                self.aggregates.push(Aggregation::open(
                    database,
                    utf8,
                    &query.plan.aggregates[index],
                    demand,
                    plan.pipelines()[input.index()].output_columns(&query.plan),
                    pipeline.output_columns(&query.plan),
                )?);
            }
        }
        assert_eq!(remaining.bytes(), 0);
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn controller_inline_bytes(&self, plan: &PhysicalPlan<'_>) -> u64 {
        self.reservation.bytes() - Self::required_bytes(plan)
    }

    pub(super) fn memory_bytes(&self) -> u64 {
        self.reservation.bytes()
            + self
                .aggregates
                .iter()
                .map(Aggregation::memory_bytes)
                .sum::<u64>()
            + self
                .nodes
                .iter()
                .map(|node| node.owner.memory_bytes())
                .sum::<u64>()
    }

    pub(super) fn output(&self) -> &Batch {
        self.nodes
            .last()
            .expect("runtime has an output producer")
            .owner
            .output()
    }
    // Let tests request replay after consuming only a prefix, exposing state
    // that a replay from normal completion might fail to reset.
    #[cfg(test)]
    pub(super) fn replay_output_for_test(&mut self) {
        assert!(matches!(self.phase, Phase::Run | Phase::Done));
        self.active = self.nodes.len() - 1;
        self.phase = Phase::Replay(self.active);
    }

    #[cfg(test)]
    pub(super) fn into_output(mut self) -> OwnedBatch<'db> {
        match self.nodes.pop().expect("result producer").owner {
            Owner::Scan { output, .. } | Owner::Join { output, .. } | Owner::Aggregate(output) => {
                output
            }
            _ => panic!("runtime output is installed"),
        }
    }

    #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
    pub(super) fn scan(&self) -> &ScanCursor<'db> {
        let Owner::Scan { cursor, .. } = &self.nodes[0].owner else {
            panic!("first producer is scan");
        };
        cursor
    }

    #[cfg(test)]
    pub(super) fn scan_mut(&mut self) -> &mut ScanCursor<'db> {
        let Owner::Scan { cursor, .. } = &mut self.nodes[0].owner else {
            panic!("first producer is scan");
        };
        cursor
    }

    #[cfg(test)]
    pub(super) fn grouping_workspace(&mut self) -> WorkspaceView<'_, 'db> {
        let (source, rest) = self.nodes.split_first_mut().expect("source");
        let Owner::Scan { cursor, .. } = &mut source.owner else {
            panic!("source cursor");
        };
        WorkspaceView {
            scan: cursor,
            output: rest
                .last_mut()
                .expect("aggregate producer")
                .owner
                .output_mut(),
        }
    }

    #[cfg(test)]
    pub(super) fn first_join_mut(&mut self) -> &mut blocking::join::Join<'db> {
        self.nodes
            .iter_mut()
            .find_map(|node| match &mut node.owner {
                Owner::Join { join, .. } => Some(&mut join[0]),
                _ => None,
            })
            .expect("test query has a join")
    }

    #[cfg(test)]
    pub(super) fn first_sorted_set_mut(&mut self) -> &mut blocking::sorted_set::SortedSet<'db> {
        self.nodes
            .iter_mut()
            .find_map(|node| match &mut node.owner {
                Owner::SortedSet { sorted_set, .. } => Some(&mut sorted_set[0]),
                _ => None,
            })
            .expect("test query has a sorted-set producer")
    }

    #[cfg(test)]
    pub(super) fn first_order_mut(&mut self) -> &mut blocking::order::Order<'db> {
        self.nodes
            .iter_mut()
            .find_map(|node| match &mut node.owner {
                Owner::Order { order, .. } => Some(&mut order[0]),
                _ => None,
            })
            .expect("test query has an order producer")
    }

    pub(super) fn step(
        &mut self,
        plan: &PhysicalPlan<'_>,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Advance, Error> {
        // Calling with &mut self proves the caller no longer borrows the last
        // result batch. Clear only that batch; children may still be partly read.
        self.nodes
            .last_mut()
            .ok_or(Error::Corrupt("runtime has no nodes"))?
            .owner
            .output_mut()
            .clear();
        // Any error in advance leaves Failed installed. A subsequent step must
        // not continue from half-updated operator state.
        let phase = std::mem::replace(&mut self.phase, Phase::Failed);
        let (advance, phase) = self.advance(phase, plan, cancel, effects)?;
        self.phase = phase;
        Ok(advance)
    }

    fn advance(
        &mut self,
        phase: Phase,
        plan: &PhysicalPlan<'_>,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(Advance, Phase), Error> {
        match phase {
            Phase::Failed => return Err(Error::Corrupt("runtime has failed")),
            Phase::Done => return Ok((Advance::Finished, Phase::Done)),
            Phase::Replay(input) => {
                cancel.check()?;
                let node = self
                    .nodes
                    .get_mut(input)
                    .ok_or(Error::Corrupt("replay input absent"))?;
                if let Producer::Aggregate { aggregate, .. } = plan.pipelines()[input].producer {
                    self.aggregates
                        .get_mut(usize::from(aggregate))
                        .ok_or(Error::Corrupt("aggregate replay owner absent"))?
                        .replay(cancel)?;
                } else {
                    node.owner.replay(cancel)?;
                }
                node.owner.output_mut().clear();
                node.state = OutputState::Ready;
                // LIMIT keeps only counters, not retained input. Replaying it
                // must also rewind its child; other replayable owners handle
                // their own retained data or request input replay when stepped.
                let next = if let Producer::Limit { input: child, .. } =
                    plan.pipelines()[input].producer
                {
                    if child.index() >= input {
                        return Err(Error::Corrupt("replay input must precede limiter"));
                    }
                    Phase::Replay(child.index())
                } else {
                    Phase::Run
                };
                return Ok((Advance::Progress, next));
            }
            Phase::Run => (),
        }
        let index = self.active;
        let pipeline = plan
            .pipelines()
            .get(index)
            .ok_or(Error::Corrupt("active producer absent"))?;
        let advanced = match pipeline.producer {
            Producer::Scan(_) => {
                let node = self
                    .nodes
                    .get_mut(index)
                    .ok_or(Error::Corrupt("scan node absent"))?;
                let Owner::Scan { cursor, output } = &mut node.owner else {
                    return Err(Error::Corrupt("scan cursor absent"));
                };
                cursor.advance(output, pipeline, cancel, effects)?
            }
            Producer::Aggregate { input, .. }
            | Producer::Analytic { input, .. }
            | Producer::Order { input, .. }
            | Producer::Distinct { input, .. }
            | Producer::Limit { input, .. } => {
                let child = input.index();
                if child >= index {
                    return Err(Error::Corrupt("aggregate input must precede consumer"));
                }
                // Input-first order makes these disjoint borrows possible:
                // the consumer can change its state while borrowing child rows.
                let (earlier, consumers) = self.nodes.split_at_mut(index);
                let input = earlier
                    .get_mut(child)
                    .ok_or(Error::Corrupt("aggregate input absent"))?;
                let owner = &mut consumers
                    .first_mut()
                    .ok_or(Error::Corrupt("unary output absent"))?
                    .owner;
                let supplied = ConsumerInput {
                    batch: input.owner.output(),
                    finished: input.state == OutputState::Finished,
                };
                let step = match (pipeline.producer, owner) {
                    (Producer::Limit { .. }, Owner::Limit { limit, output }) => {
                        limit.step(supplied, output, pipeline, cancel)?
                    }
                    (Producer::Analytic { .. }, Owner::Count { count, output }) => {
                        count.step(supplied, output, pipeline, cancel)?
                    }
                    (
                        Producer::Order { .. }
                        | Producer::Distinct { .. }
                        | Producer::Analytic { .. },
                        Owner::Order { order, output },
                    ) => order[0].step(supplied, output, pipeline, cancel, effects)?,
                    (Producer::Aggregate { aggregate, .. }, Owner::Aggregate(output)) => match self
                        .aggregates
                        .get_mut(usize::from(aggregate))
                        .ok_or(Error::Corrupt("aggregate state absent"))?
                    {
                        Aggregation::Dense(groups) => {
                            groups.step(supplied, output, pipeline, cancel)?
                        }
                        Aggregation::General(groups) => {
                            groups[0].step(supplied, output, pipeline, cancel, effects)?
                        }
                    },
                    _ => return Err(Error::Corrupt("unary producer owner mismatch")),
                };
                match step {
                    ConsumerStep::Input => {
                        if input.state == OutputState::Finished {
                            return Err(Error::Corrupt("consumer requested finished input"));
                        }
                        // Input means the consumer has finished this batch.
                        // Only now may its producer reuse the output storage.
                        input.owner.output_mut().clear();
                        input.state = OutputState::Ready;
                        self.active = child;
                        return Ok((Advance::Progress, Phase::Run));
                    }
                    ConsumerStep::Replay => return Ok((Advance::Progress, Phase::Replay(child))),
                    ConsumerStep::Progress => Advance::Progress,
                    ConsumerStep::Rows => Advance::Rows,
                    ConsumerStep::Finished => Advance::Finished,
                }
            }
            Producer::SetOperation { left, right, .. } => {
                let children = [left.index(), right.index()];
                if children[0] >= index || children[1] >= index || left == right {
                    return Err(Error::Corrupt(
                        "set inputs must be distinct earlier producers",
                    ));
                }
                let (earlier, consumers) = self.nodes.split_at_mut(index);
                let supplied = children.map(|child| ConsumerInput {
                    batch: earlier[child].owner.output(),
                    finished: earlier[child].state == OutputState::Finished,
                });
                let step = match &mut consumers[0].owner {
                    Owner::Union { union, output } => {
                        union.step(supplied, output, pipeline, cancel)?
                    }
                    Owner::SortedSet { sorted_set, output } => {
                        sorted_set[0].step(supplied, output, pipeline, cancel, effects)?
                    }
                    _ => return Err(Error::Corrupt("set operation owner absent")),
                };
                match step {
                    SetStep::Input(side) => {
                        let child = children[side];
                        let input = &mut earlier[child];
                        if input.state == OutputState::Finished {
                            return Err(Error::Corrupt("set operation requested finished input"));
                        }
                        input.owner.output_mut().clear();
                        input.state = OutputState::Ready;
                        self.active = child;
                        return Ok((Advance::Progress, Phase::Run));
                    }
                    SetStep::Replay(side) => {
                        return Ok((Advance::Progress, Phase::Replay(children[side])));
                    }
                    SetStep::Progress => Advance::Progress,
                    SetStep::Rows => Advance::Rows,
                    SetStep::Finished => Advance::Finished,
                }
            }
            Producer::Join { left, right, .. } => {
                let (earlier, consumers) = self.nodes.split_at_mut(index);
                let left = left.index();
                let right = right.index();
                if left >= index || right >= index || left == right {
                    return Err(Error::Corrupt(
                        "join inputs must be distinct earlier producers",
                    ));
                }
                let supplied = [left, right].map(|child| ConsumerInput {
                    batch: earlier[child].owner.output(),
                    finished: earlier[child].state == OutputState::Finished,
                });
                let Owner::Join { join, output } = &mut consumers[0].owner else {
                    return Err(Error::Corrupt("join owner absent"));
                };
                match join[0].step(supplied, output, pipeline, cancel, effects)? {
                    blocking::join::Step::Input(side) => {
                        let child = [left, right][side];
                        let input = &mut earlier[child];
                        if input.state == OutputState::Finished {
                            return Err(Error::Corrupt("join requested finished input"));
                        }
                        input.owner.output_mut().clear();
                        input.state = OutputState::Ready;
                        self.active = child;
                        return Ok((Advance::Progress, Phase::Run));
                    }
                    blocking::join::Step::Progress => Advance::Progress,
                    blocking::join::Step::Rows => Advance::Rows,
                    blocking::join::Step::Finished => Advance::Finished,
                }
            }
        };
        match advanced {
            Advance::Progress => Ok((Advance::Progress, Phase::Run)),
            Advance::Rows | Advance::Finished => {
                let finished = matches!(advanced, Advance::Finished);
                self.nodes[index].state = if finished {
                    OutputState::Finished
                } else {
                    OutputState::Rows
                };
                // Rows and end-of-input are both messages for the consumer.
                // Return public rows only when this node has no parent.
                if let Some(parent) = self.parents[index] {
                    self.active = usize::from(parent);
                    Ok((Advance::Progress, Phase::Run))
                } else {
                    Ok((advanced, if finished { Phase::Done } else { Phase::Run }))
                }
            }
        }
    }
}

// Use the query's text representation at every operator boundary. Even legacy
// input can produce variable-length text through a later constant expression;
// an intermediate batch must be able to carry it to the result.
fn producer_output<'db>(
    database: &'db Database,
    query: &PreparedQuery<'_>,
    pipeline: &Pipeline,
) -> Result<OwnedBatch<'db>, Error> {
    let mut types = [DataType::Int64; MAX_ROW_VALUES];
    let mut text = [None; MAX_ROW_VALUES];
    for (index, column) in pipeline.output_columns(&query.plan).enumerate() {
        types[index] = column.data_type();
        if types[index] == DataType::String {
            text[index] = super::output_text_capacity(query);
        }
    }
    let types = &types[..pipeline.column_count];
    let text = &text[..pipeline.column_count];
    let mut reservation = database.reserve_memory(
        Batch::required_bytes_with_text(types, text)?,
        "producer output",
    )?;
    OwnedBatch::new_with_text(types, text, &mut reservation)
}
