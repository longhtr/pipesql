//! Demand evaluation inside a producer, without crossing materialization boundaries.
use super::{Error, Pipeline, Value};
use crate::frontend::{
    Computation, DataType, MAX_COLUMNS, MAX_COMPUTED, MAX_ROW_VALUES, SemanticColumn,
};
use crate::scalar::{Evaluation, MAX_OPS, Number, NumericInput, NumericValues, Op};

// One live producer step owns these fixed arrays. The result reserves their
// explicit payload while computed row producers are live; native call frames
// and compiler spill space remain part of the separately observed stack bound.
pub(super) const ROW_SCRATCH_BYTES: u64 =
    (std::mem::size_of::<[Option<Option<u64>>; MAX_COMPUTED]>()
        + std::mem::size_of::<[bool; MAX_ROW_VALUES + MAX_COMPUTED]>()
        + std::mem::size_of::<[Option<crate::frontend::SemanticColumn>; MAX_OPS]>()
        + std::mem::size_of::<[Option<NumericInput<'static>>; MAX_OPS]>()
        + 3 * std::mem::size_of::<[u64; MAX_OPS]>()
        + std::mem::size_of::<[[u64; 4]; MAX_OPS]>()
        + std::mem::size_of::<[DataType; MAX_OPS]>()
        + Evaluation::SCRATCH_BYTES
        + std::mem::size_of::<[u8; MAX_COMPUTED]>()) as u64;

pub(super) struct RowValues<'a, 'query, F> {
    plan: &'a Pipeline<'query>,
    raw: F,
    partition_count: Option<u64>,
    // No heap owner or retained replay state. None keeps direct projections on
    // their existing path; the bounded cache is initialized on first computation.
    cache: Option<[Option<Option<u64>>; MAX_COMPUTED]>,
}

impl<'a, 'query, F> RowValues<'a, 'query, F> {
    pub(super) fn new(plan: &'a Pipeline<'query>, raw: F) -> Self {
        Self {
            plan,
            raw,
            partition_count: None,
            cache: None,
        }
    }

    pub(super) fn with_partition_count(mut self, count: u64) -> Self {
        self.partition_count = Some(count);
        self
    }

    pub(super) fn value<'row>(&mut self, slot: u8) -> Result<Value<'row>, Error>
    where
        F: FnMut(u8) -> Result<Value<'row>, Error>,
        'query: 'row,
    {
        if usize::from(slot) < MAX_ROW_VALUES {
            return (self.raw)(slot);
        }
        let target = usize::from(slot) - MAX_ROW_VALUES;
        let definition = self
            .plan
            .computed
            .get(target)
            .ok_or(Error::Corrupt("computed value slot"))?;
        if let Computation::Constant(value) = &definition.expression {
            return Ok(value.value());
        }
        if let Computation::ByteLength(column) = definition.expression {
            return number(
                byte_length(self.plan, column, &mut self.raw)?,
                DataType::Int64,
            );
        }
        if self.plan.has_conditional_work() {
            return self.conditional_value(target);
        }
        let cache = self.cache.get_or_insert([None; MAX_COMPUTED]);
        let needed = self.plan.dependencies(&[slot])?;
        for (index, needed) in needed[MAX_ROW_VALUES..=MAX_ROW_VALUES + target]
            .iter()
            .enumerate()
        {
            if !needed || cache[index].is_some() {
                continue;
            }
            let current = &self.plan.computed[index];
            if matches!(current.expression, Computation::Constant(_)) {
                // BYTE_LENGTH reads its owned text constant directly. That
                // dependency has no numeric program or cached numeric payload.
                continue;
            }
            if let Computation::ByteLength(column) = current.expression {
                cache[index] = Some(byte_length(self.plan, column, &mut self.raw)?);
                continue;
            }
            if matches!(current.expression, Computation::WindowCount) {
                let count = self
                    .partition_count
                    .ok_or(Error::Corrupt("analytic count outside its producer"))?;
                let count = i64::try_from(count)
                    .map_err(|_| Error::Corrupt("analytic count exceeds INT64"))?;
                cache[index] = Some(Some(count as u64));
                continue;
            }
            let expression = current.expression.numeric()?;
            let mut columns = [None; MAX_OPS];
            let mut bits = [[0_u64; 1]; MAX_OPS];
            let mut valid = [[0_u64; 1]; MAX_OPS];
            let mut count = 0;
            for op in &expression.ops[..usize::from(expression.len)] {
                let Op::Column(column) = *op else {
                    continue;
                };
                if columns[..count].contains(&Some(column)) {
                    continue;
                }
                let slot = self.plan.slots[column.identity().value() as usize];
                let value = if usize::from(slot) < MAX_ROW_VALUES {
                    (self.raw)(slot)?
                } else {
                    let bits = cache
                        .get(usize::from(slot) - MAX_ROW_VALUES)
                        .copied()
                        .flatten()
                        .ok_or(Error::Corrupt("computed dependency not evaluated"))?;
                    number(bits, column.data_type())?
                };
                match (value, column.data_type()) {
                    (Value::Null, _) => (),
                    (Value::Int64(value), DataType::Int64) => {
                        bits[count][0] = u64::from_ne_bytes(value.to_ne_bytes());
                        valid[count][0] = 1;
                    }
                    (Value::Double(value), DataType::Double) => {
                        bits[count][0] = value.to_bits();
                        valid[count][0] = 1;
                    }
                    _ => return Err(Error::Corrupt("computed input type")),
                }
                columns[count] = Some(column);
                count += 1;
            }
            let mut inputs = [None; MAX_OPS];
            for index in 0..count {
                let column = columns[index].expect("populated scalar input");
                let values = NumericValues::Bits {
                    values: &bits[index],
                    kind: column.data_type(),
                };
                inputs[index] = Some(NumericInput::new(column, values, Some(&valid[index]))?);
            }
            let mut scratch = [0; MAX_OPS];
            let output = expression
                .evaluate_batch(&inputs[..count], 0..1, &mut scratch)
                .map_err(|failure| failure.into_error(current.span))?;
            cache[index] = Some(output.value(0));
        }
        number(
            cache[target].ok_or(Error::Corrupt("computed value absent"))?,
            definition.column.data_type(),
        )
    }

    fn conditional_value<'row>(&mut self, target: usize) -> Result<Value<'row>, Error>
    where
        F: FnMut(u8) -> Result<Value<'row>, Error>,
        'query: 'row,
    {
        let cache = self.cache.get_or_insert([None; MAX_COMPUTED]);
        let mut pending = [0_u8; MAX_COMPUTED];
        pending[0] = target as u8;
        let mut depth = 1;
        while depth != 0 {
            let index = usize::from(pending[depth - 1]);
            if cache[index].is_some() {
                depth -= 1;
                continue;
            }
            let definition = &self.plan.computed[index];
            if let Computation::ByteLength(column) = definition.expression {
                cache[index] = Some(byte_length(self.plan, column, &mut self.raw)?);
                depth -= 1;
                continue;
            }
            if matches!(definition.expression, Computation::WindowCount) {
                let count = self
                    .partition_count
                    .ok_or(Error::Corrupt("analytic count outside its producer"))?;
                cache[index] = Some(Some(
                    i64::try_from(count)
                        .map_err(|_| Error::Corrupt("analytic count exceeds INT64"))?
                        as u64,
                ));
                continue;
            }
            let expression = definition.expression.numeric()?;
            let mut evaluation = Evaluation::new(expression);
            let mut dependency = None;
            while let Some(column) = evaluation
                .next_column()
                .map_err(|failure| failure.into_error(definition.span))?
            {
                let slot = usize::from(self.plan.slots[column.identity().value() as usize]);
                let value = if slot < MAX_ROW_VALUES {
                    (self.raw)(slot as u8)?
                } else {
                    let input = slot - MAX_ROW_VALUES;
                    if input >= index {
                        return Err(Error::Corrupt(
                            "computed dependency must precede definition",
                        ));
                    }
                    if let Some(bits) = cache[input] {
                        number(bits, column.data_type())?
                    } else {
                        dependency = Some(input);
                        break;
                    }
                };
                evaluation.supply(match (value, column.data_type()) {
                    (Value::Null, _) => Number::Null,
                    (Value::Int64(value), DataType::Int64) => Number::Integer(value),
                    (Value::Double(value), DataType::Double) => Number::Double(value),
                    _ => return Err(Error::Corrupt("computed input type")),
                })?;
            }
            if let Some(input) = dependency {
                // Each descent names an earlier definition. Once it is cached,
                // retry this bounded program; no expression or input is recursive.
                pending[depth] = input as u8;
                depth += 1;
                continue;
            }
            cache[index] = Some(match evaluation.value() {
                Number::Null => None,
                Number::Integer(value) => Some(value as u64),
                Number::Double(value) => Some(value.to_bits()),
            });
        }
        number(
            cache[target].expect("completed computed target"),
            self.plan.computed[target].column.data_type(),
        )
    }

    pub(super) fn retains<'row>(&mut self) -> Result<bool, Error>
    where
        F: FnMut(u8) -> Result<Value<'row>, Error>,
        'query: 'row,
    {
        let mut index = 0;
        for _ in 0..self.plan.filter_count {
            if index == self.plan.filter_count {
                return Ok(true);
            }
            let filter = self.plan.filters[index];
            let offset = if filter.matches(self.value(filter.column)?)? {
                filter.control.matched
            } else {
                filter.control.other
            };
            if offset == 0 {
                return Ok(false);
            }
            index += usize::from(offset);
            if index > self.plan.filter_count {
                return Err(Error::Corrupt("filter decision outside pipeline"));
            }
        }
        Ok(true)
    }
}

fn byte_length<'row>(
    plan: &Pipeline<'_>,
    column: SemanticColumn,
    mut raw: impl FnMut(u8) -> Result<Value<'row>, Error>,
) -> Result<Option<u64>, Error> {
    let slot = plan.slots[column.identity().value() as usize];
    let value = if usize::from(slot) < MAX_ROW_VALUES {
        raw(slot)?
    } else {
        // Copies map back to their input. The only computed STRING producer
        // within this pipeline is an owned constant; materialized text is raw.
        let definition = plan
            .computed
            .get(usize::from(slot) - MAX_ROW_VALUES)
            .ok_or(Error::Corrupt("byte-length input slot"))?;
        let Computation::Constant(value) = &definition.expression else {
            return Err(Error::Corrupt("byte-length input is not stored text"));
        };
        value.value()
    };
    match value {
        Value::Null if column.nullable() => Ok(None),
        Value::String(value) => i64::try_from(value.as_str().len())
            .map(|length| Some(length as u64))
            .map_err(|_| Error::Corrupt("STRING byte length exceeds INT64")),
        _ => Err(Error::Corrupt("byte-length input type or nullability")),
    }
}

