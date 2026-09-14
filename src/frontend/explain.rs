//! Borrowed logical relationships; formatting has no execution or catalog authority.
use super::{
    AggregateArgument, AggregateKind, ColumnId, Comparison, Computation, Constant, DataType,
    Direction, FilterLiteral, Node, NullPlacement, Plan, Predicate, RelationId, SetKind, Stage,
};
use crate::scalar::{Expression, Op};
use crate::string_length::Unit;
use std::fmt::{self, Write};

/// A borrowed explanation of a prepared query's logical structure.
///
/// Obtain this view with [`super::PreparedQuery::logical_plan`], then format it
/// with `{}`. Relation labels identify semantic nodes; column labels identify
/// values within this plan. Neither label is a physical position or persistent
/// identity. Output positions are printed separately and may repeat an identity.
/// Source occurrence numbers distinguish repeated table references. The plan
/// does not retain every source name or the original SQL text.
///
/// Formatting traverses bounded, validated plan metadata without engine effects,
/// allocation or input-controlled recursion. The caller owns the output sink and
/// any allocation or side effects it performs. Sink failures propagate through
/// [`fmt::Error`] and may leave a partial report. This diagnostic text is unstable
/// and is not a query serialization, physical plan or performance estimate.
pub struct LogicalPlan<'query> {
    plan: &'query Plan,
}

impl<'query> LogicalPlan<'query> {
    pub(super) fn new(plan: &'query Plan) -> Self {
        Self { plan }
    }

    fn columns(&self, relation: RelationId, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(" columns=[")?;
        let columns = self
            .plan
            .relation_columns(relation)
            .expect("validated relation");
        for (position, column) in columns.iter().enumerate() {
            if position != 0 {
                output.write_str(", ")?;
            }
            write!(output, "c{}", column.value())?;
        }
        output.write_str("]\n")
    }

    fn node(&self, node: &Node, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        let input = node.input.0;
        match node.stage {
            Stage::Empty => unreachable!("validated plan has no empty stage"),
            // A source occurrence is an independent input, not a consumer of the
            // placeholder backward link in its compact node representation.
            Stage::Source(source) => write!(output, "source occurrence={source}"),
            Stage::Alias => write!(output, "alias input=r{input}"),
            Stage::Derived => write!(output, "derived input=r{input}"),
            Stage::Select { .. } => write!(output, "select input=r{input}"),
            Stage::Extend { .. } => write!(output, "extend input=r{input}"),
            Stage::Set { .. } => write!(output, "set input=r{input}"),
            Stage::Drop { .. } => write!(output, "drop input=r{input}"),
            Stage::Rename => write!(output, "rename input=r{input}"),
            Stage::Distinct(_) => write!(output, "distinct input=r{input}"),
            Stage::Limit(bounds) => write!(
                output,
                "limit input=r{input} count={} offset={}",
                bounds.count(),
                bounds.offset()
            ),
            Stage::SetOperation { right, descriptor } => {
                let kind = match self.plan.set_operations[usize::from(descriptor)].kind() {
                    SetKind::UnionAll => "union-all",
                    SetKind::ExceptDistinct => "except-distinct",
                    SetKind::ExceptAll => "except-all",
                    SetKind::IntersectDistinct => "intersect-distinct",
                    SetKind::IntersectAll => "intersect-all",
                };
                write!(output, "{kind} left=r{input} right=r{}", right.0)
            }
            Stage::Join {
                nulls,
                right,
                left_key,
                right_key,
            } => write!(
                output,
                "{} left=r{input} right=r{} on=c{} = c{}",
                if nulls.is_some() {
                    "left-join"
                } else {
                    "inner-join"
                },
                right.0,
                left_key.value(),
                right_key.value()
            ),
            Stage::Order { start, len } => {
                write!(output, "order input=r{input} keys=[")?;
                for (index, key) in self
                    .plan
                    .order_items(start, len)
                    .expect("validated order")
                    .iter()
                    .enumerate()
                {
                    if index != 0 {
                        output.write_str(", ")?;
                    }
                    write!(
                        output,
                        "c{} {} NULLS {}",
                        key.column.value(),
                        match key.direction {
                            Direction::Ascending => "ASC",
                            Direction::Descending => "DESC",
                        },
                        match key.nulls {
                            NullPlacement::First => "FIRST",
                            NullPlacement::Last => "LAST",
                        }
                    )?;
                }
                output.write_char(']')
            }
            Stage::Where(filter) => {
                write!(output, "filter input=r{input} c{} ", filter.column.value())?;
                predicate(&filter.predicate, output)?;
                write!(
                    output,
                    " negated={} matched=+{} other=+{} end=+{}",
                    filter.control.negated,
                    filter.control.matched,
                    filter.control.other,
                    filter.control.end
                )
            }
            Stage::Aggregate(index) => {
                let aggregate = &self.plan.aggregates[usize::from(index)];
                write!(
                    output,
                    "aggregate input=r{input} ordered={} groups=[",
                    aggregate.ordered
                )?;
                for (index, group) in aggregate.group_columns().enumerate() {
                    if index != 0 {
                        output.write_str(", ")?;
                    }
                    write!(output, "c{}", group.identity().value())?;
                }
                output.write_str("] values=[")?;
                for (index, entry) in aggregate.entries.iter().enumerate() {
                    if index != 0 {
                        output.write_str(", ")?;
                    }
                    output.write_str(match entry.kind {
                        AggregateKind::Min => "min",
                        AggregateKind::Max => "max",
                        AggregateKind::Sum => "sum",
                        AggregateKind::Avg => "avg",
                        AggregateKind::Count => "count",
                    })?;
                    output.write_char('(')?;
                    match &entry.argument {
                        None => output.write_char('*')?,
                        Some(AggregateArgument::Column(column)) => {
                            write!(output, "c{}", column.identity().value())?
                        }
                        Some(AggregateArgument::Numeric(program)) => expression(program, output)?,
                    }
                    output.write_char(')')?;
                }
                output.write_char(']')
            }
        }
    }

