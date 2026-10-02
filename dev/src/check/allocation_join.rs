//! Supervise wide joins and validate every promised refusal observation.
//!
//! The driver checks SQL rows and live owners; this parser checks its phase records,
//! allocation prefixes and terminal markers. Preparation, construction and execution
//! must all be represented, including demanded failures. Mutated traces and driver
//! controls remove samples or alter attribution to verify rejection. A successful
//! process with an incomplete or duplicated trace does not satisfy the campaign.

use super::{
    Campaign, Result, record, require_line,
    trace::{fields, samples, unsigned},
};

const DONE: &str = "wide left join passed: 64 columns, 11 pairs; rows, ownership and release";
const LIFECYCLE: &str =
    "wide left join lifecycle passed: preparation, finished release, two abandonments";
const EXECUTION: &str = "wide left join execution failures passed: 3 demanded errors; external work, owned spans and release";
const EXPRESSIONS: [&str; 3] = [
    "LOG10(ABS(r.id-3))",
    "SAFE_DIVIDE(1, LOG10(ABS(r.id-3)))",
    "COALESCE(NULLIF(1, 1), LOG10(ABS(r.id-3)))",
];

impl Campaign {
    pub(super) fn joins(&mut self) -> Result<()> {
        growth(&self.execute("optional-sort-growth", None)?)?;
        self.rejected(
            "optional-sort-growth-observer-negative",
            "missing optional sort allocation events",
        )?;
        byte_growth(&self.execute("optional-sort-bytes", None)?)?;
        self.rejected(
            "optional-sort-bytes-observer-negative",
            "missing optional byte growth allocation events",
        )?;
        replacement_growth(
            &self.execute("optional-sort-rows", None)?,
            Replacement::Rows,
            cfg!(target_os = "linux"),
        )?;
        self.rejected(
            "optional-sort-rows-observer-negative",
            "missing optional row growth allocation events",
        )?;
        replacement_growth(
            &self.execute("optional-sort-join-bytes", None)?,
            Replacement::JoinedBytes,
            cfg!(target_os = "linux"),
        )?;
        for (mode, diagnostic) in [
            (
                "optional-sort-join-bytes-observer-negative",
                "missing optional joined-byte growth allocation events",
            ),
            (
                "optional-sort-join-bytes-pairs-negative",
                "join growth pair relation",
            ),
            (
                "optional-sort-join-bytes-attribution-negative",
                "workspace replacement exceeds reservation",
            ),
        ] {
            self.rejected(mode, diagnostic)?;
        }
        for length in [None, Some(384)] {
            for (mode, construction) in [
                ("wide-left-join-shape", false),
                ("wide-left-join-construction-sequence", true),
            ] {
                validate(&self.execute(mode, length)?, construction)?;
                println!("{mode}: pathname={length:?} passed");
            }
        }
        for (mode, diagnostic) in [
            (
                "wide-left-join-attribution-negative",
                "wide left join usable ownership attribution",
            ),
            (
                "wide-left-join-observer-negative",
                "transient ownership calibration missed uncharged allocation",
            ),
            (
                "wide-left-join-lifecycle-negative",
                "missing transient join lifecycle events: preparation",
            ),
            (
                "wide-left-join-failure-negative",
                "missing failed preparation events: prefix=1",
            ),
            (
                "wide-left-join-execution-negative",
                "missing failed execution events",
            ),
            (
                "wide-left-join-construction-negative",
                "missing failed construction events: prefix=1",
            ),
        ] {
            self.rejected(mode, diagnostic)?;
        }
        Ok(())
    }
}

fn growth(output: &str) -> Result<()> {
    let count = unsigned(record(output, "sort growth census allocations=")?)?;
    if !(5..=16).contains(&count) {
        return Err("sort growth census outside bound".into());
    }
    require_line(
        output,
        "sort growth passed: fallbacks=3; typed rows and release",
    )?;
    let rows: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix("sort growth prefix="))
        .collect();
    if rows.len() != 4 {
        return Err("sort growth prefixes missing or duplicated".into());
    }
    for (prefix, row) in (count - 3..=count).zip(rows) {
        let (counts, observation) = row
            .split_once(" samples=")
            .ok_or("missing sort growth sample")?;
        let values = fields(counts, &["", "calls=", "refusals=", "rows="])?;
        let values: Vec<_> = values.into_iter().map(unsigned).collect::<Result<_>>()?;
        let expected = [
            prefix,
            if prefix == count { count } else { prefix + 1 },
            usize::from(prefix < count),
            9,
        ];
        let sample = samples(observation)?;
        if values != expected || sample.allocations != prefix || sample.frees < prefix {
            return Err("sort growth refusal or release differs".into());
        }
    }
    Ok(())
}