fn number<'row>(bits: Option<u64>, kind: DataType) -> Result<Value<'row>, Error> {
    Ok(match (bits, kind) {
        (None, _) => Value::Null,
        (Some(bits), DataType::Int64) => Value::Int64(i64::from_ne_bytes(bits.to_ne_bytes())),
        (Some(bits), DataType::Double) => Value::Double(f64::from_bits(bits)),
        _ => return Err(Error::Corrupt("computed output type")),
    })
}

impl Pipeline<'_> {
    pub(super) fn raw_demand(&self) -> Result<u64, Error> {
        let mut demand = self.raw_columns(&self.columns[..self.column_count])?;
        for filter in &self.filters[..self.filter_count] {
            demand |= self.raw_columns(&[filter.column])?;
        }
        Ok(demand)
    }

    fn dependencies(&self, slots: &[u8]) -> Result<[bool; MAX_ROW_VALUES + MAX_COMPUTED], Error> {
        let mut needed = [false; MAX_ROW_VALUES + MAX_COMPUTED];
        for &slot in slots {
            *needed
                .get_mut(usize::from(slot))
                .ok_or(Error::Corrupt("computed slot bound"))? = true;
        }
        for index in (0..self.computed.len()).rev() {
            if !needed[MAX_ROW_VALUES + index] {
                continue;
            }
            for column in self.computed[index].expression.columns() {
                let slot = usize::from(self.slots[column.identity().value() as usize]);
                if slot >= MAX_ROW_VALUES + index {
                    return Err(Error::Corrupt(
                        "computed dependency must precede definition",
                    ));
                }
                needed[slot] = true;
            }
        }
        Ok(needed)
    }

    pub(super) fn raw_columns(&self, slots: &[u8]) -> Result<u64, Error> {
        if slots.iter().all(|slot| usize::from(*slot) < MAX_COLUMNS) {
            return Ok(slots.iter().fold(0, |mask, slot| mask | (1_u64 << slot)));
        }
        let needed = self.dependencies(slots)?;
        if needed[MAX_COLUMNS..MAX_ROW_VALUES]
            .iter()
            .any(|needed| *needed)
        {
            return Err(Error::Corrupt("scan dependency outside source schema"));
        }
        Ok(needed[..MAX_COLUMNS]
            .iter()
            .enumerate()
            .fold(0, |mask, (slot, needed)| {
                mask | (u64::from(*needed) << slot)
            }))
    }

    fn has_conditional_work(&self) -> bool {
        self.computed.iter().any(|definition| {
            matches!(&definition.expression, Computation::Numeric(expression) if expression.requires_ordered_evaluation())
        })
    }

    pub(super) fn has_computed_work(&self) -> bool {
        self.has_computed_outputs()
            || self.filters[..self.filter_count]
                .iter()
                .any(|filter| usize::from(filter.column) >= MAX_ROW_VALUES)
    }

    pub(super) fn has_computed_outputs(&self) -> bool {
        self.columns[..self.column_count]
            .iter()
            .any(|column| usize::from(*column) >= MAX_ROW_VALUES)
    }
}

