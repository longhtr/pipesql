//! Demonstrate that AddressSanitizer observes an out-of-bounds heap read.
//!
//! The clean mode reads the fourth element; fault mode reads one past it through
//! the same volatile pointer path. check-native-sanitizer.py requires both normal
//! completion and the specific sanitizer failure before accepting engine tests.
//! The deliberate unsafe access is confined to this diagnostic executable.

fn main() {
    let mode = std::env::args().nth(1).expect("clean or fault");
    assert!(mode == "clean" || mode == "fault");
    let values = Box::new([11_u64, 22, 33, 44]);
    let index = std::hint::black_box(if mode == "fault" { 4 } else { 3 });
    // Deliberate isolated bounds violation verifies compiler instrumentation.
    // The clean mode reads the final element of the same live allocation.
    let value = unsafe { std::ptr::read_volatile(values.as_ptr().add(index)) };
    assert_eq!(value, 44);
    println!("address control clean");
}
