//! Remove a test's owned directory without replacing the failure being diagnosed.
//!
//! A missing directory is harmless: setup may have failed before creating it, or
//! the scenario may already have removed it. Other removal errors fail a healthy
//! test. During panic unwinding, preserve the original failure instead of panicking
//! again, which would abort the process. Callers must pass only directories they
//! own and drop open database/file owners before requesting removal.

pub(crate) fn directory(path: &std::path::Path) {
    if let Err(error) = std::fs::remove_dir_all(path) {
        // Setup may have failed before creating the directory, or a scenario
        // may already have removed it. Other errors must fail a successful test.
        if error.kind() != std::io::ErrorKind::NotFound && !std::thread::panicking() {
            panic!("remove test directory {}: {error}", path.display());
        }
    }
}
