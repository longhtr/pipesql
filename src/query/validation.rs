//! Reject inconsistent query plans before they become usable prepared queries.
//!
//! Binding builds a plan; this module checks the completed result without calling
//! the binder. It first checks array bounds and source columns, then follows the
//! stages in order. Each stage must use available inputs, have valid type facts
//! and account for the expressions or column mappings it introduces.
//!
//! A backward walk also checks that every producer contributes to the final
//! result and has only one consumer. Otherwise a malformed link could silently
//! drop a filter or make two consumers share execution state unexpectedly.
//!
//! The validator shares types and read-only plan accessors with binding, but
//! reconstructs the required relationships separately. Failures return `Corrupt`
//! because they indicate a malformed internal plan. User syntax and name errors
//! should already have failed in parsing or binding.

use super::{
    AggregateArgument, AggregateKind, ColumnId, ColumnSet, DataType, Error, Group,
    MAX_AGGREGATE_COLUMNS, MAX_COLUMNS, MAX_COMPUTED, MAX_ORDER_ITEMS, MAX_PROJECTIONS,
    MAX_QUERY_COLUMNS, MAX_ROW_VALUES, MAX_SOURCE_BYTES, MAX_STAGES, Name, Node, OrderKey, Output,
    Plan, RelationId, SetAssignment, SourceColumn, SourceOccurrence, Stage, initial_outputs,
};

