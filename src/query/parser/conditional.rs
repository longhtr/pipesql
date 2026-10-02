//! Parse the boundaries between a searched CASE's conditions and results.
//!
//! Each arm has numeric comparison operands, a selected result and a fallback.
//! A later WHEN supplies the previous arm's fallback. END emits these operations
//! from the last arm backward, without recursive parsing or extra expression arrays.
//! Arithmetic and call parentheses retain their existing precedence boundaries.

use super::{
    Comparison, Error, Kind, MAX_OPS, ParsedExpression, ParsedOp, Parser, PendingOp, SourceSpan,
};

pub(super) enum CaseStep {
    Operand,
    Value,
    Outside,
}

impl Parser<'_> {
    pub(super) fn case_boundary(
        &mut self,
        expression: &mut ParsedExpression,
        pending: &mut [PendingOp; MAX_OPS],
        depth: &mut usize,
        at: SourceSpan,
    ) -> Result<CaseStep, Error> {
        let kind = self.peek();
        while *depth != 0 && pending[*depth - 1].precedence() != 0 {
            *depth -= 1;
            expression.push(pending[*depth].parsed(), at)?;
        }
        let boundary = depth.checked_sub(1).map(|index| pending[index]);
        if matches!(boundary, Some(PendingOp::CaseLeft)) {
            let comparison = if let Kind::Compare(comparison) = kind {
                self.take(kind)?;
                comparison
            } else if self.is_word("IS") {
                self.word("IS")?;
                let negated = self.is_word("NOT");
                if negated {
                    self.word("NOT")?;
                }
                let null = self.word("NULL")?;
                expression.push(ParsedOp::Null, null)?;
                if negated {
                    Comparison::IsDistinct
                } else {
                    Comparison::IsNotDistinct
                }
            } else {
                return Err(Error::Parse {
                    message: "CASE WHEN requires a numeric comparison or IS NULL",
                    span: at,
                });
            };
            pending[*depth - 1] = PendingOp::CaseRight(comparison);
            return Ok(if matches!(kind, Kind::Compare(_)) {
                CaseStep::Operand
            } else {
                CaseStep::Value
            });
        }
        if let Some(PendingOp::CaseRight(comparison)) = boundary {
            self.word("THEN")?;
            pending[*depth - 1] = PendingOp::CaseResult(comparison);
            return Ok(CaseStep::Operand);
        }
        if let Some(PendingOp::CaseResult(comparison)) = boundary {
            if self.is_word("WHEN") {
                if *depth == MAX_OPS {
                    return Err(Error::Parse {
                        message: "scalar stack limit exceeded",
                        span: at,
                    });
                }
                self.word("WHEN")?;
                pending[*depth - 1] = PendingOp::CaseTail(comparison);
                pending[*depth] = PendingOp::CaseLeft;
                *depth += 1;
                return Ok(CaseStep::Operand);
            }
            if self.is_word("ELSE") {
                self.word("ELSE")?;
                pending[*depth - 1] = PendingOp::CaseElse(comparison);
                return Ok(CaseStep::Operand);
            }
            if self.is_word("END") {
                expression.push(ParsedOp::Null, at)?;
            }
        }
        if self.is_word("END") {
            let Some(PendingOp::CaseResult(comparison) | PendingOp::CaseElse(comparison)) =
                boundary
            else {
                return Err(Error::Parse {
                    message: "END requires a complete CASE arm",
                    span: at,
                });
            };
            self.word("END")?;
            *depth -= 1;
            expression.push(ParsedOp::Case(comparison), at)?;
            while *depth != 0 {
                let PendingOp::CaseTail(comparison) = pending[*depth - 1] else {
                    break;
                };
                *depth -= 1;
                expression.push(ParsedOp::Case(comparison), at)?;
            }
            return Ok(CaseStep::Value);
        }
        Ok(CaseStep::Outside)
    }
}