fn byte_growth(output: &str) -> Result<()> {
    replacement_growth(output, Replacement::Bytes, cfg!(target_os = "linux"))
}

enum Replacement {
    Bytes,
    Rows,
    JoinedBytes,
}

fn replacement_growth(output: &str, replacement: Replacement, physical_spill: bool) -> Result<()> {
    let (label, kind, row_count, allocations, events) = match replacement {
        Replacement::Bytes => ("bytes", "byte", 1024, 1, 1..=16),
        Replacement::Rows => ("rows", "row", 32768, 3, 2..=2),
        Replacement::JoinedBytes => ("join-bytes", "joined-byte", 3022, 1, 1..=16),
    };
    let count = unsigned(record(output, &format!("sort {label} census events="))?)?;
    if !events.contains(&count) {
        return Err("missing sort replacement census".into());
    }
    require_line(
        output,
        &format!("sort {kind} growth passed: complete rows, replacement fallback and release"),
    )?;
    let written: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix(&format!("sort {label} written event=")))
        .collect();
    if written.len() != count {
        return Err("missing or duplicate written spill observations".into());
    }
    for (event, record) in written.into_iter().enumerate() {
        let values = fields(record, &["", "bytes="])?;
        // The native macOS driver reports zero here: only Linux observes the
        // unlinked scratch-file lengths. Both drivers enforce temp accounting.
        if unsigned(values[0])? != event || (physical_spill && unsigned(values[1])? == 0) {
            return Err("replacement did not observe written spill".into());
        }
    }
    if matches!(replacement, Replacement::JoinedBytes) {
        let retained: Vec<_> = output
            .lines()
            .filter_map(|line| line.strip_prefix("sort join-bytes retained event="))
            .collect();
        if retained.len() != count {
            return Err("missing or duplicate retained join inputs".into());
        }
        for (event, record) in retained.into_iter().enumerate() {
            let values = fields(record, &["", "files=", "descriptors="])?;
            if unsigned(values[0])? != event
                || unsigned(values[2])? < 4
                || (physical_spill && !(2..=4).contains(&unsigned(values[1])?))
                || (!physical_spill && unsigned(values[1])? != 0)
            {
                return Err("join replacement lost retained inputs".into());
            }
        }
    }
    let prefix_count = allocations + 1;
    let rows: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix(&format!("sort {label} event=")))
        .collect();
    if rows.len() != count * prefix_count {
        return Err("sort replacement prefixes missing or duplicated".into());
    }
    let mut last_step = 0;
    for (index, row) in rows.into_iter().enumerate() {
        let (counts, observation) = row
            .split_once(" samples=")
            .ok_or("missing replacement sample")?;
        let values = fields(
            counts,
            &["", "step=", "prefix=", "calls=", "refusals=", "rows="],
        )?;
        let values: Vec<_> = values.into_iter().map(unsigned).collect::<Result<_>>()?;
        let prefix = index % prefix_count;
        if values
            != [
                index / prefix_count,
                values[1],
                prefix,
                (prefix + 1).min(allocations),
                usize::from(prefix < allocations),
                row_count,
            ]
            || (prefix == 0 && values[1] <= last_step)
            || (prefix != 0 && values[1] != last_step)
        {
            return Err("sort replacement refusal or complete rows differ".into());
        }
        last_step = values[1];
        let sample = samples(observation)?;
        if sample.allocations != prefix || sample.frees != prefix {
            return Err("sort replacement transient events differ".into());
        }
    }
    if output
        .lines()
        .filter(|line| line.starts_with(&format!("sort {label} cancellation event=")))
        .count()
        != count
    {
        return Err("sort replacement cancellation observations differ".into());
    }
    for event in 0..count {
        require_line(
            output,
            &format!("sort {label} cancellation event={event} terminal=2 retry_rows={row_count}"),
        )?;
    }
    Ok(())
}

