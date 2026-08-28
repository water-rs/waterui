//! Build script for the `waterui-ffi` crate.
//!
//! Links the C++ standard library on Android, which the native NDK toolchain
//! requires but does not link by default.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-lib=dylib=c++_shared");
    }
}
