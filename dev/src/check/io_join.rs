//! Judge native join I/O by complete typed pairs and separate API outcomes.
//!
//! Input ranges define the relation independently of engine execution. Every
//! wanted pair occurs once; NULL keys never match. A validated retry bounds a
//! failed attempt's byte prefix without requiring public row-order guarantees.

use super::*;
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::path::Path;

#[derive(Clone, Copy, PartialEq)]
enum Keys {
    Integer,
    Double,
    Date,
    String,
}

struct Answers {
    left: bool,
    text: bool,
    keys: Keys,
}
impl Answers {
    fn new(mode: &str) -> Result<Self> {
        match mode {
            "join-inner" => Ok(Self {
                left: false,
                text: true,
                keys: Keys::Integer,
            }),
            "join-left" => Ok(Self {
                left: true,
                text: true,
                keys: Keys::Integer,
            }),
            "join-numeric" => Ok(Self {
                left: true,
                text: false,
                keys: Keys::Integer,
            }),
            "join-double" | "join-double-numeric" | "join-date" | "join-date-numeric" => Ok(Self {
                left: true,
                text: !mode.ends_with("numeric"),
                keys: if mode.starts_with("join-double") {
                    Keys::Double
                } else {
                    Keys::Date
                },
            }),
            "join-string" | "join-string-numeric" => Ok(Self {
                left: true,
                text: mode == "join-string",
                keys: Keys::String,
            }),
            _ => Err("unknown join I/O answers".into()),
        }
    }
    fn rows(&self) -> usize {
        if self.keys == Keys::Integer {
            2 * 16 * 48 + 2 * 16 + if self.left { 6 } else { 0 }
        } else if self.keys == Keys::String {
            if self.text {
                16 * 48 + 2 * 8 * 24 + 2 * 16 + 2 + 4
            } else {
                2 * 513 + 128 + 64 + 2 * 32 + 2 + 32
            }
        } else if self.text {
            16 * 48 + 2 * 8 * 24 + 2 * 16 + 6
        } else {
            2 * 2048 + 1024 + 512 + 2 * 256 + 34
        }
    }
    fn right_rows(&self) -> usize {
        if self.keys == Keys::Integer {
            130
        } else if self.keys == Keys::String && !self.text {
            767
        } else if self.text {
            131
        } else {
            4096
        }
    }
    fn schema(&self) -> Value {
        let mut columns = vec![
            json!({"name":"left_id","type":"int64","nullable":false}),
            json!({"name":"right_id","type":"int64","nullable":self.left}),
            json!({"name":"left_value","type":"int64","nullable":true}),
            json!({"name":"right_value","type":"int64","nullable":true}),
        ];
        if self.text {
            columns.extend([
                json!({"name":"left_note","type":"string","nullable":false}),
                json!({"name":if self.keys == Keys::String {"right_key"} else {"right_note"},"type":"string","nullable":self.left}),
            ]);
        }
        json!({"format":"pipesql-jsonl","version":1,"columns":columns})
    }
    fn right(&self, left: usize) -> std::ops::Range<usize> {
        if self.keys == Keys::String {
            if left == 1 {
                return 1..2;
            }
            if left == 2 {
                return 2..3;
            }
            if !self.text {
                return match left {
                    4..6 => 3..516,
                    20 => 516..644,
                    28 => 644..708,
                    36..38 => 708..740,
                    _ => 0..0,
                };
            }
        }
        if self.keys == Keys::Integer {
            match left {
                4..20 => 2..50,
                20..36 => 50..98,
                36..38 => 98..114,
                _ => 0..0,
            }
        } else if self.text {
            match left {
                4..20 => 3..51,
                20..28 => 51..75,
                28..36 => 75..99,
                36..38 => 99..115,
                _ => 0..0,
            }
        } else {
            match left {
                4..6 => 3..2051,
                20 => 2051..3075,
                28 => 3075..3587,
                36..38 => 3587..3843,
                _ => 0..0,
            }
        }
    }
    fn wanted(&self, left: usize, right: Option<usize>) -> bool {
        left < 40
            && match right {
                Some(right) => self.right(left).contains(&right),
                None => self.left && self.right(left).is_empty(),
            }
    }
    fn row(&self, left: usize, right: Option<usize>) -> Value {
        let mut cells = vec![
            json!(left.to_string()),
            json!(right.map(|id| id.to_string())),
            json!((!left.is_multiple_of(3)).then(|| (left as i64 * 7 - 30).to_string())),
            json!(
                right
                    .filter(|id| id % 5 != 0)
                    .map(|id| (id as i64 - 70).to_string())
            ),
        ];
        if self.text {
            cells.push(json!(format!("L{left:04}雪\0xxxxxxx")));
            cells.push(if self.keys == Keys::String {
                json!(right.and_then(|id| self.source_string(false, id)))
            } else {
                json!(right.map(|id| format!("R{id:04}雪\0{}", "x".repeat(2039))))
            });
        }
        json!({"row":cells})
    }
    fn pair(record: &Value) -> Result<(usize, Option<usize>)> {
        let left = record["row"][0]
            .as_str()
            .ok_or("missing typed left identity")?
            .parse()?;
        let right = match &record["row"][1] {
            Value::Null => None,
            value => Some(
                value
                    .as_str()
                    .ok_or("invalid typed right identity")?
                    .parse()?,
            ),
        };
        Ok((left, right))
    }
    fn complete(&self, bytes: &[u8]) -> Result<()> {
        let records = records(bytes)?;
        if records.len() != self.rows() + 2
            || records[0] != self.schema()
            || records.last() != Some(&json!({"complete":true,"rows":self.rows()}))
        {
            return Err("join I/O schema, row count or completion differs".into());
        }
        // The final slot represents the unmatched left row, distinct from right 0.
        let stride = self.right_rows() + 1;
        let mut seen = vec![false; 40 * stride];
        for record in &records[1..records.len() - 1] {
            let (left, right) = Self::pair(record)?;
            if !self.wanted(left, right) || *record != self.row(left, right) {
                return Err("join I/O differs from independent typed pair answers".into());
            }
            let slot = left * stride + right.unwrap_or(self.right_rows());
            if std::mem::replace(&mut seen[slot], true) {
                return Err("join I/O repeated a pair".into());
            }
        }
        for left in 0..40 {
            for right in (0..self.right_rows()).map(Some).chain([None]) {
                if seen[left * stride + right.unwrap_or(self.right_rows())]
                    != self.wanted(left, right)
                {
                    return Err("join I/O omitted a wanted pair".into());
                }
            }
        }
        Ok(())
    }
    fn streams(&self, initial: &[u8], retry: &[u8], finished: bool) -> Result<()> {
        self.complete(retry)?;
        if finished {
            self.complete(initial)?;
        } else if initial.len() >= retry.len() {
            return Err("failed join I/O produced a complete stream".into());
        }
        // Failure may stop within a record or a UTF-8 character.
        if !retry.starts_with(initial) {
            return Err("join I/O bytes differ from the validated retry prefix".into());
        }
        Ok(())
    }
    fn controls(&self, bytes: &[u8]) -> Result<usize> {
        self.complete(bytes)?;
        let valid = records(bytes)?;
        let matched = valid.iter().position(|r| r["row"][1].is_string()).unwrap();
        let nullable = valid
            .iter()
            .position(|r| r.get("row").is_some() && r["row"][2].is_null())
            .unwrap();
        let cases = if self.text { 8 } else { 7 };
        for case in 0..cases {
            let mut wrong = valid.clone();
            match case {
                // Every cell describes a real input row, but these keys differ.
                0 => wrong[matched] = self.row(4, Some(self.right(4).end)),
                1 => wrong[nullable]["row"][2] = json!("0"),
                2 => wrong[matched]["row"][3] = json!("12345"),
                3 => {
                    wrong.remove(matched);
                }
                4 => wrong[matched + 1] = wrong[matched].clone(),
                5 => {
                    wrong.pop();
                }
                6 => wrong[0]["columns"][1]["nullable"] = json!(!self.left),
                _ => {
                    let mut note = wrong[matched]["row"][5].as_str().unwrap().to_string();
                    note.pop();
                    note.push('y');
                    wrong[matched]["row"][5] = json!(note);
                }
            }
            if self.complete(&encode(&wrong)?).is_ok() {
                return Err(format!("join I/O answer control {case} was accepted").into());
            }
        }
        let mut prefix = bytes[..1024].to_vec();
        self.streams(&prefix, bytes, false)?;
        prefix[0] ^= 1;
        if self.streams(&prefix, bytes, false).is_ok()
            || self.streams(bytes, bytes, false).is_ok()
            || self.streams(&bytes[..1024], bytes, true).is_ok()
        {
            return Err("join I/O corrupt prefix or false completion was accepted".into());
        }
        let false_pairs = match self.keys {
            Keys::Integer => vec![],
            Keys::Double => vec![
                (0, Some(0)),
                (1, Some(1)), // Identical NaN bits still cannot match.
                (2, Some(2)), // Distinct NaNs cannot match each other either.
                (3, Some(self.right_rows() - 1)), // Opposite subnormals.
            ],
            Keys::Date => vec![(0, Some(0)), (20, Some(self.right(28).start))],
            Keys::String => vec![
                (0, Some(0)),
                (0, Some(1)), // NULL is different from present empty text.
                (1, Some(0)),
                (3, Some(2)), // The byte after NUL still decides equality.
                (38, Some(3)),
                (4, Some(self.right(20).start)), // Shared prefix, different last byte.
                (4, Some(if self.text { 115 } else { 740 })),
                (2, Some(if self.text { 117 } else { 742 })),
                (28, Some(self.right(36).start)), // No Unicode normalization.
            ],
        };
        for &(left, right) in &false_pairs {
            let mut wrong = valid.clone();
            wrong[matched] = self.row(left, right);
            if self.complete(&encode(&wrong)?).is_ok() {
                return Err("join I/O accepted a false special-key pair".into());
            }
        }
        Ok(cases + 3 + false_pairs.len())
    }

