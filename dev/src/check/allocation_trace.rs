//! Reconcile allocator events across refusal, partial results and report phases.
//!
//! The driver checks live owners; this reader independently checks that every
//! promised observation occurred, in order, with complete release and no deficit.
//!
//! Report histories and partial results have different required phases. Their
//! parsers retain those distinctions while sharing field and sample decoding.
//! Mutation tests delete or duplicate records and change counters to challenge the
//! checker independently of a healthy driver run.

use super::{Campaign, Result, record, require_line};

const PARTIAL_DONE: &str =
    "partial result ownership passed: 3 cases; rows, terminal events and release";
const CALIBRATED: &str =
    "transient ownership calibration passed: hidden allocation detected; entry/exit agree";

impl Campaign {
    pub(super) fn reports(&mut self) -> Result<()> {
        for length in [None, Some(384)] {
            report(&self.execute("event-report-history", length)?)?;
            partial(&self.execute("partial-result-shapes", length)?)?;
            println!("transient report and partial results: pathname={length:?} passed");
        }
        for (mode, diagnostic) in [
            (
                "event-report-attribution-negative",
                "event report requested ownership: prepare",
            ),
            (
                "event-report-preparation-negative",
                "missing report preparation allocation events",
            ),
            (
                "event-report-construction-negative",
                "missing report construction allocation events",
            ),
            (
                "event-report-terminal-negative",
                "missing report terminal allocation events",
            ),
            (
                "partial-result-prefix-negative",
                "partial result prefix oracle",
            ),
            (
                "partial-result-terminal-negative",
                "missing partial result terminal events",
            ),
        ] {
            self.rejected(mode, diagnostic)?;
        }
        Ok(())
    }
}

pub(super) fn unsigned(text: &str) -> Result<usize> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("invalid allocation event count".into());
    }
    Ok(text.parse()?)
}

fn headroom(text: &str) -> Result<()> {
    if text.parse::<i128>()? < 0 {
        return Err("allocation exceeded reservation".into());
    }
    Ok(())
}

pub(super) fn fields<'a>(text: &'a str, names: &[&str]) -> Result<Vec<&'a str>> {
    let mut words = text.split(' ');
    let mut values = Vec::with_capacity(names.len());
    for name in names {
        values.push(
            words
                .next()
                .and_then(|word| word.strip_prefix(name))
                .ok_or("missing allocation trace field")?,
        );
    }
    if words.next().is_some() {
        return Err("extra allocation trace field".into());
    }
    Ok(values)
}

#[derive(Debug)]
pub(super) struct Samples {
    pub(super) allocations: usize,
    pub(super) frees: usize,
}

pub(super) fn samples(text: &str) -> Result<Samples> {
    let text = text
        .strip_prefix("Samples { ")
        .and_then(|text| text.strip_suffix(" }"))
        .ok_or("invalid allocation sample")?;
    let mut values = text.split(", ");
    let mut field = |name| {
        values
            .next()
            .and_then(|value| value.strip_prefix(name))
            .ok_or("missing sample field")
    };
    let allocations = unsigned(field("allocations: ")?)?;
    let frees = unsigned(field("frees: ")?)?;
    headroom(field("requested_headroom: ")?)?;
    headroom(field("usable_headroom: ")?)?;
    if values.next().is_some() {
        return Err("extra sample field".into());
    }
    Ok(Samples { allocations, frees })
}