fn prefixes(output: &str, phase: &str, maximum: usize) -> Result<()> {
    let count = unsigned(record(
        output,
        &format!("join {phase} census allocations="),
    )?)?;
    if !(2..=maximum).contains(&count) {
        return Err("join census outside bound".into());
    }
    let suffix = if phase == "preparation" {
        "live errors, owned span and release"
    } else {
        "live errors and release"
    };
    require_line(
        output,
        &format!("wide left join {phase} failures passed: prefixes=0..={count}; {suffix}"),
    )?;
    let prefix = format!("join {phase} prefix=");
    let rows: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .collect();
    if rows.len() != count + 1 {
        return Err("join prefixes missing or duplicated".into());
    }
    for (expected, row) in rows.into_iter().enumerate() {
        let (counts, observation) = row
            .split_once(" samples=")
            .ok_or("missing join refusal sample")?;
        let values = fields(counts, &["", "calls=", "refusals="])?;
        let (prefix, calls, refusals) = (
            unsigned(values[0])?,
            unsigned(values[1])?,
            unsigned(values[2])?,
        );
        if prefix != expected
            || if prefix < count {
                calls <= prefix || refusals == 0
            } else {
                calls != count || refusals != 0
            }
        {
            return Err("join refusal events disagree with census".into());
        }
        if prefix == 0 {
            if observation != "none" {
                return Err("zero-prefix join unexpectedly allocated".into());
            }
        } else {
            let sample = samples(observation)?;
            // The full-prefix observer covers construction, while dropping the
            // successful result happens outside that observation interval.
            if sample.allocations != prefix || (prefix < count && sample.frees != prefix) {
                return Err("join refusal did not release observed allocations".into());
            }
        }
    }
    Ok(())
}

fn execution(output: &str) -> Result<()> {
    require_line(output, EXECUTION)?;
    let rows: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix("wide left join demanded failure: expression="))
        .collect();
    if rows.len() != EXPRESSIONS.len() {
        return Err("missing demanded join failure".into());
    }
    for (row, expected) in rows.into_iter().zip(EXPRESSIONS) {
        let values: Vec<_> = row.split("; ").collect();
        if values.len() != 4 || values[0] != expected {
            return Err("demanded join expression differs".into());
        }
        let step = unsigned(
            values[1]
                .strip_prefix("step=")
                .ok_or("missing failure step")?,
        )?;
        let temporary = unsigned(
            values[2]
                .strip_prefix("temporary=")
                .ok_or("missing temporary peak")?,
        )?;
        let sample = samples(values[3])?;
        if !(2..200_000).contains(&step) || temporary == 0 || sample.frees == 0 {
            return Err("join error did not follow external work and observed release".into());
        }
    }
    Ok(())
}

