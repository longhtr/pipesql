//! Bounded syntax and source spans. Parsing does not resolve names or read a database.
use crate::scalar::MAX_OPS;

mod boolean;
use super::lexer::{Kind, Tokens, ZERO_SPAN, lex};
use super::{
    AggregateKind, Comparison, Direction, Error, FilterControl, MAX_AGGREGATE_COLUMNS, MAX_COLUMNS,
    MAX_ORDER_ITEMS, MAX_PROJECTIONS, MAX_STAGES, MAX_TOKENS, NullPlacement, SourceSpan,
    bind_error, parse, span, text,
};

#[derive(Clone, Copy)]
pub(super) enum ParsedStage {
    Empty,
    Distinct(SourceSpan),
    Limit {
        count: ParsedRange,
        offset: Option<ParsedRange>,
        span: SourceSpan,
    },
    Source(u8),
    Alias(SourceSpan),
    Derived(SourceSpan),
    Join {
        left: SourceSpan,
        right: SourceSpan,
    },
    Aggregate(ParsedAggregateRange),
    Order {
        start: u8,
        len: u8,
    },
    Select {
        start: u8,
        len: u8,
    },
    Where {
        column: SourceSpan,
        comparison: Comparison,
        literal: ParsedLiteral,
    },
    WhereNull {
        column: SourceSpan,
        negated: bool,
    },
}
const MAX_DATE_SHIFTS: usize = 8;

#[derive(Clone, Copy)]
pub(super) struct ParsedDateShift {
    pub(super) subtract: bool,
    pub(super) negative: bool,
    pub(super) amount: SourceSpan,
    pub(super) part: SourceSpan,
}

#[derive(Clone, Copy)]
pub(super) enum ParsedLiteral {
    String(SourceSpan),
    Numeric(ParsedRange),
    Date {
        span: SourceSpan,
        shifts: [ParsedDateShift; MAX_DATE_SHIFTS],
        count: u8,
    },
}

#[derive(Clone, Copy)]
pub(super) enum ParsedOp {
    Empty,
    Column(SourceSpan),
    Number(SourceSpan),
    NegativeNumber(SourceSpan),
    Add,
    Subtract,
    Multiply,
    Negate,
}

#[derive(Clone, Copy)]
pub(super) struct ParsedExpression {
    pub(super) span: SourceSpan,
    pub(super) ops: [ParsedOp; MAX_OPS],
    pub(super) len: u8,
}

impl ParsedExpression {
    const EMPTY: Self = Self {
        ops: [ParsedOp::Empty; MAX_OPS],
        span: ZERO_SPAN,
        len: 0,
    };