pub(crate) fn validate(plan: &Plan) -> Result<(), Error> {
    let catalog = plan.table().is_some();
    let (_, stock_sources, stock_count) = initial_outputs();
    if (!catalog && plan.generation > 1)
        || (catalog && plan.generation > crate::storage::format::MAX_SUCCESSES)
        || usize::from(plan.source_bytes) > MAX_SOURCE_BYTES
        || usize::from(plan.count) > MAX_STAGES
        || plan.source_count == 0
        || usize::from(plan.source_count) > MAX_COLUMNS
        || plan.occurrence_count == 0
        || usize::from(plan.occurrence_count) > MAX_STAGES
    {
        return Err(Error::Corrupt("invalid plan envelope"));
    }
    let mut source_end = 0;
    // Source occurrences partition one column array. Each use of a table needs
    // distinct query identities, even if its persistent columns are the same.
    for occurrence in &plan.occurrences[..usize::from(plan.occurrence_count)] {
        let start = usize::from(occurrence.start);
        let end = start + usize::from(occurrence.columns);
        if start != source_end
            || occurrence.columns == 0
            || end > usize::from(plan.source_count)
            || occurrence.table.is_some() != catalog
            || (!catalog && occurrence.columns != stock_count)
        {
            return Err(Error::Corrupt("invalid source occurrence"));
        }
        for (ordinal, column) in plan.source_columns[start..end].iter().enumerate() {
            let index = start + ordinal;
            if column.identity != ColumnId::new(index as u32 + 1)
                || usize::from(column.storage) != ordinal
                || (catalog
                    && (plan.catalog_columns[index] == 0
                        || plan.catalog_columns[start..index]
                            .contains(&plan.catalog_columns[index])))
                || (!catalog
                    && (column.kind != stock_sources[ordinal].kind
                        || column.nullable != stock_sources[ordinal].nullable))
            {
                return Err(Error::Corrupt("invalid source identity or position"));
            }
        }
        source_end = end;
    }
    if source_end != usize::from(plan.source_count)
        || plan.occurrences[usize::from(plan.occurrence_count)..]
            .iter()
            .any(|entry| *entry != SourceOccurrence::EMPTY)
    {
        return Err(Error::Corrupt("invalid source occurrence tail"));
    }
    if plan.source_columns[usize::from(plan.source_count)..]
        .iter()
        .any(|column| *column != SourceColumn::QUANTITY)
    {
        return Err(Error::Corrupt("invalid source row tail"));
    }
    let catalog_tail = if catalog {
        usize::from(plan.source_count)
    } else {
        0
    };
    if plan.catalog_columns[catalog_tail..]
        .iter()
        .any(|id| *id != 0)
    {
        return Err(Error::Corrupt("invalid catalog origin tail"));
    }
    let initial_ranges = plan.relation_columns(RelationId::SOURCE)?.identity_set()?;
    if plan.range_columns[0] != initial_ranges
        || plan.range_columns[usize::from(plan.count) + 1..]
            .iter()
            .any(|set| *set != ColumnSet::EMPTY)
    {
        return Err(Error::Corrupt("invalid range scope envelope"));
    }
    let mut null_cursor = 0;
    // A descriptor cursor advances as its owning stage is checked. Exact final
    // counts below reject both unused descriptors and reuse by multiple stages.
    if plan.null_extensions.len() > MAX_STAGES
        || plan.null_extensions.capacity() != plan.null_extensions.len()
    {
        return Err(Error::Corrupt("null extension descriptor capacity"));
    }
    let mut set_cursor = 0;
    if plan.set_operations.len() > MAX_STAGES
        || plan.set_operations.capacity() != plan.set_operations.len()
    {
        return Err(Error::Corrupt("set descriptor capacity"));
    }
    let mut distinct_cursor = 0;
    if plan.distinct.len() > MAX_STAGES || plan.distinct.capacity() != plan.distinct.len() {
        return Err(Error::Corrupt("DISTINCT descriptor capacity"));
    }
    let mut aggregate_cursor = 0;
    let mut aggregate_outputs = 0;
    if plan.aggregates.len() > MAX_AGGREGATE_COLUMNS
        || plan.aggregates.capacity() != plan.aggregates.len()
    {
        return Err(Error::Corrupt("aggregate plan capacity"));
    }
    let mut next_identity = u32::from(plan.source_count) + 1;
    let mut computed_cursor = 0;
    if plan.computed.len() > MAX_COMPUTED {
        return Err(Error::Corrupt("computed definition capacity"));
    }
    // Recompute the accepted vector capacity without the binder's allocation
    // helper. Whole slots fit below a 16-KiB boundary with 32 bytes of headroom;
    // extra capacity must not be mistaken for additional expression definitions.
    let width = std::mem::size_of::<super::Computed>();
    let payload = plan.computed.len() * width;
    let capacity = if payload <= 16_384 {
        plan.computed.len()
    } else {
        ((payload + 32).div_ceil(16_384) * 16_384 - 32) / width
    };
    if plan.computed.capacity() != capacity {
        return Err(Error::Corrupt("computed definition capacity"));
    }
    let mut projection_cursor = 0;
    let mut assignment_cursor = 0;
    if usize::from(plan.assignment_count) > MAX_PROJECTIONS
        || plan.assignments[usize::from(plan.assignment_count)..]
            .iter()
            .any(|assignment| *assignment != SetAssignment::EMPTY)
    {
        return Err(Error::Corrupt("invalid SET assignment envelope"));
    }
    let mut order_cursor = 0;
    if usize::from(plan.order_count) > MAX_ORDER_ITEMS {
        return Err(Error::Corrupt("order item count"));
    }
    if usize::from(plan.projection_count) > MAX_PROJECTIONS {
        return Err(Error::Corrupt("invalid projection count"));
    }
    for (index, node) in plan.stages[..usize::from(plan.count)].iter().enumerate() {
        plan.node(RelationId(index as u8 + 1))?;
        let input = plan.relation_columns(node.input)?;
        validate_range_scope(plan, index, node)?;
        let available = plan.available_columns(node.input)?;
        let width = match &node.stage {
            Stage::Source(source) => {
                if *source == 0
                    || *source >= plan.occurrence_count
                    || node.input != RelationId::SOURCE
                    || plan.stages[..index]
                        .iter()
                        .any(|prior| prior.stage == Stage::Source(*source))
                {
                    return Err(Error::Corrupt("invalid source producer"));
                }
                usize::from(plan.occurrences[usize::from(*source)].columns)
            }
            Stage::Join {
                nulls: _,
                right,
                left_key,
                right_key,
            } => {
                let right_available = plan.available_columns(*right)?;
                let right = plan.relation_columns(*right)?;
                if !available.contains(*left_key)
                    || !right_available.contains(*right_key)
                    || plan.column_type(*left_key).map(|facts| facts.0)
                        != plan.column_type(*right_key).map(|facts| facts.0)
                {
                    return Err(Error::Corrupt("invalid join keys"));
                }
                input
                    .len()
                    .checked_add(right.len())
                    .ok_or(Error::Corrupt("join width overflow"))?
            }
            Stage::Alias
            | Stage::Rename
            | Stage::Set { .. }
            | Stage::Derived
            | Stage::Where(_)
            | Stage::Order { .. }
            | Stage::Limit(_)
            | Stage::Distinct(_)
            | Stage::SetOperation { .. } => input.len(),
            Stage::Drop { keep } => keep.count_ones() as usize,
            Stage::Select { len, .. } => usize::from(*len),
            Stage::Extend { len, .. } => input.len() + usize::from(*len),
            Stage::Aggregate(aggregate_index) => plan
                .aggregates
                .get(usize::from(*aggregate_index))
                .map(|aggregate| usize::from(aggregate.group_count) + aggregate.entries.len())
                .ok_or(Error::Corrupt("aggregate producer absent"))?,
            Stage::Empty => return Err(Error::Corrupt("empty stage inside plan")),
        };
        if width == 0 || width > MAX_COLUMNS || usize::from(node.columns) != width {
            return Err(Error::Corrupt("invalid producer width"));
        }
        match &node.stage {
            Stage::Join {
                nulls: Some(descriptor),
                right,
                ..
            } => {
                if !catalog || usize::from(*descriptor) != null_cursor {
                    return Err(Error::Corrupt("null extension descriptor ownership"));
                }
                let bound = plan
                    .null_extensions
                    .get(null_cursor)
                    .ok_or(Error::Corrupt("null extension descriptor absent"))?;
                next_identity = bound.validate(plan, *right, next_identity)?;
                null_cursor += 1;
            }
            Stage::SetOperation { right, descriptor } => {
                if !catalog || usize::from(*descriptor) != set_cursor {
                    return Err(Error::Corrupt("set descriptor ownership"));
                }
                let set = plan
                    .set_operations
                    .get(set_cursor)
                    .ok_or(Error::Corrupt("set descriptor absent"))?;
                next_identity =
                    set.validate(plan, input, plan.relation_columns(*right)?, next_identity)?;
                set_cursor += 1;
            }
            Stage::Distinct(descriptor) => {
                if !catalog || usize::from(*descriptor) != distinct_cursor {
                    return Err(Error::Corrupt("DISTINCT descriptor ownership"));
                }
                let bound = plan
                    .distinct
                    .get(distinct_cursor)
                    .ok_or(Error::Corrupt("DISTINCT descriptor absent"))?;
                next_identity = bound.validate(plan, input, next_identity)?;
                distinct_cursor += 1;
            }
            Stage::Limit(bounds) => bounds.validate()?,
            Stage::Order { start, len } => {
                if !catalog || usize::from(*start) != order_cursor {
                    return Err(Error::Corrupt("order producer range"));
                }
                for item in plan.order_items(*start, *len)? {
                    if !available.contains(item.column) || plan.column_type(item.column).is_none() {
                        return Err(Error::Corrupt("order key outside input"));
                    }
                }
                order_cursor += usize::from(*len);
            }
            Stage::Source(_)
            | Stage::Alias
            | Stage::Rename
            | Stage::Derived
            | Stage::Join { .. } => (),
            Stage::Aggregate(aggregate_index) => {
                let aggregate = plan
                    .aggregates
                    .get(usize::from(*aggregate_index))
                    .ok_or(Error::Corrupt("missing aggregate plan"))?;
                if usize::from(*aggregate_index) != aggregate_cursor
                    || aggregate.first_output != ColumnId::new(next_identity)
                    || aggregate.entries.is_empty()
                    || aggregate.entries.len() + usize::from(aggregate.group_count)
                        > MAX_AGGREGATE_COLUMNS
                    || usize::from(aggregate.group_count) > MAX_AGGREGATE_COLUMNS - 1
                    || (!catalog && aggregate.group_count > 2)
                {
                    return Err(Error::Corrupt("invalid aggregate shape"));
                }
                let mut sources = [SourceColumn::QUANTITY.semantic(); MAX_ROW_VALUES];
                for (source, id) in sources.iter_mut().zip(available.iter()) {
                    *source = plan
                        .column(id)
                        .ok_or(Error::Corrupt("aggregate input has no semantic facts"))?;
                }
                for entry in &aggregate.entries {
                    let valid = match (entry.kind, &entry.argument) {
                        (AggregateKind::Count, None) => true,
                        (_, Some(AggregateArgument::Numeric(expression))) => {
                            expression.validate(&sources[..available.len()])?;
                            matches!(expression.data_type, DataType::Int64 | DataType::Double)
                        }
                        (AggregateKind::Count, Some(AggregateArgument::Column(column))) => {
                            matches!(column.data_type(), DataType::String | DataType::Date)
                                && sources[..available.len()].contains(column)
                        }
                        (
                            AggregateKind::Min | AggregateKind::Max,
                            Some(AggregateArgument::Column(column)),
                        ) => {
                            matches!(column.data_type(), DataType::Date | DataType::String)
                                && sources[..available.len()].contains(column)
                        }
                        _ => false,
                    };
                    if !valid
                        || !entry.name.valid()
                        || entry.span.start >= entry.span.end
                        || entry.span.end > plan.source_bytes
                    {
                        return Err(Error::Corrupt("invalid aggregate input"));
                    }
                }
                if aggregate.entries.capacity() != aggregate.entries.len()
                    || aggregate.groups[usize::from(aggregate.group_count)..]
                        .iter()
                        .any(|group| *group != Group::EMPTY)
                {
                    return Err(Error::Corrupt("invalid aggregate tail"));
                }
                for (index, group) in aggregate.groups[..usize::from(aggregate.group_count)]
                    .iter()
                    .enumerate()
                {
                    if plan.column(group.input.identity) != Some(group.input)
                        || (!catalog
                            && (group.input.data_type(), group.input.nullable())
                                != (DataType::String, false))
                        || !group.name.valid()
                        || !available.contains(group.input.identity)
                        || aggregate.groups[..index]
                            .iter()
                            .any(|prior| prior.input.identity == group.input.identity)
                    {
                        return Err(Error::Corrupt("invalid aggregate key"));
                    }
                }
                next_identity += width as u32;
                aggregate_cursor += 1;
                aggregate_outputs += width;
                if aggregate_outputs > MAX_AGGREGATE_COLUMNS {
                    return Err(Error::Corrupt("aggregate output identity limit"));
                }
            }
            Stage::Drop { keep } => {
                let allowed = u64::MAX >> (64 - input.len());
                if *keep == 0 || *keep == allowed || *keep & !allowed != 0 {
                    return Err(Error::Corrupt("invalid DROP column mask"));
                }
            }
            Stage::Set { start, len } => {
                let end = usize::from(*start) + usize::from(*len);
                if *len == 0
                    || usize::from(*start) != assignment_cursor
                    || end > usize::from(plan.assignment_count)
                {
                    return Err(Error::Corrupt("invalid SET assignment range"));
                }
                let mut positions = 0_u64;
                let mut columns = [SourceColumn::QUANTITY.semantic(); MAX_ROW_VALUES];
                for (slot, id) in columns.iter_mut().zip(available.iter()) {
                    *slot = plan.column(id).ok_or(Error::Corrupt("SET input facts"))?;
                }
                for assignment in &plan.assignments[assignment_cursor..end] {
                    let position = usize::from(assignment.position);
                    if position >= input.len() || positions & (1_u64 << position) != 0 {
                        return Err(Error::Corrupt("invalid SET target position"));
                    }
                    positions |= 1_u64 << position;
                    let definition = plan
                        .computed
                        .get(computed_cursor)
                        .ok_or(Error::Corrupt("SET definition absent"))?;
                    if definition.expression.is_analytic()
                        || definition.column.identity() != assignment.column
                        || assignment.column.value() != next_identity
                        || definition.input != node.input
                        || definition.span.start >= definition.span.end
                        || definition.span.end > plan.source_bytes
                        || definition.column.data_type() != definition.expression.data_type()
                        || definition.column.nullable() != definition.expression.nullable()
                    {
                        return Err(Error::Corrupt("invalid SET definition"));
                    }
                    definition
                        .expression
                        .validate(&columns[..available.len()])?;
                    next_identity += 1;
                    computed_cursor += 1;
                }
                assignment_cursor = end;
            }
            Stage::Select { start, len } | Stage::Extend { start, len } => {
                let end = usize::from(*start) + usize::from(*len);
                if *len == 0
                    || usize::from(*len) > MAX_COLUMNS
                    || usize::from(*start) != projection_cursor
                    || end > usize::from(plan.projection_count)
                {
                    return Err(Error::Corrupt("invalid projection range"));
                }
                let outputs = &plan.projections[projection_cursor..end];
                let mut visible = [SourceColumn::QUANTITY.semantic(); MAX_ROW_VALUES];
                for (slot, id) in visible.iter_mut().zip(available.iter()) {
                    *slot = plan
                        .column(id)
                        .ok_or(Error::Corrupt("projection input facts"))?;
                }
                let mut window = None;
                for output in outputs {
                    if available.contains(*output) {
                        continue;
                    }
                    let definition = plan
                        .computed
                        .get(computed_cursor)
                        .ok_or(Error::Corrupt("missing computed definition"))?;
                    if definition.column.identity() != *output
                        || output.value() != next_identity
                        || definition.input != node.input
                        || definition.span.start >= definition.span.end
                        || definition.span.end > plan.source_bytes
                        || definition.column.data_type() != definition.expression.data_type()
                        || definition.column.nullable() != definition.expression.nullable()
                    {
                        return Err(Error::Corrupt("invalid computed definition"));
                    }
                    definition
                        .expression
                        .validate(&visible[..available.len()])?;
                    if definition.expression.is_analytic() {
                        if window.is_some_and(|prior| prior != &definition.expression) {
                            return Err(Error::Corrupt(
                                "projection has different analytic specifications",
                            ));
                        }
                        window = Some(&definition.expression);
                    }
                    next_identity += 1;
                    computed_cursor += 1;
                }
                projection_cursor = end;
            }
            Stage::Where(filter) => {
                // A Boolean branch can skip comparisons inside its own WHERE,
                // but must never jump over a projection, LIMIT or other stage.
                let end = usize::from(filter.control.end);
                if end == 0
                    || index + end > usize::from(plan.count)
                    || filter.control.matched > filter.control.end
                    || filter.control.other > filter.control.end
                    || (filter.control.matched == 0 && filter.control.other == 0)
                {
                    return Err(Error::Corrupt("invalid filter control bounds"));
                }
                for offset in 0..end {
                    let next = &plan.stages[index + offset];
                    let Stage::Where(next_filter) = &next.stage else {
                        return Err(Error::Corrupt("filter branch crosses relational stage"));
                    };
                    if usize::from(next_filter.control.end) != end - offset
                        || (offset > 0 && next.input != RelationId((index + offset) as u8))
                    {
                        return Err(Error::Corrupt("filter branch region disagrees"));
                    }
                }
                if !plan
                    .column_type(filter.column)
                    .is_some_and(|(data_type, _)| filter.predicate.valid_for(data_type))
                    || filter.span.start >= filter.span.end
                    || filter.span.end > plan.source_bytes
                    || !available.contains(filter.column)
                {
                    return Err(Error::Corrupt("invalid filter semantics"));
                }
            }
            Stage::Empty => return Err(Error::Corrupt("empty stage inside plan")),
        }
    }
    if null_cursor != plan.null_extensions.len()
        || set_cursor != plan.set_operations.len()
        || distinct_cursor != plan.distinct.len()
        || computed_cursor != plan.computed.len()
        || next_identity > MAX_QUERY_COLUMNS as u32 + 1
    {
        return Err(Error::Corrupt("computed definition ownership"));
    }
    if order_cursor != usize::from(plan.order_count)
        || plan.order_items[order_cursor..]
            .iter()
            .any(|item| *item != OrderKey::EMPTY)
    {
        return Err(Error::Corrupt("order storage tail"));
    }
    // A projection can hide a sort key without removing the established order.
    // Validate those keys too; checking only visible outputs would miss them.
    for relation in 0..=plan.count {
        for key in 0..MAX_ORDER_ITEMS {
            let Some(order) = plan.order_key(RelationId(relation), key)? else {
                break;
            };
            if plan.column_type(order.column).is_none() {
                return Err(Error::Corrupt("hidden order identity"));
            }
        }
    }
    if projection_cursor != usize::from(plan.projection_count)
        || plan.projections[projection_cursor..]
            .iter()
            .any(|id| *id != ColumnId::EMPTY)
    {
        return Err(Error::Corrupt("invalid projection storage tail"));
    }
    if aggregate_cursor != plan.aggregates.len() {
        return Err(Error::Corrupt("aggregate marker disagrees"));
    }
    if plan.stages[usize::from(plan.count)..]
        .iter()
        .any(|node| *node != Node::EMPTY)
        || usize::from(plan.output_count) != plan.relation_columns(RelationId(plan.count))?.len()
        || plan.outputs[usize::from(plan.output_count)..]
            .iter()
            .any(|output| *output != Output::EMPTY)
    {
        return Err(Error::Corrupt("invalid final row shape"));
    }
    // Inputs precede their consumers, so a backward walk reaches both branches
    // of joins and set operations without recursion. Each producer must be reached
    // exactly once: zero would discard work, and two would share execution state.
    let mut reached = 1_u32 << plan.count;
    let mut source_nodes = 1;
    for index in (0..usize::from(plan.count)).rev() {
        if reached & (1 << (index + 1)) == 0 {
            return Err(Error::Corrupt("unreachable relation producer"));
        }
        let node = plan.node(RelationId(index as u8 + 1))?;
        if matches!(node.stage, Stage::Source(_)) {
            source_nodes += 1;
        } else {
            if reached & (1 << node.input.0) != 0 {
                return Err(Error::Corrupt("relation producer has multiple consumers"));
            }
            reached |= 1 << node.input.0;
        }
        if let Stage::Join { right, .. } | Stage::SetOperation { right, .. } = node.stage {
            if reached & (1 << right.0) != 0 {
                return Err(Error::Corrupt("relation producer has multiple consumers"));
            }
            reached |= 1 << right.0;
        }
    }
    if reached & 1 == 0 || source_nodes != usize::from(plan.occurrence_count) {
        return Err(Error::Corrupt("unreachable source occurrence"));
    }
    let final_columns = plan.relation_columns(RelationId(plan.count))?;
    for (output, expected) in plan.outputs[..usize::from(plan.output_count)]
        .iter()
        .zip(final_columns.iter())
    {
        if output.id != expected || !(output.name == Name::EMPTY || output.name.valid()) {
            return Err(Error::Corrupt("invalid final output"));
        }
    }
    if assignment_cursor != usize::from(plan.assignment_count) {
        return Err(Error::Corrupt("unused SET assignments"));
    }
    Ok(())
}

