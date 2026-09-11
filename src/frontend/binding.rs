//! Name, type, identity, and catalog binding; owns transient scope and prepared-plan admission.
use super::lexer::ZERO_SPAN;
use super::parser::{
    Parsed, ParsedAggregateEntry, ParsedAggregateRange, ParsedExpression, ParsedLiteral, ParsedOp,
    ParsedProjection, ParsedRange, ParsedStage, parse_query,
};
use super::validate;
use super::{
    AggregateEntry, AggregatePlan, ColumnFacts, ColumnId, Comparison, Computed, DataType, Database,
    DateValue, DistinctPlan, Error, Expression, Filter, FilterLiteral, Group, LimitBounds,
    MAX_AGGREGATE_COLUMNS, MAX_COLUMNS, MAX_ORDER_ITEMS, MAX_PROJECTIONS, MAX_QUERY_COLUMNS,
    MAX_STAGES, Name, Node, Op, OrderKey, Output, OwnedPlan, PREPARED_ALLOCATION_ALLOWANCE, Plan,
    Predicate, PreparedQuery, RelationId, SemanticColumn, SourceColumn, SourceOccurrence,
    SourceSpan, Stage, bind_error, initial_outputs, text,
};
use crate::date::DatePart;
use std::mem::size_of;

mod admission;
use admission::{BindingBudget, Descriptors};

/// Catalog facts for each FROM/JOIN occurrence. Repeated reads of the same table
/// receive different query identities, so qualified names cannot alias inputs.
struct SourceBindings {
    outputs: [Output; MAX_COLUMNS],
    columns: [SourceColumn; MAX_COLUMNS],
    catalog: [u32; MAX_COLUMNS],
    occurrences: [SourceOccurrence; MAX_STAGES],
    count: u8,
}

impl SourceBindings {
    fn empty() -> Self {
        Self {
            outputs: [Output::EMPTY; MAX_COLUMNS],
            columns: [SourceColumn::QUANTITY; MAX_COLUMNS],
            catalog: [0; MAX_COLUMNS],
            occurrences: [SourceOccurrence::EMPTY; MAX_STAGES],
            count: 0,
        }
    }
}

fn reference_name(source: &str, span: SourceSpan) -> &str {
    text(source, span)
        .rsplit('.')
        .next()
        .expect("nonempty reference")
        .trim()
}

#[derive(Clone, Copy)]
struct Range {
    name: Name,
    span: SourceSpan,
    start: u8,
    len: u8,
}

impl Range {
    const EMPTY: Self = Self {
        name: Name::EMPTY,
        span: ZERO_SPAN,
        start: 0,
        len: 0,
    };
}

struct Ranges {
    ranges: [Range; MAX_STAGES],
    count: usize,
    members: [Output; MAX_COLUMNS],
    len: usize,
}

impl Ranges {
    fn remap_distinct(&mut self, distinct: &DistinctPlan) {
        // Compact each range in place, keeping names and order but dropping
        // memberships not represented in the ordinary output row.
        let mut end = 0;
        for range in &mut self.ranges[..self.count] {
            let start = end;
            for index in usize::from(range.start)..usize::from(range.start) + usize::from(range.len)
            {
                let mut member = self.members[index];
                if let Some(id) = distinct.output_for(member.id) {
                    member.id = id;
                    self.members[end] = member;
                    end += 1;
                }
            }
            range.start = start as u8;
            range.len = (end - start) as u8;
        }
        self.members[end..].fill(Output::EMPTY);
        self.len = end;
    }

    fn empty() -> Self {
        Self {
            ranges: [Range::EMPTY; MAX_STAGES],
            count: 0,
            members: [Output::EMPTY; MAX_COLUMNS],
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.count = 0;
        self.len = 0;
    }

    fn add(&mut self, source: &str, alias: SourceSpan, outputs: &[Output]) -> Result<(), Error> {
        let name = Name::new(text(source, alias));
        if self.ranges[..self.count]
            .iter()
            .any(|range| range.name.as_str().eq_ignore_ascii_case(name.as_str()))
        {
            return Err(bind_error("duplicate table alias", alias));
        }
        let named = outputs
            .iter()
            .filter(|output| output.name != Name::EMPTY)
            .count();
        let end = self
            .len
            .checked_add(named)
            .ok_or(Error::Corrupt("range width overflow"))?;
        if self.count == MAX_STAGES || end > MAX_COLUMNS {
            return Err(bind_error("range membership limit exceeded", alias));
        }
        for (slot, output) in self.members[self.len..end]
            .iter_mut()
            .zip(outputs.iter().filter(|output| output.name != Name::EMPTY))
        {
            *slot = *output;
        }
        self.ranges[self.count] = Range {
            name,
            span: alias,
            start: self.len as u8,
            len: named as u8,
        };
        self.count += 1;
        self.len = end;
        Ok(())
    }
}

// Each frame stores ordinary outputs followed by qualified range members in
// `SavedScopes::values`. Starts address the shared vectors; counts describe
// this frame. Joining restores the last frame and truncates both vectors.
#[derive(Clone, Copy)]
struct SavedScope {
    values_start: usize,
    output_count: usize,
    member_count: usize,
    ranges_start: usize,
    range_count: usize,
    input: RelationId,
}

impl SavedScope {
    const EMPTY: Self = Self {
        values_start: 0,
        output_count: 0,
        member_count: 0,
        ranges_start: 0,
        range_count: 0,
        input: RelationId::SOURCE,
    };
}

// Only pending left inputs are retained. Their branches have disjoint source
// occurrences; projection and aggregate output entries have query-wide pools.
struct SavedScopes<'db> {
    values: Vec<Output>,
    ranges: Vec<Range>,
    frames: [SavedScope; MAX_STAGES],
    depth: usize,
    _reservation: crate::resources::Reservation<'db>,
}

