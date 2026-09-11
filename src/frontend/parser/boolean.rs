//! Bounded WHERE syntax and forward decisions. Names and types stay in binding.
use super::{
    Comparison, Error, FilterControl, Kind, MAX_TOKENS, Parsed, ParsedStage, Parser, SourceSpan,
};

#[derive(Clone, Copy, PartialEq)]
enum Operator {
    Left,
    Or,
    And,
    Not,
    Leaf,
}

impl Operator {
    fn precedence(self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Or => 1,
            Self::And => 2,
            Self::Not => 3,
            Self::Leaf => 4,
        }
    }
}

#[derive(Clone, Copy)]
struct Node {
    kind: Operator,
    left: u8,
    right: u8,
    first: u8,
}

impl Node {
    const EMPTY: Self = Self {
        kind: Operator::Leaf,
        left: 0,
        right: 0,
        first: 0,
    };
}

struct Syntax {
    nodes: [Node; MAX_TOKENS],
    count: usize,
    values: [u8; MAX_TOKENS],
    value_count: usize,
    operators: [Operator; MAX_TOKENS],
    operator_count: usize,
}

impl Syntax {
    fn new() -> Self {
        Self {
            nodes: [Node::EMPTY; MAX_TOKENS],
            count: 0,
            values: [0; MAX_TOKENS],
            value_count: 0,
            operators: [Operator::Left; MAX_TOKENS],
            operator_count: 0,
        }
    }

    fn push(&mut self, node: Node) -> Result<(), Error> {
        if self.count == MAX_TOKENS || self.value_count == MAX_TOKENS {
            return Err(Error::Corrupt("Boolean nodes exceed lexical bound"));
        }
        self.nodes[self.count] = node;
        self.values[self.value_count] = self.count as u8;
        self.count += 1;
        self.value_count += 1;
        Ok(())
    }

    fn leaf(&mut self, stage: u8) -> Result<(), Error> {
        self.push(Node {
            first: stage,
            ..Node::EMPTY
        })
    }

    fn operator(&mut self, op: Operator) -> Result<(), Error> {
        if self.operator_count == MAX_TOKENS {
            return Err(Error::Corrupt("Boolean operators exceed lexical bound"));
        }
        self.operators[self.operator_count] = op;
        self.operator_count += 1;
        Ok(())
    }

    fn reduce(&mut self, span: SourceSpan) -> Result<(), Error> {
        self.operator_count -= 1;
        let kind = self.operators[self.operator_count];
        let needed = if kind == Operator::Not { 1 } else { 2 };
        if kind == Operator::Left || self.value_count < needed {
            return Err(Error::Parse {
                message: "incomplete Boolean expression",
                span,
            });
        }
        let left = self.values[self.value_count - needed];
        let right = self.values[self.value_count - 1];
        self.value_count -= needed;
        self.push(Node {
            kind,
            left,
            right,
            first: self.nodes[usize::from(left)].first,
        })
    }

    fn finish(&mut self, parsed: &mut Parsed, span: SourceSpan) -> Result<(), Error> {
        while self.operator_count != 0 {
            self.reduce(span)?;
        }
        if self.value_count != 1 {
            return Err(Error::Parse {
                message: "incomplete Boolean expression",
                span,
            });
        }
        // A syntax node has exactly one parent. Reverse topological order assigns
        // requested truth and branch destinations before visiting either child.
        let mut controls = [FilterControl::LINEAR; MAX_TOKENS];
        controls[usize::from(self.values[0])] = FilterControl {
            matched: parsed.len,
            other: u8::MAX,
            end: 0,
            negated: false,
        };
        for index in (0..self.count).rev() {
            let node = self.nodes[index];
            let control = controls[index];
            match node.kind {
                Operator::Leaf => {
                    let relative = |target: u8| -> Result<u8, Error> {
                        if target == u8::MAX {
                            return Ok(0);
                        }
                        target
                            .checked_sub(node.first)
                            .filter(|offset| *offset != 0)
                            .ok_or(Error::Corrupt("Boolean decision is not forward"))
                    };
                    parsed.controls[usize::from(node.first)] = FilterControl {
                        matched: relative(control.matched)?,
                        other: relative(control.other)?,
                        end: parsed.len - node.first,
                        negated: control.negated,
                    };
                }
                Operator::Not => {
                    controls[usize::from(node.left)] = FilterControl {
                        negated: !control.negated,
                        ..control
                    }
                }
                Operator::And | Operator::Or => {
                    let right = self.nodes[usize::from(node.right)].first;
                    let both = (node.kind == Operator::And) != control.negated;
                    controls[usize::from(node.right)] = control;
                    controls[usize::from(node.left)] = FilterControl {
                        matched: if both { right } else { control.matched },
                        other: if both { control.other } else { right },
                        ..control
                    };
                }
                Operator::Left => return Err(Error::Corrupt("parenthesis in Boolean tree")),
            }
        }
        Ok(())
    }
}

