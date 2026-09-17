//! Build the C bridge needed to intercept Darwin's variadic fcntl calls.
//!
//! Only macOS needs this object; Linux uses the Rust wrappers directly. The bridge
//! handles the native calling convention while Rust owns observation and fault
//! decisions. Cargo rebuilds it when the C source changes. Compilation failure or
//! a warning stops the build rather than producing an observer without the bridge.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../bindings/fcntl.c");
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() != "macos" {
        return;
    }
    let object = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("fcntl.o");
    let status = Command::new("cc")
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-fPIC",
            "-c",
            "../bindings/fcntl.c",
            "-o",
        ])
        .arg(&object)
        .status()
        .expect("launch native compiler");
    assert!(status.success(), "native compiler failed");
    println!("cargo:rustc-link-arg={}", object.display());
}