impl<'db> SavedScopes<'db> {
    fn new(database: &'db Database, parsed: &Parsed, source_columns: usize) -> Result<Self, Error> {
        let entries = source_columns
            .checked_add(usize::from(parsed.projection_count))
            .and_then(|n| n.checked_add(usize::from(parsed.aggregate_entry_count)))
            .and_then(|n| n.checked_add(usize::from(parsed.aggregate_group_count)))
            .ok_or(Error::Corrupt("scope entry count overflow"))?;
        if entries > MAX_QUERY_COLUMNS {
            return Err(Error::Corrupt("scope entry bound exceeds query identities"));
        }
        let capacity = entries
            .checked_mul(2)
            .ok_or(Error::Corrupt("scope capacity overflow"))?;
        let range_capacity = usize::from(parsed.source_count);
        let bytes = capacity
            .checked_mul(size_of::<Output>())
            .and_then(|n| n.checked_add(range_capacity.checked_mul(size_of::<Range>())?))
            .and_then(|n| n.checked_add(2 * PREPARED_ALLOCATION_ALLOWANCE))
            .and_then(|n| u64::try_from(n).ok())
            .ok_or(Error::Corrupt("scope allocation size overflow"))?;
        let reservation = database.reserve_memory(bytes, "binder scope storage")?;
        let mut values = Vec::new();
        let mut ranges = Vec::new();
        values
            .try_reserve_exact(capacity)
            .map_err(|_| Error::Resource {
                owner: "binder scope outputs",
                required: bytes,
                limit: bytes,
            })?;
        ranges
            .try_reserve_exact(range_capacity)
            .map_err(|_| Error::Resource {
                owner: "binder scope ranges",
                required: bytes,
                limit: bytes,
            })?;
        if values.capacity() != capacity || ranges.capacity() != range_capacity {
            return Err(Error::Resource {
                owner: "binder scope capacity",
                required: bytes,
                limit: bytes,
            });
        }
        Ok(Self {
            values,
            ranges,
            frames: [SavedScope::EMPTY; MAX_STAGES],
            depth: 0,
            _reservation: reservation,
        })
    }

    fn push(
        &mut self,
        outputs: &[Output],
        ranges: &Ranges,
        input: RelationId,
    ) -> Result<(), Error> {
        if self.depth == self.frames.len()
            || self.values.len() + outputs.len() + ranges.len > self.values.capacity()
            || self.ranges.len() + ranges.count > self.ranges.capacity()
        {
            return Err(Error::Corrupt("pending scope exceeds admitted capacity"));
        }
        self.frames[self.depth] = SavedScope {
            values_start: self.values.len(),
            output_count: outputs.len(),
            member_count: ranges.len,
            ranges_start: self.ranges.len(),
            range_count: ranges.count,
            input,
        };
        self.values.extend_from_slice(outputs);
        self.values.extend_from_slice(&ranges.members[..ranges.len]);
        self.ranges
            .extend_from_slice(&ranges.ranges[..ranges.count]);
        self.depth += 1;
        Ok(())
    }

    fn join(
        &mut self,
        outputs: &mut [Output; MAX_COLUMNS],
        count: &mut u8,
        ranges: &mut Ranges,
        span: SourceSpan,
    ) -> Result<(RelationId, usize), Error> {
        if self.depth == 0 {
            return Err(Error::Corrupt("join left scope absent"));
        }
        let frame = self.frames[self.depth - 1];
        let right_width = usize::from(*count);
        let width = frame.output_count + right_width;
        if width > MAX_COLUMNS
            || frame.member_count + ranges.len > MAX_COLUMNS
            || frame.range_count + ranges.count > MAX_STAGES
        {
            return Err(bind_error("join output width exceeds limit", span));
        }
        for right in &ranges.ranges[..ranges.count] {
            if self.ranges[frame.ranges_start..]
                .iter()
                .any(|left| left.name.as_str().eq_ignore_ascii_case(right.name.as_str()))
            {
                return Err(bind_error("duplicate table alias", right.span));
            }
        }
        outputs.copy_within(..right_width, frame.output_count);
        outputs[..frame.output_count].copy_from_slice(
            &self.values[frame.values_start..frame.values_start + frame.output_count],
        );
        ranges.members.copy_within(..ranges.len, frame.member_count);
        ranges.members[..frame.member_count]
            .copy_from_slice(&self.values[frame.values_start + frame.output_count..]);
        ranges.ranges.copy_within(..ranges.count, frame.range_count);
        for range in &mut ranges.ranges[frame.range_count..frame.range_count + ranges.count] {
            range.start += frame.member_count as u8;
        }
        ranges.ranges[..frame.range_count].copy_from_slice(&self.ranges[frame.ranges_start..]);
        ranges.len += frame.member_count;
        ranges.count += frame.range_count;
        *count = width as u8;
        self.values.truncate(frame.values_start);
        self.ranges.truncate(frame.ranges_start);
        self.depth -= 1;
        Ok((frame.input, frame.output_count))
    }
}