    // Original source bits/days/bytes are a separate premise from SQL pair answers.
    // These literals deliberately include equal NaN bits and opposite zero
    // signs; converting every source to integers or NULLs cannot pass preflight.
    fn source_cell(&self, left: bool, id: usize) -> String {
        if self.keys == Keys::String {
            return match self.source_string(left, id) {
                None => "null".into(),
                Some(value) => {
                    let mut field = String::with_capacity(4 + value.len() * 2);
                    field.push_str("hex=");
                    for byte in value.bytes() {
                        write!(field, "{byte:02x}").unwrap();
                    }
                    field
                }
            };
        }
        let group = if left {
            match id {
                4..20 if self.text => Some(0),
                20..28 if self.text => Some(1),
                28..36 if self.text => Some(2),
                4..6 if !self.text => Some(0),
                20 if !self.text => Some(1),
                28 if !self.text => Some(2),
                36..38 => Some(3),
                _ => None,
            }
        } else {
            let ranges = if self.text {
                [3..51, 51..75, 75..99, 99..115]
            } else {
                [3..2051, 2051..3075, 3075..3587, 3587..3843]
            };
            ranges.iter().position(|range| range.contains(&id))
        };
        if self.keys == Keys::Date {
            return if id < 2 {
                "null".to_string()
            } else {
                match group {
                    Some(0) => "0",
                    Some(1) => "-719162",
                    Some(2) => "2932896",
                    Some(3) => "-1",
                    _ if left => "1",
                    _ => "2",
                }
                .to_string()
            };
        }
        let negative_zero = if left {
            if self.text {
                id.is_multiple_of(3)
            } else {
                id == 5
            }
        } else {
            id.is_multiple_of(5)
        };
        match id {
            0 => "null".to_string(),
            1 => "7ff8000000000042".to_string(),
            2 if left => "7ff8000000000099".to_string(),
            2 => "fff8000000000007".to_string(),
            _ => match group {
                Some(0) if negative_zero => "8000000000000000",
                Some(0) => "0000000000000000",
                Some(1) => "7ff0000000000000",
                Some(2) => "7fefffffffffffff",
                Some(3) => "fff0000000000000",
                _ if left && id == 3 => "0000000000000001",
                _ if left => "3ff0000000000000",
                _ if id == self.right_rows() - 1 => "8000000000000001",
                _ if id == if self.text { 115 } else { 3843 } => "ffefffffffffffff",
                _ => "4000000000000000",
            }
            .to_string(),
        }
    }
    // This source-byte premise is separate from the literal pair ranges above.
    // Empty text has a present hex= marker; NUL is encoded as 00, never a terminator.
    fn source_string(&self, left: bool, id: usize) -> Option<String> {
        if id == 0 {
            return None;
        }
        if id == 1 {
            return Some(String::new());
        }
        if id == 2 {
            return Some("a\0b".into());
        }
        let width = if self.text { 3072 } else { 512 };
        let prefix = format!("K\0{}", "x".repeat(width - 3));
        let ranges = if left {
            if self.text {
                [4..20, 20..28, 28..36, 36..38]
            } else {
                [4..6, 20..21, 28..29, 36..38]
            }
        } else if self.text {
            [3..51, 51..75, 75..99, 99..115]
        } else {
            [3..516, 516..644, 644..708, 708..740]
        };
        Some(if ranges[0].contains(&id) {
            prefix + "A"
        } else if ranges[1].contains(&id) {
            prefix + "B"
        } else if ranges[2].contains(&id) {
            "é".into()
        } else if ranges[3].contains(&id) {
            "e\u{301}".into()
        } else if left {
            match id {
                3 => "a\0c".into(),
                38 => "K".into(),
                39 => prefix + "C",
                _ => "unmatched-left".into(),
            }
        } else {
            let tail = if self.text { 115 } else { 740 };
            match id - tail {
                0 => prefix,
                1 => prefix + "D",
                2 => "a\0bZ".into(),
                3 => "null".into(),
                _ => "unmatched-right".into(),
            }
        })
    }
    fn source(&self, bytes: &str) -> Result<()> {
        if self.keys == Keys::Integer || bytes.len() > 1_000_000 || !bytes.ends_with('\n') {
            return Err("invalid join key preflight".into());
        }
        let mut lines = bytes.lines();
        for (left, name, count) in [
            (true, "left_rows", 40),
            (false, "right_rows", self.right_rows()),
        ] {
            let kind = match self.keys {
                Keys::Double => "Double",
                Keys::Date => "Date",
                Keys::String => "String",
                Keys::Integer => unreachable!(),
            };
            if lines.next() != Some(format!("source {name} type={kind} rows={count}").as_str()) {
                return Err("join source key type or row count differs".into());
            }
            let mut seen = vec![false; count];
            for _ in 0..count {
                let fields = lines
                    .next()
                    .ok_or("missing source key row")?
                    .split_whitespace()
                    .collect::<Vec<_>>();
                if fields.len() != 3 || fields[0] != name {
                    return Err("join source key record differs".into());
                }
                let id: usize = fields[1].parse()?;
                if id >= count
                    || std::mem::replace(&mut seen[id], true)
                    || fields[2] != self.source_cell(left, id)
                {
                    return Err("join original key bits/days or multiplicity differs".into());
                }
            }
            if !seen.iter().all(|present| *present) {
                return Err("join source key omitted a row".into());
            }
        }
        if lines.next().is_some() {
            return Err("extra join source key record".into());
        }
        Ok(())
    }
    fn source_controls(&self, bytes: &str) -> Result<usize> {
        self.source(bytes)?;
        let left_negative = if self.text { 6 } else { 5 };
        let cases = if self.keys == Keys::String {
            vec![
                bytes.replacen("type=String", "type=Int64", 1),
                bytes.replace("left_rows 0 null\n", "left_rows 0 hex=\n"),
                bytes.replace("left_rows 1 hex=\n", "left_rows 1 null\n"),
                bytes.replace("left_rows 2 hex=610062\n", "left_rows 2 hex=61\n"),
                bytes.replace(
                    &format!("left_rows 4 {}\n", self.source_cell(true, 4)),
                    &format!("left_rows 4 {}\n", self.source_cell(true, 20)),
                ),
                bytes.replace("left_rows 28 hex=c3a9\n", "left_rows 28 hex=65cc81\n"),
                bytes.replace("right_rows 1 hex=\n", "left_rows 1 hex=\n"),
            ]
        } else if self.keys == Keys::Double {
            vec![
                bytes.replacen("type=Double", "type=Int64", 1),
                bytes.replace("left_rows 1 7ff8000000000042", "left_rows 1 null"),
                bytes.replace(
                    "left_rows 1 7ff8000000000042",
                    "left_rows 1 7ff8000000000099",
                ),
                bytes.replace(
                    "right_rows 5 8000000000000000",
                    "right_rows 5 0000000000000000",
                ),
                bytes.replace(
                    &format!("left_rows {left_negative} 8000000000000000"),
                    &format!("left_rows {left_negative} 0000000000000000"),
                ),
                bytes.replace(
                    "right_rows 1 7ff8000000000042",
                    "left_rows 1 7ff8000000000042",
                ),
            ]
        } else {
            vec![
                bytes.replacen("type=Date", "type=Int64", 1),
                bytes.replace("left_rows 0 null", "left_rows 0 0"),
                bytes.replace("left_rows 20 -719162", "left_rows 20 -719161"),
                bytes.replace("left_rows 28 2932896", "left_rows 28 2932895"),
            ]
        };
        for wrong in &cases {
            if wrong == bytes || self.source(wrong).is_ok() {
                return Err("join source key control was absent or accepted".into());
            }
        }
        Ok(cases.len())
    }
    fn cancelled(&self, prefix: &[u8], retry: &[u8]) -> Result<()> {
        self.streams(prefix, retry, false)?;
        let mut groups =
            std::collections::BTreeMap::<usize, std::collections::BTreeSet<usize>>::new();
        for line in prefix.split_inclusive(|byte| *byte == b'\n').skip(1) {
            if !line.ends_with(b"\n") {
                continue;
            }
            let record: Value = serde_json::from_slice(line)?;
            let (left, right) = Self::pair(&record)?;
            if self.right(left) == self.right(4)
                && let Some(right) = right
            {
                groups.entry(left).or_default().insert(right);
            }
        }
        if groups
            .values()
            .filter(|group| group.len() == self.right(4).len())
            .count()
            < 2
        {
            return Err(
                "join key cancellation missed two complete cross-buffer group traversals".into(),
            );
        }
        Ok(())
    }
}

