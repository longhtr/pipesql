//! Run allocation checks across data shapes and allocator reuse histories.
//!
//! The campaign selects driver workloads for types, widths, prepared aggregates,
//! legacy constants and overlapping readers. Short and long database paths exercise
//! different retained pathname sizes. Required completion lines and observed
//! allocator thresholds establish that each requested case ran. Negative controls
//! must fail with their expected diagnostic rather than merely exit unsuccessfully.

use super::{Campaign, Result, require_line};

impl Campaign {
    pub(super) fn shapes(&mut self) -> Result<()> {
        for length in [None, Some(384)] {
            for (mode, complete) in [
                (
                    "analytic-shapes",
                    "analytic shapes passed: 25 cases; rows, attribution and release",
                ),
                (
                    "wide-set-shapes",
                    "wide set shapes passed: 6 cases; complete rows, step ownership and release",
                ),
                (
                    "prepared-aggregate-shapes",
                    "prepared aggregate shapes passed: 14 accepted and 54 rejected; attribution, rows and release",
                ),
                (
                    "legacy-constant-shapes",
                    "legacy constant shapes passed: 6 direct and 4 extrema cases; rows, attribution and release",
                ),
            ] {
                require_line(&self.execute(mode, length)?, complete)?;
                println!("{mode}: pathname={length:?} passed");
            }
        }
        for (mode, complete) in [
            (
                "joined-shapes",
                "joined shapes passed: 2 budgets; complete rows, step ownership and release",
            ),
            (
                "mixed-grouping-shapes",
                "mixed shapes completed: 40 cases, deficits=0",
            ),
            (
                "grouped-allocation-shapes",
                "grouped allocation shapes passed: buffers=514 hash-layouts=91",
            ),
            (
                "append-allocation-shapes",
                "append allocation shapes passed: workspace=460865 references=4096",
            ),
            // Keep reuse history in a fresh process. The preceding size census
            // leaves better-fitting cached blocks that could conceal large reuse.
            (
                "append-cached-buffers",
                "append shapes passed: full-width maximum-column growth reuse publication release",
            ),
        ] {
            self.completed(mode, complete)?;
            println!("{mode}: passed");
        }
        let thresholds: &[Option<usize>] = if cfg!(target_os = "linux") {
            &[None, Some(131_072), Some(67_108_864)]
        } else {
            &[None]
        };
        for &threshold in thresholds {
            for length in [None, Some(384)] {
                let output =
                    self.execute_with_threshold("reader-allocation-shapes", length, threshold)?;
                require_line(
                    &output,
                    "reader shapes passed: 12 fixed-width and 8 STRING ordering/distinct cases; rows, admission and release",
                )?;
                if let Some(threshold) = threshold {
                    let marker = format!("reader allocator threshold={threshold} observed ");
                    if output
                        .lines()
                        .filter(|line| line.starts_with(&marker))
                        .count()
                        != 1
                    {
                        return Err("reader allocator threshold was not observed".into());
                    }
                }
                println!("reader shapes: threshold={threshold:?}, pathname={length:?} passed");
            }
        }
        for (mode, diagnostic) in [
            (
                "analytic-attribution-negative",
                "execution ownership attribution: analytic-admitted",
            ),
            (
                "prepared-aggregate-attribution-negative",
                "prepared ownership attribution",
            ),
            (
                "legacy-constant-attribution-negative",
                "execution ownership attribution: legacy-text",
            ),
            (
                "joined-attribution-negative",
                "joined usable ownership attribution",
            ),
            (
                "wide-set-attribution-negative",
                "wide set usable ownership attribution",
            ),
            (
                "append-allocation-shapes-negative",
                "append allocation rounding",
            ),
        ] {
            self.rejected(mode, diagnostic)?;
        }
        Ok(())
    }
}