const SLOTS: usize = MAX_ROW_VALUES + MAX_COMPUTED;
const ROWS: usize = crate::scalar::MAX_ROWS;
const WORDS_PER_COLUMN: usize = ROWS + ROWS / 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BatchLayout {
    mapping: [u8; SLOTS],
    buffers: u8,
    depth: u8,
}

impl BatchLayout {
    pub(super) const EMPTY: Self = Self {
        mapping: [u8::MAX; SLOTS],
        buffers: 0,
        depth: 0,
    };
    pub(super) const MAX_BYTES: u64 =
        ((SLOTS * WORDS_PER_COLUMN + MAX_OPS * ROWS) * 8 + 4096) as u64;
    pub(super) fn new(plan: &Pipeline) -> Result<Self, Error> {
        let mut targets = [0; MAX_ROW_VALUES + crate::frontend::MAX_STAGES];
        let mut count = 0;
        for slot in plan.columns[..plan.column_count].iter().copied().chain(
            plan.filters[..plan.filter_count]
                .iter()
                .map(|filter| filter.column),
        ) {
            if usize::from(slot) >= MAX_ROW_VALUES {
                targets[count] = slot;
                count += 1;
            }
        }
        let mut needed = plan.dependencies(&targets[..count])?;
        // Length borrows checked text directly; numeric scratch stores only its
        // result. A validated STRING input cannot also be a numeric-kernel input.
        for (index, definition) in plan.computed.iter().enumerate() {
            if needed[MAX_ROW_VALUES + index]
                && let Computation::ByteLength(column) = definition.expression
            {
                let slot = usize::from(plan.slots[column.identity().value() as usize]);
                if slot < MAX_ROW_VALUES {
                    needed[slot] = false;
                }
            }
        }
        let mut layout = Self::EMPTY;
        for (slot, needed) in needed.iter().enumerate() {
            if *needed {
                if slot >= MAX_ROW_VALUES
                    && matches!(
                        plan.computed[slot - MAX_ROW_VALUES].expression,
                        Computation::Constant(_)
                    )
                {
                    continue;
                }
                layout.mapping[slot] = layout.buffers;
                layout.buffers += 1;
                if slot >= MAX_ROW_VALUES {
                    match &plan.computed[slot - MAX_ROW_VALUES].expression {
                        Computation::ByteLength(_) => (),
                        computation => {
                            let depth = computation.numeric()?.stack_depth();
                            layout.depth = layout.depth.max(depth as u8);
                        }
                    }
                }
            }
        }
        Ok(layout)
    }

