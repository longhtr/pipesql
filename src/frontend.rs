//! Semantic identities, immutable plans, and prepared-query ownership.
mod binding;
mod distinct;
mod join;
mod lexer;
mod parser;
mod union;
mod validation;

mod column_set;
pub(crate) use column_set::ColumnSet;

use crate::date::DateValue;
use crate::resources::Reservation;
use crate::scalar::{Expression, Op};
use crate::{Database, DatabaseId, Error, SourceSpan};
pub(crate) use binding::{prepare, prepare_catalog};
use distinct::DistinctPlan;
pub(crate) use join::NullExtension;
use lexer::reserved_identifier;
use std::mem::size_of;
pub(crate) use union::UnionPlan;
pub(crate) use validation::validate;

const MAX_SOURCE_BYTES: usize = 4096;
const MAX_TOKENS: usize = 160;
// Each explicit SELECT or EXTEND output consumes a name and a separator or stage prefix. Bound
// total projection entries by syntax size, not row width times stage count.
const MAX_PROJECTIONS: usize = MAX_TOKENS / 2;
pub(crate) const MAX_ORDER_ITEMS: usize = MAX_TOKENS / 2;
const MAX_NAME_BYTES: usize = 32;
// Covers allocator rounding/metadata for each prepared-plan or aggregate-entry
// allocation; native observations must remain inside each charged allowance.
const PREPARED_ALLOCATION_ALLOWANCE: usize = 4096;
pub(crate) const MAX_STAGES: usize = 16;
pub(crate) const MAX_COLUMNS: usize = crate::catalog_schema::MAX_COLUMNS;
// A producer can carry the visible row plus original values retained by ranges.
// Source schemas and public result rows remain bounded by MAX_COLUMNS.
pub(crate) const MAX_ROW_VALUES: usize = MAX_COLUMNS * 2;
pub(crate) const MAX_AGGREGATE_COLUMNS: usize = 10;
const _: () = assert!(MAX_AGGREGATE_COLUMNS < u16::BITS as usize);

/// A declared column or query result type. NULLability is a separate property.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataType {
    Double,
    Int64,
    String,
    Date,
}

/// One output column's schema, borrowed from its prepared query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResultColumn<'a> {
    /// Explicit or inferred name; None marks an anonymous output.
    pub name: Option<&'a str>,
    /// The type of each non-NULL value in this column.
    pub data_type: DataType,
    /// Whether the column may contain SQL NULL.
    pub nullable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ColumnId(u32);

impl ColumnId {
    pub(crate) fn value(self) -> u32 {
        self.0
    }
    pub(crate) const EMPTY: Self = Self(0);

    const fn new(value: u32) -> Self {
        assert!(value != 0);
        Self(value)
    }
}

// Facts belong to one bound relation input; identity is independent of storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SemanticColumn {
    identity: ColumnId,
    kind: DataType,
    nullable: bool,
}

impl SemanticColumn {
    pub(crate) fn identity(self) -> ColumnId {
        self.identity
    }

    pub(crate) const fn new(identity: u32, kind: DataType, nullable: bool) -> Self {
        Self {
            identity: ColumnId::new(identity),
            kind,
            nullable,
        }
    }

    pub(crate) fn data_type(self) -> DataType {
        self.kind
    }

    pub(crate) fn nullable(self) -> bool {
        self.nullable
    }
}

// Identity is independent of spelling and position in a projected output list.
// A source identity is independent of its physical position and semantic type.
// The binder supplies canonical properties; later stages preserve this value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SourceColumn {
    identity: ColumnId,
    storage: u8,
    kind: DataType,
    nullable: bool,
}

impl SourceColumn {
    // Fixed legacy/test metadata; catalog binding assigns query identity separately.
    pub(crate) const fn declared(
        identity: u32,
        storage: u8,
        kind: DataType,
        nullable: bool,
    ) -> Self {
        assert!(storage < 64);
        Self {
            identity: ColumnId::new(identity),
            storage,
            kind,
            nullable,
        }
    }
    pub(crate) const QUANTITY: Self = Self::declared(1, 0, DataType::Double, false);
    pub(crate) const PRICE: Self = Self::declared(2, 1, DataType::Double, false);
    pub(crate) const DISCOUNT: Self = Self::declared(3, 2, DataType::Double, false);
    pub(crate) const TAX: Self = Self::declared(4, 3, DataType::Double, false);
    pub(crate) const RETURN_FLAG: Self = Self::declared(5, 4, DataType::String, false);
    pub(crate) const LINE_STATUS: Self = Self::declared(6, 5, DataType::String, false);
    pub(crate) const SHIP_DATE: Self = Self::declared(7, 6, DataType::Date, false);

    fn bind_declaration(
        column: crate::catalog_schema::ColumnSpec<'_>,
        ordinal: usize,
        identity: u32,
    ) -> Self {
        assert!(ordinal < MAX_COLUMNS);
        Self {
            identity: ColumnId::new(identity),
            storage: ordinal as u8,
            kind: column.data_type(),
            nullable: column.nullable(),
        }
    }

    pub(crate) const fn semantic(self) -> SemanticColumn {
        SemanticColumn::new(self.identity.0, self.kind, self.nullable)
    }

    pub(crate) fn nullable(self) -> bool {
        self.nullable
    }

    pub(crate) fn storage_slot(self) -> u8 {
        self.storage
    }

