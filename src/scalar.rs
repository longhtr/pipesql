//! Bounded numeric programs shared by binding and execution.
#[cfg(test)]
use crate::frontend::SourceColumn;
use crate::frontend::{DataType, MAX_COLUMNS, SemanticColumn};
use crate::{Error, SourceSpan};
use std::ops::Range;

#[derive(Clone, Copy, Debug)]
pub(crate) enum ArithmeticFailure {
    Add,
    Subtract,
    Multiply,
    Negate,
}

impl ArithmeticFailure {
    pub(crate) fn into_error(self, span: SourceSpan) -> Error {
        Error::ArithmeticOverflow {
            operation: match self {
                Self::Add => "addition",
                Self::Subtract => "subtraction",
                Self::Multiply => "multiplication",
                Self::Negate => "negation",
            },
            span,
        }
    }
}

// Numeric kernels borrow one bounded batch. Identities travel with payloads;
// storage ordinals and the order of these inputs are not expression identities.
pub(crate) const MAX_ROWS: usize = 256;
const VALID_WORDS: usize = MAX_ROWS / 64;

#[derive(Clone, Copy)]
pub(crate) enum NumericValues<'a> {
    Int64(&'a [i64]),
    Double(&'a [f64]),
    // Checked scalar intermediates retain the kernel's own canonical bit payload.
    Bits { values: &'a [u64], kind: DataType },
}

#[derive(Clone, Copy)]
pub(crate) struct NumericInput<'a> {
    column: SemanticColumn,
    values: NumericValues<'a>,
    valid: Option<&'a [u64]>,
}

impl<'a> NumericInput<'a> {
    pub(crate) fn len(&self) -> usize {
        match self.values {
            NumericValues::Int64(values) => values.len(),
            NumericValues::Double(values) => values.len(),
            NumericValues::Bits { values, .. } => values.len(),
        }
    }

    pub(crate) fn new(
        column: SemanticColumn,
        values: NumericValues<'a>,
        valid: Option<&'a [u64]>,
    ) -> Result<Self, Error> {
        let (kind, rows) = match values {
            NumericValues::Int64(values) => (DataType::Int64, values.len()),
            NumericValues::Double(values) => (DataType::Double, values.len()),
            NumericValues::Bits { values, kind } => (kind, values.len()),
        };
        if !matches!(kind, DataType::Int64 | DataType::Double)
            || column.data_type() != kind
            || rows > MAX_ROWS
        {
            return Err(Error::Corrupt("scalar input type or row extent"));
        }
        let mut all_valid = true;
        if let Some(valid) = valid {
            if valid.len() != rows.div_ceil(64) {
                return Err(Error::Corrupt("scalar validity extent"));
            }
            for (index, word) in valid.iter().enumerate() {
                let used = (rows - index * 64).min(64);
                let mask = u64::MAX >> (64 - used);
                all_valid &= word & mask == mask;
            }
        }
        if !column.nullable() && !all_valid {
            return Err(Error::Corrupt("nonnull scalar input contains NULL"));
        }
        Ok(Self {
            column,
            values,
            valid: if all_valid { None } else { valid },
        })
    }
}
// Payload and validity have the same lane numbering. Null payloads have no
// semantic value; consumers must inspect validity before decoding them.
pub(crate) struct NumericOutput<'a> {
    values: &'a [u64],
    valid: [u64; VALID_WORDS],
}

impl NumericOutput<'_> {
    // General grouping replays already evaluated, checked raw arguments through
    // the same accumulator.
    pub(crate) fn from_bits(
        values: &[u64],
        valid: [u64; VALID_WORDS],
    ) -> Result<NumericOutput<'_>, Error> {
        if values.len() > MAX_ROWS {
            return Err(Error::Corrupt("numeric output row bound"));
        }
        Ok(NumericOutput { values, valid })
    }

    pub(crate) fn nonnull_values(&self) -> &[u64] {
        assert_eq!(self.valid, [u64::MAX; VALID_WORDS], "nonnull scalar result");
        self.values
    }

    pub(crate) fn value(&self, row: usize) -> Option<u64> {
        assert!(row < self.values.len());
        (self.valid[row / 64] & (1 << (row % 64)) != 0).then_some(self.values[row])
    }
}

