//! Judge running SUM refusal against literal peer-inclusive typed answers.
//!
//! The native driver captures rows without deriving their expected totals. Each
//! retry must contain all 8,200 distinct identities with the correct nullable
//! partition, peer and total. An initial attempt must be a matching typed prefix;
//! its terminal outcome is checked separately, even if all rows preceded failure.

use super::*;
use serde_json::{Value, json};

const ROWS: usize = 8200;
const SCHEMA: &str =
    "id:int64:required|part:int64:nullable|peer:int64:nullable|total:int64:nullable";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    id: usize,
    values: [Option<i64>; 3],
}

fn expected(id: usize) -> Result<[Option<i64>; 3]> {
    let group = id / 1025;
    if id >= ROWS {
        return Err("window returned an out-of-range identity".into());
    }
    let partition = [None, Some(-1), Some(0), Some(1)][group / 2];
    let peer = if group.is_multiple_of(2) {
        None
    } else {
        Some(1)
    };
    // Complete peer totals include cancellation beyond INT64, all-NULL frames,
    // NULL order keys and the prior peer in the same partition.
    let total = [
        Some(7),
        Some(16),
        None,
        Some(i64::MAX),
        Some(i64::MIN),
        Some(0),
        Some(512),
        Some(0),
    ][group];
    Ok([partition, peer, total])
}

fn rows(bytes: &[u8]) -> Result<Vec<Row>> {
    if bytes.len() > 2_000_000 || (!bytes.is_empty() && !bytes.ends_with(b"\n")) {
        return Err("window capture is oversized or lacks its final newline".into());
    }
    std::str::from_utf8(bytes)?
        .split_terminator('\n')
        .map(|line| {
            let mut fields = line.split('|');
            let id = trace::unsigned(fields.next().ok_or("missing window identity")?)?;
            let mut values = [None; 3];
            for value in &mut values {
                let text = fields.next().ok_or("missing typed window value")?;
                *value = if text == "null" {
                    None
                } else {
                    Some(text.parse::<i64>()?)
                };
            }
            if fields.next().is_some() {
                return Err("extra window column".into());
            }
            Ok(Row { id, values })
        })
        .collect()
}

fn complete(rows: &[Row]) -> Result<()> {
    if rows.len() != ROWS {
        return Err("window retry has an incomplete row count".into());
    }
    let mut seen = vec![false; ROWS];
    for row in rows {
        if row.values != expected(row.id)? || seen[row.id] {
            return Err("window differs from complete independent typed answers".into());
        }
        seen[row.id] = true;
    }
    Ok(())
}

fn prefix(initial: &[Row], retry: &[Row]) -> Result<()> {
    complete(retry)?;
    if initial.len() > retry.len() || initial != &retry[..initial.len()] {
        return Err("window failed attempt differs from the validated retry prefix".into());
    }
    Ok(())
}

struct Report<'a> {
    outcome: &'a str,
    rows: usize,
    spill: usize,
    temporary: usize,
}

fn report<'a>(output: &'a str, prefix: &str) -> Result<Report<'a>> {
    let line = record(output, prefix)?;
    let fields = trace::fields(line, &["outcome=", "rows=", "spill=", "temp="])?;
    Ok(Report {
        outcome: fields[0],
        rows: trace::unsigned(fields[1])?,
        spill: trace::unsigned(fields[2])?,
        temporary: trace::unsigned(fields[3])?,
    })
}

