//! Evaluate a numeric expression one row at a time, requesting inputs as needed.
//!
//! Call `next_column` until it returns an input column or reports completion.
//! For each requested column, obtain its value and pass it to `supply`. Once
//! complete, read `value`. This exchange lets the caller compute an input only
//! when evaluation reaches it, instead of evaluating every possible dependency.
//!
//! The program stores operands before their operation, called postfix order.
//! For example, addition visits its left value, right value, then Add. COALESCE
//! needs an exception: after a non-NULL left value, skip the entire fallback and
//! its operation. Thus `COALESCE(7, 1 / 0)` never divides by zero. Construction
//! derives these jumps from the validated program using fixed-size stacks.
//!
//! CASE has four operands: comparison left/right, selected result and fallback.
//! Once both comparison inputs are available, a true comparison enters the result
//! and skips the fallback; false or NULL enters the fallback directly. Later WHEN
//! arms are nested fallbacks. The selected value uses the arms' common result type.
//!
//! NULLIF evaluates both operands, left first, and returns NULL if they compare
//! equal. Otherwise it keeps the left value. Arithmetic failures stop evaluation
//! where they occur; even SAFE_DIVIDE cannot suppress a failure in its arguments.

use super::*;

pub(crate) struct Evaluation<'a> {
    expression: &'a Expression,
    fallback_end: [u8; MAX_OPS],
    case_alternative: [u8; MAX_OPS],
    case_end: [u8; MAX_OPS],
    result_types: [DataType; MAX_OPS],
    values: [Number; MAX_OPS],
    depth: usize,
    position: usize,
}

impl<'a> Evaluation<'a> {
    pub(crate) const SCRATCH_BYTES: usize = std::mem::size_of::<Self>()
        + std::mem::size_of::<[u8; MAX_OPS]>()
        + std::mem::size_of::<[DataType; MAX_OPS]>();

    // The caller must validate the program first. Stack sizes and operand types
    // below rely on that validation rather than checking the program again.
    pub(crate) fn new(expression: &'a Expression) -> Self {
        let mut evaluation = Self {
            expression,
            fallback_end: [0; MAX_OPS],
            case_alternative: [0; MAX_OPS],
            case_end: [0; MAX_OPS],
            result_types: [DataType::Int64; MAX_OPS],
            values: [Number::Null; MAX_OPS],
            depth: 0,
            position: 0,
        };
        // Each stack entry remembers where an operand starts and its result type.
        // At COALESCE, the right operand's start becomes a skip point. Its end
        // is this operation, whose result type also applies when the right is skipped.
        let mut starts = [0_u8; MAX_OPS];
        let mut types = [DataType::Int64; MAX_OPS];
        let mut depth = 0;
        for (position, op) in expression.ops[..usize::from(expression.len)]
            .iter()
            .enumerate()
        {
            match *op {
                Op::Column(_) | Op::Integer(_) | Op::Double(_) | Op::Null => {
                    starts[depth] = position as u8;
                    types[depth] = match op {
                        Op::Column(column) => column.data_type(),
                        Op::Double(_) => DataType::Double,
                        _ => DataType::Int64,
                    };
                    depth += 1;
                }
                Op::Case(_) => {
                    let selected = usize::from(starts[depth - 2]);
                    let alternative = usize::from(starts[depth - 1]);
                    evaluation.case_alternative[selected] = alternative as u8;
                    evaluation.case_end[alternative] = position as u8;
                    types[depth - 4] = if types[depth - 2] == DataType::Double
                        || types[depth - 1] == DataType::Double
                    {
                        DataType::Double
                    } else {
                        DataType::Int64
                    };
                    depth -= 3;
                }
                Op::Abs | Op::Negate | Op::Sign => (),
                Op::ToDouble
                | Op::Floor
                | Op::Ceil
                | Op::Round
                | Op::Sqrt
                | Op::Log10
                | Op::Ln
                | Op::Exp => types[depth - 1] = DataType::Double,
                Op::Empty => unreachable!("validated scalar extent"),
                _ => {
                    depth -= 1;
                    if *op == Op::Coalesce {
                        evaluation.fallback_end[usize::from(starts[depth])] = position as u8;
                    }
                    if matches!(op, Op::Divide | Op::SafeDivide | Op::Power)
                        || types[depth] == DataType::Double
                    {
                        types[depth - 1] = DataType::Double;
                    }
                }
            }
            evaluation.result_types[position] = types[depth - 1];
        }
        evaluation
    }

