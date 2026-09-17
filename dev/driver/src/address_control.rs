//! Prove that the diagnostic build detects a read past a heap allocation.
//!
//! The development supervisor builds this separate executable with AddressSanitizer.
//! Both modes read through a raw pointer into a four-element array. `clean` reads
//! element 3 and must finish normally; `fault` reads element 4 and must produce a
//! heap-buffer-overflow report with the configured failure exit code. An ordinary
//! assertion failure does not satisfy the runner: it would not prove that the
//! sanitizer caught the access before the value was used.

fn main() {
    let mode = std::env::args().nth(1).expect("clean or fault");
    assert!(mode == "clean" || mode == "fault");
    let values = Box::new([11_u64, 22, 33, 44]);
    let index = std::hint::black_box(if mode == "fault" { 4 } else { 3 });
    // Deliberately undefined in fault mode; never run that mode without the
    // sanitizer. Volatile reading and black_box keep the access observable.
    let value = unsafe { std::ptr::read_volatile(values.as_ptr().add(index)) };
    assert_eq!(value, 44);
    println!("address control clean");
}
