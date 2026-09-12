//! Cleanup for an owned test directory; never mask the original scenario failure.

pub(crate) fn directory(path: &std::path::Path) {
    if let Err(error) = std::fs::remove_dir_all(path) {
        // Setup may have failed before creating the directory, or a scenario
        // may already have removed it. Other errors must fail a successful test.
        if error.kind() != std::io::ErrorKind::NotFound && !std::thread::panicking() {
            panic!("remove test directory {}: {error}", path.display());
        }
    }
}