    /// Advance until a column is needed or evaluation is complete (`None`).
    /// On an arithmetic failure, discard this evaluation; it cannot resume safely.
    pub(crate) fn next_column(&mut self) -> Result<Option<SemanticColumn>, ArithmeticFailure> {
        while self.position < usize::from(self.expression.len) {
            // Reaching ELSE from a completed THEN skips the alternative. A
            // false/NULL comparison jumps there below, after this check, so its
            // alternative is evaluated exactly once.
            let case_end = usize::from(self.case_end[self.position]);
            if case_end != 0 {
                self.values[self.depth - 1] =
                    coerce(self.values[self.depth - 1], self.result_types[case_end]);
                self.position = case_end + 1;
                continue;
            }
            let end = usize::from(self.fallback_end[self.position]);
            if end != 0 && !matches!(self.values[self.depth - 1], Number::Null) {
                self.values[self.depth - 1] =
                    coerce(self.values[self.depth - 1], self.result_types[end]);
                self.position = end + 1;
                continue;
            }
            let alternative = usize::from(self.case_alternative[self.position]);
            if alternative != 0 {
                let end = usize::from(self.case_end[alternative]);
                let Op::Case(comparison) = self.expression.ops[end] else {
                    unreachable!("validated CASE boundary");
                };
                let selected = compare(
                    comparison,
                    self.values[self.depth - 2],
                    self.values[self.depth - 1],
                );
                self.depth -= 2;
                if !selected {
                    self.position = alternative;
                }
            }
            let op = self.expression.ops[self.position];
            match op {
                Op::Column(column) => return Ok(Some(column)),
                Op::Integer(value) => self.push(Number::Integer(value)),
                Op::Null => self.push(Number::Null),
                Op::Case(_) => {
                    self.values[self.depth - 1] = coerce(
                        self.values[self.depth - 1],
                        self.result_types[self.position],
                    );
                }
                Op::Double(bits) => self.push(Number::Double(f64::from_bits(bits))),
                Op::ToDouble
                | Op::Floor
                | Op::Ceil
                | Op::Round
                | Op::Sqrt
                | Op::Log10
                | Op::Ln
                | Op::Exp => {
                    self.values[self.depth - 1] = match self.values[self.depth - 1] {
                        Number::Null => Number::Null,
                        Number::Integer(value) => Number::Double(double_unary(op, value as f64)?),
                        Number::Double(value) => Number::Double(double_unary(op, value)?),
                    };
                }
                Op::Abs | Op::Negate | Op::Sign => {
                    self.values[self.depth - 1] = match self.values[self.depth - 1] {
                        Number::Null => Number::Null,
                        Number::Integer(value) => Number::Integer(match op {
                            Op::Abs => value.checked_abs().ok_or(ArithmeticFailure::Abs)?,
                            Op::Negate => value.checked_neg().ok_or(ArithmeticFailure::Negate)?,
                            Op::Sign => value.signum(),
                            _ => unreachable!("unary numeric operation"),
                        }),
                        Number::Double(value) => Number::Double(match op {
                            Op::Abs => value.abs(),
                            Op::Negate => -value,
                            Op::Sign => sign_double(value),
                            _ => unreachable!("unary numeric operation"),
                        }),
                    };
                }
                Op::Empty => unreachable!("validated scalar extent"),
                _ => {
                    self.depth -= 1;
                    let left = self.values[self.depth - 1];
                    let right = self.values[self.depth];
                    self.values[self.depth - 1] = if op == Op::Coalesce {
                        // A non-NULL left value jumped past both the fallback and
                        // this operation. Reaching here means the fallback was needed.
                        debug_assert!(matches!(left, Number::Null));
                        coerce(right, self.result_types[self.position])
                    } else if op == Op::NullIf {
                        // Convert to the call's resolved type before comparing.
                        // A NULL right operand still makes an INT64 left operand
                        // become DOUBLE when that is the call's result type.
                        let kind = self.result_types[self.position];
                        let left = coerce(left, kind);
                        let right = coerce(right, kind);
                        let equal = match (left, right) {
                            (Number::Integer(a), Number::Integer(b)) => a == b,
                            (Number::Double(a), Number::Double(b)) => a == b,
                            _ => false,
                        };
                        if equal { Number::Null } else { left }
                    } else {
                        binary(op, left, right)?
                    };
                }
            }
            self.position += 1;
        }
        Ok(None)
    }

