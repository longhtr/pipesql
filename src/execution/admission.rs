//! Validate a prepared query and admit every retained owner before source I/O.
use super::{QueryResult, RESULT_BYTES, State, computed};
use crate::effects::Effects;
use crate::execution::computed::BatchLayout;
use crate::execution::planning::snapshot_matches;
use crate::execution::planning::{PhysicalPlan, lower, validate_physical};
use crate::execution::runtime::Runtime;
use crate::execution::scan::{declared, legacy};
use crate::frontend::{self, PreparedQuery};
use crate::namespace::inspect_namespace;
use crate::storage_format::{self, RootState};
use crate::{CancellationToken, Database, Error};

impl Database {
    /// Upper bound for legacy or declared scan workspace, transient admission
    /// scratch, bounded controller metadata and result ownership. Excludes
    /// database, prepared, aggregate buffers, join, ordering and additional
    /// producer-output ownership. This is a scan
    /// ceiling, not a bound for an arbitrary query graph.
    pub const fn execution_memory_requirement_bytes() -> u64 {
        let workspace = if legacy::MAX_WORKSPACE_BYTES > declared::MAX_WORKSPACE_BYTES {
            legacy::MAX_WORKSPACE_BYTES
        } else {
            declared::MAX_WORKSPACE_BYTES
        };
        workspace
            + BatchLayout::MAX_BYTES
            + RESULT_BYTES
            + computed::ROW_SCRATCH_BYTES
            + PhysicalPlan::maximum_bytes()
            + Runtime::maximum_bytes()
    }

    /// Admit execution of a prepared query against its retained snapshot.
    ///
    /// The result borrows the database, query, and cancellation token. Successful
    /// admission does not mean the query completed: consume [`crate::QueryResult::step`]
    /// through [`crate::QueryStep::Finished`] or handle its terminal failure.
    ///
    /// An unavailable handle or a query from another database/snapshot returns
    /// [`Error::Unsupported`]. Validation, resource admission, cancellation, and
    /// opening the sources may also fail before a result is returned.
    pub fn execute<'db, 'cancel>(
        &'db self,
        query: &'db PreparedQuery<'_>,
        cancellation: &'cancel CancellationToken,
    ) -> Result<QueryResult<'db, 'cancel>, Error> {
        self.execute_with_effects(query, cancellation, &mut Effects::default())
    }

    pub(in crate::execution) fn execute_with_effects<'db, 'cancel>(
        &'db self,
        query: &'db PreparedQuery<'_>,
        cancellation: &'cancel CancellationToken,
        effects: &mut Effects,
    ) -> Result<QueryResult<'db, 'cancel>, Error> {
        let unpublished_legacy = self.temporary.reserved() != 0;
        // Catalog queries prove ownership against a pinned commit below.
        // Private append reservations do not change that commit.
        let unpublished_legacy = unpublished_legacy && query.snapshot.is_none();
        if self.needs_reopen() || unpublished_legacy {
            return Err(Error::Unsupported("reopen is required before execution"));
        }
        cancellation.check()?;
        frontend::validate(&query.plan)?;
        if query.plan.database != self.database_identity() || !snapshot_matches(query, self) {
            return Err(Error::Unsupported(
                "prepared query belongs to a different snapshot",
            ));
        }
        if let Some(snapshot) = &query.snapshot {
            let plan = lower(self, query, snapshot.state(), 0)?;
            validate_physical(&plan, query, self, snapshot.state(), 0)?;
            let reservation =
                self.reserve_memory(RESULT_BYTES + plan.row_scratch_bytes(), "streaming result")?;
            let runtime = Runtime::admit(self, &plan)?;
            let runtime = runtime.open_native(self, query, &plan, cancellation, effects)?;
            let state = State::Running(runtime);
            return Ok(QueryResult {
                plan,
                state,
                cancellation,
                reservation,
            });
        }
        let mut plan = lower(self, query, RootState::Empty, 0)?;
        if query.plan.has_joins() {
            validate_physical(&plan, query, self, RootState::Empty, 0)?;
            return Err(Error::Corrupt("legacy plan contains a join"));
        }
        let runtime = Runtime::admit(self, &plan)?;
        let buffers = legacy::Layout::new(query, &plan);
        let workspace_reservation =
            self.reserve_memory(buffers.workspace_bytes()?, "streaming scan workspace")?;
        let mut result_reservation =
            self.reserve_memory(RESULT_BYTES + plan.row_scratch_bytes(), "streaming result")?;
        let mut runtime = runtime.admit_legacy_outputs(self, query, &plan)?;
        runtime.open_aggregates(self, query, &plan)?;
        let namespace = inspect_namespace(self.path(), &self.memory, effects)?;
        if namespace.database_id != self.database_identity()
            || namespace.generation != self.generation()
        {
            return Err(Error::Corrupt("database changed before planning"));
        }
        plan.root = namespace.state;
        plan.projected_crc32c = namespace.projected_crc32c;
        validate_physical(
            &plan,
            query,
            self,
            namespace.state,
            namespace.projected_crc32c,
        )?;
        buffers.validate(&plan, query)?;
        let has_aggregate = !runtime.aggregates.is_empty();
        let state = match namespace.state {
            RootState::Catalog(_) => return Err(Error::UnsupportedVersion(6)),
            RootState::Empty => {
                if has_aggregate {
                    State::Running(
                        runtime
                            .install_workspace(buffers.open_empty(workspace_reservation)?, &plan)?,
                    )
                } else {
                    drop(workspace_reservation);
                    drop(runtime);
                    State::Finished
                }
            }
            RootState::Data {
                rows,
                unit_bytes,
                unit_metadata_crc32c,
                ..
            } => {
                let layout = storage_format::layout(rows)
                    .map_err(|_| Error::Corrupt("invalid stored row count"))?;
                State::Running(runtime.install_workspace(
                    legacy::open_workspace(
                        self.path(),
                        legacy::UnitExpectation {
                            database_id: self.database_identity(),
                            rows,
                            unit_bytes,
                            unit_metadata_crc32c,
                            projected_crc32c: namespace.projected_crc32c,
                            descriptor_count: layout.descriptor_count,
                        },
                        layout.double_blocks,
                        buffers,
                        workspace_reservation,
                        cancellation,
                        effects,
                    )?,
                    &plan,
                )?)
            }
        };
        if matches!(state, State::Finished) {
            plan.clear();
            result_reservation.shrink_to(RESULT_BYTES);
        }
        Ok(QueryResult {
            plan,
            state,
            cancellation,
            reservation: result_reservation,
        })
    }
}
