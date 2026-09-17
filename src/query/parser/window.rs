//! Parse bounded analytic COUNT and ordered SUM projections.
//!
//! A header identifies the operation, followed by partition columns and ordered
//! columns in the existing expression array. SUM requires ordering; its implicit
//! frame includes every peer with the current order key. Explicit frames and
//! expressions inside the argument or key lists are outside this profile.

use super::{Direction, Error, Kind, NullPlacement, ParsedExpression, ParsedOp, Parser, span};
use crate::query::{MAX_PARTITION_KEYS, MAX_WINDOW_ORDER_KEYS};

impl Parser<'_> {
    pub(super) fn window_projection(
        &mut self,
        first: usize,
        parentheses: usize,
    ) -> Result<ParsedExpression, Error> {
        let sum = self.is_word("SUM");
        let call = self.take(Kind::Identifier)?;
        self.take(Kind::LeftParen)?;
        let operation = if sum {
            ParsedOp::WindowSum(self.column()?)
        } else {
            self.take(Kind::Star)?;
            ParsedOp::WindowCount
        };
        self.take(Kind::RightParen)?;
        if !self.is_word("OVER") {
            return Err(Error::Parse {
                message: "analytic projection requires OVER",
                span: call,
            });
        }
        self.word("OVER")?;
        self.take(Kind::LeftParen)?;
        let mut expression = ParsedExpression::EMPTY;
        expression.push(operation, call)?;
        let mut partitions = 0;
        if self.is_word("PARTITION") {
            self.word("PARTITION")?;
            self.word("BY")?;
            loop {
                let column = self.column()?;
                if partitions == MAX_PARTITION_KEYS {
                    return Err(Error::Parse {
                        message: "partition key limit exceeded",
                        span: column,
                    });
                }
                expression.push(ParsedOp::Column(column), column)?;
                partitions += 1;
                if self.peek() != Kind::Comma {
                    break;
                }
                self.take(Kind::Comma)?;
            }
        }
        if sum {
            self.take(Kind::Order)?;
            self.word("BY")?;
            let mut ordered = 0;
            loop {
                let column = self.column()?;
                if ordered == MAX_WINDOW_ORDER_KEYS {
                    return Err(Error::Parse {
                        message: "window order key limit exceeded",
                        span: column,
                    });
                }
                let (direction, nulls) = self.order_policy()?;
                expression.push(ParsedOp::WindowOrder { direction, nulls }, column)?;
                expression.push(ParsedOp::Column(column), column)?;
                ordered += 1;
                if self.peek() != Kind::Comma {
                    break;
                }
                self.take(Kind::Comma)?;
            }
        }
        self.take(Kind::RightParen)?;
        for _ in 0..parentheses {
            self.take(Kind::RightParen)?;
        }
        expression.span = span(
            usize::from(self.tokens.values[first].span.start),
            usize::from(self.tokens.values[self.position - 1].span.end),
        );
        Ok(expression)
    }

    pub(super) fn order_policy(&mut self) -> Result<(Direction, NullPlacement), Error> {
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
        Ok((direction, nulls))
    }
}