fn validate(output: &str, construction: bool) -> Result<()> {
    require_line(output, DONE)?;
    require_line(output, LIFECYCLE)?;
    require_line(
        output,
        "transient ownership calibration passed: hidden allocation detected; entry/exit agree",
    )?;
    prefixes(output, "preparation", 32)?;
    if construction {
        prefixes(output, "construction", 512)?;
    }
    execution(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;

    #[test]
    fn growth_requires_every_refusal_fallback_and_observation() {
        let mut output = String::from("sort growth census allocations=5\n");
        for prefix in 2..=5 {
            let calls = if prefix == 5 { 5 } else { prefix + 1 };
            let refusals = usize::from(prefix < 5);
            let rows = 9;
            writeln!(output, "sort growth prefix={prefix} calls={calls} refusals={refusals} rows={rows} samples=Samples {{ allocations: {prefix}, frees: 5, requested_headroom: 0, usable_headroom: 0 }}").unwrap();
        }
        writeln!(
            output,
            "sort growth passed: fallbacks=3; typed rows and release"
        )
        .unwrap();
        growth(&output).unwrap();
        for (from, to) in [
            ("refusals=1", "refusals=0"),
            ("rows=9", "rows=8"),
            ("allocations: 2", "allocations: 0"),
            ("frees: 5", "frees: 0"),
            ("requested_headroom: 0", "requested_headroom: -1"),
            ("fallbacks=3", "fallbacks=2"),
        ] {
            assert!(growth(&output.replace(from, to)).is_err(), "{from}");
        }
        let row = output
            .lines()
            .find(|line| line.starts_with("sort growth prefix=2 "))
            .unwrap();
        assert!(growth(&output.replace(&format!("{row}\n"), "")).is_err());
        assert!(growth(&format!("{output}{row}\n")).is_err());
        assert!(growth("").is_err());
    }

    #[test]
    fn byte_growth_requires_each_replacement_refusal_and_observation() {
        let mut output = String::from("sort bytes census events=2\n");
        for event in 0..2 {
            writeln!(output, "sort bytes written event={event} bytes=65536").unwrap();
            for prefix in 0..=1 {
                let step = 500 + event * 100;
                let refused = 1 - prefix;
                writeln!(output, "sort bytes event={event} step={step} prefix={prefix} calls=1 refusals={refused} rows=1024 samples=Samples {{ allocations: {prefix}, frees: {prefix}, requested_headroom: 0, usable_headroom: 0 }}").unwrap();
            }
            writeln!(
                output,
                "sort bytes cancellation event={event} terminal=2 retry_rows=1024"
            )
            .unwrap();
        }
        writeln!(
            output,
            "sort byte growth passed: complete rows, replacement fallback and release"
        )
        .unwrap();
        replacement_growth(&output, Replacement::Bytes, true).unwrap();
        for (from, to) in [
            ("events=2", "events=0"),
            ("bytes=65536", "bytes=0"),
            ("prefix=0", "prefix=1"),
            ("event=1", "event=0"),
            ("step=600", "step=500"),
            ("calls=1", "calls=2"),
            ("refusals=1", "refusals=0"),
            ("rows=1024", "rows=1023"),
            ("allocations: 1", "allocations: 0"),
            ("frees: 1", "frees: 0"),
            ("usable_headroom: 0", "usable_headroom: -1"),
            ("terminal=2", "terminal=1"),
            ("retry_rows=1024", "retry_rows=1023"),
        ] {
            assert!(
                replacement_growth(&output.replace(from, to), Replacement::Bytes, true).is_err(),
                "{from}"
            );
        }
        let row = output
            .lines()
            .find(|line| line.starts_with("sort bytes event="))
            .unwrap();
        assert!(
            replacement_growth(
                &output.replace(&format!("{row}\n"), ""),
                Replacement::Bytes,
                true
            )
            .is_err()
        );
        assert!(replacement_growth(&format!("{output}{row}\n"), Replacement::Bytes, true).is_err());
        assert!(replacement_growth("", Replacement::Bytes, true).is_err());
    }

    #[test]
    fn row_growth_requires_three_array_prefixes_spill_and_cancellation() {
        let mut output = String::from("sort rows census events=2\n");
        for event in 0..2 {
            writeln!(output, "sort rows written event={event} bytes=100000").unwrap();
            for prefix in 0..=3 {
                let step = 10000 + event * 20000;
                let calls = (prefix + 1).min(3);
                let refusals = usize::from(prefix < 3);
                writeln!(output, "sort rows event={event} step={step} prefix={prefix} calls={calls} refusals={refusals} rows=32768 samples=Samples {{ allocations: {prefix}, frees: {prefix}, requested_headroom: 0, usable_headroom: 0 }}").unwrap();
            }
            writeln!(
                output,
                "sort rows cancellation event={event} terminal=2 retry_rows=32768"
            )
            .unwrap();
        }
        writeln!(
            output,
            "sort row growth passed: complete rows, replacement fallback and release"
        )
        .unwrap();
        replacement_growth(&output, Replacement::Rows, true).unwrap();
        for (from, to) in [
            ("events=2", "events=1"),
            ("bytes=100000", "bytes=0"),
            ("event=1", "event=0"),
            ("step=30000", "step=10000"),
            ("step=10000 prefix=2", "step=99999 prefix=2"),
            ("prefix=2", "prefix=1"),
            ("calls=3", "calls=2"),
            ("refusals=1", "refusals=0"),
            ("rows=32768", "rows=32767"),
            ("allocations: 2", "allocations: 1"),
            ("frees: 2", "frees: 1"),
            ("requested_headroom: 0", "requested_headroom: -1"),
            ("usable_headroom: 0", "usable_headroom: -1"),
            ("terminal=2", "terminal=1"),
            ("retry_rows=32768", "retry_rows=32767"),
        ] {
            assert!(
                replacement_growth(&output.replace(from, to), Replacement::Rows, true).is_err(),
                "{from}"
            );
        }
        for prefix in [
            "sort rows event=",
            "sort rows written event=",
            "sort rows cancellation event=",
        ] {
            let row = output
                .lines()
                .find(|line| line.starts_with(prefix))
                .unwrap();
            assert!(
                replacement_growth(
                    &output.replace(&format!("{row}\n"), ""),
                    Replacement::Rows,
                    true
                )
                .is_err()
            );
            assert!(
                replacement_growth(&format!("{output}{row}\n"), Replacement::Rows, true).is_err()
            );
        }
        assert!(replacement_growth("", Replacement::Rows, true).is_err());
    }

    #[test]
    fn joined_byte_growth_requires_retained_inputs_and_complete_refusal_rows() {
        let mut output = String::from("sort join-bytes census events=1\n");
        output.push_str("sort join-bytes written event=0 bytes=230000\n");
        output.push_str("sort join-bytes retained event=0 files=2 descriptors=7\n");
        for prefix in 0..=1 {
            let refused = 1 - prefix;
            writeln!(output, "sort join-bytes event=0 step=6062 prefix={prefix} calls=1 refusals={refused} rows=3022 samples=Samples {{ allocations: {prefix}, frees: {prefix}, requested_headroom: 0, usable_headroom: 0 }}").unwrap();
        }
        output.push_str("sort join-bytes cancellation event=0 terminal=2 retry_rows=3022\n");
        output.push_str(
            "sort joined-byte growth passed: complete rows, replacement fallback and release\n",
        );
        replacement_growth(&output, Replacement::JoinedBytes, true).unwrap();
        for (from, to) in [
            ("bytes=230000", "bytes=0"),
            ("files=2", "files=1"),
            ("descriptors=7", "descriptors=3"),
            ("rows=3022", "rows=3086"),
            ("refusals=1", "refusals=0"),
            ("allocations: 1", "allocations: 0"),
            ("frees: 1", "frees: 0"),
            ("usable_headroom: 0", "usable_headroom: -1"),
            ("retry_rows=3022", "retry_rows=3021"),
        ] {
            assert!(
                replacement_growth(&output.replace(from, to), Replacement::JoinedBytes, true)
                    .is_err(),
                "{from}"
            );
        }
        for prefix in [
            "sort join-bytes retained",
            "sort join-bytes event=",
            "sort join-bytes cancellation",
        ] {
            let row = output
                .lines()
                .find(|line| line.starts_with(prefix))
                .unwrap();
            assert!(
                replacement_growth(
                    &output.replace(&format!("{row}\n"), ""),
                    Replacement::JoinedBytes,
                    true
                )
                .is_err()
            );
            assert!(
                replacement_growth(&format!("{output}{row}\n"), Replacement::JoinedBytes, true)
                    .is_err()
            );
        }
        let mac = output
            .replace("bytes=230000", "bytes=0")
            .replace("files=2", "files=0");
        replacement_growth(&mac, Replacement::JoinedBytes, false).unwrap();
        assert!(replacement_growth(&mac, Replacement::JoinedBytes, true).is_err());
        assert!(replacement_growth(&output, Replacement::JoinedBytes, false).is_err());
    }

    fn trace() -> String {
        let mut output = String::new();
        for phase in ["preparation", "construction"] {
            writeln!(output, "join {phase} census allocations=2").unwrap();
            writeln!(
                output,
                "join {phase} prefix=0 calls=1 refusals=1 samples=none"
            )
            .unwrap();
            for (prefix, calls, refusals, frees) in [(1, 2, 1, 1), (2, 2, 0, 0)] {
                writeln!(output, "join {phase} prefix={prefix} calls={calls} refusals={refusals} samples=Samples {{ allocations: {prefix}, frees: {frees}, requested_headroom: 0, usable_headroom: 0 }}").unwrap();
            }
            let suffix = if phase == "preparation" {
                "live errors, owned span and release"
            } else {
                "live errors and release"
            };
            writeln!(
                output,
                "wide left join {phase} failures passed: prefixes=0..=2; {suffix}"
            )
            .unwrap();
        }
        for expression in EXPRESSIONS {
            writeln!(output, "wide left join demanded failure: expression={expression}; step=2; temporary=1; Samples {{ allocations: 0, frees: 1, requested_headroom: 0, usable_headroom: 0 }}").unwrap();
        }
        writeln!(output, "{DONE}\n{LIFECYCLE}\n{EXECUTION}\ntransient ownership calibration passed: hidden allocation detected; entry/exit agree").unwrap();
        output
    }

    #[test]
    fn joins_reject_missing_prefixes_wrong_errors_and_incomplete_release() {
        let healthy = trace();
        validate(&healthy, true).unwrap();
        for line in healthy.lines() {
            assert!(
                validate(&healthy.replacen(&format!("{line}\n"), "", 1), true).is_err(),
                "accepted missing {line}"
            );
        }
        for (from, to) in [
            ("prefix=1 calls=2", "prefix=0 calls=2"),
            ("refusals=1", "refusals=0"),
            ("allocations: 1, frees: 1", "allocations: 1, frees: 0"),
            ("usable_headroom: 0", "usable_headroom: -1"),
            ("step=2", "step=1"),
            ("temporary=1", "temporary=0"),
            ("expression=LOG10", "expression=ABS"),
        ] {
            assert!(
                validate(&healthy.replacen(from, to, 1), true).is_err(),
                "accepted {from}"
            );
        }
        assert!(validate(&(healthy.clone() + &healthy), true).is_err());
    }
}