impl Parser<'_> {
    pub(super) fn boolean_filter(
        &mut self,
        parsed: &mut Parsed,
        span: SourceSpan,
    ) -> Result<(), Error> {
        let mut syntax = Syntax::new();
        let mut operand = true;
        let mut depth = 0;
        // Every iteration consumes at least one token, unless it terminates.
        for _ in 0..=MAX_TOKENS {
            if operand {
                if self.is_word("NOT") {
                    self.word("NOT")?;
                    syntax.operator(Operator::Not)?;
                    continue;
                }
                if self.peek() == Kind::LeftParen {
                    self.take(Kind::LeftParen)?;
                    syntax.operator(Operator::Left)?;
                    depth += 1;
                    continue;
                }
                self.boolean_leaf(parsed, &mut syntax, span)?;
                operand = false;
            } else if self.peek() == Kind::RightParen && depth != 0 {
                while syntax.operator_count != 0
                    && syntax.operators[syntax.operator_count - 1] != Operator::Left
                {
                    syntax.reduce(span)?;
                }
                if syntax.operator_count == 0 {
                    return Err(Error::Corrupt("Boolean parenthesis stack"));
                }
                syntax.operator_count -= 1;
                depth -= 1;
                self.take(Kind::RightParen)?;
            } else {
                let op = if self.is_word("AND") {
                    Operator::And
                } else if self.is_word("OR") {
                    Operator::Or
                } else {
                    return syntax.finish(parsed, span);
                };
                while syntax.operator_count != 0
                    && syntax.operators[syntax.operator_count - 1].precedence() >= op.precedence()
                {
                    syntax.reduce(span)?;
                }
                self.word(if op == Operator::And { "AND" } else { "OR" })?;
                syntax.operator(op)?;
                operand = true;
            }
        }
        Err(Error::Corrupt("Boolean parser exceeded token bound"))
    }

    fn boolean_leaf(
        &mut self,
        parsed: &mut Parsed,
        syntax: &mut Syntax,
        span: SourceSpan,
    ) -> Result<(), Error> {
        let column = self.column()?;
        let first = parsed.len;
        if self.is_word("IS") {
            self.word("IS")?;
            let negated = self.is_word("NOT");
            if negated {
                self.word("NOT")?;
            }
            self.word("NULL")?;
            parsed.push_stage(ParsedStage::WhereNull { column, negated }, span)?;
            syntax.leaf(first)
        } else if self.is_word("BETWEEN") {
            self.word("BETWEEN")?;
            let lower = self.comparison_literal(parsed)?;
            self.word("AND")?;
            let upper = self.comparison_literal(parsed)?;
            parsed.push_stage(
                ParsedStage::Where {
                    column,
                    comparison: Comparison::GreaterEqual,
                    literal: lower,
                },
                span,
            )?;
            parsed.push_stage(
                ParsedStage::Where {
                    column,
                    comparison: Comparison::LessEqual,
                    literal: upper,
                },
                span,
            )?;
            syntax.leaf(first)?;
            syntax.leaf(first + 1)?;
            syntax.operator(Operator::And)?;
            syntax.reduce(span)
        } else {
            let Kind::Compare(comparison) = self.peek() else {
                return Err(Error::Parse {
                    message: "comparison required",
                    span: column,
                });
            };
            self.take(Kind::Compare(comparison))?;
            let literal = self.comparison_literal(parsed)?;
            parsed.push_stage(
                ParsedStage::Where {
                    column,
                    comparison,
                    literal,
                },
                span,
            )?;
            syntax.leaf(first)
        }
    }
}
