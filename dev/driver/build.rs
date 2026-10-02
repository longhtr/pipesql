//! Compile the platform driver's independent checks against installed C headers.
//!
//! The C object supplies mutex layout, stack minimum and function-signature checks
//! for comparison with Rust's native bindings. Cargo links it only into the
//! platform executable. Compiler warnings are errors, and a failed compiler launch
//! or header assertion stops the build before runtime checks can claim success.

use std::{path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=../bindings/platform.c");
    let object = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("platform.o");
    let status = Command::new("cc")
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-fPIC",
            "-c",
            "../bindings/platform.c",
            "-o",
        ])
        .arg(&object)
        .status()
        .expect("launch SDK compiler");
    assert!(status.success(), "SDK assertions failed");
    println!("cargo:rustc-link-arg-bin=platform={}", object.display());
}