fn reports(output: &str, prefix: usize, count: usize, cancelled: bool) -> Result<Report<'_>> {
    require_line(output, "window release and complete retry passed")?;
    if record(output, "window schema=")? != SCHEMA {
        return Err("window schema differs".into());
    }
    let observed = report(output, "window attempt ")?;
    if cancelled != (observed.outcome == "cancelled")
        || (cancelled && (observed.rows == 0 || observed.rows >= ROWS))
        || !matches!(
            observed.outcome,
            "finished" | "refused" | "recovery" | "cancelled"
        )
        || (number(output, "window refusals=")? > 0) != (prefix < count)
        || (prefix == count && (observed.outcome != "finished" || observed.rows != ROWS))
    {
        return Err("window refusal or outcome disagrees with its prefix".into());
    }
    let retry = trace::fields(
        record(output, "window retry ")?,
        &["rows=", "spill=", "temp="],
    )?;
    if trace::unsigned(retry[0])? != ROWS
        || trace::unsigned(retry[2])? == 0
        || (cfg!(target_os = "linux") && trace::unsigned(retry[1])? == 0)
    {
        return Err("window retry lacks complete rows or actual spill".into());
    }
    let samples = trace::samples(record(output, "window retry-events=")?)?;
    if samples.allocations == 0 || samples.frees == 0 {
        return Err("window retry lacks native allocation/free events".into());
    }
    trace::samples(record(output, "window events=")?)?;
    if matches!(observed.outcome, "finished" | "cancelled")
        && number(output, "window terminal frees=")? == 0
    {
        return Err("window terminal free events are missing".into());
    }
    if observed.outcome == "finished"
        && (observed.rows != ROWS
            || observed.temporary == 0
            || (cfg!(target_os = "linux") && observed.spill == 0))
    {
        return Err("finished window lacks complete rows or actual spill".into());
    }
    Ok(observed)
}

fn controls(retry: &[Row], output: &str, count: usize) -> Result<usize> {
    complete(retry)?;
    for case in 0..5 {
        let mut wrong = retry.to_vec();
        match case {
            0 => wrong[0].values[2] = Some(8),
            1 => {
                wrong.pop();
            }
            2 => wrong[1] = wrong[0].clone(),
            3 => {
                wrong
                    .iter_mut()
                    .find(|row| row.values[2].is_none())
                    .ok_or("missing NULL control row")?
                    .values[2] = Some(0)
            }
            _ => {
                wrong
                    .iter_mut()
                    .find(|row| row.values[0].is_none())
                    .ok_or("missing NULL partition control row")?
                    .values[0] = Some(-1)
            }
        }
        if complete(&wrong).is_ok() {
            return Err(format!("window answer control {case} was accepted").into());
        }
    }
    let mut wrong = retry[..3].to_vec();
    prefix(&wrong, retry)?;
    wrong[0].id = ROWS;
    if prefix(&wrong, retry).is_ok() {
        return Err("window corrupted prefix was accepted".into());
    }
    if reports(output, 0, count, false).is_ok() {
        return Err("window disabled refusal was accepted".into());
    }
    let wrong = output.replace(SCHEMA, "id:double:required");
    if reports(&wrong, count, count, false).is_ok() {
        return Err("window incorrect schema was accepted".into());
    }
    let terminal = number(output, "window terminal frees=")?;
    let wrong = output.replace(
        &format!("window terminal frees={terminal}"),
        "window terminal frees=0",
    );
    if reports(&wrong, count, count, false).is_ok() {
        return Err("window missing terminal events were accepted".into());
    }
    let retry_line = record(output, "window retry ")?;
    let fields = trace::fields(retry_line, &["rows=", "spill=", "temp="])?;
    let wrong = output.replace(
        &format!("window retry {retry_line}"),
        &format!("window retry rows={} spill={} temp=0", fields[0], fields[1]),
    );
    if reports(&wrong, count, count, false).is_ok() {
        return Err("window missing spill reservation was accepted".into());
    }
    let mut rejected = 10;
    if cfg!(target_os = "linux") {
        let wrong = output.replace(
            &format!("window retry {retry_line}"),
            &format!("window retry rows={} spill=0 temp={}", fields[0], fields[2]),
        );
        if reports(&wrong, count, count, false).is_ok() {
            return Err("window missing written spill was accepted".into());
        }
        rejected += 1;
    }
    Ok(rejected)
}