fn resolve(
    source: &str,
    span: SourceSpan,
    outputs: &[Output],
    ranges: &Ranges,
) -> Result<ColumnId, Error> {
    let reference = text(source, span);
    let (name, outputs) = if let Some((qualifier, name)) = reference.split_once('.') {
        let range = ranges.ranges[..ranges.count]
            .iter()
            .find(|range| range.name.as_str().eq_ignore_ascii_case(qualifier.trim()))
            .ok_or_else(|| bind_error("table alias is not visible", span))?;
        let start = usize::from(range.start);
        (
            name.trim(),
            &ranges.members[start..start + usize::from(range.len)],
        )
    } else {
        (reference, outputs)
    };
    let mut found = None;
    for output in outputs {
        if output.name.as_str().eq_ignore_ascii_case(name) {
            if found.is_some() {
                return Err(bind_error("ambiguous column name", span));
            }
            found = Some(output.id);
        }
    }
    found.ok_or_else(|| bind_error("column name is not visible", span))
}

fn bind_expression(
    source: &str,
    parsed: &ParsedExpression,
    outputs: &[Output],
    facts: &ColumnFacts<'_>,
    ranges: &Ranges,
) -> Result<Expression, Error> {
    let mut expression = Expression {
        len: parsed.len,
        ..Expression::EMPTY
    };
    for (op, syntax) in expression
        .ops
        .iter_mut()
        .zip(&parsed.ops[..usize::from(parsed.len)])
    {
        *op = match *syntax {
            ParsedOp::Column(span) => {
                let id = facts
                    .column(resolve(source, span, outputs, ranges)?)
                    .ok_or(Error::Corrupt("expression input has no semantic facts"))?;
                if !matches!(id.data_type(), DataType::Int64 | DataType::Double) {
                    return Err(bind_error(
                        "numeric expression requires numeric columns",
                        span,
                    ));
                }
                Op::Column(id)
            }
            ParsedOp::Number(span) | ParsedOp::NegativeNumber(span) => {
                let negative = matches!(syntax, ParsedOp::NegativeNumber(_));
                let literal = text(source, span);
                if literal
                    .bytes()
                    .any(|byte| matches!(byte, b'.' | b'e' | b'E'))
                {
                    let value: f64 = literal
                        .parse()
                        .map_err(|_| bind_error("invalid DOUBLE literal", span))?;
                    if !value.is_finite() {
                        return Err(bind_error("literal is not finite", span));
                    }
                    Op::Double(if negative { -value } else { value }.to_bits())
                } else {
                    // Parse the magnitude before applying its lexical sign so
                    // INT64::MIN does not require a positive INT64 intermediate.
                    let magnitude: i128 = literal
                        .parse()
                        .map_err(|_| bind_error("INT64 literal is outside range", span))?;
                    let value = if negative { -magnitude } else { magnitude };
                    Op::Integer(
                        i64::try_from(value)
                            .map_err(|_| bind_error("INT64 literal is outside range", span))?,
                    )
                }
            }
            ParsedOp::Add => Op::Add,
            ParsedOp::Subtract => Op::Subtract,
            ParsedOp::Multiply => Op::Multiply,
            ParsedOp::Negate => Op::Negate,
            ParsedOp::Empty => return Err(Error::Corrupt("empty parsed scalar operation")),
        };
    }
    let mut visible = [SourceColumn::QUANTITY.semantic(); MAX_COLUMNS];
    for (slot, output) in visible.iter_mut().zip(outputs) {
        *slot = facts
            .column(output.id)
            .ok_or(Error::Corrupt("expression input has no semantic facts"))?;
    }
    expression.data_type = expression.infer(&visible[..outputs.len()])?;
    Ok(expression)
}