fn records(bytes: &[u8]) -> Result<Vec<Value>> {
    if bytes.len() > 4_000_000 || !bytes.ends_with(b"\n") {
        return Err("join I/O stream is oversized or lacks its final newline".into());
    }
    Ok(std::str::from_utf8(bytes)?
        .split_terminator('\n')
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?)
}
fn encode(records: &[Value]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for record in records {
        bytes.extend(serde_json::to_vec(record)?);
        bytes.push(b'\n');
    }
    Ok(bytes)
}
fn record<'a>(output: &'a str, prefix: &str) -> Result<&'a str> {
    let mut lines = output.lines().filter_map(|line| line.strip_prefix(prefix));
    let line = lines
        .next()
        .ok_or_else(|| format!("missing {prefix} record"))?;
    if lines.next().is_some() || line.is_empty() {
        return Err("duplicate or empty join I/O record".into());
    }
    Ok(line)
}
fn number(field: Option<&str>, prefix: &str) -> Result<usize> {
    let value = field
        .and_then(|f| f.strip_prefix(prefix))
        .ok_or("missing join I/O numeric field")?;
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err("invalid join I/O numeric field".into());
    }
    Ok(value.parse()?)
}
fn spill(fields: &mut std::str::SplitWhitespace<'_>) -> Result<(usize, usize)> {
    let written = number(fields.next(), "spill=")?;
    let temporary = number(fields.next(), "temp=")?;
    if fields.next().is_some() {
        return Err("extra join I/O field".into());
    }
    Ok((written, temporary))
}
fn report(
    answers: &Answers,
    output: &str,
    initial_bytes: usize,
    at: u32,
    error: i32,
) -> Result<(usize, usize, usize, usize)> {
    if output
        .lines()
        .filter(|line| *line == "join I/O release and complete retries passed")
        .count()
        != 1
    {
        return Err("missing or duplicate join I/O completion".into());
    }
    let observed = counts(output)?;
    let finished = at == 0 || error == 0;
    if (observed[1] > 0) == finished {
        return Err("join I/O fault was disabled or its outcome differs".into());
    }
    let mut fields = record(output, "join transfers ")?.split_whitespace();
    let requested = number(fields.next(), "requested=")?;
    let transferred = number(fields.next(), "transferred=")?;
    if fields.next().is_some()
        || transferred > requested
        || (observed[0] == 0 && requested != 0)
        || (observed[0] > 0 && requested == 0)
        || (finished && observed[0] > 0 && transferred == 0)
    {
        return Err("join I/O transfer observation differs".into());
    }
    let mut fields = record(output, "join initial ")?.split_whitespace();
    if fields.next()
        != Some(if finished {
            "outcome=finished"
        } else {
            "outcome=failed"
        })
        || number(fields.next(), "bytes=")? != initial_bytes
    {
        return Err("join I/O outcome or bytes differ".into());
    }
    let (written, temporary) = spill(&mut fields)?;
    for prefix in ["join retry ", "join reopened "] {
        let mut fields = record(output, prefix)?.split_whitespace();
        if number(fields.next(), "rows=")? != answers.rows() {
            return Err("join I/O retry row count differs".into());
        }
        let (written, temporary) = spill(&mut fields)?;
        if temporary == 0 || (cfg!(target_os = "linux") && written == 0) {
            return Err("join I/O retry lacks spill observation".into());
        }
    }
    if finished && (temporary == 0 || (cfg!(target_os = "linux") && written == 0)) {
        return Err("healthy join I/O lacks spill observation".into());
    }
    Ok((written, temporary, requested, transferred))
}

