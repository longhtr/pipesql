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