    fn words(self) -> usize {
        usize::from(self.buffers) * WORDS_PER_COLUMN + usize::from(self.depth) * ROWS
    }

    pub(super) fn bytes(self) -> u64 {
        if self.buffers == 0 {
            0
        } else {
            (self.words() * 8 + 4096) as u64
        }
    }
}

pub(super) struct BatchScratch {
    layout: BatchLayout,
    data: Vec<u64>,
    rows: usize,
    ready: [bool; SLOTS],
}

impl BatchScratch {
    // The enclosing scan reservation owns this allocation; it drops after data.
    pub(super) fn new(layout: BatchLayout) -> Result<Self, Error> {
        let mut data = super::allocate(
            layout.words(),
            layout.words(),
            "computed batch workspace",
            layout.bytes(),
        )?;
        data.resize(layout.words(), 0);
        Ok(Self {
            layout,
            data,
            rows: 0,
            ready: [false; SLOTS],
        })
    }

    pub(super) fn invalidate(&mut self) {
        self.rows = 0;
        self.ready.fill(false);
    }

    pub(super) fn evaluate<'row>(
        &mut self,
        plan: &Pipeline,
        targets: &[u8],
        selection: &[u32],
        mut raw: impl FnMut(u8, usize) -> Result<Value<'row>, Error>,
    ) -> Result<(), Error> {
        if selection.is_empty() || selection.len() > ROWS {
            return Err(Error::Corrupt("computed batch extent"));
        }
        self.rows = selection.len();
        self.ready.fill(false);
        if plan.has_conditional_work() {
            return self.evaluate_conditional(plan, targets, selection, raw);
        }
        let mut computed = [0; MAX_ROW_VALUES];
        let mut count = 0;
        for &slot in targets {
            if usize::from(slot) >= MAX_ROW_VALUES {
                computed[count] = slot;
                count += 1;
            }
        }
        let needed = plan.dependencies(&computed[..count])?;
        let (data, scratch) = self
            .data
            .split_at_mut(usize::from(self.layout.buffers) * WORDS_PER_COLUMN);
        for index in 0..plan.computed.len() {
            let slot = MAX_ROW_VALUES + index;
            if !needed[slot] {
                continue;
            }
            let definition = &plan.computed[index];
            if matches!(definition.expression, Computation::Constant(_)) {
                self.ready[slot] = true;
                continue;
            }
            if let Computation::ByteLength(column) = definition.expression {
                let offset = usize::from(self.layout.mapping[slot]) * WORDS_PER_COLUMN;
                let (values, validity) = data
                    .get_mut(offset..offset + WORDS_PER_COLUMN)
                    .ok_or(Error::Corrupt("byte-length output buffer mapping"))?
                    .split_at_mut(ROWS);
                validity.fill(0);
                for (lane, &row) in selection.iter().enumerate() {
                    let value = byte_length(plan, column, |slot| raw(slot, row as usize))?;
                    values[lane] = value.unwrap_or(0);
                    if value.is_some() {
                        validity[lane / 64] |= 1 << (lane % 64);
                    }
                }
                self.ready[slot] = true;
                continue;
            }
            let expression = definition.expression.numeric()?;
            // Gather each source dependency once for this selection. No I/O occurs
            // here: the scan loaded these checked payloads in earlier steps.
            for op in &expression.ops[..usize::from(expression.len)] {
                let Op::Column(column) = op else {
                    continue;
                };
                let input = usize::from(plan.slots[column.identity().value() as usize]);
                if self.ready[input] {
                    continue;
                }
                if input >= MAX_ROW_VALUES {
                    return Err(Error::Corrupt("computed batch dependency not ready"));
                }
                let offset = usize::from(self.layout.mapping[input]) * WORDS_PER_COLUMN;
                let (values, validity) = data
                    .get_mut(offset..offset + WORDS_PER_COLUMN)
                    .ok_or(Error::Corrupt("computed raw buffer mapping"))?
                    .split_at_mut(ROWS);
                validity.fill(0);
                for (lane, &row) in selection.iter().enumerate() {
                    match (raw(input as u8, row as usize)?, column.data_type()) {
                        (Value::Null, _) => values[lane] = 0,
                        (Value::Int64(value), DataType::Int64) => {
                            values[lane] = u64::from_ne_bytes(value.to_ne_bytes());
                            validity[lane / 64] |= 1 << (lane % 64);
                        }
                        (Value::Double(value), DataType::Double) => {
                            values[lane] = value.to_bits();
                            validity[lane / 64] |= 1 << (lane % 64);
                        }
                        _ => return Err(Error::Corrupt("computed batch input type")),
                    }
                }
                self.ready[input] = true;
            }
            let mut inputs = [None; MAX_OPS];
            let mut columns = [None; MAX_OPS];
            let mut input_count = 0;
            for op in &expression.ops[..usize::from(expression.len)] {
                let Op::Column(column) = *op else {
                    continue;
                };
                if columns[..input_count].contains(&Some(column)) {
                    continue;
                }
                let input = usize::from(plan.slots[column.identity().value() as usize]);
                let offset = usize::from(self.layout.mapping[input]) * WORDS_PER_COLUMN;
                let buffer = data
                    .get(offset..offset + WORDS_PER_COLUMN)
                    .ok_or(Error::Corrupt("computed buffer mapping"))?;
                inputs[input_count] = Some(NumericInput::new(
                    column,
                    NumericValues::Bits {
                        values: &buffer[..self.rows],
                        kind: column.data_type(),
                    },
                    Some(&buffer[ROWS..ROWS + self.rows.div_ceil(64)]),
                )?);
                columns[input_count] = Some(column);
                input_count += 1;
            }
            let output = expression
                .evaluate_batch(&inputs[..input_count], 0..self.rows, scratch)
                .map_err(|failure| failure.into_error(definition.span))?;
            let offset = usize::from(self.layout.mapping[slot]) * WORDS_PER_COLUMN;
            let (values, validity) = data
                .get_mut(offset..offset + WORDS_PER_COLUMN)
                .ok_or(Error::Corrupt("computed output buffer mapping"))?
                .split_at_mut(ROWS);
            validity.fill(0);
            for (lane, value) in values[..self.rows].iter_mut().enumerate() {
                *value = output.value(lane).unwrap_or(0);
                if output.value(lane).is_some() {
                    validity[lane / 64] |= 1 << (lane % 64);
                }
            }
            self.ready[slot] = true;
        }
        Ok(())
    }

    fn evaluate_conditional<'row>(
        &mut self,
        plan: &Pipeline,
        targets: &[u8],
        selection: &[u32],
        mut raw: impl FnMut(u8, usize) -> Result<Value<'row>, Error>,
    ) -> Result<(), Error> {
        for (lane, &row) in selection.iter().enumerate() {
            let mut values = RowValues::new(plan, |slot| raw(slot, row as usize));
            for &slot in targets {
                if usize::from(slot) < MAX_ROW_VALUES {
                    continue;
                }
                let value = values.value(slot)?;
                if matches!(
                    plan.computed[usize::from(slot) - MAX_ROW_VALUES].expression,
                    Computation::Constant(_)
                ) {
                    self.ready[usize::from(slot)] = true;
                    continue;
                }
                let offset = usize::from(self.layout.mapping[usize::from(slot)]) * WORDS_PER_COLUMN;
                let mask = 1 << (lane % 64);
                self.data[offset + ROWS + lane / 64] &= !mask;
                self.data[offset + lane] = match value {
                    Value::Null => 0,
                    Value::Int64(value) => {
                        self.data[offset + ROWS + lane / 64] |= mask;
                        value as u64
                    }
                    Value::Double(value) => {
                        self.data[offset + ROWS + lane / 64] |= mask;
                        value.to_bits()
                    }
                    _ => return Err(Error::Corrupt("computed batch output type")),
                };
                self.ready[usize::from(slot)] = true;
            }
        }
        Ok(())
    }

    pub(super) fn value<'query>(
        &self,
        plan: &Pipeline<'query>,
        slot: u8,
        row: usize,
    ) -> Result<Value<'query>, Error> {
        let slot = usize::from(slot);
        if row >= self.rows || !(MAX_ROW_VALUES..SLOTS).contains(&slot) || !self.ready[slot] {
            return Err(Error::Corrupt("computed batch value is not ready"));
        }
        if let Computation::Constant(value) = &plan.computed[slot - MAX_ROW_VALUES].expression {
            return Ok(value.value());
        }
        let offset = usize::from(self.layout.mapping[slot]) * WORDS_PER_COLUMN;
        let bits = (self.data[offset + ROWS + row / 64] & (1 << (row % 64)) != 0)
            .then_some(self.data[offset + row]);
        number(
            bits,
            plan.computed[slot - MAX_ROW_VALUES].column.data_type(),
        )
    }
}

const _: () = assert!(MAX_ROW_VALUES + MAX_COMPUTED < u8::MAX as usize);
