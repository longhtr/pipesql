//! Observe spill and release query owners in analytical examples.
//!
//! Linux observations require nonempty unlinked files, then closed descriptors
//! after disposal. Other platforms check resource accounts. Cancellation waits
//! for spill before requesting failure; completed queries are not cancellation.
//! These helpers share I/O mechanics, not expected analytical answers.

use pipesql::{CancellationToken, Database, PreparedQuery, QueryStep};
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

// Scratch files have no directory names while in use. Read the process's open
// descriptors during the untimed warm-up rather than mistaking directory cleanup
// or a reservation alone for written spill data. Other open files are ignored.
#[cfg(target_os = "linux")]
pub(super) fn scratch_usage(path: &Path) -> Result<(usize, u64)> {
    let mut files = 0;
    let mut bytes = 0;
    for entry in std::fs::read_dir("/proc/self/fd")? {
        let entry = entry?;
        let target = match std::fs::read_link(entry.path()) {
            Ok(target) => target,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if target.starts_with(path) && target.to_string_lossy().ends_with(" (deleted)") {
            files += 1;
            bytes += std::fs::metadata(entry.path())?.len();
        }
    }
    Ok((files, bytes))
}

#[cfg(not(target_os = "linux"))]
pub(super) fn scratch_usage(_: &Path) -> Result<(usize, u64)> {
    Ok((0, 0))
}

pub(super) fn released(db: &Database, baseline: u64, path: &Path) -> Result<()> {
    if db.reserved_memory_bytes() != baseline
        || db.reserved_temp_bytes() != 0
        || scratch_usage(path)?.0 != 0
    {
        return Err("query resources remain after cursor disposal".into());
    }
    Ok(())
}

pub(super) fn cancel(db: &Database, query: &PreparedQuery<'_>, path: &Path) -> Result<()> {
    let baseline = db.reserved_memory_bytes();
    let token = CancellationToken::new();
    let mut result = db.execute(query, &token)?;
    let mut spilled = false;
    for _ in 0..20_000_000 {
        // Rows are still unfinished work. A producer can spill and emit its
        // first row within one public step without an intervening Progress.
        if !matches!(result.step(), QueryStep::Progress | QueryStep::Rows(_)) {
            return Err("query ended before spill cancellation".into());
        }
        if db.reserved_temp_bytes() > 0
            && (!cfg!(target_os = "linux") || scratch_usage(path)?.1 > 0)
        {
            spilled = true;
            break;
        }
    }
    if !spilled {
        return Err("query did not reach cancellation point".into());
    }
    token.cancel();
    for _ in 0..2 {
        if !matches!(result.step(), QueryStep::Failed(pipesql::Error::Cancelled)) {
            return Err("missing terminal cancellation error".into());
        }
    }
    drop(result);
    released(db, baseline, path)
}