pub(crate) const MAX_OPS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Op {
    Empty,
    Column(SemanticColumn),
    Integer(i64),
    Double(u64),
    Add,
    Subtract,
    Multiply,
    Negate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Expression {
    pub(crate) ops: [Op; MAX_OPS],
    pub(crate) len: u8,
    pub(crate) data_type: DataType,
}

impl Expression {
    pub(crate) const EMPTY: Self = Self {
        ops: [Op::Empty; MAX_OPS],
        len: 0,
        data_type: DataType::Double,
    };

    pub(crate) fn nullable(&self) -> bool {
        self.ops[..usize::from(self.len)]
            .iter()
            .any(|op| matches!(op, Op::Column(column) if column.nullable()))
    }

    #[cfg(test)]
    pub(crate) fn needs(&self, column: SemanticColumn) -> bool {
        self.ops[..usize::from(self.len)].contains(&Op::Column(column))
    }

    pub(crate) fn infer(&self, visible: &[SemanticColumn]) -> Result<DataType, Error> {
        if self.len == 0
            || usize::from(self.len) > MAX_OPS
            || self.ops[usize::from(self.len)..]
                .iter()
                .any(|op| *op != Op::Empty)
        {
            return Err(Error::Corrupt("invalid scalar program extent"));
        }
        let mut types = [DataType::Int64; MAX_OPS];
        let mut depth = 0;
        for op in &self.ops[..usize::from(self.len)] {
            let ty = match *op {
                Op::Column(column) => {
                    if !matches!(column.data_type(), DataType::Int64 | DataType::Double)
                        || !visible.contains(&column)
                    {
                        return Err(Error::Corrupt("invalid scalar input identity or type"));
                    }
                    column.data_type()
                }
                Op::Integer(_) => DataType::Int64,
                Op::Double(bits) => {
                    if !f64::from_bits(bits).is_finite() {
                        return Err(Error::Corrupt("nonfinite scalar literal"));
                    }
                    DataType::Double
                }
                Op::Negate => {
                    if depth == 0 {
                        return Err(Error::Corrupt("scalar unary stack underflow"));
                    }
                    continue;
                }
                Op::Add | Op::Subtract | Op::Multiply => {
                    if depth < 2 {
                        return Err(Error::Corrupt("scalar binary stack underflow"));
                    }
                    depth -= 1;
                    types[depth - 1] = if types[depth - 1] == DataType::Double
                        || types[depth] == DataType::Double
                    {
                        DataType::Double
                    } else {
                        DataType::Int64
                    };
                    continue;
                }
                Op::Empty => return Err(Error::Corrupt("empty scalar operation")),
            };
            types[depth] = ty;
            depth += 1;
        }
        if depth != 1 {
            return Err(Error::Corrupt("scalar program does not produce one value"));
        }
        Ok(types[0])
    }

    pub(crate) fn validate(&self, visible: &[SemanticColumn]) -> Result<(), Error> {
        if self.infer(visible)? != self.data_type {
            return Err(Error::Corrupt("scalar result type disagrees"));
        }
        Ok(())
    }

    pub(crate) fn stack_depth(&self) -> usize {
        // Binding has already checked stack balance and instruction extent.
        let mut depth = 0_usize;
        let mut peak = 0;
        for op in &self.ops[..usize::from(self.len)] {
            match op {
                Op::Column(_) | Op::Integer(_) | Op::Double(_) => depth += 1,
                Op::Add | Op::Subtract | Op::Multiply => {
                    depth = depth.checked_sub(1).expect("validated binary inputs")
                }
                Op::Negate => (),
                Op::Empty => unreachable!("validated scalar program"),
            }
            peak = peak.max(depth);
        }
        peak
    }

    pub(crate) fn evaluate_constant(&self) -> Result<Number, ArithmeticFailure> {
        let mut scratch = [0; MAX_OPS];
        self.evaluate_batch(&[], 0..1, &mut scratch)
            .map(|output| match self.data_type {
                DataType::Int64 => Number::Integer(integer(output.nonnull_values()[0])),
                DataType::Double => Number::Double(f64::from_bits(output.nonnull_values()[0])),
                _ => unreachable!("validated numeric result"),
            })
    }
    // Each stack vector has one type and `rows` raw 64-bit payloads. Integer
    // payloads preserve two's-complement bits; DOUBLE payloads preserve IEEE bits.
    // No lane can change type. Only the published final vector escapes scratch.
    pub(crate) fn evaluate_batch<'scratch>(
        &self,
        columns: &[Option<NumericInput<'_>>],
        range: Range<usize>,
        scratch: &'scratch mut [u64],
    ) -> Result<NumericOutput<'scratch>, ArithmeticFailure> {
        assert!(range.start < range.end && range.end <= MAX_ROWS);
        let rows = range.end - range.start;
        assert!(columns.len() <= MAX_COLUMNS && scratch.len() / rows >= self.stack_depth());
        for (index, input) in columns.iter().enumerate() {
            if let Some(input) = input {
                assert!(range.end <= input.len(), "scalar input extent");
                assert!(
                    !columns[..index]
                        .iter()
                        .flatten()
                        .any(|prior| prior.column == input.column),
                    "scalar inputs have unique identities"
                );
            }
        }
        let mut valid = [[u64::MAX; VALID_WORDS]; MAX_OPS];
        let mut types = [DataType::Int64; MAX_OPS];
        let mut depth = 0;
        for op in &self.ops[..usize::from(self.len)] {
            match *op {
                Op::Column(column) => {
                    let input = columns
                        .iter()
                        .flatten()
                        .find(|input| input.column == column)
                        .expect("validated numeric input identity");
                    let output = &mut scratch[depth * rows..(depth + 1) * rows];
                    valid[depth] = [u64::MAX; VALID_WORDS];
                    match input.values {
                        NumericValues::Bits { values, .. } => {
                            output.copy_from_slice(&values[range.clone()])
                        }
                        NumericValues::Double(values) => {
                            for (out, value) in output.iter_mut().zip(&values[range.clone()]) {
                                *out = value.to_bits();
                            }
                        }
                        NumericValues::Int64(values) => {
                            for (out, value) in output.iter_mut().zip(&values[range.clone()]) {
                                *out = integer_bits(*value);
                            }
                        }
                    }
                    if let Some(input_valid) = input.valid {
                        for (row, source) in range.clone().enumerate() {
                            if input_valid[source / 64] & (1 << (source % 64)) == 0 {
                                valid[depth][row / 64] &= !(1 << (row % 64));
                                output[row] = 0;
                            }
                        }
                    }
                    types[depth] = column.data_type();
                    depth += 1;
                }
                Op::Integer(value) => {
                    scratch[depth * rows..(depth + 1) * rows].fill(integer_bits(value));
                    types[depth] = DataType::Int64;
                    valid[depth] = [u64::MAX; VALID_WORDS];
                    depth += 1;
                }
                Op::Double(bits) => {
                    scratch[depth * rows..(depth + 1) * rows].fill(bits);
                    types[depth] = DataType::Double;
                    valid[depth] = [u64::MAX; VALID_WORDS];
                    depth += 1;
                }
                Op::Negate => {
                    let values = &mut scratch[(depth - 1) * rows..depth * rows];
                    match types[depth - 1] {
                        DataType::Int64 => {
                            for (row, value) in values.iter_mut().enumerate() {
                                if valid[depth - 1][row / 64] & (1 << (row % 64)) == 0 {
                                    continue;
                                }
                                *value = integer_bits(
                                    integer(*value)
                                        .checked_neg()
                                        .ok_or(ArithmeticFailure::Negate)?,
                                );
                            }
                        }
                        DataType::Double => {
                            for (row, value) in values.iter_mut().enumerate() {
                                if valid[depth - 1][row / 64] & (1 << (row % 64)) == 0 {
                                    continue;
                                }
                                *value = (-f64::from_bits(*value)).to_bits();
                            }
                        }
                        _ => unreachable!("validated numeric operand"),
                    }
                }
                Op::Add | Op::Subtract | Op::Multiply => {
                    depth -= 1;
                    let (left, right) = scratch.split_at_mut(depth * rows);
                    let left = &mut left[(depth - 1) * rows..];
                    let right = &right[..rows];
                    let left_type = types[depth - 1];
                    let right_type = types[depth];
                    let right_valid = valid[depth];
                    for (left, right) in valid[depth - 1].iter_mut().zip(right_valid) {
                        *left &= right;
                    }
                    let both_integer =
                        left_type == DataType::Int64 && right_type == DataType::Int64;
                    let all_valid = valid[depth - 1] == [u64::MAX; VALID_WORDS];
                    for (row, (left, right)) in left.iter_mut().zip(right).enumerate() {
                        if !all_valid && valid[depth - 1][row / 64] & (1 << (row % 64)) == 0 {
                            *left = 0;
                            continue;
                        }
                        if both_integer {
                            *left =
                                integer_bits(integer_binary(*op, integer(*left), integer(*right))?);
                        } else {
                            // Coerce at this operation, after earlier checked integer work.
                            let a = if left_type == DataType::Int64 {
                                integer(*left) as f64
                            } else {
                                f64::from_bits(*left)
                            };
                            let b = if right_type == DataType::Int64 {
                                integer(*right) as f64
                            } else {
                                f64::from_bits(*right)
                            };
                            *left = double_binary(*op, a, b)?.to_bits();
                        }
                    }
                    if !both_integer {
                        types[depth - 1] = DataType::Double;
                    }
                }
                Op::Empty => unreachable!("validated scalar program"),
            }
        }
        assert_eq!(types[0], self.data_type, "validated scalar result type");
        Ok(NumericOutput {
            values: &scratch[..rows],
            valid: valid[0],
        })
    }
}