fn validate_range_scope(plan: &Plan, index: usize, node: &Node) -> Result<(), Error> {
    let input = plan.range_columns[usize::from(node.input.0)];
    let actual = plan.range_columns[index + 1];
    if actual.len() > MAX_COLUMNS {
        return Err(Error::Corrupt("qualified column limit"));
    }
    let visible =
        |relation| -> Result<ColumnSet, Error> { plan.relation_columns(relation)?.identity_set() };
    let expected = match node.stage {
        Stage::Source(_) => visible(RelationId(index as u8 + 1))?,
        Stage::Alias | Stage::Derived => {
            // An unnamed output cannot be qualified, and a derived input may
            // have no alias. Allow a subset of visible columns, but no hidden
            // input column to become accessible through this new boundary.
            if actual & visible(node.input)? == actual {
                return Ok(());
            }
            return Err(Error::Corrupt("range exposes an input outside its row"));
        }
        Stage::Select { .. } | Stage::Aggregate(_) | Stage::SetOperation { .. } => ColumnSet::EMPTY,
        Stage::Join { right, nulls, .. } => {
            let right = plan.range_columns[usize::from(right.0)];
            if let Some(descriptor) = nulls {
                let extension = plan
                    .null_extensions
                    .get(usize::from(descriptor))
                    .ok_or(Error::Corrupt("range null extension descriptor"))?;
                let mut mapped = input;
                for id in right.iter() {
                    mapped.insert(
                        extension
                            .output_for(id)
                            .ok_or(Error::Corrupt("range null extension mapping"))?,
                    );
                }
                mapped
            } else {
                input | right
            }
        }
        Stage::Distinct(descriptor) => {
            let descriptor = plan
                .distinct
                .get(usize::from(descriptor))
                .ok_or(Error::Corrupt("range DISTINCT descriptor"))?;
            let mut mapped = ColumnSet::EMPTY;
            for id in input.iter() {
                if let Some(output) = descriptor.output_for(id) {
                    mapped.insert(output);
                }
            }
            mapped
        }
        Stage::Drop { .. } | Stage::Set { .. } => {
            // SET and DROP can remove a colliding range name. They cannot add
            // a qualified column or revive one an earlier stage removed.
            if actual & input == actual {
                return Ok(());
            }
            return Err(Error::Corrupt("column transform widens range scope"));
        }
        _ => input,
    };
    if actual != expected {
        return Err(Error::Corrupt("range scope transition"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;
