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
