//! Check that ThreadSanitizer distinguishes atomic writes from a data race.
//!
//! Two threads start together and write the same cell. `clean` uses atomic stores;
//! `fault` deliberately writes through an unsynchronized raw pointer. Run the
//! fault only with ThreadSanitizer. The runner requires its data-race report and
//! configured failure status; an ordinary panic does not establish detection.

use std::cell::UnsafeCell;
use std::sync::Barrier;
use std::sync::atomic::{AtomicUsize, Ordering};

struct RacyCell(UnsafeCell<usize>);

// Deliberately unsound for the isolated race control, never a production owner.
unsafe impl Sync for RacyCell {}

impl RacyCell {
    unsafe fn write(&self, value: usize) {
        // Volatile keeps the conflicting accesses observable to instrumentation.
        unsafe { self.0.get().write_volatile(value) };
    }
}

fn main() {
    let mut arguments = std::env::args().skip(1);
    let mode = arguments.next().expect("clean or fault");
    assert!(arguments.next().is_none() && (mode == "clean" || mode == "fault"));
    let fault = mode == "fault";
    let racy = RacyCell(UnsafeCell::new(0));
    let atomic = AtomicUsize::new(0);
    let start = Barrier::new(2);
    std::thread::scope(|scope| {
        let write = || {
            start.wait();
            for value in 0..1000 {
                if fault {
                    unsafe { racy.write(value) };
                } else {
                    atomic.store(value, Ordering::Relaxed);
                }
            }
        };
        scope.spawn(write);
        write();
    });
    println!("thread control clean");
}