fn bind_literal(
    source: &str,
    literal: ParsedLiteral,
    parsed: &Parsed,
) -> Result<FilterLiteral, Error> {
    match literal {
        ParsedLiteral::String(span) => {
            let (value, _) = crate::text_literal::TextLiteral::parse(text(source, span))
                .map_err(|message| bind_error(message, span))?;
            Ok(FilterLiteral::String(value))
        }
        ParsedLiteral::Numeric(range) => {
            let parsed = parsed.expression(range)?;
            let expression = bind_expression(
                source,
                &parsed,
                &[],
                &ColumnFacts {
                    sources: &[],
                    aggregates: &[],
                    computed: &[],
                    distinct: &[],
                },
                &Ranges::empty(),
            )?;
            let value = expression
                .evaluate_constant()
                .map_err(|failure| failure.into_error(parsed.span))?;
            Ok(match value {
                crate::scalar::Number::Integer(value) => FilterLiteral::Int64(value),
                crate::scalar::Number::Double(value) => FilterLiteral::Double(value.to_bits()),
            })
        }
        ParsedLiteral::Date {
            span,
            shifts,
            count,
        } => {
            let (literal, _) = crate::text_literal::TextLiteral::parse(text(source, span))
                .map_err(|message| bind_error(message, span))?;
            let mut date = DateValue::parse(literal.as_str().as_bytes())
                .ok_or_else(|| bind_error("invalid DATE literal", span))?;
            for shift in &shifts[..usize::from(count)] {
                let magnitude: i128 = text(source, shift.amount)
                    .parse()
                    .map_err(|_| bind_error("INTERVAL requires an INT64 integer", shift.amount))?;
                let signed = if shift.negative {
                    -magnitude
                } else {
                    magnitude
                };
                let amount = i64::try_from(signed)
                    .map_err(|_| bind_error("INTERVAL exceeds INT64", shift.amount))?;
                let amount = if shift.subtract {
                    amount
                        .checked_neg()
                        .ok_or_else(|| bind_error("DATE interval overflow", shift.amount))?
                } else {
                    amount
                };
                let part = text(source, shift.part);
                let part = if part.eq_ignore_ascii_case("DAY") {
                    DatePart::Day
                } else if part.eq_ignore_ascii_case("MONTH") {
                    DatePart::Month
                } else if part.eq_ignore_ascii_case("YEAR") {
                    DatePart::Year
                } else {
                    return Err(bind_error("unsupported DATE interval unit", shift.part));
                };
                date = date
                    .shift(amount, part)
                    .ok_or_else(|| bind_error("DATE is outside supported calendar", span))?;
            }
            Ok(FilterLiteral::Date(date))
        }
    }
}

pub(crate) fn prepare<'db>(
    database: &'db Database,
    source: &str,
) -> Result<PreparedQuery<'db>, Error> {
    let parsed = parse_query(source)?;
    if parsed.source_count != 1 {
        return Err(bind_error(
            "joins require declared-table storage",
            parsed.sources[1].table,
        ));
    }

    if let Some(item) = parsed.order_items[..usize::from(parsed.order_count)].first() {
        return Err(bind_error(
            "standalone ordering requires declared-table storage",
            item.reference,
        ));
    }
    let table = parsed.sources[0].table;
    if !text(source, table).eq_ignore_ascii_case("lineitem") {
        return Err(bind_error("unknown source table", table));
    }
    let (outputs, columns, count) = initial_outputs();
    let mut sources = SourceBindings::empty();
    sources.outputs = outputs;
    sources.columns = columns;
    sources.count = count;
    sources.occurrences[0] = SourceOccurrence {
        table: None,
        start: 0,
        columns: count,
    };
    bind_plan(
        database,
        source,
        &parsed,
        sources,
        database.catalog_generation(),
    )
}

fn bind_plan<'db>(
    database: &'db Database,
    source: &str,
    parsed: &Parsed,
    sources: SourceBindings,
    generation: u64,
) -> Result<PreparedQuery<'db>, Error> {
    let output_count = sources.occurrences[0].columns;
    let mut outputs = [Output::EMPTY; MAX_COLUMNS];
    outputs[..usize::from(output_count)]
        .copy_from_slice(&sources.outputs[..usize::from(output_count)]);
    let mut ranges = Ranges::empty();
    ranges.add(
        source,
        parsed.sources[0].alias,
        &outputs[..usize::from(output_count)],
    )?;

    let budget = BindingBudget::calculate(parsed)?;
    let reservation = database.reserve_memory(budget.bytes, "prepared operator chain")?;
    let scopes = if parsed.source_count > 1 {
        Some(SavedScopes::new(
            database,
            parsed,
            usize::from(sources.count),
        )?)
    } else {
        None
    };
    let descriptors = budget.allocate()?;
    let mut plan = Plan {
        database: database.database_identity(),
        generation,
        occurrences: sources.occurrences,
        occurrence_count: parsed.source_count,
        source_columns: sources.columns,
        catalog_columns: sources.catalog,
        source_count: sources.count,
        source_bytes: u16::try_from(source.len()).expect("source bound"),
        stages: [Node::EMPTY; MAX_STAGES],
        count: parsed.len,
        projections: [ColumnId::EMPTY; MAX_PROJECTIONS],
        projection_count: parsed.projection_count,
        order_items: [OrderKey::EMPTY; MAX_ORDER_ITEMS],
        order_count: parsed.order_count,
        outputs,
        output_count,
        aggregates: Vec::new(),
        computed: Vec::new(),
        distinct: Vec::new(),
    };
    let descriptors = {
        let mut binder = Binder {
            source,
            parsed,
            sources: &sources,
            plan: &mut plan,
            next_identity: u32::from(sources.count) + 1,
            ranges,
            descriptors,
            scopes,
            prepared_bytes: budget.bytes,
        };
        for (index, syntax) in parsed.stages[..usize::from(parsed.len)].iter().enumerate() {
            binder.plan.stages[index] = binder.bind_stage(index, *syntax)?;
        }
        if binder
            .scopes
            .as_ref()
            .is_some_and(|scopes| scopes.depth != 0)
        {
            return Err(Error::Corrupt("unfinished binder scope"));
        }
        drop(binder.scopes.take());
        binder.descriptors
    };
    plan.aggregates = descriptors.aggregates;
    plan.computed = descriptors.computed;
    plan.distinct = descriptors.distinct;
    validate(&plan)?;
    Ok(PreparedQuery {
        plan: OwnedPlan::new(plan, database.config().memory_limit_bytes())?,
        snapshot: None,
        reservation,
    })
}

