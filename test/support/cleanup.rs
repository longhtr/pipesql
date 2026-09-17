//! Remove an owned test directory and report cleanup failures.
//!
//! Call this after dropping handles that use the directory. A missing directory
//! counts as success because the test may already have removed it. Other errors
//! fail a healthy test. During unwinding, they are suppressed: a second panic
//! would abort the test process and hide the original failure.

pub(crate) fn directory(path: &std::path::Path) {
    if let Err(error) = std::fs::remove_dir_all(path)
        && error.kind() != std::io::ErrorKind::NotFound
        && !std::thread::panicking()
    {
        panic!("remove test directory {}: {error}", path.display());
    }
}