    pub(crate) fn data_type(self) -> DataType {
        self.kind
    }
}
const CATALOG: [(SourceColumn, &str); 7] = [
    (SourceColumn::QUANTITY, "l_quantity"),
    (SourceColumn::PRICE, "l_extendedprice"),
    (SourceColumn::DISCOUNT, "l_discount"),
    (SourceColumn::TAX, "l_tax"),
    (SourceColumn::RETURN_FLAG, "l_returnflag"),
    (SourceColumn::LINE_STATUS, "l_linestatus"),
    (SourceColumn::SHIP_DATE, "l_shipdate"),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Name {
    bytes: [u8; MAX_NAME_BYTES],
    len: u8,
}

impl Name {
    const EMPTY: Self = Self {
        bytes: [0; MAX_NAME_BYTES],
        len: 0,
    };

    fn new(text: &str) -> Self {
        assert!(!text.is_empty() && text.len() <= MAX_NAME_BYTES);
        let mut name = Self::EMPTY;
        name.bytes[..text.len()].copy_from_slice(text.as_bytes());
        name.len = u8::try_from(text.len()).expect("bounded name");
        name
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).expect("validated ASCII name")
    }

    fn valid(self) -> bool {
        let len = usize::from(self.len);
        len > 0
            && len <= MAX_NAME_BYTES
            && (self.bytes[0].is_ascii_alphabetic() || self.bytes[0] == b'_')
            && self.bytes[..len]
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
            && self.bytes[len..].iter().all(|b| *b == 0)
            && !reserved_identifier(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Output {
    name: Name,
    id: ColumnId,
}

impl Output {
    const EMPTY: Self = Self {
        name: Name::EMPTY,
        id: ColumnId::EMPTY,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Comparison {
    Less,
    LessEqual,
    Equal,
    NotEqual,
    GreaterEqual,
    Greater,
}

impl Comparison {
    pub(crate) fn test_order(self, order: std::cmp::Ordering) -> bool {
        match self {
            Self::Less => order.is_lt(),
            Self::LessEqual => !order.is_gt(),
            Self::Equal => order.is_eq(),
            Self::NotEqual => !order.is_eq(),
            Self::GreaterEqual => !order.is_lt(),
            Self::Greater => order.is_gt(),
        }
    }

    pub(crate) fn test(self, left: f64, right: f64) -> bool {
        match self {
            Self::Less => left < right,
            Self::LessEqual => left <= right,
            Self::Equal => left == right,
            Self::NotEqual => left != right,
            Self::GreaterEqual => left >= right,
            Self::Greater => left > right,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FilterLiteral {
    Null,
    // A folded numeric NULL must not become an untyped NULL for comparison binding.
    NullDouble,
    Double(u64),
    Int64(i64),
    Date(DateValue),
    String(crate::text_literal::TextLiteral),
}

impl FilterLiteral {
    fn valid_for(self, data_type: DataType) -> bool {
        match self {
            Self::Null => true,
            Self::NullDouble => matches!(data_type, DataType::Int64 | DataType::Double),
            Self::Double(bits) => {
                matches!(data_type, DataType::Double | DataType::Int64)
                    && f64::from_bits(bits).is_finite()
            }
            Self::Int64(_) => data_type == DataType::Int64,
            Self::Date(_) => data_type == DataType::Date,
            Self::String(value) => data_type == DataType::String && value.valid(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Predicate {
    Compare {
        comparison: Comparison,
        literal: FilterLiteral,
    },
    IsNull {
        negated: bool,
    },
}

impl Predicate {
    fn valid_for(self, data_type: DataType) -> bool {
        match self {
            Self::Compare {
                comparison,
                literal,
            } => {
                literal.valid_for(data_type)
                    && (literal != FilterLiteral::Null || comparison == Comparison::Equal)
            }
            Self::IsNull { .. } => true,
        }
    }
}

/// Forward leaf decisions within one WHERE expression. Zero rejects the row;
/// positive offsets advance to another leaf or the expression's continuation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FilterControl {
    pub(crate) matched: u8,
    pub(crate) other: u8,
    pub(crate) end: u8,
    pub(crate) negated: bool,
}

impl FilterControl {
    pub(crate) const LINEAR: Self = Self {
        matched: 1,
        other: 0,
        end: 1,
        negated: false,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Filter {
    pub(crate) column: ColumnId,
    pub(crate) predicate: Predicate,
    pub(crate) control: FilterControl,
    span: SourceSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AggregateKind {
    Min,
    Max,
    Sum,
    Avg,
    Count,
}

// Programs stay inline in the admitted plan; boxing would add a separate
// fallible allocation and owner for every numeric aggregate argument.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AggregateArgument {
    Numeric(Expression),
    /// Direct typed input; demanded aggregate kinds determine which values are retained.
    Column(SemanticColumn),
}

impl AggregateArgument {
    pub(crate) fn data_type(&self) -> DataType {
        match self {
            Self::Numeric(expression) => expression.data_type,
            Self::Column(column) => column.data_type(),
        }
    }

    pub(crate) fn nullable(&self) -> bool {
        match self {
            Self::Numeric(expression) => expression.nullable(),
            Self::Column(column) => column.nullable(),
        }
    }

    pub(crate) fn stack_depth(&self) -> usize {
        match self {
            Self::Numeric(expression) => expression.stack_depth(),
            Self::Column(_) => 0,
        }
    }

    pub(crate) fn columns(&self) -> impl Iterator<Item = SemanticColumn> + '_ {
        let (ops, column): (&[Op], _) = match self {
            Self::Numeric(expression) => (&expression.ops[..usize::from(expression.len)], None),
            Self::Column(column) => (&[], Some(*column)),
        };
        ops.iter()
            .filter_map(|op| match op {
                Op::Column(column) => Some(*column),
                _ => None,
            })
            .chain(column)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AggregateEntry {
    pub(crate) kind: AggregateKind,
    pub(crate) argument: Option<AggregateArgument>,
    pub(crate) span: SourceSpan,
    name: Name,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Group {
    name: Name,
    input: SemanticColumn,
}

impl Group {
    const EMPTY: Self = Self {
        name: Name::EMPTY,
        input: SourceColumn::QUANTITY.semantic(),
    };
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct AggregatePlan {
    pub(crate) entries: Vec<AggregateEntry>,
    groups: [Group; MAX_AGGREGATE_COLUMNS - 1],
    first_output: ColumnId,
    pub(crate) group_count: u8,
    pub(crate) ordered: bool,
}

impl AggregatePlan {
    pub(crate) fn group_columns(&self) -> impl Iterator<Item = SemanticColumn> + '_ {
        self.groups[..usize::from(self.group_count)]
            .iter()
            .map(|group| group.input)
    }

    fn output(&self, index: usize) -> Option<Output> {
        if self.first_output == ColumnId::EMPTY {
            return None;
        }
        let name = if index < usize::from(self.group_count) {
            self.groups[index].name
        } else {
            self.entries
                .get(index - usize::from(self.group_count))?
                .name
        };
        let identity = self
            .first_output
            .0
            .checked_add(u32::try_from(index).ok()?)?;
        Some(Output {
            name,
            id: ColumnId::new(identity),
        })
    }

    pub(crate) fn output_position(&self, id: ColumnId) -> Option<usize> {
        (0..usize::from(self.group_count) + self.entries.len())
            .find(|index| self.output(*index).is_some_and(|output| output.id == id))
    }

    pub(crate) fn column_type(&self, id: ColumnId) -> Option<(DataType, bool)> {
        let index = self.output_position(id)?;
        if index < usize::from(self.group_count) {
            let input = self.groups[index].input;
            return Some((input.data_type(), input.nullable()));
        }
        let entry = &self.entries[index - usize::from(self.group_count)];
        Some(match entry.kind {
            AggregateKind::Count => (DataType::Int64, false),
            AggregateKind::Sum | AggregateKind::Min | AggregateKind::Max => (
                entry
                    .argument
                    .as_ref()
                    .expect("validated SUM input")
                    .data_type(),
                true,
            ),
            AggregateKind::Avg => (DataType::Double, true),
        })
    }

    #[cfg(test)]
    fn needs(&self, column: SourceColumn, demand: u16) -> bool {
        self.group_columns().any(|id| id == column.semantic())
            || self.entries.iter().enumerate().any(|(index, entry)| {
                demand & (1 << index) != 0
                    && entry
                        .argument
                        .as_ref()
                        .is_some_and(|argument| argument.columns().any(|c| c == column.semantic()))
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Direction {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NullPlacement {
    First,
    Last,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OrderKey {
    pub(crate) column: ColumnId,
    pub(crate) direction: Direction,
    pub(crate) nulls: NullPlacement,
}

impl OrderKey {
    const EMPTY: Self = Self {
        column: ColumnId::EMPTY,
        direction: Direction::Ascending,
        nulls: NullPlacement::First,
    };
}

/// Separate cardinalities avoid overflowing when count + offset exceeds INT64.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LimitBounds {
    count: u64,
    offset: u64,
}

impl LimitBounds {
    pub(crate) fn count(self) -> u64 {
        self.count
    }

    pub(crate) fn offset(self) -> u64 {
        self.offset
    }

    fn validate(self) -> Result<(), Error> {
        if self.count > i64::MAX as u64 || self.offset > i64::MAX as u64 {
            return Err(Error::Corrupt("limit bounds outside INT64"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JoinKind {
    Inner,
    Left,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Stage {
    Empty,
    Limit(LimitBounds),
    Source(u8),
    Alias,
    Derived,
    UnionAll {
        right: RelationId,
        descriptor: u8,
    },
    Join {
        nulls: Option<u8>,
        right: RelationId,
        left_key: ColumnId,
        right_key: ColumnId,
    },
    Aggregate(u8),
    Distinct(u8),
    Order {
        start: u8,
        len: u8,
    },
    Select {
        start: u8,
        len: u8,
    },
    Rename,
    Set {
        start: u8,
        len: u8,
    },
    // Bit i retains visible input position i; the input row has at most 64 columns.
    Drop {
        keep: u64,
    },
    // Only appended entries occupy the projection pool; input columns are inherited.
    Extend {
        start: u8,
        len: u8,
    },
    Where(Filter),
}

// Relation zero is the source; stage i produces relation i + 1.
// Backward-only inputs bound every traversal without recursive state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RelationId(pub(crate) u8);

impl RelationId {
    pub(crate) const SOURCE: Self = Self(0);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Node {
    pub(crate) input: RelationId,
    pub(crate) stage: Stage,
    pub(crate) columns: u8,
}

impl Node {
    const EMPTY: Self = Self {
        input: RelationId::SOURCE,
        stage: Stage::Empty,
        columns: 0,
    };
}

#[derive(Clone, Copy)]
pub(crate) struct RelationColumns<'a> {
    plan: &'a Plan,
    relation: RelationId,
    count: usize,
}

impl RelationColumns<'_> {
    pub(crate) fn len(self) -> usize {
        self.count
    }

    pub(crate) fn get(self, mut index: usize) -> Option<ColumnId> {
        if index >= self.count {
            return None;
        }
        let mut relation = self.relation;
        // Every hop follows an earlier relation. Neither syntax depth nor joins
        // consume the Rust stack.
        while relation != RelationId::SOURCE {
            let node = self.plan.node(relation).ok()?;
            match node.stage {
                Stage::Alias
                | Stage::Rename
                | Stage::Derived
                | Stage::Where(_)
                | Stage::Order { .. }
                | Stage::Limit(_) => relation = node.input,
                Stage::Drop { mut keep } => {
                    for _ in 0..index {
                        keep &= keep.checked_sub(1)?;
                    }
                    if keep == 0 {
                        return None;
                    }
                    index = keep.trailing_zeros() as usize;
                    relation = node.input;
                }
                Stage::Set { start, len } => {
                    let end = usize::from(start) + usize::from(len);
                    let assignments = self.plan.assignments.get(usize::from(start)..end)?;
                    if let Some(assignment) = assignments
                        .iter()
                        .find(|entry| usize::from(entry.position) == index)
                    {
                        return Some(assignment.column);
                    }
                    relation = node.input;
                }
                Stage::Extend { start, len } => {
                    let inherited = self.plan.relation_columns(node.input).ok()?.len();
                    if index < inherited {
                        relation = node.input;
                    } else {
                        index -= inherited;
                        if index >= usize::from(len) {
                            return None;
                        }
                        return self
                            .plan
                            .projections
                            .get(usize::from(start) + index)
                            .copied();
                    }
                }
                Stage::Select { start, len } => {
                    if index >= usize::from(len) {
                        return None;
                    }
                    return self
                        .plan
                        .projections
                        .get(usize::from(start) + index)
                        .copied();
                }
                Stage::UnionAll { descriptor, .. } => {
                    return self
                        .plan
                        .unions
                        .get(usize::from(descriptor))?
                        .output(index)
                        .map(|c| c.identity());
                }
                Stage::Distinct(descriptor) => {
                    return self
                        .plan
                        .distinct
                        .get(usize::from(descriptor))?
                        .output(index);
                }
                Stage::Aggregate(aggregate_index) => {
                    return self
                        .plan
                        .aggregates
                        .get(usize::from(aggregate_index))?
                        .output(index)
                        .map(|o| o.id);
                }
                Stage::Source(source) => return self.plan.occurrence_column(source, index),
                Stage::Join { right, nulls, .. } => {
                    let left_count = self.plan.relation_columns(node.input).ok()?.len();
                    if index < left_count {
                        relation = node.input;
                    } else {
                        index -= left_count;
                        if let Some(descriptor) = nulls {
                            return self
                                .plan
                                .null_extensions
                                .get(usize::from(descriptor))?
                                .visible(index);
                        }
                        relation = right;
                    }
                }
                Stage::Empty => return None,
            }
        }
        self.plan.occurrence_column(0, index)
    }

    pub(crate) fn iter(self) -> impl Iterator<Item = ColumnId> {
        (0..self.len()).map(move |index| self.get(index).expect("validated relation output"))
    }

    fn identity_set(self) -> Result<ColumnSet, Error> {
        let mut set = ColumnSet::EMPTY;
        for index in 0..self.len() {
            let id = self
                .get(index)
                .ok_or(Error::Corrupt("relation output absent"))?;
            if id.value() == 0 || id.value() as usize > MAX_QUERY_COLUMNS {
                return Err(Error::Corrupt("relation identity outside plan"));
            }
            set.insert(id);
        }
        Ok(set)
    }

    pub(crate) fn contains(self, id: ColumnId) -> bool {
        self.iter().any(|column| column == id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SourceOccurrence {
    pub(crate) table: Option<crate::catalog_schema::TableId>,
    start: u8,
    columns: u8,
}

impl SourceOccurrence {
    const EMPTY: Self = Self {
        table: None,
        start: 0,
        columns: 0,
    };
}

pub(crate) const MAX_COMPUTED: usize = MAX_PROJECTIONS;
// A LEFT JOIN can extend a visible row plus retained qualified values (2 widths).
// Each join also requires a distinct source stage, which creates no fresh output
// identities. Charging one width to each of those stages retains this bound.
pub(crate) const MAX_QUERY_COLUMNS: usize =
    MAX_COLUMNS + MAX_COMPUTED + MAX_AGGREGATE_COLUMNS + MAX_STAGES * MAX_COLUMNS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SetAssignment {
    column: ColumnId,
    position: u8,
}

impl SetAssignment {
    const EMPTY: Self = Self {
        column: ColumnId::EMPTY,
        position: 0,
    };
}

/// Nonnumeric constants own their bounded payload in the prepared descriptor.
/// They have no input dependencies and never enter the numeric scratch cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Constant {
    String(crate::text_literal::TextLiteral),
    Date(DateValue),
}

impl Constant {
    pub(crate) fn value(&self) -> crate::Value<'_> {
        match self {
            Self::String(value) => crate::Value::String(crate::StringValue::new(value.as_str())),
            Self::Date(value) => crate::Value::Date(*value),
        }
    }

    fn data_type(&self) -> DataType {
        match self {
            Self::String(_) => DataType::String,
            Self::Date(_) => DataType::Date,
        }
    }

    fn valid(&self) -> bool {
        match self {
            Self::String(value) => value.valid(),
            Self::Date(value) => DateValue::from_days(value.days_since_unix_epoch()).is_some(),
        }
    }
}

// Inline programs keep one admitted descriptor allocation; boxing each numeric
// expression would add another fallible owner per assignment.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Computation {
    Copy(SemanticColumn),
    Numeric(Expression),
    Constant(Constant),
    WindowCount,
}

impl Computation {
    pub(crate) fn columns(&self) -> impl Iterator<Item = SemanticColumn> {
        let (copy, expression) = match self {
            Self::Copy(column) => (Some(*column), None),
            Self::Numeric(expression) => (None, Some(expression)),
            Self::Constant(_) | Self::WindowCount => (None, None),
        };
        copy.into_iter()
            .chain(expression.into_iter().flat_map(|expression| {
                expression.ops[..usize::from(expression.len)]
                    .iter()
                    .filter_map(|op| {
                        if let Op::Column(column) = op {
                            Some(*column)
                        } else {
                            None
                        }
                    })
            }))
    }

    pub(crate) fn numeric(&self) -> Result<&Expression, Error> {
        match self {
            Self::Numeric(expression) => Ok(expression),
            Self::Copy(_) | Self::Constant(_) | Self::WindowCount => Err(Error::Corrupt(
                "nonnumeric computation reached a numeric kernel",
            )),
        }
    }

    fn data_type(&self) -> DataType {
        match self {
            Self::Copy(column) => column.data_type(),
            Self::Numeric(expression) => expression.data_type,
            Self::Constant(value) => value.data_type(),
            Self::WindowCount => DataType::Int64,
        }
    }

    fn nullable(&self) -> bool {
        match self {
            Self::Copy(column) => column.nullable(),
            Self::Numeric(expression) => expression.nullable(),
            Self::Constant(_) | Self::WindowCount => false,
        }
    }

    fn validate(&self, available: &[SemanticColumn]) -> Result<(), Error> {
        match self {
            Self::Copy(column) if available.contains(column) => Ok(()),
            Self::Copy(_) => Err(Error::Corrupt("copy input outside scope")),
            Self::Numeric(expression) => expression.validate(available),
            Self::WindowCount => Ok(()),
            Self::Constant(value) if value.valid() => Ok(()),
            Self::Constant(_) => Err(Error::Corrupt("invalid computed constant")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Computed {
    pub(crate) column: SemanticColumn,
    pub(crate) expression: Computation,
    pub(crate) span: SourceSpan,
    pub(crate) input: RelationId,
}

impl Computed {
    fn allocation_capacity(count: usize) -> Option<usize> {
        // Preserve the logical descriptor count while admitting physical slots
        // near the existing native allocation boundary. Rounding is charged as
        // requested storage; it does not enlarge the expression-count limit.
        count
            .checked_mul(size_of::<Self>())
            .and_then(crate::resources::buffer_capacity)
            .map(|bytes| bytes / size_of::<Self>())
    }
}

pub(crate) struct Plan {
    pub(crate) database: DatabaseId,
    pub(crate) generation: u64,
    occurrences: [SourceOccurrence; MAX_STAGES],
    occurrence_count: u8,
    source_bytes: u16,
    source_columns: [SourceColumn; MAX_COLUMNS],
    catalog_columns: [u32; MAX_COLUMNS],
    source_count: u8,
    stages: [Node; MAX_STAGES],
    range_columns: [ColumnSet; MAX_STAGES + 1],
    assignments: [SetAssignment; MAX_PROJECTIONS],
    assignment_count: u8,
    count: u8,
    projections: [ColumnId; MAX_PROJECTIONS],
    projection_count: u8,
    order_items: [OrderKey; MAX_ORDER_ITEMS],
    order_count: u8,
    outputs: [Output; MAX_COLUMNS],
    output_count: u8,
    pub(crate) aggregates: Vec<AggregatePlan>,
    pub(crate) distinct: Vec<DistinctPlan>,
    pub(crate) unions: Vec<UnionPlan>,
    pub(crate) null_extensions: Vec<NullExtension>,
    pub(crate) computed: Vec<Computed>,
}
// One allocation keeps the caller's prepared handle bounded as typed plans grow.
// Its physical storage drops before PreparedQuery releases the database charge.
pub(crate) struct OwnedPlan {
    values: Vec<Plan>,
}

impl OwnedPlan {
    fn as_mut(&mut self) -> &mut Plan {
        &mut self.values[0]
    }

    fn new(plan: Plan, limit: u64) -> Result<Self, Error> {
        let mut values = Vec::new();
        values.try_reserve_exact(1).map_err(|_| Error::Resource {
            owner: "prepared plan allocation",
            required: size_of::<Plan>() as u64,
            limit,
        })?;
        if values.capacity() != 1 {
            return Err(Error::Resource {
                owner: "prepared plan capacity",
                required: values.capacity() as u64,
                limit: 1,
            });
        }
        values.push(plan);
        Ok(Self { values })
    }
}

impl std::ops::Deref for OwnedPlan {
    type Target = Plan;

    fn deref(&self) -> &Plan {
        &self.values[0]
    }
}

#[cfg(test)]
impl std::ops::DerefMut for OwnedPlan {
    fn deref_mut(&mut self) -> &mut Plan {
        &mut self.values[0]
    }
}

/// A validated query that borrows its database and retains its input snapshot.
///
/// Declared-table queries keep the generation selected during preparation, even
/// after later appends. A running result borrows this query. Drop the result before
/// dropping the query to release its retained plan, reservation, and snapshot pin.
pub struct PreparedQuery<'database> {
    pub(crate) plan: OwnedPlan,
    pub(crate) snapshot: Option<crate::catalog_snapshot::Snapshot<'database>>,
    reservation: Reservation<'database>,
}

impl PreparedQuery<'_> {
    /// Number of columns in the query's output schema, including repeated outputs.
    pub fn result_column_count(&self) -> usize {
        usize::from(self.plan.output_count)
    }

    /// Inspect a zero-based output position, or return `None` outside the schema.
    pub fn result_column(&self, index: usize) -> Option<ResultColumn<'_>> {
        let output = self.plan.outputs[..usize::from(self.plan.output_count)].get(index)?;
        let (data_type, nullable) = self.plan.column_type(output.id)?;
        Some(ResultColumn {
            name: (output.name != Name::EMPTY).then(|| output.name.as_str()),
            data_type,
            nullable,
        })
    }

    /// The retained plan's logical memory charge, excluding running-query owners.
    pub fn accounted_memory_bytes(&self) -> u64 {
        self.reservation.bytes()
    }

    /// A conservative retained-plan ceiling in bytes, not an exact query minimum.
    /// Preparation also needs transient catalog and name-scope storage. This value
    /// excludes those temporary peaks and the database's resident charge.
    pub fn memory_requirement_bytes() -> u64 {
        // Conservative retained-plan ceiling. Preparation also charges transient
        // scope storage when a query has more than one source occurrence.
        (size_of::<Self>()
            + size_of::<Plan>()
            + MAX_AGGREGATE_COLUMNS * (size_of::<AggregateEntry>() + size_of::<AggregatePlan>())
            + Computed::allocation_capacity(MAX_COMPUTED).expect("bounded computed capacity")
                * size_of::<Computed>()
            + MAX_STAGES * size_of::<DistinctPlan>()
            + MAX_STAGES * size_of::<UnionPlan>()
            + MAX_STAGES * size_of::<NullExtension>()
            + PREPARED_ALLOCATION_ALLOWANCE
            + (MAX_AGGREGATE_COLUMNS + 5) * PREPARED_ALLOCATION_ALLOWANCE) as u64
    }
}

impl Plan {
    pub(crate) fn nodes(&self) -> &[Node] {
        &self.stages[..usize::from(self.count)]
    }

    pub(crate) fn final_relation(&self) -> RelationId {
        RelationId(self.count)
    }

    pub(crate) fn occurrence(&self, index: u8) -> Result<&SourceOccurrence, Error> {
        self.occurrences[..usize::from(self.occurrence_count)]
            .get(usize::from(index))
            .ok_or(Error::Corrupt("source occurrence outside plan"))
    }

    pub(crate) fn occurrence_columns(&self, index: u8) -> Result<&[SourceColumn], Error> {
        let source = self.occurrence(index)?;
        let start = usize::from(source.start);
        self.source_columns
            .get(start..start + usize::from(source.columns))
            .ok_or(Error::Corrupt("source occurrence columns outside plan"))
    }

    pub(crate) fn matches_source_declaration(
        &self,
        occurrence: u8,
        source: SourceColumn,
        column: crate::catalog_schema::ColumnSpec<'_>,
        ordinal: usize,
    ) -> bool {
        let Ok(binding) = self.occurrence(occurrence) else {
            return false;
        };
        ordinal < usize::from(binding.columns)
            && self.source_columns[usize::from(binding.start) + ordinal] == source
            && self.catalog_columns[usize::from(binding.start) + ordinal] == column.id().value()
            && usize::from(source.storage) == ordinal
            && source.kind == column.data_type()
            && source.nullable == column.nullable()
    }

    pub(crate) fn has_joins(&self) -> bool {
        self.stages[..usize::from(self.count)]
            .iter()
            .any(|node| matches!(node.stage, Stage::Join { .. }))
    }

    pub(crate) fn table(&self) -> Option<crate::catalog_schema::TableId> {
        self.occurrences[0].table
    }

    fn occurrence_column(&self, source: u8, index: usize) -> Option<ColumnId> {
        if source >= self.occurrence_count {
            return None;
        }
        let source = self.occurrences.get(usize::from(source))?;
        if index >= usize::from(source.columns) {
            return None;
        }
        self.source_columns
            .get(usize::from(source.start) + index)
            .map(|column| column.identity)
    }

    #[cfg(test)]
    pub(crate) fn matches_declaration(
        &self,
        source: SourceColumn,
        column: crate::catalog_schema::ColumnSpec<'_>,
        ordinal: usize,
    ) -> bool {
        self.catalog_columns.get(ordinal).copied() == Some(column.id().value())
            && usize::from(source.storage) == ordinal
            && source.kind == column.data_type()
            && source.nullable == column.nullable()
    }

    pub(crate) fn column_type(&self, id: ColumnId) -> Option<(DataType, bool)> {
        column_type(
            id,
            &self.source_columns[..usize::from(self.source_count)],
            &self.aggregates,
            &self.computed,
            &self.distinct,
            &self.unions,
            &self.null_extensions,
        )
    }

    pub(crate) fn column(&self, id: ColumnId) -> Option<SemanticColumn> {
        let (kind, nullable) = self.column_type(id)?;
        Some(SemanticColumn::new(id.value(), kind, nullable))
    }

    pub(crate) fn node(&self, relation: RelationId) -> Result<&Node, Error> {
        if relation.0 == 0 || relation.0 > self.count {
            return Err(Error::Corrupt("relation reference outside plan"));
        }
        let node = self
            .stages
            .get(usize::from(relation.0 - 1))
            .ok_or(Error::Corrupt("relation reference outside capacity"))?;
        if node.input.0 >= relation.0 {
            return Err(Error::Corrupt("relation input is not an earlier producer"));
        }
        if let Stage::Join { right, .. } | Stage::UnionAll { right, .. } = node.stage
            && (right.0 >= relation.0 || right == node.input)
        {
            return Err(Error::Corrupt(
                "join inputs are not distinct earlier producers",
            ));
        }
        Ok(node)
    }

    pub(crate) fn available_columns(&self, relation: RelationId) -> Result<ColumnSet, Error> {
        let columns = *self
            .range_columns
            .get(usize::from(relation.0))
            .ok_or(Error::Corrupt("range scope outside plan"))?;
        Ok(columns | self.relation_columns(relation)?.identity_set()?)
    }

    pub(crate) fn relation_columns(
        &self,
        relation: RelationId,
    ) -> Result<RelationColumns<'_>, Error> {
        let count = if relation == RelationId::SOURCE {
            self.occurrences[0].columns
        } else {
            self.node(relation)?.columns
        };
        if count == 0 || usize::from(count) > MAX_COLUMNS {
            return Err(Error::Corrupt("relation width outside capacity"));
        }
        Ok(RelationColumns {
            plan: self,
            relation,
            count: usize::from(count),
        })
    }

    // A projection containing an analytic computation consumes its full input
    // before emitting rows. Its other expressions still use the input scope,
    // but evaluate during emission, so later LIMIT can leave them undemanded.
    pub(crate) fn analytic_projection(&self, relation: RelationId) -> bool {
        let Ok(node) = self.node(relation) else {
            return false;
        };
        let (Stage::Select { start, len } | Stage::Extend { start, len }) = node.stage else {
            return false;
        };
        let Some(outputs) = self
            .projections
            .get(usize::from(start)..usize::from(start) + usize::from(len))
        else {
            return false;
        };
        self.computed.iter().any(|definition| {
            definition.input == node.input
                && outputs.contains(&definition.column.identity())
                && matches!(definition.expression, Computation::WindowCount)
        })
    }

    pub(crate) fn computation_producer(&self, definition: &Computed) -> Result<RelationId, Error> {
        for (index, node) in self.nodes().iter().enumerate() {
            if node.input == definition.input {
                let relation = RelationId(index as u8 + 1);
                if self.analytic_projection(relation) {
                    return Ok(relation);
                }
            }
        }
        self.producer(definition.input)
    }

    pub(crate) fn producer(&self, mut relation: RelationId) -> Result<RelationId, Error> {
        while relation != RelationId::SOURCE {
            let node = self.node(relation)?;
            if self.analytic_projection(relation) {
                return Ok(relation);
            }
            match node.stage {
                Stage::Alias
                | Stage::Rename
                | Stage::Drop { .. }
                | Stage::Derived
                | Stage::Where(_)
                | Stage::Select { .. }
                | Stage::Extend { .. }
                | Stage::Set { .. } => relation = node.input,
                Stage::Aggregate(_)
                | Stage::Distinct(_)
                | Stage::Source(_)
                | Stage::Join { .. }
                | Stage::UnionAll { .. }
                | Stage::Order { .. }
                | Stage::Limit(_) => {
                    return Ok(relation);
                }
                Stage::Empty => return Err(Error::Corrupt("empty relation producer")),
            }
        }
        Ok(relation)
    }

    pub(crate) fn outputs(&self) -> impl Iterator<Item = ColumnId> + '_ {
        self.outputs[..usize::from(self.output_count)]
            .iter()
            .map(|output| output.id)
    }

    pub(crate) fn order_items(&self, start: u8, len: u8) -> Result<&[OrderKey], Error> {
        let end = usize::from(start) + usize::from(len);
        if len == 0 || end > usize::from(self.order_count) || end > MAX_ORDER_ITEMS {
            return Err(Error::Corrupt("order item range"));
        }
        Ok(&self.order_items[usize::from(start)..end])
    }
    // Hidden order identities survive transparent operators but never become
    // ordinary output names. The walk is bounded by earlier relation indices.
    pub(crate) fn order_key(
        &self,
        mut relation: RelationId,
        index: usize,
    ) -> Result<Option<OrderKey>, Error> {
        while relation != RelationId::SOURCE {
            let node = self.node(relation)?;
            if self.analytic_projection(relation) {
                return Ok(None);
            }
            match node.stage {
                Stage::Alias
                | Stage::Rename
                | Stage::Drop { .. }
                | Stage::Select { .. }
                | Stage::Extend { .. }
                | Stage::Set { .. }
                | Stage::Where(_)
                | Stage::Limit(_) => relation = node.input,
                Stage::Order { start, len } => {
                    return Ok(self.order_items(start, len)?.get(index).copied());
                }
                Stage::Aggregate(aggregate_index) => {
                    let aggregate = self
                        .aggregates
                        .get(usize::from(aggregate_index))
                        .ok_or(Error::Corrupt("ordered aggregate absent"))?;
                    return Ok(
                        (aggregate.ordered && index < usize::from(aggregate.group_count)).then(
                            || OrderKey {
                                column: aggregate.output(index).expect("bounded group output").id,
                                direction: Direction::Ascending,
                                nulls: NullPlacement::First,
                            },
                        ),
                    );
                }
                Stage::Join { .. }
                | Stage::UnionAll { .. }
                | Stage::Source(_)
                | Stage::Derived
                | Stage::Distinct(_) => {
                    return Ok(None);
                }
                Stage::Empty => return Err(Error::Corrupt("empty order producer")),
            }
        }
        Ok(None)
    }

    pub(crate) fn aggregate_demand(&self, target: u8) -> u16 {
        // Walk semantic stages independently of physical relation masks. Reverse
        // stage order preserves dependencies across projections and aggregates.
        let mut needed = [false; MAX_QUERY_COLUMNS + 1];
        for output in self.outputs() {
            needed[output.value() as usize] = true;
        }
        for node in self.nodes().iter().rev() {
            match node.stage {
                Stage::UnionAll { descriptor, .. } => {
                    let union = &self.unions[usize::from(descriptor)];
                    for position in 0..usize::from(node.columns) {
                        let output = union.output(position).expect("validated union output");
                        if needed[output.identity().value() as usize] {
                            for input in union.inputs(position).expect("validated union inputs") {
                                needed[input.identity().value() as usize] = true;
                            }
                        }
                    }
                }
                Stage::Where(filter) => needed[filter.column.value() as usize] = true,
                Stage::Distinct(index) => {
                    for column in self.distinct[usize::from(index)].inputs() {
                        needed[column.identity().value() as usize] = true;
                    }
                }
                Stage::Join {
                    nulls,
                    left_key,
                    right_key,
                    ..
                } => {
                    if let Some(descriptor) = nulls {
                        let extension = &self.null_extensions[usize::from(descriptor)];
                        for value in 1..=MAX_QUERY_COLUMNS {
                            if needed[value]
                                && let Some(input) =
                                    extension.input_for(ColumnId::new(value as u32))
                            {
                                needed[input.identity().value() as usize] = true;
                            }
                        }
                    }
                    needed[left_key.value() as usize] = true;
                    needed[right_key.value() as usize] = true;
                }
                Stage::Order { start, len } => {
                    for key in
                        &self.order_items[usize::from(start)..usize::from(start) + usize::from(len)]
                    {
                        needed[key.column.value() as usize] = true;
                    }
                }
                Stage::Select { start, len } | Stage::Extend { start, len } => {
                    for id in self.projections
                        [usize::from(start)..usize::from(start) + usize::from(len)]
                        .iter()
                        .rev()
                    {
                        if needed[id.value() as usize]
                            && let Some(definition) = self
                                .computed
                                .iter()
                                .find(|definition| definition.column.identity() == *id)
                        {
                            for column in definition.expression.columns() {
                                needed[column.identity().value() as usize] = true;
                            }
                        }
                    }
                }
                Stage::Set { start, len } => {
                    for assignment in
                        &self.assignments[usize::from(start)..usize::from(start) + usize::from(len)]
                    {
                        if needed[assignment.column.value() as usize]
                            && let Some(definition) = self.computed.iter().find(|definition| {
                                definition.column.identity() == assignment.column
                            })
                        {
                            for column in definition.expression.columns() {
                                needed[column.identity().value() as usize] = true;
                            }
                        }
                    }
                }
                Stage::Aggregate(index) => {
                    let aggregate = &self.aggregates[usize::from(index)];
                    let mut demand = 0;
                    for (entry_index, entry) in aggregate.entries.iter().enumerate() {
                        let id = aggregate
                            .output(usize::from(aggregate.group_count) + entry_index)
                            .expect("validated aggregate output")
                            .id;
                        if needed[id.value() as usize] {
                            demand |= 1 << entry_index;
                            if let Some(argument) = &entry.argument {
                                for column in argument.columns() {
                                    needed[column.identity().value() as usize] = true;
                                }
                            }
                        }
                    }
                    if index == target {
                        return demand;
                    }
                    for key in aggregate.group_columns() {
                        needed[key.identity().value() as usize] = true;
                    }
                }
                _ => (),
            }
        }
        0
    }

    #[cfg(test)]
    pub(crate) fn input_columns(&self) -> impl Iterator<Item = SemanticColumn> + Clone + '_ {
        self.columns().map(SourceColumn::semantic)
    }

    #[cfg(test)]
    pub(crate) fn columns(&self) -> impl Iterator<Item = SourceColumn> + Clone + '_ {
        let demand = self.aggregate_demand(0);
        let count = if !self.aggregates.is_empty() {
            usize::from(self.source_count)
        } else {
            usize::from(self.output_count)
        };
        (0..count).filter_map(move |index| {
            if let Some(aggregate) = self.aggregates.first() {
                let column = self.source_columns[index];
                aggregate.needs(column, demand).then_some(column)
            } else {
                source_column(
                    self.outputs[index].id,
                    &self.source_columns[..usize::from(self.source_count)],
                )
            }
        })
    }

    #[cfg(test)]
    pub(crate) fn source_columns(&self) -> impl Iterator<Item = SourceColumn> + '_ {
        self.source_columns[..usize::from(self.source_count)]
            .iter()
            .copied()
    }
}

struct ColumnFacts<'a> {
    sources: &'a [SourceColumn],
    aggregates: &'a [AggregatePlan],
    computed: &'a [Computed],
    distinct: &'a [DistinctPlan],
    unions: &'a [UnionPlan],
    null_extensions: &'a [NullExtension],
}

impl ColumnFacts<'_> {
    fn column(&self, id: ColumnId) -> Option<SemanticColumn> {
        let (kind, nullable) = column_type(
            id,
            self.sources,
            self.aggregates,
            self.computed,
            self.distinct,
            self.unions,
            self.null_extensions,
        )?;
        Some(SemanticColumn::new(id.value(), kind, nullable))
    }
}

fn source_column(id: ColumnId, sources: &[SourceColumn]) -> Option<SourceColumn> {
    sources.iter().find(|column| column.identity == id).copied()
}

fn column_type(
    id: ColumnId,
    sources: &[SourceColumn],
    aggregates: &[AggregatePlan],
    computed: &[Computed],
    distinct: &[DistinctPlan],
    unions: &[UnionPlan],
    null_extensions: &[NullExtension],
) -> Option<(DataType, bool)> {
    if let Some(column) = null_extensions
        .iter()
        .find_map(|extension| extension.column(id))
    {
        Some((column.data_type(), true))
    } else if let Some(column) = unions.iter().find_map(|union| union.column(id)) {
        Some((column.data_type(), column.nullable()))
    } else if let Some(column) = distinct.iter().find_map(|stage| stage.input_for(id)) {
        Some((column.data_type(), column.nullable()))
    } else if let Some(column) = source_column(id, sources) {
        Some((column.data_type(), column.nullable()))
    } else if let Some(definition) = computed.iter().find(|entry| entry.column.identity() == id) {
        Some((definition.column.data_type(), definition.column.nullable()))
    } else {
        aggregates
            .iter()
            .find_map(|aggregate| aggregate.column_type(id))
    }
}

fn initial_outputs() -> ([Output; MAX_COLUMNS], [SourceColumn; MAX_COLUMNS], u8) {
    let mut outputs = [Output::EMPTY; MAX_COLUMNS];
    let mut sources = [SourceColumn::QUANTITY; MAX_COLUMNS];
    for (index, (column, name)) in CATALOG.into_iter().enumerate() {
        outputs[index] = Output {
            id: column.identity,
            name: Name::new(name),
        };
        sources[index] = column;
    }
    (outputs, sources, 7)
}

fn span(start: usize, end: usize) -> SourceSpan {
    SourceSpan {
        start: u16::try_from(start).expect("source admission bounds span"),
        end: u16::try_from(end).expect("source admission bounds span"),
    }
}

fn parse(message: &'static str, start: usize, end: usize) -> Error {
    Error::Parse {
        message,
        span: span(start, end),
    }
}

fn bind_error(message: &'static str, span: SourceSpan) -> Error {
    Error::Bind { message, span }
}

fn text(source: &str, span: SourceSpan) -> &str {
    &source[span.start()..span.end()]
}

#[cfg(test)]
mod streaming_tests;