fn partial(output: &str) -> Result<()> {
    require_line(output, PARTIAL_DONE)?;
    let rows: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix("partial result case="))
        .collect();
    if rows.len() != 3 {
        return Err("partial result case count differs".into());
    }
    for (index, row) in rows.into_iter().enumerate() {
        let (name, body) = row.split_once(' ').ok_or("missing partial result name")?;
        if name != ["overflow", "cancelled", "finished"][index] {
            return Err("partial result order differs".into());
        }
        let values = fields(
            body,
            &[
                "rows=",
                "steps=",
                "prepare_allocations=",
                "execute_allocations=",
                "terminal_frees=",
                "prepared_frees=",
                "requested_headroom=",
                "usable_headroom=",
                "release=",
            ],
        )?;
        let rows = unsigned(values[0])?;
        if !match index {
            0 => rows == 256,
            1 => (1..257).contains(&rows),
            _ => rows == 257,
        } {
            return Err("partial row prefix differs".into());
        }
        if !(2..20_000).contains(&unsigned(values[1])?) || values[8] != "complete" {
            return Err("partial result did not complete within bound".into());
        }
        for value in &values[2..6] {
            if unsigned(value)? == 0 {
                return Err("partial result missed allocation/free events".into());
            }
        }
        headroom(values[6])?;
        headroom(values[7])?;
    }
    Ok(())
}

fn report_prefixes(output: &str, phase: &str, maximum: usize) -> Result<usize> {
    let completion = format!("event report {phase} passed: prefixes=0..=");
    let count = unsigned(
        record(output, &completion)?
            .strip_suffix("; live errors and release")
            .ok_or("invalid refusal completion")?,
    )?;
    if !(1..=maximum).contains(&count) {
        return Err("report refusal census outside bound".into());
    }
    let census = samples(record(output, &format!("event report {phase} census: "))?)?;
    if census.allocations != count || census.frees != count {
        return Err("report census allocation/release disagrees".into());
    }
    let prefix = format!("event report {phase} prefix=");
    let rows: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .collect();
    if rows.len() != count + 1 {
        return Err("report refusal prefixes missing or duplicated".into());
    }
    for (expected, row) in rows.into_iter().enumerate() {
        let (counts, observation) = row
            .split_once(" samples=")
            .ok_or("missing refusal sample")?;
        let values = fields(counts, &["", "calls=", "refusals="])?;
        let (prefix, calls, refusals) = (
            unsigned(values[0])?,
            unsigned(values[1])?,
            unsigned(values[2])?,
        );
        let sample = samples(observation)?;
        if prefix != expected
            || sample.allocations != prefix
            || sample.frees != prefix
            || if prefix < count {
                calls <= prefix || refusals == 0
            } else {
                calls != count || refusals != 0
            }
        {
            return Err("report refusal events disagree with census".into());
        }
    }
    Ok(count)
}