/// Mutable name scope and descriptors under construction. It has no execution
/// authority: only a complete plan accepted by `validate` can leave binding.
/// Source facts are borrowed to avoid a second large copy on the caller's stack.
struct Binder<'input, 'db> {
    source: &'input str,
    parsed: &'input Parsed,
    sources: &'input SourceBindings,
    plan: &'input mut Plan,
    ranges: Ranges,
    next_identity: u32,
    descriptors: Descriptors,
    scopes: Option<SavedScopes<'db>>,
    prepared_bytes: u64,
}

impl Binder<'_, '_> {
    fn bind_stage(&mut self, index: usize, syntax: ParsedStage) -> Result<Node, Error> {
        let mut input = RelationId(u8::try_from(index).expect("stage capacity"));
        let stage = match syntax {
            ParsedStage::Distinct(span) => self.bind_distinct(span)?,
            ParsedStage::Derived(alias) => {
                self.ranges.clear();
                if alias != ZERO_SPAN {
                    self.ranges.add(
                        self.source,
                        alias,
                        &self.plan.outputs[..usize::from(self.plan.output_count)],
                    )?;
                }
                Stage::Derived
            }
            ParsedStage::Alias(alias) => {
                self.ranges.clear();
                self.ranges.add(
                    self.source,
                    alias,
                    &self.plan.outputs[..usize::from(self.plan.output_count)],
                )?;
                Stage::Alias
            }
            ParsedStage::Source(occurrence) => self.bind_source(occurrence, &mut input)?,
            ParsedStage::Join { left, right } => self.bind_join(index, left, right, &mut input)?,
            ParsedStage::Limit {
                count,
                offset,
                span,
            } => Stage::Limit(self.bind_limit(count, offset, span)?),
            ParsedStage::Order { start, len } => self.bind_order(start, len)?,
            ParsedStage::Aggregate(aggregate) => self.bind_aggregate(aggregate)?,
            ParsedStage::Select { start, len } => self.bind_select(start, len, input)?,
            ParsedStage::WhereNull { column, negated } => Stage::Where(Filter {
                column: self.resolve(column)?,
                predicate: Predicate::IsNull { negated },
                control: self.parsed.controls[index],
                span: column,
            }),
            ParsedStage::Where {
                column,
                comparison,
                literal,
            } => self.bind_comparison(index, column, comparison, literal)?,
            ParsedStage::Empty => return Err(Error::Corrupt("parser published empty stage")),
        };
        let columns = if let Stage::Source(source) = stage {
            self.sources.occurrences[usize::from(source)].columns
        } else {
            self.plan.output_count
        };
        Ok(Node {
            input,
            stage,
            columns,
        })
    }

    fn resolve(&self, span: SourceSpan) -> Result<ColumnId, Error> {
        resolve(
            self.source,
            span,
            &self.plan.outputs[..usize::from(self.plan.output_count)],
            &self.ranges,
        )
    }

