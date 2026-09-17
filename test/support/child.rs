//! Keep a test's child process from outliving its temporary inputs.
//!
//! Wrap the child immediately after spawning it, after creating its directory
//! owner. On drop, the guard checks whether the child has exited. If it is still
//! running, the guard kills it and waits for its exit before directory cleanup.
//! A cleanup error fails the test unless it is already unwinding from a panic.
//!
//! Tests must check readiness, deadlines and the expected exit status themselves.
//! This guard owns one direct child; it cannot clean up descendants.

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
            panic!("stop test child and wait for exit: {error}");
        }
    }
}