fn report(output: &str) -> Result<()> {
    let preparation = report_prefixes(output, "preparation", 32)?;
    let construction = report_prefixes(output, "construction", 512)?;
    let phases: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix("event report "))
        .filter_map(|line| line.split_once(": Samples "))
        .filter(|(name, _)| !matches!(*name, "preparation census" | "construction census"))
        .collect();
    if phases.len() != 15 {
        return Err("report phase count differs".into());
    }
    for history in phases.as_chunks::<5>().0 {
        let mut observations = Vec::with_capacity(5);
        for ((name, body), expected) in history.iter().zip([
            "prepare",
            "partial drop",
            "cancelled",
            "execute/finish/drop",
            "prepared drop",
        ]) {
            if *name != expected {
                return Err("report phase order differs".into());
            }
            observations.push(samples(&format!("Samples {body}"))?);
        }
        let [prepare, partial, cancelled, finished, released] = observations.as_slice() else {
            unreachable!()
        };
        if prepare.allocations != preparation
            || released.allocations != 0
            || released.frees == 0
            || prepare.frees.checked_add(released.frees) != Some(prepare.allocations)
        {
            return Err("report preparation/release accounting differs".into());
        }
        for observation in [partial, cancelled, finished] {
            if observation.allocations < construction
                || observation.frees != observation.allocations
            {
                return Err("report execution/release accounting differs".into());
            }
        }
    }
    let histories: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix("event report history="))
        .collect();
    if histories
        != [
            "0 rows=13 release=complete",
            "1 rows=13 release=complete",
            "2 rows=13 release=complete",
        ]
    {
        return Err("report histories incomplete".into());
    }
    require_line(output, "event report histories passed: 3 complete runs")?;
    require_line(output, CALIBRATED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;

    fn sample(allocations: usize, frees: usize) -> String {
        format!(
            "Samples {{ allocations: {allocations}, frees: {frees}, requested_headroom: 0, usable_headroom: 0 }}"
        )
    }

    fn report_trace() -> String {
        let mut text = String::new();
        for (phase, count) in [("preparation", 2), ("construction", 1)] {
            writeln!(
                text,
                "event report {phase} census: {}",
                sample(count, count)
            )
            .unwrap();
            for prefix in 0..=count {
                let calls = if prefix < count { prefix + 1 } else { count };
                let refusals = usize::from(prefix < count);
                writeln!(text, "event report {phase} prefix={prefix} calls={calls} refusals={refusals} samples={}", sample(prefix, prefix)).unwrap();
            }
            writeln!(
                text,
                "event report {phase} passed: prefixes=0..={count}; live errors and release"
            )
            .unwrap();
        }
        for history in 0..3 {
            for (phase, allocations, frees) in [
                ("prepare", 2, 1),
                ("partial drop", 1, 1),
                ("cancelled", 1, 1),
                ("execute/finish/drop", 1, 1),
                ("prepared drop", 0, 1),
            ] {
                writeln!(text, "event report {phase}: {}", sample(allocations, frees)).unwrap();
            }
            writeln!(
                text,
                "event report history={history} rows=13 release=complete"
            )
            .unwrap();
        }
        writeln!(
            text,
            "event report histories passed: 3 complete runs\n{CALIBRATED}"
        )
        .unwrap();
        text
    }

    #[test]
    fn report_trace_requires_complete_ordered_events_and_balanced_release() {
        let healthy = report_trace();
        report(&healthy).unwrap();
        for (from, to) in [
            ("preparation prefix=1", "preparation prefix=0"),
            (
                "construction prefix=0 calls=1 refusals=1",
                "construction prefix=0 calls=0 refusals=0",
            ),
            ("requested_headroom: 0", "requested_headroom: -1"),
            ("usable_headroom: 0", "usable_headroom: -1"),
            ("allocations: 2, frees: 2", "allocations: 2, frees: 1"),
            ("allocations: 0, frees: 1", "allocations: 0, frees: 0"),
            ("event report cancelled:", "event report partial drop:"),
            ("history=2 rows=13", "history=2 rows=12"),
            (CALIBRATED, ""),
        ] {
            assert!(healthy.contains(from));
            assert!(
                report(&healthy.replacen(from, to, 1)).is_err(),
                "accepted mutation: {from}"
            );
        }
        assert!(report(&(healthy.clone() + &healthy)).is_err());
        for line in healthy.lines() {
            let shortened = healthy.replacen(&format!("{line}\n"), "", 1);
            assert!(report(&shortened).is_err(), "accepted missing {line}");
        }
    }

    #[test]
    fn partial_trace_rejects_wrong_prefix_missing_events_and_unfinished_release() {
        let mut healthy = String::new();
        for (name, rows) in [("overflow", 256), ("cancelled", 1), ("finished", 257)] {
            writeln!(healthy, "partial result case={name} rows={rows} steps=2 prepare_allocations=1 execute_allocations=1 terminal_frees=1 prepared_frees=1 requested_headroom=0 usable_headroom=0 release=complete").unwrap();
        }
        writeln!(healthy, "{PARTIAL_DONE}").unwrap();
        partial(&healthy).unwrap();
        for (from, to) in [
            ("rows=256", "rows=255"),
            ("rows=1", "rows=0"),
            ("rows=257", "rows=256"),
            ("terminal_frees=1", "terminal_frees=0"),
            ("steps=2", "steps=1"),
            ("usable_headroom=0", "usable_headroom=-1"),
            ("release=complete", "release=partial"),
        ] {
            assert!(partial(&healthy.replacen(from, to, 1)).is_err());
        }
        for line in healthy.lines() {
            assert!(partial(&healthy.replacen(&format!("{line}\n"), "", 1)).is_err());
        }
    }
}