    fn facts(&self) -> ColumnFacts<'_> {
        ColumnFacts {
            sources: &self.sources.columns[..usize::from(self.sources.count)],
            aggregates: &self.descriptors.aggregates,
            computed: &self.descriptors.computed,
            distinct: &self.descriptors.distinct,
        }
    }

    fn column_type(&self, id: ColumnId) -> Option<(DataType, bool)> {
        self.facts()
            .column(id)
            .map(|column| (column.data_type(), column.nullable()))
    }

    fn bind_expression(&self, syntax: &ParsedExpression) -> Result<Expression, Error> {
        bind_expression(
            self.source,
            syntax,
            &self.plan.outputs[..usize::from(self.plan.output_count)],
            &self.facts(),
            &self.ranges,
        )
    }

    fn bind_limit(
        &self,
        count: ParsedRange,
        offset: Option<ParsedRange>,
        span: SourceSpan,
    ) -> Result<LimitBounds, Error> {
        let bind = |value| -> Result<u64, Error> {
            match bind_literal(self.source, ParsedLiteral::Numeric(value), self.parsed)? {
                FilterLiteral::Int64(value) if value >= 0 => Ok(value as u64),
                _ => Err(bind_error(
                    "LIMIT and OFFSET require non-negative INT64 constants",
                    span,
                )),
            }
        };
        Ok(LimitBounds {
            count: bind(count)?,
            offset: offset.map(bind).transpose()?.unwrap_or(0),
        })
    }

    fn bind_distinct(&mut self, span: SourceSpan) -> Result<Stage, Error> {
        if self.sources.occurrences[0].table.is_none() {
            return Err(bind_error("DISTINCT requires declared-table storage", span));
        }
        let bound = DistinctPlan::bind(
            &mut self.plan.outputs[..usize::from(self.plan.output_count)],
            &ColumnFacts {
                sources: &self.sources.columns[..usize::from(self.sources.count)],
                aggregates: &self.descriptors.aggregates,
                computed: &self.descriptors.computed,
                distinct: &self.descriptors.distinct,
            },
            &mut self.next_identity,
        )?;
        self.ranges.remap_distinct(&bound);

        let descriptor = self.descriptors.distinct.len() as u8;
        self.descriptors.distinct.push(bound);
        Ok(Stage::Distinct(descriptor))
    }

    fn bind_source(&mut self, occurrence: u8, input: &mut RelationId) -> Result<Stage, Error> {
        let occurrence_index = usize::from(occurrence);
        self.scopes
            .as_mut()
            .ok_or(Error::Corrupt("source scope owner absent"))?
            .push(
                &self.plan.outputs[..usize::from(self.plan.output_count)],
                &self.ranges,
                *input,
            )?;
        self.ranges.clear();
        self.plan.outputs.fill(Output::EMPTY);
        self.plan.output_count = 0;
        let binding = self.sources.occurrences[occurrence_index];
        let start = usize::from(binding.start);
        let width = usize::from(binding.columns);
        let end = usize::from(self.plan.output_count) + width;
        if end > MAX_COLUMNS {
            return Err(bind_error(
                "join output width exceeds limit",
                self.parsed.sources[occurrence_index].table,
            ));
        }
        let added = &self.sources.outputs[start..start + width];
        self.ranges.add(
            self.source,
            self.parsed.sources[occurrence_index].alias,
            added,
        )?;
        self.plan.outputs[usize::from(self.plan.output_count)..end].copy_from_slice(added);
        self.plan.output_count = end as u8;
        *input = RelationId::SOURCE;
        Ok(Stage::Source(occurrence))
    }

    fn bind_join(
        &mut self,
        index: usize,
        left: SourceSpan,
        right: SourceSpan,
        input: &mut RelationId,
    ) -> Result<Stage, Error> {
        let (left_input, left_width) = self
            .scopes
            .as_mut()
            .ok_or(Error::Corrupt("join scope owner absent"))?
            .join(
                &mut self.plan.outputs,
                &mut self.plan.output_count,
                &mut self.ranges,
                left,
            )?;
        let first = self.resolve(left)?;
        let second = self.resolve(right)?;
        let is_left = |id| {
            self.plan.outputs[..left_width]
                .iter()
                .any(|output| output.id == id)
        };
        let is_right = |id| {
            self.plan.outputs[left_width..usize::from(self.plan.output_count)]
                .iter()
                .any(|output| output.id == id)
        };
        let (left_key, right_key) = if is_left(first) && is_right(second) {
            (first, second)
        } else if is_left(second) && is_right(first) {
            (second, first)
        } else {
            return Err(bind_error(
                "join equality must connect its two inputs",
                left,
            ));
        };
        let first_type = self
            .column_type(left_key)
            .ok_or(Error::Corrupt("join key type absent"))?
            .0;
        let second_type = self
            .column_type(right_key)
            .ok_or(Error::Corrupt("join key type absent"))?
            .0;
        if first_type != second_type {
            return Err(bind_error("join key coercion is not implemented", left));
        }
        *input = left_input;
        Ok(Stage::Join {
            right: RelationId(index as u8),
            left_key,
            right_key,
        })
    }

    fn bind_order(&mut self, start: u8, len: u8) -> Result<Stage, Error> {
        let end = usize::from(start) + usize::from(len);
        for (offset, item) in self.parsed.order_items[usize::from(start)..end]
            .iter()
            .enumerate()
        {
            let column = if item.ordinal {
                let ordinal = text(self.source, item.reference)
                    .parse::<usize>()
                    .ok()
                    .filter(|n| *n > 0 && *n <= usize::from(self.plan.output_count))
                    .ok_or_else(|| {
                        bind_error("order ordinal outside visible row", item.reference)
                    })?;
                self.plan.outputs[ordinal - 1].id
            } else {
                self.resolve(item.reference)?
            };
            self.plan.order_items[usize::from(start) + offset] = OrderKey {
                column,
                direction: item.direction,
                nulls: item.nulls,
            };
        }
        Ok(Stage::Order { start, len })
    }

    fn bind_aggregate(&mut self, parsed_aggregate: ParsedAggregateRange) -> Result<Stage, Error> {
        let entry_count = usize::from(parsed_aggregate.count);
        let entry_start = usize::from(parsed_aggregate.entry_start);
        let group_start = usize::from(parsed_aggregate.group_start);
        let parsed_entries = &self.parsed.aggregate_entries[entry_start..entry_start + entry_count];
        let parsed_groups = &self.parsed.aggregate_groups
            [group_start..group_start + usize::from(parsed_aggregate.group_count)];
        let mut bound = AggregatePlan {
            entries: Vec::new(),
            groups: [Group::EMPTY; MAX_AGGREGATE_COLUMNS - 1],
            first_output: ColumnId::new(self.next_identity),
            group_count: parsed_aggregate.group_count,
            ordered: parsed_aggregate.ordered,
        };
        if usize::from(parsed_aggregate.count) + usize::from(parsed_aggregate.group_count)
            > MAX_AGGREGATE_COLUMNS
        {
            return Err(bind_error(
                "aggregate row width exceeds limit",
                parsed_entries[0].alias,
            ));
        }
        bound.entries = admission::aggregate_entries(entry_count, self.prepared_bytes)?;
        for entry in parsed_entries {
            bound.entries.push(self.bind_aggregate_entry(entry)?);
        }
        for (index, span) in parsed_groups.iter().enumerate() {
            bound.groups[index] = self.bind_group(*span, &bound.groups[..index])?;
        }
        self.plan.output_count = u8::try_from(usize::from(bound.group_count) + bound.entries.len())
            .expect("bounded output");
        self.plan.outputs = [Output::EMPTY; MAX_COLUMNS];
        for (index, output) in self.plan.outputs[..usize::from(self.plan.output_count)]
            .iter_mut()
            .enumerate()
        {
            *output = bound.output(index).expect("aggregate output");
        }
        self.next_identity += u32::from(self.plan.output_count);
        let aggregate_index = self.descriptors.aggregates.len() as u8;
        self.descriptors.aggregates.push(bound);
        self.ranges.clear();
        Ok(Stage::Aggregate(aggregate_index))
    }

    fn bind_aggregate_entry(&self, entry: &ParsedAggregateEntry) -> Result<AggregateEntry, Error> {
        let ParsedAggregateEntry {
            kind,
            argument: column,
            alias,
            span,
        } = entry;
        let expression = column
            .as_ref()
            .map(|range| self.bind_expression(&self.parsed.expression(*range)?))
            .transpose()?;
        if expression.as_ref().is_some_and(|expression| {
            !matches!(expression.data_type, DataType::Int64 | DataType::Double)
        }) {
            return Err(bind_error("SUM and AVG require numeric input", *alias));
        }
        Ok(AggregateEntry {
            kind: *kind,
            expression,
            span: *span,
            name: Name::new(reference_name(self.source, *alias)),
        })
    }

    fn bind_group(&self, span: SourceSpan, preceding: &[Group]) -> Result<Group, Error> {
        let index = preceding.len();
        let id = self.resolve(span)?;
        if self.sources.occurrences[0].table.is_none()
            && (index >= 2 || self.column_type(id) != Some((DataType::String, false)))
        {
            return Err(bind_error(
                "legacy grouping requires at most two bounded STRING keys",
                span,
            ));
        }
        if preceding.iter().any(|group| group.input.identity == id) {
            return Err(bind_error("duplicate grouping keys are not admitted", span));
        }
        let output = self.plan.outputs[..usize::from(self.plan.output_count)]
            .iter()
            .find(|output| {
                output.id == id
                    && output
                        .name
                        .as_str()
                        .eq_ignore_ascii_case(reference_name(self.source, span))
            })
            .copied()
            .ok_or(Error::Corrupt("resolved group is missing"))?;
        Ok(Group {
            name: output.name,
            input: self
                .facts()
                .column(id)
                .ok_or(Error::Corrupt("group input has no semantic facts"))?,
        })
    }

    fn bind_select(&mut self, start: u8, len: u8, input: RelationId) -> Result<Stage, Error> {
        let begin = usize::from(start);
        let end = begin + usize::from(len);
        let columns = self
            .parsed
            .projections
            .get(begin..end)
            .ok_or(Error::Corrupt("parsed projection range"))?;
        let mut next = [Output::EMPTY; MAX_COLUMNS];
        for (output, entry) in next.iter_mut().zip(columns) {
            *output = self.bind_projection(entry, input)?;
        }
        self.plan.outputs = next;
        self.plan.output_count = len;
        for (id, output) in self.plan.projections[begin..end]
            .iter_mut()
            .zip(self.plan.outputs)
        {
            *id = output.id;
        }
        self.ranges.clear();
        Ok(Stage::Select { start, len })
    }

    fn bind_projection(
        &mut self,
        entry: &ParsedProjection,
        input: RelationId,
    ) -> Result<Output, Error> {
        let syntax = self.parsed.expression(entry.expression)?;
        let reference = if let [ParsedOp::Column(span)] = &syntax.ops[..usize::from(syntax.len)] {
            Some(*span)
        } else {
            None
        };
        let direct = reference.filter(|_| entry.direct);
        let name = if entry.alias != ZERO_SPAN {
            Name::new(text(self.source, entry.alias))
        } else if let Some(span) = direct {
            Name::new(reference_name(self.source, span))
        } else {
            Name::EMPTY
        };
        let id = if let Some(span) = direct {
            self.resolve(span)?
        } else {
            let expression = self.bind_expression(&syntax)?;
            if let [Op::Column(column)] = &expression.ops[..usize::from(expression.len)] {
                column.identity()
            } else {
                let column = SemanticColumn::new(
                    self.next_identity,
                    expression.data_type,
                    expression.nullable(),
                );
                self.next_identity += 1;
                assert!(
                    self.descriptors.computed.len() < self.descriptors.computed.capacity(),
                    "parsed computation allocation bound"
                );
                self.descriptors.computed.push(Computed {
                    column,
                    expression,
                    span: syntax.span,
                    input,
                });
                column.identity()
            }
        };
        Ok(Output { id, name })
    }

    fn bind_comparison(
        &self,
        index: usize,
        column: SourceSpan,
        comparison: Comparison,
        literal: ParsedLiteral,
    ) -> Result<Stage, Error> {
        let id = self.resolve(column)?;
        let data_type = self
            .column_type(id)
            .ok_or(Error::Corrupt("bound identity has no type"))?
            .0;
        let literal = match (bind_literal(self.source, literal, self.parsed)?, data_type) {
            (FilterLiteral::Int64(value), DataType::Double) => {
                FilterLiteral::Double((value as f64).to_bits())
            }
            (literal, _) => literal,
        };
        if !literal.valid_for(data_type) {
            return Err(bind_error("comparison operand types disagree", column));
        }
        Ok(Stage::Where(Filter {
            column: id,
            predicate: Predicate::Compare {
                comparison,
                literal,
            },
            control: self.parsed.controls[index],
            span: column,
        }))
    }
}

