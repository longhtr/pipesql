//! Reap a test child before its temporary inputs can be removed.
//!
//! Drop kills a still-running child, then waits for it. Cleanup errors fail a
//! healthy test without masking an existing panic. Callers own readiness,
//! deadlines and exit-status checks. This guard handles only direct children;
//! these test children must not start descendants.

pub(crate) struct ChildProcess(pub(crate) std::process::Child);

impl Drop for ChildProcess {
    fn drop(&mut self) {
        let cleanup = (|| -> std::io::Result<()> {
            if self.0.try_wait()?.is_none() {
                self.0.kill()?;
                self.0.wait()?;
            }
            Ok(())
        })();
        if let Err(error) = cleanup
            && !std::thread::panicking()
        {
            panic!("reap test child: {error}");
        }
    }
}