pub(super) fn verify_files(
    root: &Path,
    output: &str,
    mode: &str,
    kind: u32,
    at: u32,
    error: i32,
) -> Result<Value> {
    let answers = Answers::new(mode)?;
    let source = if answers.keys != Keys::Integer {
        let bytes = fs::read_to_string(root.join("source-keys.txt"))?;
        answers.source(&bytes)?;
        Some(bytes)
    } else {
        None
    };
    let initial = fs::read(root.join("initial.jsonl"))?;
    let retry = fs::read(root.join("retry.jsonl"))?;
    answers.streams(&initial, &retry, at == 0 || error == 0)?;
    answers.complete(&fs::read(root.join("reopened.jsonl"))?)?;
    let (written, temporary, requested, transferred) =
        report(&answers, output, initial.len(), at, error)?;
    // Only scratch files are written during observation. Text and the larger
    // DOUBLE/DATE numeric inputs must show writes from intermediate sort runs.
    // Header rewrites cannot explain an extra half-file's worth of bytes beyond
    // the final retained files.
    if cfg!(target_os = "linux")
        && at == 0
        && kind == 3
        && (answers.text || answers.keys != Keys::Integer)
        && transferred <= written + written / 2
    {
        return Err("join fixture missed intermediate sort writes".into());
    }
    let mut rejected = 0;
    let mut cancelled_bytes = 0;
    if at == 0 {
        rejected = answers.controls(&retry)?;
        if report(&answers, output, initial.len(), 1, libc::EIO).is_ok() {
            return Err("join I/O disabled fault control was accepted".into());
        }
        rejected += 1;
        if answers.text || answers.keys != Keys::Integer {
            let cancelled = fs::read(root.join("cancelled.jsonl"))?;
            let mut fields = record(output, "join cancelled ")?.split_whitespace();
            if number(fields.next(), "bytes=")? != cancelled.len() || fields.next().is_some() {
                return Err("join I/O cancellation bytes differ".into());
            }
            if answers.keys != Keys::Integer {
                answers.cancelled(&cancelled, &retry)?;
            } else {
                answers.streams(&cancelled, &retry, false)?;
                let mut identities = std::collections::BTreeSet::new();
                // Only complete records are needed to show a duplicate traversal.
                for line in cancelled.split(|byte| *byte == b'\n').skip(1) {
                    if let Ok(value) = serde_json::from_slice::<Value>(line) {
                        let (left, right) = Answers::pair(&value)?;
                        if right.is_some() {
                            identities.insert(left);
                        }
                    }
                }
                if identities.len() < 2 {
                    return Err("join cancellation missed duplicate-group replay".into());
                }
            }
            cancelled_bytes = cancelled.len();
        }
    }
    let mut receipt = json!({"mode":mode,"rows":answers.rows(),"columns":if answers.text {6} else {4},
        "initial_bytes":initial.len(),"outcome":if at == 0 || error == 0 {"finished"} else {"failed"},
        "written_spill":written,"temporary_peak":temporary,"requested_bytes":requested,"transferred_bytes":transferred,
        "independent_answers":true,"answer_controls":rejected,"cancelled_bytes":cancelled_bytes,"counts":counts(output)?,
        "initial_sha256":workspace::hash(&root.join("initial.jsonl"))?,
        "retry_sha256":workspace::hash(&root.join("retry.jsonl"))?,
        "reopened_sha256":workspace::hash(&root.join("reopened.jsonl"))?});
    if let Some(source) = source {
        receipt["source_rows"] = json!(40 + answers.right_rows());
        receipt["source_controls"] = json!(if at == 0 {
            answers.source_controls(&source)?
        } else {
            0
        });
        receipt["source_sha256"] = json!(workspace::hash(&root.join("source-keys.txt"))?);
    }
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn join_io_rejects_wrong_relations_values_text_multiplicity_and_completion() {
        for mode in [
            "join-inner",
            "join-left",
            "join-numeric",
            "join-double",
            "join-date",
            "join-double-numeric",
            "join-date-numeric",
            "join-string",
            "join-string-numeric",
        ] {
            let answers = Answers::new(mode).unwrap();
            let mut values = vec![answers.schema()];
            for left in (0..40).rev() {
                for right in (0..answers.right_rows()).map(Some).chain([None]) {
                    if answers.wanted(left, right) {
                        values.push(answers.row(left, right));
                    }
                }
            }
            values.push(json!({"complete":true,"rows":answers.rows()}));
            assert_eq!(
                answers.controls(&encode(&values).unwrap()).unwrap(),
                (if answers.text { 11 } else { 10 })
                    + match answers.keys {
                        Keys::Integer => 0,
                        Keys::Double => 4,
                        Keys::Date => 2,
                        Keys::String => 9,
                    }
            );
        }
    }
    #[test]
    fn join_key_preflight_rejects_wrong_types_bits_days_and_source_identity() {
        for mode in [
            "join-double",
            "join-date",
            "join-double-numeric",
            "join-date-numeric",
        ] {
            let answers = Answers::new(mode).unwrap();
            let mut bytes = String::new();
            for (left, name, count) in [
                (true, "left_rows", 40),
                (false, "right_rows", answers.right_rows()),
            ] {
                bytes += &format!(
                    "source {name} type={} rows={count}\n",
                    if answers.keys == Keys::Double {
                        "Double"
                    } else {
                        "Date"
                    }
                );
                for id in (0..count).rev() {
                    bytes += &format!("{name} {id} {}\n", answers.source_cell(left, id));
                }
            }
            assert_eq!(
                answers.source_controls(&bytes).unwrap(),
                if answers.keys == Keys::Double { 6 } else { 4 }
            );
            assert!(
                answers
                    .source(&bytes.replace("left_rows 39 ", "left_rows 38 "))
                    .is_err()
            );
            assert!(answers.source(bytes.trim_end()).is_err());
            assert!(answers.source(&format!("{bytes}extra\n")).is_err());
            assert!(
                answers.wanted(4, Some(5)),
                "opposite zero signs/epoch must match"
            );
            assert!(!answers.wanted(0, Some(0)), "NULL cannot match");
            assert!(
                !answers.wanted(20, Some(answers.right(28).start)),
                "extreme keys differ"
            );
            if answers.keys == Keys::Double {
                assert_eq!(answers.source_cell(true, 1), answers.source_cell(false, 1));
                assert!(
                    !answers.wanted(1, Some(1)),
                    "identical NaN bits cannot match"
                );
                assert!(!answers.wanted(2, Some(2)), "different NaNs cannot match");
                assert_ne!(answers.source_cell(true, 4), answers.source_cell(false, 5));
                let left_negative = if answers.text { 6 } else { 5 };
                assert_eq!(answers.source_cell(true, left_negative), "8000000000000000");
                assert_eq!(answers.source_cell(false, 3), "0000000000000000");
                assert!(
                    answers.wanted(left_negative, Some(3)),
                    "negative left and positive right zeros must match"
                );
            }
        }
    }
    #[test]
    fn join_key_cancellation_requires_two_complete_traversals_of_the_same_large_group() {
        for mode in [
            "join-double",
            "join-date",
            "join-double-numeric",
            "join-date-numeric",
            "join-string",
            "join-string-numeric",
        ] {
            let answers = Answers::new(mode).unwrap();
            for second in [5, 20] {
                let mut values = vec![answers.schema()];
                let mut boundaries = vec![];
                for left in [4, second]
                    .into_iter()
                    .chain((0..40).filter(|id| *id != 4 && *id != second))
                {
                    for right in (0..answers.right_rows()).map(Some).chain([None]) {
                        if answers.wanted(left, right) {
                            values.push(answers.row(left, right));
                        }
                    }
                    boundaries.push(values.len());
                }
                let one = encode(&values[..boundaries[0]]).unwrap();
                let two = encode(&values[..boundaries[1]]).unwrap();
                let incomplete_two = encode(&values[..boundaries[1] - 1]).unwrap();
                values.push(json!({"complete":true,"rows":answers.rows()}));
                let full = encode(&values).unwrap();
                assert!(answers.cancelled(&one, &full).is_err());
                assert!(answers.cancelled(&incomplete_two, &full).is_err());
                assert_eq!(
                    answers.cancelled(&two, &full).is_ok(),
                    second == 5,
                    "two identities in different groups are insufficient"
                );
            }
        }
    }
    #[test]
    fn join_string_preflight_rejects_empty_nul_prefix_unicode_and_source_identity() {
        for mode in ["join-string", "join-string-numeric"] {
            let answers = Answers::new(mode).unwrap();
            let mut bytes = String::new();
            for (left, name, count) in [
                (true, "left_rows", 40),
                (false, "right_rows", answers.right_rows()),
            ] {
                bytes += &format!("source {name} type=String rows={count}\n");
                for id in (0..count).rev() {
                    bytes += &format!("{name} {id} {}\n", answers.source_cell(left, id));
                }
            }
            assert_eq!(answers.source_controls(&bytes).unwrap(), 7);
            assert_eq!(answers.source_cell(true, 0), "null");
            assert_eq!(answers.source_cell(true, 1), "hex=");
            assert_eq!(answers.source_cell(true, 2), "hex=610062");
            assert_eq!(answers.source_cell(true, 28), "hex=c3a9");
            assert_eq!(answers.source_cell(true, 36), "hex=65cc81");
            let first = answers.source_string(true, 4).unwrap();
            let second = answers.source_string(true, 20).unwrap();
            assert_eq!(first.len(), if answers.text { 3072 } else { 512 });
            assert_eq!(
                &first.as_bytes()[..first.len() - 1],
                &second.as_bytes()[..second.len() - 1]
            );
            assert_ne!(first.as_bytes().last(), second.as_bytes().last());
            assert_eq!(first.as_bytes()[1], 0);
            assert!(answers.wanted(1, Some(1)));
            assert!(answers.wanted(2, Some(2)));
            assert!(!answers.wanted(0, Some(1)));
            assert!(!answers.wanted(3, Some(2)));
            assert!(!answers.wanted(38, Some(3)));
            assert!(!answers.wanted(4, Some(answers.right(20).start)));
            assert!(!answers.wanted(28, Some(answers.right(36).start)));
            assert!(
                answers
                    .source(&bytes.replace("left_rows 39 ", "left_rows 38 "))
                    .is_err()
            );
            assert!(answers.source(bytes.trim_end()).is_err());
            assert!(answers.source(&format!("{bytes}extra\n")).is_err());
        }
    }
    #[test]
    fn join_io_rejects_disabled_faults_and_invalid_transfer_reports() {
        let answers = Answers::new("join-left").unwrap();
        let valid = "calls=1 refused=0 partial=0 result=Ok(1574)\njoin transfers requested=20 transferred=20\njoin initial outcome=finished bytes=100 spill=10 temp=10\njoin retry rows=1574 spill=10 temp=10\njoin reopened rows=1574 spill=10 temp=10\njoin I/O release and complete retries passed\n";
        report(&answers, valid, 100, 0, libc::EIO).unwrap();
        assert!(report(&answers, valid, 100, 1, libc::EIO).is_err());
        for wrong in [
            valid.replace("transferred=20", "transferred=21"),
            valid.replace("requested=20", "requested=0"),
            valid.replace("calls=1", "calls=0"),
            valid.replace("bytes=100", "bytes=99"),
            valid.replace("rows=1574", "rows=1573"),
            valid.replace("spill=10 temp=10", "spill=0 temp=0"),
            valid.replace("join I/O release and complete retries passed", ""),
            format!("{valid}join transfers requested=20 transferred=20\n"),
        ] {
            assert!(report(&answers, &wrong, 100, 0, libc::EIO).is_err());
        }
    }
}