pub(crate) fn prepare_catalog<'db>(
    database: &'db Database,
    source: &str,
) -> Result<PreparedQuery<'db>, Error> {
    let parsed = parse_query(source)?;
    let snapshot = database.catalog_snapshot()?;
    let generation = snapshot.generation();
    let mut scratch = crate::catalog::Scratch::new(&database.memory, "query catalog binding")?;
    let (catalog_bytes, schema_bytes) = scratch.bytes().split_at_mut(crate::catalog::MAX_BYTES);
    let cancel = crate::CancellationToken::new();
    let mut effects = crate::effects::Effects::default();
    let catalog = snapshot
        .read_catalog(catalog_bytes, &cancel, &mut effects)?
        .ok_or_else(|| bind_error("unknown source table", parsed.table))?;
    let objects = crate::path::joined_path(database.path(), crate::namespace::UNITS_NAME)?;
    let mut sources = SourceBindings::empty();
    for (occurrence, parsed_source) in parsed.sources[..usize::from(parsed.source_count)]
        .iter()
        .enumerate()
    {
        let ordinal = (0..catalog.len())
            .find(|&index| {
                catalog.table(index).is_some_and(|table| {
                    table
                        .name()
                        .eq_ignore_ascii_case(text(source, parsed_source.table))
                })
            })
            .ok_or_else(|| bind_error("unknown source table", parsed_source.table))?;
        let schema = catalog.read_schema(&objects, ordinal, schema_bytes, &cancel, &mut effects)?;
        let start = usize::from(sources.count);
        let end = start
            .checked_add(schema.len())
            .ok_or(Error::Corrupt("source width overflow"))?;
        if end > MAX_COLUMNS {
            return Err(bind_error(
                "query source column limit exceeded",
                parsed_source.table,
            ));
        }
        sources.occurrences[occurrence] = SourceOccurrence {
            table: Some(schema.table()),
            start: sources.count,
            columns: schema.len() as u8,
        };
        for index in 0..schema.len() {
            let column = schema.column(index).expect("validated schema ordinal");
            let slot = start + index;
            sources.catalog[slot] = column.id().value();
            let name = Name::new(column.name());
            if !name.valid() {
                return Err(bind_error(
                    "source column name is not admitted",
                    parsed_source.table,
                ));
            }
            sources.columns[slot] = SourceColumn::bind_declaration(column, index, slot as u32 + 1);
            sources.outputs[slot] = Output {
                name,
                id: sources.columns[slot].identity,
            };
        }
        sources.count = end as u8;
    }
    drop(objects);
    drop(scratch);
    let mut query = bind_plan(database, source, &parsed, sources, generation)?;
    query.snapshot = Some(snapshot);
    Ok(query)
}

#[cfg(test)]
mod tests;