pub(super) fn verify_files(root: &Path, output: &str, check_controls: bool) -> Result<Value> {
    let initial = rows(&fs::read(root.join("initial.rows"))?)?;
    let retry = rows(&fs::read(root.join("retry.rows"))?)?;
    prefix(&initial, &retry)?;
    let observed = report(output, "window attempt ")?;
    if observed.rows != initial.len() || (observed.outcome == "finished" && initial != retry) {
        return Err("window report differs from captured rows".into());
    }
    let rejected = if check_controls {
        controls(&retry, output, number(output, "window allocations=")?)?
    } else {
        0
    };
    Ok(
        json!({"rows":ROWS,"columns":4,"initial_rows":initial.len(),"outcome":observed.outcome,"initial_sha256":workspace::hash(&root.join("initial.rows"))?,"retry_sha256":workspace::hash(&root.join("retry.rows"))?,"answer_controls":rejected,"independent_answers":true}),
    )
}

impl Campaign {
    pub(super) fn windows(&mut self) -> Result<()> {
        let healthy = self.execute("window", None)?;
        let count = number(&healthy, "window allocations=")?;
        if !(1..=1024).contains(&count) {
            return Err("invalid window allocation census".into());
        }
        reports(&healthy, count, count, false)?;
        let (mut refused, mut recovery, mut finished_under_denial) = (0, 0, 0);
        for prefix in std::iter::once(count).chain(0..count) {
            let output = self.execute(&format!("window-{prefix}"), None)?;
            let observed = reports(&output, prefix, count, false)?;
            if prefix < count {
                match observed.outcome {
                    "refused" => refused += 1,
                    "recovery" => recovery += 1,
                    _ => finished_under_denial += 1,
                }
            }
        }
        if refused == 0 {
            return Err("window sweep missed native refusal".into());
        }
        let cancelled = self.execute("window-cancel", None)?;
        let observed = report(&cancelled, "window attempt ")?;
        let calls = number(&cancelled, "window allocations=")?;
        reports(
            &cancelled,
            calls
                .checked_sub(1)
                .ok_or("missing window cancellation census")?,
            calls,
            true,
        )?;
        if observed.outcome != "cancelled" || observed.rows == 0 || observed.rows >= ROWS {
            return Err("window cancelled prefix is missing".into());
        }
        self.rejected(
            "window-terminal-unobserved",
            "window terminal release lacked free events",
        )?;
        fs::write(
            self.run.directory.join("window.json"),
            serde_json::to_vec_pretty(
                &json!({"refusal_prefixes":count,"refused":refused,"recovery_required":recovery,"finished_under_denial":finished_under_denial,"rows":ROWS,"full_prefix_passed":true,"disabled_refusal_rejected":true,"written_spill_checked":cfg!(target_os="linux"),"cancelled_prefix_under_denial":true,"missing_terminal_observer_rejected":true}),
            )?,
        )?;
        println!(
            "window allocation: {count} prefixes, complete independent peer totals, release and retry passed"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_window_answers_require_all_typed_identities_and_peers() {
        let rows = (0..ROWS)
            .map(|id| Row {
                id,
                values: expected(id).unwrap(),
            })
            .collect::<Vec<_>>();
        complete(&rows).unwrap();
        let mut wrong = rows.clone();
        wrong[1025].values[2] = Some(9);
        assert!(
            complete(&wrong).is_err(),
            "row-local total omitted its preceding peer"
        );
        let mut wrong = rows.clone();
        wrong[2050].values[2] = Some(0);
        assert!(complete(&wrong).is_err(), "all-NULL frame became zero");
        let mut wrong = rows.clone();
        wrong[1] = wrong[0].clone();
        assert!(complete(&wrong).is_err(), "duplicate replaced an identity");
        assert!(complete(&rows[..ROWS - 1]).is_err());
        prefix(&rows[..1025], &rows).unwrap();
        let mut wrong = rows[..1025].to_vec();
        wrong[0].values[0] = Some(-2);
        assert!(prefix(&wrong, &rows).is_err());
    }
}