    /// Provide the column requested by `next_column`. A type or nullability
    /// mismatch leaves that request pending; no value has been pushed yet.
    pub(crate) fn supply(&mut self, value: Number) -> Result<(), Error> {
        let Op::Column(column) = self.expression.ops[self.position] else {
            unreachable!("numeric cursor must request an input before receiving it");
        };
        match (value, column.data_type()) {
            (Number::Null, _) if !column.nullable() => {
                return Err(Error::Corrupt("nonnull scalar input contains NULL"));
            }
            (Number::Null, _)
            | (Number::Integer(_), DataType::Int64)
            | (Number::Double(_), DataType::Double) => (),
            _ => return Err(Error::Corrupt("scalar input type or row extent")),
        }
        self.push(value);
        self.position += 1;
        Ok(())
    }

    fn push(&mut self, value: Number) {
        self.values[self.depth] = value;
        self.depth += 1;
    }

    pub(crate) fn value(&self) -> Number {
        assert_eq!(self.position, usize::from(self.expression.len));
        assert_eq!(self.depth, 1);
        self.values[0]
    }
}

// CASE needs only TRUE. NULL comparisons do not select an arm; null-safe
// comparisons implement IS [NOT] NULL using a literal NULL right operand.
fn compare(comparison: Comparison, left: Number, right: Number) -> bool {
    match (left, right) {
        (Number::Null, Number::Null) => comparison == Comparison::IsNotDistinct,
        (Number::Null, _) | (_, Number::Null) => comparison == Comparison::IsDistinct,
        (Number::Integer(a), Number::Integer(b)) => comparison.test_order(a.cmp(&b)),
        (Number::Integer(a), Number::Double(b)) => comparison.test(a as f64, b),
        (Number::Double(a), Number::Integer(b)) => comparison.test(a, b as f64),
        (Number::Double(a), Number::Double(b)) => comparison.test(a, b),
    }
}

fn coerce(value: Number, kind: DataType) -> Number {
    match (value, kind) {
        (Number::Integer(value), DataType::Double) => Number::Double(value as f64),
        _ => value,
    }
}

// Both operands have already been evaluated. SAFE_DIVIDE may convert its own
// division failure to NULL; errors while obtaining either operand occur earlier.
fn binary(op: Op, left: Number, right: Number) -> Result<Number, ArithmeticFailure> {
    match (left, right) {
        (Number::Null, _) | (_, Number::Null) => Ok(Number::Null),
        (Number::Integer(a), Number::Integer(b))
            if !matches!(op, Op::Divide | Op::SafeDivide | Op::Power) =>
        {
            integer_binary(op, a, b).map(Number::Integer)
        }
        _ => {
            let double = |value| match value {
                Number::Integer(value) => value as f64,
                Number::Double(value) => value,
                Number::Null => unreachable!("NULL operands handled above"),
            };
            match double_binary(op, double(left), double(right)) {
                Ok(value) => Ok(Number::Double(value)),
                Err(ArithmeticFailure::Divide | ArithmeticFailure::DivideByZero)
                    if op == Op::SafeDivide =>
                {
                    Ok(Number::Null)
                }
                Err(failure) => Err(failure),
            }
        }
    }
}

