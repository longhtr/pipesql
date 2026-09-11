//! Independent semantic-plan validation. Does not call the parser or binder.
use super::{
    AggregateKind, ColumnId, DataType, Error, Group, MAX_AGGREGATE_COLUMNS, MAX_COLUMNS,
    MAX_COMPUTED, MAX_ORDER_ITEMS, MAX_PROJECTIONS, MAX_QUERY_COLUMNS, MAX_SOURCE_BYTES,
    MAX_STAGES, Name, Node, OrderKey, Output, Plan, RelationId, SourceColumn, SourceOccurrence,
    Stage, initial_outputs,
};

pub(crate) fn validate(plan: &Plan) -> Result<(), Error> {
    let catalog = plan.table().is_some();
    let (_, stock_sources, stock_count) = initial_outputs();
    if (!catalog && plan.generation > 1)
        || (catalog && plan.generation > crate::storage_format::MAX_SUCCESSES)
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
    if plan.computed.len() > MAX_COMPUTED || plan.computed.capacity() != plan.computed.len() {
        return Err(Error::Corrupt("computed definition capacity"));
    }
    let mut projection_cursor = 0;
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
                right,
                left_key,
                right_key,
            } => {
                let right = plan.relation_columns(*right)?;
                if !input.contains(*left_key)
                    || !right.contains(*right_key)
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
            | Stage::Derived
            | Stage::Where(_)
            | Stage::Order { .. }
            | Stage::Limit(_)
            | Stage::Distinct(_) => input.len(),
            Stage::Select { len, .. } => usize::from(*len),
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
                    if !input.contains(item.column) || plan.column_type(item.column).is_none() {
                        return Err(Error::Corrupt("order key outside input"));
                    }
                }
                order_cursor += usize::from(*len);
            }
            Stage::Source(_) | Stage::Alias | Stage::Derived | Stage::Join { .. } => (),
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
                let mut sources = [SourceColumn::QUANTITY.semantic(); MAX_COLUMNS];
                for (source, id) in sources.iter_mut().zip(input.iter()) {
                    *source = plan
                        .column(id)
                        .ok_or(Error::Corrupt("aggregate input has no semantic facts"))?;
                }
                for entry in &aggregate.entries {
                    let valid = match (entry.kind, &entry.expression) {
                        (AggregateKind::Count, None) => true,
                        (AggregateKind::Sum | AggregateKind::Avg, Some(expression)) => {
                            expression.validate(&sources[..input.len()])?;
                            matches!(expression.data_type, DataType::Int64 | DataType::Double)
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
                        || !input.contains(group.input.identity)
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
            Stage::Select { start, len } => {
                let end = usize::from(*start) + usize::from(*len);
                if *len == 0
                    || usize::from(*len) > MAX_COLUMNS
                    || usize::from(*start) != projection_cursor
                    || end > usize::from(plan.projection_count)
                {
                    return Err(Error::Corrupt("invalid projection range"));
                }
                let outputs = &plan.projections[projection_cursor..end];
                let mut visible = [SourceColumn::QUANTITY.semantic(); MAX_COLUMNS];
                for (slot, id) in visible.iter_mut().zip(input.iter()) {
                    *slot = plan
                        .column(id)
                        .ok_or(Error::Corrupt("projection input facts"))?;
                }
                for output in outputs {
                    if input.contains(*output) {
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
                        || definition.column.data_type() != definition.expression.data_type
                        || definition.column.nullable() != definition.expression.nullable()
                    {
                        return Err(Error::Corrupt("invalid computed definition"));
                    }
                    definition.expression.validate(&visible[..input.len()])?;
                    next_identity += 1;
                    computed_cursor += 1;
                }
                projection_cursor = end;
            }
            Stage::Where(filter) => {
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
                    || !input.contains(filter.column)
                {
                    return Err(Error::Corrupt("invalid filter semantics"));
                }
            }
            Stage::Empty => return Err(Error::Corrupt("empty stage inside plan")),
        }
    }
    if distinct_cursor != plan.distinct.len()
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
    // Force bounded order-state validation for every relation, including hidden keys.
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
    // A backwards pass visits both join inputs without recursion or a work queue.
    // Unreachable nodes cannot silently discard a predicate or source.
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
        if let Stage::Join { right, .. } = node.stage {
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
    Ok(())
}