fn integer(bits: u64) -> i64 {
    i64::from_ne_bytes(bits.to_ne_bytes())
}

fn integer_bits(value: i64) -> u64 {
    u64::from_ne_bytes(value.to_ne_bytes())
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Number {
    Integer(i64),
    Double(f64),
}

fn integer_binary(op: Op, left: i64, right: i64) -> Result<i64, ArithmeticFailure> {
    match op {
        Op::Add => left.checked_add(right).ok_or(ArithmeticFailure::Add),
        Op::Subtract => left.checked_sub(right).ok_or(ArithmeticFailure::Subtract),
        Op::Multiply => left.checked_mul(right).ok_or(ArithmeticFailure::Multiply),
        _ => unreachable!("binary operation"),
    }
}

fn double_binary(op: Op, left: f64, right: f64) -> Result<f64, ArithmeticFailure> {
    let (value, failure) = match op {
        Op::Add => (left + right, ArithmeticFailure::Add),
        Op::Subtract => (left - right, ArithmeticFailure::Subtract),
        Op::Multiply => (left * right, ArithmeticFailure::Multiply),
        _ => unreachable!("binary operation"),
    };
    if left.is_finite() && right.is_finite() && !value.is_finite() {
        return Err(failure);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Expression {
        fn evaluate(&self, columns: &[f64; 4]) -> Result<Number, ArithmeticFailure> {
            let mut scratch = [0; MAX_OPS];
            self.evaluate_doubles(
                &std::array::from_fn(|i| &columns[i..i + 1]),
                1,
                &mut scratch,
            )
            .map(|values| match self.data_type {
                DataType::Int64 => Number::Integer(integer(values[0])),
                DataType::Double => Number::Double(f64::from_bits(values[0])),
                _ => unreachable!(),
            })
        }

        fn evaluate_doubles<'a>(
            &self,
            columns: &[&[f64]; 4],
            rows: usize,
            scratch: &'a mut [u64],
        ) -> Result<&'a [u64], ArithmeticFailure> {
            let ids = [
                SourceColumn::QUANTITY.semantic(),
                SourceColumn::PRICE.semantic(),
                SourceColumn::DISCOUNT.semantic(),
                SourceColumn::TAX.semantic(),
            ];
            let inputs: [Option<NumericInput<'_>>; 4] = std::array::from_fn(|i| {
                (!columns[i].is_empty()).then(|| {
                    NumericInput::new(ids[i], NumericValues::Double(columns[i]), None).unwrap()
                })
            });
            let output = self.evaluate_batch(&inputs, 0..rows, scratch)?;
            assert_eq!(output.valid, [u64::MAX; VALID_WORDS]);
            Ok(output.values)
        }
    }

    #[test]
    fn semantic_integer_inputs_preserve_exact_arithmetic_and_identity() {
        let x = SemanticColumn::new(91, DataType::Int64, false);
        let y = SemanticColumn::new(17, DataType::Int64, false);
        let values = [
            i64::MIN,
            -(1_i64 << 53) - 1,
            -1,
            0,
            1,
            (1_i64 << 53) + 1,
            i64::MAX,
        ];
        let mut scratch = [0; MAX_OPS];
        for left in values {
            for right in values {
                for op in [Op::Add, Op::Subtract, Op::Multiply] {
                    let a = [left];
                    let b = [right];
                    let inputs = [
                        Some(NumericInput::new(y, NumericValues::Int64(&b), None).unwrap()),
                        None,
                        Some(NumericInput::new(x, NumericValues::Int64(&a), None).unwrap()),
                    ];
                    let mut expression = Expression::EMPTY;
                    expression.ops[..3].copy_from_slice(&[Op::Column(x), Op::Column(y), op]);
                    expression.len = 3;
                    expression.data_type = DataType::Int64;
                    expression.validate(&[y, x]).unwrap();
                    let expected = match op {
                        Op::Add => i128::from(left) + i128::from(right),
                        Op::Subtract => i128::from(left) - i128::from(right),
                        Op::Multiply => i128::from(left) * i128::from(right),
                        _ => unreachable!(),
                    };
                    let output = expression.evaluate_batch(&inputs, 0..1, &mut scratch);
                    match i64::try_from(expected) {
                        Ok(value) => assert_eq!(integer(output.unwrap().value(0).unwrap()), value),
                        Err(_) => assert!(output.is_err()),
                    }
                    // A later DOUBLE conversion cannot rescue earlier INT64 overflow.
                    expression.ops[3..5]
                        .copy_from_slice(&[Op::Double(0.5_f64.to_bits()), Op::Multiply]);
                    expression.len = 5;
                    expression.data_type = DataType::Double;
                    expression.validate(&[x, y]).unwrap();
                    let output = expression.evaluate_batch(&inputs, 0..1, &mut scratch);
                    match i64::try_from(expected) {
                        Ok(value) => assert_eq!(
                            output.unwrap().value(0).unwrap(),
                            ((value as f64) * 0.5).to_bits()
                        ),
                        Err(_) => assert!(output.is_err()),
                    }
                }
            }
        }
    }

    #[test]
    fn nullable_batch_inputs_preserve_word_boundaries_and_reused_stack_lanes() {
        use crate::batch::Batch;
        use crate::value::Value;
        let x = SemanticColumn::new(91, DataType::Int64, true);
        let y = SemanticColumn::new(17, DataType::Int64, true);
        let mut batch = Batch::new(&[DataType::Int64, DataType::Int64], 100_000).unwrap();
        for row in 0..MAX_ROWS {
            batch
                .set(
                    row,
                    0,
                    if row % 3 == 0 {
                        Value::Null
                    } else {
                        Value::Int64((1_i64 << 53) + row as i64)
                    },
                )
                .unwrap();
            batch
                .set(
                    row,
                    1,
                    if row % 5 == 0 {
                        Value::Null
                    } else {
                        Value::Int64(row as i64)
                    },
                )
                .unwrap();
        }
        batch.publish_rows(MAX_ROWS);
        let inputs = [
            Some(batch.numeric(1, y).unwrap()),
            Some(batch.numeric(0, x).unwrap()),
        ];
        let mut expression = Expression::EMPTY;
        expression.ops[..5].copy_from_slice(&[
            Op::Column(x),
            Op::Column(y),
            Op::Subtract,
            Op::Integer(1),
            Op::Add,
        ]);
        expression.len = 5;
        expression.data_type = DataType::Int64;
        expression.validate(&[x, y]).unwrap();
        let mut scratch = [u64::MAX; MAX_OPS * MAX_ROWS];
        for range in [0..256, 63..129, 64..65, 255..256, 1..2, 0..256] {
            let output = expression
                .evaluate_batch(&inputs, range.clone(), &mut scratch)
                .unwrap();
            for (lane, row) in range.enumerate() {
                assert_eq!(
                    output.value(lane).map(integer),
                    if row % 3 == 0 || row % 5 == 0 {
                        None
                    } else {
                        Some((1_i64 << 53) + 1)
                    }
                );
            }
        }
    }

    #[test]
    fn null_arithmetic_ignores_payload_and_preserves_demanded_child_errors() {
        let id = SemanticColumn::new(11, DataType::Int64, true);
        let values = [i64::MIN, i64::MAX];
        let valid = [0];
        let inputs = [Some(
            NumericInput::new(id, NumericValues::Int64(&values), Some(&valid)).unwrap(),
        )];
        let mut expression = Expression::EMPTY;
        expression.ops[..2].copy_from_slice(&[Op::Column(id), Op::Negate]);
        expression.len = 2;
        expression.data_type = DataType::Int64;
        expression.validate(&[id]).unwrap();
        let mut scratch = [0; MAX_OPS * 2];
        let output = expression
            .evaluate_batch(&inputs, 0..2, &mut scratch)
            .unwrap();
        assert_eq!(output.value(0), None);
        assert_eq!(output.value(1), None);
        expression.ops[..5].copy_from_slice(&[
            Op::Column(id),
            Op::Integer(i64::MAX),
            Op::Integer(1),
            Op::Add,
            Op::Add,
        ]);
        expression.len = 5;
        expression.validate(&[id]).unwrap();
        assert!(matches!(
            expression.evaluate_batch(&inputs, 0..2, &mut scratch),
            Err(ArithmeticFailure::Add)
        ));
        // The actual operation skips null lanes even if their raw payload would overflow.
        expression.ops[..3].copy_from_slice(&[Op::Column(id), Op::Integer(1), Op::Add]);
        expression.ops[3..].fill(Op::Empty);
        expression.len = 3;
        expression.validate(&[id]).unwrap();
        let output = expression
            .evaluate_batch(&inputs, 0..2, &mut scratch)
            .unwrap();
        assert_eq!(output.value(0), None);
        assert_eq!(output.value(1), None);
    }

    #[test]
    fn nullable_double_and_integer_operands_preserve_nonfinite_and_mixed_rules() {
        let x = SemanticColumn::new(41, DataType::Double, true);
        let y = SemanticColumn::new(19, DataType::Int64, true);
        let a = [f64::MAX, f64::NAN, f64::INFINITY, -0.0, 0.5];
        let b = [2, 1, 2, 1, (1_i64 << 53) + 1];
        // The finite-overflow lane is NULL; the nonfinite lanes remain demanded.
        let valid = [0b11110];
        let inputs = [
            Some(NumericInput::new(y, NumericValues::Int64(&b), None).unwrap()),
            Some(NumericInput::new(x, NumericValues::Double(&a), Some(&valid)).unwrap()),
        ];
        let mut expression = Expression::EMPTY;
        expression.ops[..3].copy_from_slice(&[Op::Column(x), Op::Column(y), Op::Multiply]);
        expression.len = 3;
        expression.validate(&[x, y]).unwrap();
        let mut scratch = [0; MAX_OPS * 5];
        let output = expression
            .evaluate_batch(&inputs, 0..5, &mut scratch)
            .unwrap();
        assert_eq!(output.value(0), None);
        assert!(f64::from_bits(output.value(1).unwrap()).is_nan());
        assert_eq!(output.value(2), Some(f64::INFINITY.to_bits()));
        assert_eq!(output.value(3), Some((-0.0_f64).to_bits()));
        assert_eq!(output.value(4), Some((0.5 * (b[4] as f64)).to_bits()));
        let inputs = [
            Some(NumericInput::new(y, NumericValues::Int64(&b), None).unwrap()),
            Some(NumericInput::new(x, NumericValues::Double(&a), None).unwrap()),
        ];
        assert!(matches!(
            expression.evaluate_batch(&inputs, 0..5, &mut scratch),
            Err(ArithmeticFailure::Multiply)
        ));
    }

    #[test]
    fn numeric_input_admission_checks_type_extent_and_nullability() {
        let id = SemanticColumn::new(11, DataType::Int64, false);
        assert!(NumericInput::new(id, NumericValues::Double(&[1.0]), None).is_err());
        assert!(NumericInput::new(id, NumericValues::Int64(&[1]), Some(&[])).is_err());
        assert!(NumericInput::new(id, NumericValues::Int64(&[1]), Some(&[0])).is_err());
        assert!(NumericInput::new(id, NumericValues::Int64(&[1; MAX_ROWS + 1]), None).is_err());
        assert!(NumericInput::new(id, NumericValues::Int64(&[1]), Some(&[1])).is_ok());
    }

    fn binary_result(op: Op, left: f64, right: f64) -> Result<f64, ArithmeticFailure> {
        let mut expression = Expression::EMPTY;
        expression.ops[..3].copy_from_slice(&[
            Op::Column(SourceColumn::QUANTITY.semantic()),
            Op::Column(SourceColumn::PRICE.semantic()),
            op,
        ]);
        expression.len = 3;
        expression
            .validate(&[
                SourceColumn::QUANTITY.semantic(),
                SourceColumn::PRICE.semantic(),
            ])
            .unwrap();
        match expression.evaluate(&[left, right, 0.0, 0.0])? {
            Number::Double(value) => Ok(value),
            Number::Integer(_) => panic!("DOUBLE expression"),
        }
    }

    #[test]
    fn checked_double_matches_bit_classification_across_exceptional_operands() {
        let values = [
            0,
            1,
            0x0010_0000_0000_0000,
            0x3ff0_0000_0000_0000,
            0x4000_0000_0000_0000,
            0x7fef_ffff_ffff_ffff,
            0x7ff0_0000_0000_0000,
            0x7ff8_0000_0000_0001,
            0x7ff0_0000_0000_0001,
        ];
        let finite = |value: f64| value.to_bits() & 0x7ff0_0000_0000_0000 != 0x7ff0_0000_0000_0000;
        for sign_left in [0, 1_u64 << 63] {
            for sign_right in [0, 1_u64 << 63] {
                for left in values.map(|bits| f64::from_bits(bits | sign_left)) {
                    for right in values.map(|bits| f64::from_bits(bits | sign_right)) {
                        for (expected, actual) in [
                            (left + right, binary_result(Op::Add, left, right)),
                            (left - right, binary_result(Op::Subtract, left, right)),
                            (left * right, binary_result(Op::Multiply, left, right)),
                        ] {
                            let overflow = finite(left) && finite(right) && !finite(expected);
                            if overflow {
                                assert!(matches!(
                                    actual,
                                    Err(ArithmeticFailure::Add
                                        | ArithmeticFailure::Subtract
                                        | ArithmeticFailure::Multiply)
                                ));
                            } else {
                                let actual = actual.unwrap();
                                if expected.is_nan() {
                                    assert!(actual.is_nan());
                                } else {
                                    assert_eq!(actual.to_bits(), expected.to_bits());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn batch_scratch_reuse_preserves_lane_values_and_errors() {
        let mut expression = Expression::EMPTY;
        expression.ops[..7].copy_from_slice(&[
            Op::Column(SourceColumn::PRICE.semantic()),
            Op::Integer(1),
            Op::Column(SourceColumn::DISCOUNT.semantic()),
            Op::Subtract,
            Op::Multiply,
            Op::Column(SourceColumn::QUANTITY.semantic()),
            Op::Add,
        ]);
        expression.len = 7;
        expression
            .validate(&[
                SourceColumn::PRICE.semantic(),
                SourceColumn::DISCOUNT.semantic(),
                SourceColumn::QUANTITY.semantic(),
            ])
            .unwrap();
        assert_eq!(expression.stack_depth(), 3);
        let mut scratch = vec![0; expression.stack_depth() * 256];
        for count in [256, 3, 1, 17, 256] {
            let inputs: Vec<_> = (0..count)
                .map(|i| [i as f64, (i + 1) as f64, (i % 17) as f64 / 128.0, 0.0])
                .collect();
            let columns: [Vec<f64>; 4] =
                std::array::from_fn(|column| inputs.iter().map(|row| row[column]).collect());
            let result = expression
                .evaluate_doubles(
                    &std::array::from_fn(|column| columns[column].as_slice()),
                    count,
                    &mut scratch,
                )
                .unwrap();
            assert_eq!(result.len(), count);
            for (actual, input) in result.iter().zip(&inputs) {
                let actual = f64::from_bits(*actual);
                let expected = input[1] * (1.0 - input[2]) + input[0];
                assert_eq!(actual.to_bits(), expected.to_bits());
            }
        }
        let mut inputs = [[0.0; 4]; 256];
        inputs[255] = [0.0, f64::MAX, -1.0, 0.0];
        let columns: [Vec<f64>; 4] =
            std::array::from_fn(|column| inputs.iter().map(|row| row[column]).collect());
        assert!(matches!(
            expression.evaluate_doubles(
                &std::array::from_fn(|column| columns[column].as_slice()),
                inputs.len(),
                &mut scratch
            ),
            Err(ArithmeticFailure::Multiply)
        ));
    }

    #[test]
    fn integer_batches_match_wide_arithmetic_before_mixed_coercion() {
        let values = [
            i64::MIN,
            i64::MIN + 1,
            -(1 << 53) - 1,
            -1,
            0,
            1,
            (1 << 53) + 1,
            i64::MAX - 1,
            i64::MAX,
        ];
        let mut scratch = [0; MAX_OPS * 17];
        for left in values {
            for right in values {
                for op in [Op::Add, Op::Subtract, Op::Multiply] {
                    let wide = match op {
                        Op::Add => i128::from(left) + i128::from(right),
                        Op::Subtract => i128::from(left) - i128::from(right),
                        Op::Multiply => i128::from(left) * i128::from(right),
                        _ => unreachable!(),
                    };
                    let expected = i64::try_from(wide);
                    let mut expression = Expression::EMPTY;
                    expression.ops[..5].copy_from_slice(&[
                        Op::Integer(left),
                        Op::Integer(right),
                        op,
                        Op::Double(0.5_f64.to_bits()),
                        Op::Multiply,
                    ]);
                    expression.len = 5;
                    expression.validate(&[]).unwrap();
                    for rows in [17, 1, 3] {
                        let actual = expression.evaluate_doubles(&[&[]; 4], rows, &mut scratch);
                        match expected {
                            Ok(expected) => {
                                let expected = ((expected as f64) * 0.5).to_bits();
                                assert!(actual.unwrap().iter().all(|bits| *bits == expected));
                            }
                            Err(_) => {
                                assert!(actual.is_err(), "integer overflow precedes coercion")
                            }
                        }
                    }
                }
            }
        }
        for value in values {
            let mut expression = Expression::EMPTY;
            expression.ops[..2].copy_from_slice(&[Op::Integer(value), Op::Negate]);
            expression.len = 2;
            expression.data_type = DataType::Int64;
            expression.validate(&[]).unwrap();
            let result = expression.evaluate_doubles(&[&[]; 4], 17, &mut scratch);
            match i64::try_from(-i128::from(value)) {
                Ok(expected) => assert!(
                    result
                        .unwrap()
                        .iter()
                        .all(|bits| integer(*bits) == expected)
                ),
                Err(_) => assert!(matches!(result, Err(ArithmeticFailure::Negate))),
            }
        }
    }

    #[test]
    fn mixed_operand_directions_and_reused_stack_types_preserve_bits() {
        let inputs = [
            0.0,
            -0.0,
            f64::MIN_POSITIVE,
            -1.5,
            f64::MAX,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::from_bits(0x7ff0_0000_0000_0001),
        ];
        let columns = [&inputs[..], &[][..], &[][..], &[][..]];
        let mut scratch = [0; MAX_OPS * 8];
        for literal in [i64::MIN, -1, 0, 1, (1 << 53) + 1, i64::MAX] {
            for reversed in [false, true] {
                for op in [Op::Add, Op::Subtract, Op::Multiply] {
                    let mut operands = [
                        Op::Integer(literal),
                        Op::Column(SourceColumn::QUANTITY.semantic()),
                    ];
                    if reversed {
                        operands.reverse();
                    }
                    let mut expression = Expression::EMPTY;
                    expression.ops[..5].copy_from_slice(&[
                        operands[0],
                        operands[1],
                        op,
                        Op::Negate,
                        Op::Negate,
                    ]);
                    expression.len = 5;
                    expression
                        .validate(&[SourceColumn::QUANTITY.semantic()])
                        .unwrap();
                    for row in 0..inputs.len() {
                        let input = inputs[row];
                        let (left, right) = if reversed {
                            (input, literal as f64)
                        } else {
                            (literal as f64, input)
                        };
                        let expected = match op {
                            Op::Add => left + right,
                            Op::Subtract => left - right,
                            Op::Multiply => left * right,
                            _ => unreachable!(),
                        };
                        let actual = expression.evaluate_doubles(
                            &[&columns[0][row..row + 1], &[], &[], &[]],
                            1,
                            &mut scratch,
                        );
                        if left.is_finite() && right.is_finite() && !expected.is_finite() {
                            assert!(actual.is_err());
                        } else {
                            let actual = f64::from_bits(actual.unwrap()[0]);
                            if expected.is_nan() {
                                assert!(actual.is_nan());
                            } else {
                                assert_eq!(actual.to_bits(), expected.to_bits());
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn independent_validation_rejects_invalid_programs() {
        let visible = [SourceColumn::QUANTITY.semantic()];
        let mut valid = Expression::EMPTY;
        valid.ops[..3].copy_from_slice(&[
            Op::Column(SourceColumn::QUANTITY.semantic()),
            Op::Integer(1),
            Op::Add,
        ]);
        valid.len = 3;
        valid.validate(&visible).unwrap();
        for mutation in 0..9 {
            let mut invalid = valid;
            match mutation {
                0 => invalid.len = 0,
                1 => invalid.len = u8::MAX,
                2 => invalid.ops[3] = Op::Integer(0),
                3 => invalid.ops[0] = Op::Column(SourceColumn::SHIP_DATE.semantic()),
                4 => invalid.ops[0] = Op::Column(SourceColumn::TAX.semantic()),
                5 => invalid.ops[1] = Op::Double(f64::INFINITY.to_bits()),
                6 => invalid.ops[0] = Op::Negate,
                7 => invalid.ops[2] = Op::Integer(2),
                8 => invalid.data_type = DataType::Int64,
                _ => unreachable!(),
            }
            assert!(invalid.validate(&visible).is_err(), "mutation {mutation}");
        }
    }

    #[test]
    fn scalar_bit_inputs_preserve_payloads_nulls_and_type_checks() {
        for kind in [DataType::Int64, DataType::Double] {
            let column = SemanticColumn::new(1, kind, true);
            let mut expression = Expression::EMPTY;
            expression.ops[0] = Op::Column(column);
            expression.len = 1;
            expression.data_type = kind;
            expression.validate(&[column]).unwrap();
            let bits = [
                0,
                1_u64 << 63,
                i64::MAX as u64,
                f64::INFINITY.to_bits(),
                0x7ff8_0000_0000_0042,
                u64::MAX,
            ];
            let valid = [0b01_1111];
            let input = NumericInput::new(
                column,
                NumericValues::Bits {
                    values: &bits,
                    kind,
                },
                Some(&valid),
            )
            .unwrap();
            let mut scratch = [0; 6];
            let result = expression
                .evaluate_batch(&[Some(input)], 0..6, &mut scratch)
                .unwrap();
            for (row, expected) in bits[..5].iter().enumerate() {
                assert_eq!(result.value(row), Some(*expected));
            }
            assert_eq!(result.value(5), None);
            let nonnull = SemanticColumn::new(1, kind, false);
            assert!(
                NumericInput::new(
                    nonnull,
                    NumericValues::Bits {
                        values: &bits,
                        kind
                    },
                    Some(&valid)
                )
                .is_err()
            );
        }
        let date = SemanticColumn::new(1, DataType::Date, false);
        assert!(
            NumericInput::new(
                date,
                NumericValues::Bits {
                    values: &[0],
                    kind: DataType::Date
                },
                None
            )
            .is_err()
        );
    }
}