    fn push(&mut self, op: ParsedOp, at: SourceSpan) -> Result<(), Error> {
        if usize::from(self.len) == MAX_OPS {
            return Err(Error::Parse {
                message: "scalar operation limit exceeded",
                span: at,
            });
        }
        self.ops[usize::from(self.len)] = op;
        self.len += 1;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum PendingOp {
    Paren,
    Unary,
    Binary(Kind),
}

impl PendingOp {
    fn precedence(self) -> u8 {
        match self {
            Self::Paren => 0,
            Self::Binary(Kind::Star) => 2,
            Self::Binary(_) => 1,
            Self::Unary => 3,
        }
    }

    fn parsed(self) -> ParsedOp {
        match self {
            Self::Unary => ParsedOp::Negate,
            Self::Binary(Kind::Plus) => ParsedOp::Add,
            Self::Binary(Kind::Minus) => ParsedOp::Subtract,
            Self::Binary(Kind::Star) => ParsedOp::Multiply,
            _ => unreachable!("pending scalar operator"),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ParsedAggregateEntry {
    pub(super) kind: AggregateKind,
    pub(super) argument: Option<ParsedRange>,
    pub(super) alias: SourceSpan,
    pub(super) span: SourceSpan,
}

impl ParsedAggregateEntry {
    const EMPTY: Self = Self {
        kind: AggregateKind::Count,
        argument: None,
        alias: ZERO_SPAN,
        span: ZERO_SPAN,
    };
}

#[derive(Clone, Copy)]
struct ParsedAggregate {
    entries: [ParsedAggregateEntry; MAX_AGGREGATE_COLUMNS],
    count: u8,
    groups: [SourceSpan; MAX_AGGREGATE_COLUMNS - 1],
    group_count: u8,
    ordered: bool,
}

#[derive(Clone, Copy)]
pub(super) struct ParsedAggregateRange {
    pub(super) entry_start: u8,
    pub(super) count: u8,
    pub(super) group_start: u8,
    pub(super) group_count: u8,
    pub(super) ordered: bool,
}

#[derive(Clone, Copy)]
pub(super) struct ParsedSource {
    pub(super) table: SourceSpan,
    pub(super) alias: SourceSpan,
}

impl ParsedSource {
    const EMPTY: Self = Self {
        table: ZERO_SPAN,
        alias: ZERO_SPAN,
    };
}

#[derive(Clone, Copy)]
pub(super) struct ParsedOrder {
    pub(super) reference: SourceSpan,
    pub(super) ordinal: bool,
    pub(super) direction: Direction,
    pub(super) nulls: NullPlacement,
}

impl ParsedOrder {
    const EMPTY: Self = Self {
        reference: ZERO_SPAN,
        ordinal: false,
        direction: Direction::Ascending,
        nulls: NullPlacement::First,
    };
}

#[derive(Clone, Copy)]
pub(super) struct ParsedRange {
    pub(super) span: SourceSpan,
    pub(super) start: u8,
    pub(super) len: u8,
}

#[derive(Clone, Copy)]
pub(super) struct ParsedProjection {
    pub(super) expression: ParsedRange,
    pub(super) alias: SourceSpan,
    pub(super) direct: bool,
}

impl ParsedProjection {
    const EMPTY: Self = Self {
        expression: ParsedRange {
            span: ZERO_SPAN,
            start: 0,
            len: 0,
        },
        alias: ZERO_SPAN,
        direct: false,
    };
}

pub(super) struct Parsed {
    // Numeric programs share the token bound: each operation consumes at least
    // one distinct token. Stage/aggregate entries retain ranges, not full stacks.
    pub(super) numeric_ops: [ParsedOp; MAX_TOKENS],
    pub(super) numeric_op_count: u8,
    pub(super) table: SourceSpan,
    pub(super) sources: [ParsedSource; MAX_STAGES],
    pub(super) source_count: u8,
    pub(super) stages: [ParsedStage; MAX_STAGES],
    pub(super) controls: [FilterControl; MAX_STAGES],
    pub(super) len: u8,
    pub(super) projections: [ParsedProjection; MAX_PROJECTIONS],
    pub(super) projection_count: u8,
    pub(super) order_items: [ParsedOrder; MAX_ORDER_ITEMS],
    pub(super) order_count: u8,
    pub(super) aggregate_entries: [ParsedAggregateEntry; MAX_AGGREGATE_COLUMNS],
    pub(super) aggregate_entry_count: u8,
    pub(super) aggregate_groups: [SourceSpan; MAX_AGGREGATE_COLUMNS - 1],
    pub(super) aggregate_group_count: u8,
}

impl Parsed {
    fn push_expression(&mut self, expression: ParsedExpression) -> Result<ParsedRange, Error> {
        let start = usize::from(self.numeric_op_count);
        let len = usize::from(expression.len);
        let end = start
            .checked_add(len)
            .filter(|end| *end <= MAX_TOKENS)
            .ok_or(Error::Corrupt("numeric operations exceed lexical bound"))?;
        self.numeric_ops[start..end].copy_from_slice(&expression.ops[..len]);
        self.numeric_op_count =
            u8::try_from(end).map_err(|_| Error::Corrupt("numeric operation count"))?;
        Ok(ParsedRange {
            span: expression.span,
            start: start as u8,
            len: expression.len,
        })
    }

    pub(super) fn expression(&self, range: ParsedRange) -> Result<ParsedExpression, Error> {
        let start = usize::from(range.start);
        let len = usize::from(range.len);
        let end = start + len;
        if len == 0 || len > MAX_OPS || end > usize::from(self.numeric_op_count) {
            return Err(Error::Corrupt("numeric expression range"));
        }
        let mut expression = ParsedExpression::EMPTY;
        expression.len = range.len;
        expression.span = range.span;
        expression.ops[..len].copy_from_slice(&self.numeric_ops[start..end]);
        Ok(expression)
    }

    fn push_stage(&mut self, stage: ParsedStage, span: SourceSpan) -> Result<(), Error> {
        if usize::from(self.len) == MAX_STAGES {
            return Err(Error::Parse {
                message: "normalized stage limit exceeded",
                span,
            });
        }
        self.stages[usize::from(self.len)] = stage;
        self.len += 1;
        Ok(())
    }
}

struct Parser<'a> {
    source: &'a str,
    tokens: &'a Tokens,
    position: usize,
    end: usize,
}

impl Parser<'_> {
    fn column(&mut self) -> Result<SourceSpan, Error> {
        let mut reference = self.take(Kind::Identifier)?;
        if self.peek() == Kind::Dot {
            self.take(Kind::Dot)?;
            reference.end = self.take(Kind::Identifier)?.end;
        }
        Ok(reference)
    }

    fn source_alias(&mut self, table: SourceSpan) -> Result<SourceSpan, Error> {
        if self.peek() == Kind::As {
            self.take(Kind::As)?;
            self.take(Kind::Identifier)
        } else {
            Ok(table)
        }
    }

    fn peek(&self) -> Kind {
        if self.position == self.tokens.len {
            Kind::Empty
        } else {
            self.tokens.values[self.position].kind
        }
    }

    fn take(&mut self, kind: Kind) -> Result<SourceSpan, Error> {
        if self.peek() != kind && !(kind == Kind::Identifier && self.peek() == Kind::Aggregate) {
            let span = if self.position == self.tokens.len {
                span(self.end, self.end)
            } else {
                self.tokens.values[self.position].span
            };
            return Err(Error::Parse {
                message: "unexpected token",
                span,
            });
        }
        let span = self.tokens.values[self.position].span;
        self.position += 1;
        Ok(span)
    }

    fn is_word(&self, word: &str) -> bool {
        self.position < self.tokens.len
            && text(self.source, self.tokens.values[self.position].span).eq_ignore_ascii_case(word)
    }

    fn word(&mut self, word: &str) -> Result<SourceSpan, Error> {
        if !self.is_word(word) {
            return Err(parse("required keyword missing", self.end, self.end));
        }
        self.take(self.peek())
    }

    fn comparison_literal(&mut self, parsed: &mut Parsed) -> Result<ParsedLiteral, Error> {
        if self.peek() == Kind::Quoted {
            return Ok(ParsedLiteral::String(self.take(Kind::Quoted)?));
        }
        if self.is_word("DATE") || self.is_word("DATE_ADD") || self.is_word("DATE_SUB") {
            let mut operators = [false; MAX_DATE_SHIFTS];
            let mut count = 0;
            while self.is_word("DATE_ADD") || self.is_word("DATE_SUB") {
                if count == MAX_DATE_SHIFTS {
                    return Err(parse("DATE nesting limit exceeded", self.end, self.end));
                }
                operators[count] = self.is_word("DATE_SUB");
                count += 1;
                self.take(Kind::Identifier)?;
                self.take(Kind::LeftParen)?;
            }
            self.word("DATE")?;
            let span = self.take(Kind::Quoted)?;
            let mut shifts = [ParsedDateShift {
                subtract: false,
                negative: false,
                amount: ZERO_SPAN,
                part: ZERO_SPAN,
            }; MAX_DATE_SHIFTS];
            for (index, subtract) in operators[..count].iter().rev().enumerate() {
                self.take(Kind::Comma)?;
                self.word("INTERVAL")?;
                let negative = self.peek() == Kind::Minus;
                if negative || self.peek() == Kind::Plus {
                    self.take(self.peek())?;
                }
                let amount = self.take(Kind::Number)?;
                let part = self.take(Kind::Identifier)?;
                self.take(Kind::RightParen)?;
                shifts[index] = ParsedDateShift {
                    subtract: *subtract,
                    negative,
                    amount,
                    part,
                };
            }
            return Ok(ParsedLiteral::Date {
                span,
                shifts,
                count: u8::try_from(count).expect("bounded DATE shifts"),
            });
        }
        Ok(ParsedLiteral::Numeric(
            parsed.push_expression(self.numeric_expression()?)?,
        ))
    }

    fn numeric_expression(&mut self) -> Result<ParsedExpression, Error> {
        let mut expression = ParsedExpression::EMPTY;
        let mut pending = [PendingOp::Paren; MAX_OPS];
        let mut depth = 0;
        let mut parentheses = 0;
        let mut operand = true;
        let at = self
            .tokens
            .values
            .get(self.position)
            .filter(|_| self.position < self.tokens.len)
            .map_or(span(self.end, self.end), |token| token.span);
        loop {
            let kind = self.peek();
            if operand {
                match kind {
                    Kind::Number => {
                        let span = self.take(kind)?;
                        expression.push(ParsedOp::Number(span), span)?;
                        operand = false;
                    }
                    Kind::Identifier | Kind::Aggregate => {
                        let span = self.column()?;
                        expression.push(ParsedOp::Column(span), span)?;
                        operand = false;
                    }
                    Kind::Plus => {
                        self.take(kind)?;
                    }
                    Kind::Minus
                        if self
                            .tokens
                            .values
                            .get(self.position + 1)
                            .is_some_and(|token| token.kind == Kind::Number) =>
                    {
                        self.take(Kind::Minus)?;
                        let span = self.take(Kind::Number)?;
                        expression.push(ParsedOp::NegativeNumber(span), span)?;
                        operand = false;
                    }
                    Kind::Minus | Kind::LeftParen => {
                        if depth == MAX_OPS {
                            return Err(Error::Parse {
                                message: "scalar stack limit exceeded",
                                span: at,
                            });
                        }
                        self.take(kind)?;
                        pending[depth] = if kind == Kind::Minus {
                            PendingOp::Unary
                        } else {
                            parentheses += 1;
                            PendingOp::Paren
                        };
                        depth += 1;
                    }
                    _ => {
                        return Err(Error::Parse {
                            message: "numeric operand required",
                            span: at,
                        });
                    }
                }
                continue;
            }
            match kind {
                Kind::Plus | Kind::Minus | Kind::Star => {
                    let operator = PendingOp::Binary(kind);
                    while depth != 0 && pending[depth - 1].precedence() >= operator.precedence() {
                        depth -= 1;
                        expression.push(pending[depth].parsed(), at)?;
                    }
                    if depth == MAX_OPS {
                        return Err(Error::Parse {
                            message: "scalar stack limit exceeded",
                            span: at,
                        });
                    }
                    pending[depth] = operator;
                    depth += 1;
                    self.take(kind)?;
                    operand = true;
                }
                Kind::RightParen if parentheses != 0 => {
                    while depth != 0 && !matches!(pending[depth - 1], PendingOp::Paren) {
                        depth -= 1;
                        expression.push(pending[depth].parsed(), at)?;
                    }
                    assert!(depth != 0);
                    depth -= 1;
                    parentheses -= 1;
                    self.take(kind)?;
                }
                _ => break,
            }
        }
        if parentheses != 0 {
            return Err(Error::Parse {
                message: "unclosed scalar parenthesis",
                span: at,
            });
        }
        while depth != 0 {
            depth -= 1;
            expression.push(pending[depth].parsed(), at)?;
        }
        expression.span = SourceSpan {
            start: at.start,
            end: self.tokens.values[self.position - 1].span.end,
        };
        Ok(expression)
    }

    fn aggregate(&mut self, parsed: &mut Parsed) -> Result<ParsedAggregate, Error> {
        let span = self.take(Kind::Aggregate)?;
        let mut aggregate = ParsedAggregate {
            entries: [ParsedAggregateEntry::EMPTY; MAX_AGGREGATE_COLUMNS],
            count: 0,
            groups: [ZERO_SPAN; MAX_AGGREGATE_COLUMNS - 1],
            group_count: 0,
            ordered: false,
        };
        loop {
            if usize::from(aggregate.count) == MAX_AGGREGATE_COLUMNS {
                return Err(Error::Parse {
                    message: "aggregate output limit exceeded",
                    span,
                });
            }
            let name = self.take(Kind::Identifier)?;
            let kind = if text(self.source, name).eq_ignore_ascii_case("SUM") {
                AggregateKind::Sum
            } else if text(self.source, name).eq_ignore_ascii_case("AVG") {
                AggregateKind::Avg
            } else if text(self.source, name).eq_ignore_ascii_case("COUNT") {
                AggregateKind::Count
            } else {
                return Err(Error::Parse {
                    message: "unsupported aggregate function",
                    span: name,
                });
            };
            self.take(Kind::LeftParen)?;
            let column = if kind == AggregateKind::Count {
                self.take(Kind::Star)?;
                None
            } else {
                Some(parsed.push_expression(self.numeric_expression()?)?)
            };
            let end = self.take(Kind::RightParen)?.end;
            self.take(Kind::As)?;
            let alias = self.take(Kind::Identifier)?;
            aggregate.entries[usize::from(aggregate.count)] = ParsedAggregateEntry {
                kind,
                argument: column,
                alias,
                span: SourceSpan {
                    start: name.start,
                    end,
                },
            };
            aggregate.count += 1;
            if self.peek() != Kind::Comma {
                break;
            }
            self.take(Kind::Comma)?;
        }
        if self.is_word("GROUP") {
            self.word("GROUP")?;
            if self.is_word("AND") {
                self.word("AND")?;
                self.word("ORDER")?;
                aggregate.ordered = true;
            }
            self.word("BY")?;
            loop {
                if usize::from(aggregate.group_count) == aggregate.groups.len() {
                    return Err(Error::Parse {
                        message: "at most nine grouping keys",
                        span,
                    });
                }
                aggregate.groups[usize::from(aggregate.group_count)] = self.column()?;
                aggregate.group_count += 1;
                if self.peek() != Kind::Comma {
                    break;
                }
                self.take(Kind::Comma)?;
            }
        }
        Ok(aggregate)
    }

    fn join_condition(&mut self, parsed: &mut Parsed, span: SourceSpan) -> Result<(), Error> {
        self.word("ON")?;
        let left = self.column()?;
        self.take(Kind::Compare(Comparison::Equal))?;
        let right = self.column()?;
        parsed.push_stage(ParsedStage::Join { left, right }, span)
    }

    fn query(&mut self) -> Result<Parsed, Error> {
        self.take(Kind::From)?;
        let mut parsed = Parsed {
            numeric_ops: [ParsedOp::Empty; MAX_TOKENS],
            numeric_op_count: 0,
            table: ZERO_SPAN,
            sources: [ParsedSource::EMPTY; MAX_STAGES],
            source_count: 0,
            stages: [ParsedStage::Empty; MAX_STAGES],
            controls: [FilterControl::LINEAR; MAX_STAGES],
            len: 0,
            projections: [ParsedProjection::EMPTY; MAX_PROJECTIONS],
            projection_count: 0,
            order_items: [ParsedOrder::EMPTY; MAX_ORDER_ITEMS],
            order_count: 0,
            aggregate_entries: [ParsedAggregateEntry::EMPTY; MAX_AGGREGATE_COLUMNS],
            aggregate_entry_count: 0,
            aggregate_groups: [ZERO_SPAN; MAX_AGGREGATE_COLUMNS - 1],
            aggregate_group_count: 0,
        };
        // Each child frame remembers whether its result completes a JOIN input.
        // All syntax shares one token stream and the original source spans.
        let mut frames = [None; MAX_STAGES];
        let mut depth = 0;
        let mut need_source = true;
        let mut pending_join = None;
        loop {
            if need_source {
                if self.peek() == Kind::LeftParen {
                    let open = self.take(Kind::LeftParen)?;
                    if depth == frames.len() {
                        return Err(Error::Parse {
                            message: "derived input nesting limit exceeded",
                            span: open,
                        });
                    }
                    frames[depth] = pending_join.take();
                    depth += 1;
                    self.take(Kind::From)?;
                    continue;
                }
                let table = self.take(Kind::Identifier)?;
                let alias = self.source_alias(table)?;
                let index = usize::from(parsed.source_count);
                if index == parsed.sources.len() {
                    return Err(bind_error("source occurrence limit exceeded", table));
                }
                parsed.sources[index] = ParsedSource { table, alias };
                parsed.source_count += 1;
                if index == 0 {
                    parsed.table = table;
                } else {
                    parsed.push_stage(ParsedStage::Source(index as u8), table)?;
                }
                need_source = false;
                if let Some(pipe) = pending_join.take() {
                    self.join_condition(&mut parsed, pipe)?;
                }
                continue;
            }
            if self.peek() == Kind::RightParen && depth != 0 {
                let close = self.take(Kind::RightParen)?;
                depth -= 1;
                let alias = self.source_alias(ZERO_SPAN)?;
                parsed.push_stage(ParsedStage::Derived(alias), close)?;
                if let Some(pipe) = frames[depth].take() {
                    self.join_condition(&mut parsed, pipe)?;
                }
                continue;
            }
            if self.peek() != Kind::Pipe {
                break;
            }
            let pipe = self.take(Kind::Pipe)?;
            if usize::from(parsed.len) == MAX_STAGES {
                return Err(Error::Parse {
                    message: "stage limit exceeded",
                    span: pipe,
                });
            }
            let stage = match self.peek() {
                Kind::Distinct => ParsedStage::Distinct(self.take(Kind::Distinct)?),
                Kind::As => {
                    self.take(Kind::As)?;
                    ParsedStage::Alias(self.take(Kind::Identifier)?)
                }
                Kind::Join => {
                    self.take(Kind::Join)?;
                    pending_join = Some(pipe);
                    need_source = true;
                    continue;
                }
                Kind::Limit => {
                    let span = self.take(Kind::Limit)?;
                    let count = parsed.push_expression(self.numeric_expression()?)?;
                    let offset = if self.is_word("OFFSET") {
                        self.word("OFFSET")?;
                        Some(parsed.push_expression(self.numeric_expression()?)?)
                    } else {
                        None
                    };
                    ParsedStage::Limit {
                        count,
                        offset,
                        span,
                    }
                }
                Kind::Order => {
                    self.take(Kind::Order)?;
                    self.word("BY")?;
                    let start = parsed.order_count;
                    loop {
                        if usize::from(parsed.order_count) == MAX_ORDER_ITEMS {
                            return Err(bind_error("order item limit exceeded", pipe));
                        }
                        let ordinal = self.peek() == Kind::Number;
                        let reference = if ordinal {
                            self.take(Kind::Number)?
                        } else {
                            self.column()?
                        };
                        let direction = if self.is_word("DESC") {
                            self.word("DESC")?;
                            Direction::Descending
                        } else {
                            if self.is_word("ASC") {
                                self.word("ASC")?;
                            }
                            Direction::Ascending
                        };
                        let nulls = if self.is_word("NULLS") {
                            self.word("NULLS")?;
                            if self.is_word("FIRST") {
                                self.word("FIRST")?;
                                NullPlacement::First
                            } else {
                                self.word("LAST")?;
                                NullPlacement::Last
                            }
                        } else if direction == Direction::Ascending {
                            NullPlacement::First
                        } else {
                            NullPlacement::Last
                        };
                        parsed.order_items[usize::from(parsed.order_count)] = ParsedOrder {
                            reference,
                            ordinal,
                            direction,
                            nulls,
                        };
                        parsed.order_count += 1;
                        if self.peek() != Kind::Comma {
                            break;
                        }
                        self.take(Kind::Comma)?;
                    }
                    ParsedStage::Order {
                        start,
                        len: parsed.order_count - start,
                    }
                }
                Kind::Aggregate => {
                    let aggregate = self.aggregate(&mut parsed)?;
                    let range = ParsedAggregateRange {
                        entry_start: parsed.aggregate_entry_count,
                        count: aggregate.count,
                        group_start: parsed.aggregate_group_count,
                        group_count: aggregate.group_count,
                        ordered: aggregate.ordered,
                    };
                    let entry_end = usize::from(range.entry_start) + usize::from(range.count);
                    let group_end = usize::from(range.group_start) + usize::from(range.group_count);
                    if entry_end > parsed.aggregate_entries.len()
                        || group_end > parsed.aggregate_groups.len()
                        || entry_end + group_end > MAX_AGGREGATE_COLUMNS
                    {
                        return Err(bind_error("aggregate output limit exceeded", pipe));
                    }
                    parsed.aggregate_entries[usize::from(range.entry_start)..entry_end]
                        .copy_from_slice(&aggregate.entries[..usize::from(range.count)]);
                    parsed.aggregate_groups[usize::from(range.group_start)..group_end]
                        .copy_from_slice(&aggregate.groups[..usize::from(range.group_count)]);
                    parsed.aggregate_entry_count = entry_end as u8;
                    parsed.aggregate_group_count = group_end as u8;
                    ParsedStage::Aggregate(range)
                }
                Kind::Select => {
                    self.take(Kind::Select)?;
                    let start = parsed.projection_count;
                    let mut len = 0;
                    loop {
                        if len == MAX_COLUMNS {
                            return Err(Error::Parse {
                                message: "output limit exceeded",
                                span: pipe,
                            });
                        }
                        let first = self.position;
                        let expression = self.numeric_expression()?;
                        // Parentheses retain an AST path reference; unary plus
                        // retains its numeric value but has no implicit name.
                        let direct = expression.len == 1
                            && matches!(expression.ops[0], ParsedOp::Column(_))
                            && self.tokens.values[first..self.position]
                                .iter()
                                .all(|token| {
                                    matches!(
                                        token.kind,
                                        Kind::Identifier
                                            | Kind::Dot
                                            | Kind::LeftParen
                                            | Kind::RightParen
                                    )
                                });
                        let expression = parsed.push_expression(expression)?;
                        let alias = if self.peek() == Kind::As {
                            self.take(Kind::As)?;
                            self.take(Kind::Identifier)?
                        } else {
                            ZERO_SPAN
                        };
                        let index = usize::from(parsed.projection_count);
                        if index == MAX_PROJECTIONS {
                            return Err(Error::Parse {
                                message: "projection entry limit exceeded",
                                span: pipe,
                            });
                        }
                        parsed.projections[index] = ParsedProjection {
                            expression,
                            alias,
                            direct,
                        };
                        parsed.projection_count += 1;
                        len += 1;
                        if self.peek() != Kind::Comma {
                            break;
                        }
                        self.take(Kind::Comma)?;
                    }
                    ParsedStage::Select {
                        start,
                        len: u8::try_from(len).expect("bounded columns"),
                    }
                }
                Kind::Where => {
                    self.take(Kind::Where)?;
                    self.boolean_filter(&mut parsed, pipe)?;
                    continue;
                }
                _ => {
                    return Err(Error::Parse {
                        message: "unsupported pipe operator",
                        span: pipe,
                    });
                }
            };
            parsed.push_stage(stage, pipe)?;
        }
        if depth != 0 || need_source {
            return Err(parse("unfinished derived input", self.end, self.end));
        }
        if self.peek() == Kind::Semicolon {
            self.take(Kind::Semicolon)?;
        }
        if self.position != self.tokens.len {
            return Err(parse("unexpected trailing syntax", self.end, self.end));
        }
        Ok(parsed)
    }
}

pub(super) fn parse_query(source: &str) -> Result<Parsed, Error> {
    let tokens = lex(source)?;
    Parser {
        source,
        tokens: &tokens,
        position: 0,
        end: source.len(),
    }
    .query()
}