impl Expression {
    pub(super) fn evaluate_conditional<'scratch>(
        &self,
        columns: &[Option<NumericInput<'_>>],
        range: Range<usize>,
        scratch: &'scratch mut [u64],
    ) -> Result<NumericOutput<'scratch>, ArithmeticFailure> {
        let rows = range.end - range.start;
        let mut valid = [0; VALID_WORDS];
        let mut evaluation = Evaluation::new(self);
        for (lane, row) in range.enumerate() {
            evaluation.position = 0;
            evaluation.depth = 0;
            while let Some(column) = evaluation.next_column()? {
                let input = columns
                    .iter()
                    .flatten()
                    .find(|input| input.column == column)
                    .expect("validated demanded scalar input");
                let value = if input
                    .valid
                    .is_some_and(|valid| valid[row / 64] & (1 << (row % 64)) == 0)
                {
                    Number::Null
                } else {
                    match input.values {
                        NumericValues::Int64(values) => Number::Integer(values[row]),
                        NumericValues::Double(values) => Number::Double(values[row]),
                        NumericValues::Bits {
                            values,
                            kind: DataType::Int64,
                        } => Number::Integer(integer(values[row])),
                        NumericValues::Bits { values, .. } => {
                            Number::Double(f64::from_bits(values[row]))
                        }
                    }
                };
                evaluation
                    .supply(value)
                    .expect("validated borrowed numeric input");
            }
            scratch[lane] = match evaluation.value() {
                Number::Null => 0,
                Number::Integer(value) => {
                    valid[lane / 64] |= 1 << (lane % 64);
                    integer_bits(value)
                }
                Number::Double(value) => {
                    valid[lane / 64] |= 1 << (lane % 64);
                    value.to_bits()
                }
            };
        }
        Ok(NumericOutput {
            values: &scratch[..rows],
            valid,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nullif_preserves_numeric_equality_coercion_and_left_bits() {
        use DataType::{Double, Int64};
        let nan = f64::from_bits(0x7ff8_0000_0000_0042);
        for (left_type, right_type, left, right, expected) in [
            (
                Int64,
                Int64,
                Number::Integer(7),
                Number::Integer(7),
                Number::Null,
            ),
            (
                Int64,
                Int64,
                Number::Integer(7),
                Number::Integer(8),
                Number::Integer(7),
            ),
            (Int64, Int64, Number::Null, Number::Integer(7), Number::Null),
            (
                Int64,
                Int64,
                Number::Integer(7),
                Number::Null,
                Number::Integer(7),
            ),
            (Int64, Int64, Number::Null, Number::Null, Number::Null),
            (
                Int64,
                Int64,
                Number::Integer(9_007_199_254_740_993),
                Number::Integer(9_007_199_254_740_992),
                Number::Integer(9_007_199_254_740_993),
            ),
            (
                Int64,
                Double,
                Number::Integer(9_007_199_254_740_993),
                Number::Double(9_007_199_254_740_992.0),
                Number::Null,
            ),
            (
                Int64,
                Double,
                Number::Integer(9_007_199_254_740_993),
                Number::Null,
                Number::Double(9_007_199_254_740_992.0),
            ),
            (
                Double,
                Int64,
                Number::Double(1.5),
                Number::Integer(1),
                Number::Double(1.5),
            ),
            (
                Double,
                Double,
                Number::Double(-0.0),
                Number::Double(0.0),
                Number::Null,
            ),
            (
                Double,
                Double,
                Number::Double(-0.0),
                Number::Double(1.0),
                Number::Double(-0.0),
            ),
            (
                Double,
                Double,
                Number::Double(nan),
                Number::Double(nan),
                Number::Double(nan),
            ),
            (
                Double,
                Double,
                Number::Double(1.0),
                Number::Double(nan),
                Number::Double(1.0),
            ),
            (
                Double,
                Double,
                Number::Double(f64::INFINITY),
                Number::Double(f64::INFINITY),
                Number::Null,
            ),
            (
                Double,
                Double,
                Number::Double(f64::NEG_INFINITY),
                Number::Double(f64::INFINITY),
                Number::Double(f64::NEG_INFINITY),
            ),
        ] {
            let a = SemanticColumn::new(1, left_type, true);
            let b = SemanticColumn::new(2, right_type, true);
            let mut expression = Expression::EMPTY;
            expression.ops[..3].copy_from_slice(&[Op::Column(a), Op::Column(b), Op::NullIf]);
            expression.len = 3;
            expression.data_type = if left_type == Double || right_type == Double {
                Double
            } else {
                Int64
            };
            expression.validate(&[a, b]).unwrap();
            assert!(expression.nullable());
            let mut evaluation = Evaluation::new(&expression);
            assert_eq!(evaluation.next_column().unwrap(), Some(a));
            evaluation.supply(left).unwrap();
            assert_eq!(evaluation.next_column().unwrap(), Some(b));
            evaluation.supply(right).unwrap();
            assert_eq!(evaluation.next_column().unwrap(), None);
            let bits = |value| match value {
                Number::Null => None,
                Number::Integer(value) => Some((Int64, value as u64)),
                Number::Double(value) => Some((Double, value.to_bits())),
            };
            assert_eq!(
                bits(evaluation.value()),
                bits(expected),
                "{left:?}, {right:?}"
            );
        }
    }

    #[test]
    fn nullif_demands_second_operand_after_null_but_stops_at_first_error() {
        let a = SemanticColumn::new(1, DataType::Int64, true);
        let b = SemanticColumn::new(2, DataType::Int64, true);
        let mut expression = Expression::EMPTY;
        // NULLIF(DIV(a, 0), DIV(b, 0))
        expression.ops[..7].copy_from_slice(&[
            Op::Column(a),
            Op::Integer(0),
            Op::IntegerDivide,
            Op::Column(b),
            Op::Integer(0),
            Op::IntegerDivide,
            Op::NullIf,
        ]);
        expression.len = 7;
        expression.data_type = DataType::Int64;
        expression.validate(&[a, b]).unwrap();
        let mut evaluation = Evaluation::new(&expression);
        assert_eq!(evaluation.next_column().unwrap(), Some(a));
        evaluation.supply(Number::Integer(1)).unwrap();
        assert!(matches!(
            evaluation.next_column(),
            Err(ArithmeticFailure::DivideByZero)
        ));
        let mut evaluation = Evaluation::new(&expression);
        assert_eq!(evaluation.next_column().unwrap(), Some(a));
        evaluation.supply(Number::Null).unwrap();
        assert_eq!(evaluation.next_column().unwrap(), Some(b));
        evaluation.supply(Number::Integer(1)).unwrap();
        assert!(matches!(
            evaluation.next_column(),
            Err(ArithmeticFailure::DivideByZero)
        ));
    }

    #[test]
    fn coalesce_requests_only_selected_columns_and_preserves_exact_values() {
        let left = SemanticColumn::new(1, DataType::Int64, true);
        let right = SemanticColumn::new(2, DataType::Int64, true);
        let mut expression = Expression::EMPTY;
        expression.ops[..5].copy_from_slice(&[
            Op::Column(left),
            Op::Column(right),
            Op::Integer(0),
            Op::IntegerDivide,
            Op::Coalesce,
        ]);
        expression.len = 5;
        expression.data_type = DataType::Int64;
        expression.validate(&[left, right]).unwrap();
        let mut evaluation = Evaluation::new(&expression);
        assert_eq!(evaluation.next_column().unwrap(), Some(left));
        evaluation
            .supply(Number::Integer(9_007_199_254_740_993))
            .unwrap();
        assert_eq!(evaluation.next_column().unwrap(), None);
        assert!(matches!(
            evaluation.value(),
            Number::Integer(9_007_199_254_740_993)
        ));

        let mut evaluation = Evaluation::new(&expression);
        assert_eq!(evaluation.next_column().unwrap(), Some(left));
        evaluation.supply(Number::Null).unwrap();
        assert_eq!(evaluation.next_column().unwrap(), Some(right));
        evaluation.supply(Number::Integer(1)).unwrap();
        assert!(matches!(
            evaluation.next_column(),
            Err(ArithmeticFailure::DivideByZero)
        ));

        // Replace COALESCE with addition: a present left value no longer skips
        // the right operand, so the same division must now fail.
        expression.ops[4] = Op::Add;
        let mut evaluation = Evaluation::new(&expression);
        assert_eq!(evaluation.next_column().unwrap(), Some(left));
        evaluation.supply(Number::Integer(7)).unwrap();
        assert_eq!(evaluation.next_column().unwrap(), Some(right));
        evaluation.supply(Number::Integer(1)).unwrap();
        assert!(matches!(
            evaluation.next_column(),
            Err(ArithmeticFailure::DivideByZero)
        ));
        let required = SemanticColumn::new(3, DataType::Int64, false);
        expression.ops.fill(Op::Empty);
        expression.ops[..3].copy_from_slice(&[Op::Column(required), Op::Integer(0), Op::Coalesce]);
        expression.len = 3;
        expression.validate(&[required]).unwrap();
        let mut evaluation = Evaluation::new(&expression);
        assert_eq!(evaluation.next_column().unwrap(), Some(required));
        assert!(matches!(
            evaluation.supply(Number::Null),
            Err(Error::Corrupt("nonnull scalar input contains NULL"))
        ));
        assert!(matches!(
            evaluation.supply(Number::Double(7.0)),
            Err(Error::Corrupt("scalar input type or row extent"))
        ));
        evaluation.supply(Number::Integer(7)).unwrap();
        assert_eq!(evaluation.next_column().unwrap(), None);
        assert!(matches!(evaluation.value(), Number::Integer(7)));
    }

    #[test]
    fn nullif_batch_preserves_validity_boundaries_and_reused_lanes() {
        let x = SemanticColumn::new(1, DataType::Int64, true);
        let y = SemanticColumn::new(2, DataType::Double, true);
        let integers = [9_007_199_254_740_993; 130];
        let doubles = [9_007_199_254_740_992.0; 130];
        let integer_valid = [0x5555_5555_5555_5555; 3];
        let double_valid = [0x3333_3333_3333_3333; 3];
        let inputs = [
            Some(
                NumericInput::new(x, NumericValues::Int64(&integers), Some(&integer_valid))
                    .unwrap(),
            ),
            Some(
                NumericInput::new(y, NumericValues::Double(&doubles), Some(&double_valid)).unwrap(),
            ),
        ];
        let mut expression = Expression::EMPTY;
        expression.ops[..3].copy_from_slice(&[Op::Column(x), Op::Column(y), Op::NullIf]);
        expression.len = 3;
        expression.data_type = DataType::Double;
        expression.validate(&[x, y]).unwrap();
        let mut scratch = [u64::MAX; 3 * MAX_ROWS];
        for range in [0..130, 33..100, 0..2, 62..67] {
            let output = expression
                .evaluate_batch(&inputs, range.clone(), &mut scratch)
                .unwrap();
            for (lane, row) in range.enumerate() {
                // Converting the large integer to DOUBLE loses its final bit.
                // The two present values then compare equal; only a present left
                // value paired with a NULL right value survives NULLIF.
                let expected = if row % 4 == 2 {
                    Some(9_007_199_254_740_992.0_f64.to_bits())
                } else {
                    None
                };
                assert_eq!(output.value(lane), expected, "source row {row}");
            }
        }
    }

    #[test]
    fn coalesce_batch_nulls_coercion_nested_branches_and_reuse() {
        let x = SemanticColumn::new(1, DataType::Int64, true);
        let y = SemanticColumn::new(2, DataType::Double, true);
        let integers = [9_007_199_254_740_993; 130];
        let doubles = [-0.0; 130];
        let integer_valid = [0x5555_5555_5555_5555; 3];
        let double_valid = [0x3333_3333_3333_3333; 3];
        let inputs = [
            Some(
                NumericInput::new(x, NumericValues::Int64(&integers), Some(&integer_valid))
                    .unwrap(),
            ),
            Some(
                NumericInput::new(y, NumericValues::Double(&doubles), Some(&double_valid)).unwrap(),
            ),
        ];
        let mut expression = Expression::EMPTY;
        expression.ops[..3].copy_from_slice(&[Op::Column(x), Op::Column(y), Op::Coalesce]);
        expression.len = 3;
        expression.data_type = DataType::Double;
        expression.validate(&[x, y]).unwrap();
        assert!(expression.nullable());
        let mut scratch = [u64::MAX; 3 * MAX_ROWS];
        for range in [0..130, 33..100, 0..2] {
            let output = expression
                .evaluate_batch(&inputs, range.clone(), &mut scratch)
                .unwrap();
            for (lane, row) in range.enumerate() {
                let expected = match row % 4 {
                    0 | 2 => Some(9_007_199_254_740_992.0_f64.to_bits()),
                    1 => Some((-0.0_f64).to_bits()),
                    _ => None,
                };
                assert_eq!(output.value(lane), expected, "source row {row}");
            }
        }
        // COALESCE(x, COALESCE(y, 7)) reaches 7 only when both inputs are NULL.
        // That final constant also makes the complete expression nonnullable.
        expression.ops[..5].copy_from_slice(&[
            Op::Column(x),
            Op::Column(y),
            Op::Integer(7),
            Op::Coalesce,
            Op::Coalesce,
        ]);
        expression.len = 5;
        expression.validate(&[x, y]).unwrap();
        assert!(!expression.nullable());
        let output = expression
            .evaluate_batch(&inputs, 0..4, &mut scratch)
            .unwrap();
        assert_eq!(
            (0..4).map(|row| output.value(row)).collect::<Vec<_>>(),
            vec![
                Some(9_007_199_254_740_992.0_f64.to_bits()),
                Some((-0.0_f64).to_bits()),
                Some(9_007_199_254_740_992.0_f64.to_bits()),
                Some(7.0_f64.to_bits()),
            ]
        );
        let special = [
            0x7ff8_0000_0000_0042,
            f64::INFINITY.to_bits(),
            f64::NEG_INFINITY.to_bits(),
            1,
            (-0.0_f64).to_bits(),
            0,
        ];
        let special_valid = [0b011111];
        let input = NumericInput::new(
            y,
            NumericValues::Bits {
                values: &special,
                kind: DataType::Double,
            },
            Some(&special_valid),
        )
        .unwrap();
        expression.ops.fill(Op::Empty);
        expression.ops[..3].copy_from_slice(&[
            Op::Column(y),
            Op::Double(0.5_f64.to_bits()),
            Op::Coalesce,
        ]);
        expression.len = 3;
        expression.validate(&[y]).unwrap();
        let output = expression
            .evaluate_batch(&[Some(input)], 0..6, &mut scratch)
            .unwrap();
        for (row, bits) in [
            0x7ff8_0000_0000_0042,
            f64::INFINITY.to_bits(),
            f64::NEG_INFINITY.to_bits(),
            1,
            (-0.0_f64).to_bits(),
            0.5_f64.to_bits(),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(output.value(row), Some(bits));
        }
        expression.data_type = DataType::Int64;
        assert!(expression.validate(&[x, y]).is_err());
        expression.len = 1;
        expression.ops.fill(Op::Empty);
        expression.ops[0] = Op::Coalesce;
        assert!(expression.validate(&[]).is_err());
        assert!(expression.nullable());
    }

    #[test]
    fn case_requests_only_the_selected_result_and_preserves_bits() {
        let a = SemanticColumn::new(1, DataType::Int64, true);
        let b = SemanticColumn::new(2, DataType::Int64, false);
        let yes = SemanticColumn::new(3, DataType::Double, false);
        let no = SemanticColumn::new(4, DataType::Int64, false);
        let mut expression = Expression::EMPTY;
        expression.ops[..5].copy_from_slice(&[
            Op::Column(a),
            Op::Column(b),
            Op::Column(yes),
            Op::Column(no),
            Op::Case(Comparison::Less),
        ]);
        expression.len = 5;
        expression.data_type = DataType::Double;
        expression.validate(&[a, b, yes, no]).unwrap();
        assert!(!expression.nullable());
        for (left, selected, value, expected) in [
            (
                Number::Integer(1),
                yes,
                Number::Double(f64::from_bits(0xfff8_0000_0000_0042)),
                0xfff8_0000_0000_0042,
            ),
            (
                Number::Integer(3),
                no,
                Number::Integer(9),
                9.0_f64.to_bits(),
            ),
            (Number::Null, no, Number::Integer(9), 9.0_f64.to_bits()),
        ] {
            let mut evaluation = Evaluation::new(&expression);
            assert_eq!(evaluation.next_column().unwrap(), Some(a));
            evaluation.supply(left).unwrap();
            assert_eq!(evaluation.next_column().unwrap(), Some(b));
            evaluation.supply(Number::Integer(2)).unwrap();
            assert_eq!(evaluation.next_column().unwrap(), Some(selected));
            evaluation.supply(value).unwrap();
            assert_eq!(evaluation.next_column().unwrap(), None);
            let Number::Double(value) = evaluation.value() else {
                panic!("common result type");
            };
            assert_eq!(value.to_bits(), expected);
        }
        expression.data_type = DataType::Int64;
        assert!(expression.validate(&[a, b, yes, no]).is_err());
        for len in 1..4 {
            expression = Expression::EMPTY;
            expression.ops[..len].fill(Op::Null);
            expression.ops[len] = Op::Case(Comparison::Equal);
            expression.len = (len + 1) as u8;
            assert!(expression.nullable());
            assert!(expression.infer(&[]).is_err());
        }
    }

    #[test]
    fn case_batch_reuses_lanes_without_losing_nulls_or_nan_bits() {
        let condition = SemanticColumn::new(1, DataType::Int64, false);
        let selected = SemanticColumn::new(2, DataType::Double, true);
        let mut expression = Expression::EMPTY;
        expression.ops[..5].copy_from_slice(&[
            Op::Column(condition),
            Op::Integer(0),
            Op::Column(selected),
            Op::Integer(9),
            Op::Case(Comparison::Greater),
        ]);
        expression.len = 5;
        expression.data_type = DataType::Double;
        expression.validate(&[condition, selected]).unwrap();
        let conditions = std::array::from_fn::<_, MAX_ROWS, _>(|index| [-1, 1, 1, 0][index % 4]);
        let values = [f64::from_bits(0x7ff8_0000_0000_0042); MAX_ROWS];
        let valid = [0xbbbb_bbbb_bbbb_bbbb; 4];
        let columns = [
            Some(NumericInput::new(condition, NumericValues::Int64(&conditions), None).unwrap()),
            Some(
                NumericInput::new(selected, NumericValues::Double(&values), Some(&valid)).unwrap(),
            ),
        ];
        let mut scratch = [u64::MAX; MAX_ROWS * 4];
        let expected = [
            Some(9.0_f64.to_bits()),
            Some(0x7ff8_0000_0000_0042),
            None,
            Some(9.0_f64.to_bits()),
        ];
        for range in [0..256, 63..130, 127..129, 0..1] {
            let output = expression
                .evaluate_batch(&columns, range.clone(), &mut scratch)
                .unwrap();
            for (row, index) in range.enumerate() {
                assert_eq!(output.value(row), expected[index % 4]);
            }
        }
    }
}