    fn column_type(&self, column: ColumnId, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (kind, nullable) = self.plan.column_type(column).expect("validated column");
        write!(
            output,
            "{} {}",
            match kind {
                DataType::Double => "DOUBLE",
                DataType::Int64 => "INT64",
                DataType::String => "STRING",
                DataType::Date => "DATE",
            },
            if nullable { "nullable" } else { "required" }
        )
    }
}

impl fmt::Display for LogicalPlan<'_> {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str("logical plan\nr0 = source occurrence=0")?;
        self.columns(RelationId::SOURCE, output)?;
        for (index, node) in self.plan.nodes().iter().enumerate() {
            let relation = RelationId((index + 1) as u8);
            write!(output, "r{} = ", relation.0)?;
            self.node(node, output)?;
            self.columns(relation, output)?;
        }
        for computed in &self.plan.computed {
            write!(output, "c{} = ", computed.column.identity().value())?;
            match &computed.expression {
                Computation::Copy(column) => write!(output, "copy c{}", column.identity().value())?,
                Computation::StringLength { input, unit } => write!(
                    output,
                    "{} c{}",
                    match unit {
                        Unit::Bytes => "byte_length",
                        Unit::UnicodeScalars => "char_length",
                    },
                    input.identity().value()
                )?,
                Computation::Numeric(program) => {
                    output.write_str("numeric ")?;
                    expression(program, output)?;
                }
                Computation::Constant(Constant::String(value)) => {
                    write!(output, "string {:?}", value.as_str())?
                }
                Computation::Constant(Constant::Date(value)) => {
                    write!(output, "date days={}", value.days_since_unix_epoch())?
                }
                Computation::WindowCount => output.write_str("count(*) over ()")?,
            }
            write!(output, " input=r{} type=", computed.input.0)?;
            self.column_type(computed.column.identity(), output)?;
            output.write_char('\n')?;
        }
        writeln!(output, "result = r{}", self.plan.final_relation().0)?;
        for (position, column) in self.plan.outputs[..usize::from(self.plan.output_count)]
            .iter()
            .enumerate()
        {
            write!(
                output,
                "output[{position}] = c{} name={:?} type=",
                column.id.value(),
                (column.name != super::Name::EMPTY).then(|| column.name.as_str())
            )?;
            self.column_type(column.id, output)?;
            output.write_char('\n')?;
        }
        Ok(())
    }
}

fn expression(program: &Expression, output: &mut fmt::Formatter<'_>) -> fmt::Result {
    output.write_char('[')?;
    for (index, op) in program.ops[..usize::from(program.len)].iter().enumerate() {
        if index != 0 {
            output.write_str(", ")?;
        }
        let name = match op {
            Op::Empty => unreachable!("validated program has no empty operation"),
            Op::Column(column) => {
                write!(output, "c{}", column.identity().value())?;
                continue;
            }
            Op::Integer(value) => {
                write!(output, "{value}")?;
                continue;
            }
            Op::Double(bits) => {
                write!(output, "double_bits(0x{bits:016x})")?;
                continue;
            }
            Op::Add => "add",
            Op::Subtract => "subtract",
            Op::Multiply => "multiply",
            Op::Divide => "divide",
            Op::SafeDivide => "safe_divide",
            Op::Power => "power",
            Op::Coalesce => "coalesce",
            Op::NullIf => "nullif",
            Op::Mod => "mod",
            Op::IntegerDivide => "div",
            Op::Negate => "negate",
            Op::Abs => "abs",
            Op::Sign => "sign",
            Op::ToDouble => "to_double",
            Op::Floor => "floor",
            Op::Ceil => "ceil",
            Op::Round => "round",
            Op::Sqrt => "sqrt",
            Op::Ln => "ln",
            Op::Log10 => "log10",
            Op::Exp => "exp",
        };
        output.write_str(name)?;
    }
    output.write_char(']')
}

fn predicate(predicate: &Predicate, output: &mut fmt::Formatter<'_>) -> fmt::Result {
    let (comparison, literal) = match predicate {
        Predicate::IsNull { negated } => {
            return output.write_str(if *negated { "IS NOT NULL" } else { "IS NULL" });
        }
        Predicate::Compare {
            comparison,
            literal,
        } => (comparison, literal),
    };
    output.write_str(match comparison {
        Comparison::Less => "<",
        Comparison::LessEqual => "<=",
        Comparison::Equal => "=",
        Comparison::NotEqual => "!=",
        Comparison::GreaterEqual => ">=",
        Comparison::Greater => ">",
        Comparison::IsDistinct => "IS DISTINCT FROM",
        Comparison::IsNotDistinct => "IS NOT DISTINCT FROM",
    })?;
    output.write_char(' ')?;
    match literal {
        FilterLiteral::Null => output.write_str("NULL"),
        FilterLiteral::NullDouble => output.write_str("NULL:DOUBLE"),
        FilterLiteral::Double(bits) => write!(output, "double_bits(0x{bits:016x})"),
        FilterLiteral::Int64(value) => write!(output, "{value}"),
        FilterLiteral::String(value) => write!(output, "{:?}", value.as_str()),
        FilterLiteral::Date(value) => write!(output, "date days={}", value.days_since_unix_epoch()),
    }
}
