//! Builds the native Windows sandbox ownership bridge.

use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-changed=native/windows_sandbox.cc");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let cef_directory = PathBuf::from(
        std::env::var("DEP_CEF_DLL_WRAPPER_CEF_DIR")
            .expect("cef-dll-sys did not expose its CEF distribution directory"),
    );
    cc::Build::new()
        .cpp(true)
        .include(&cef_directory)
        .file("native/windows_sandbox.cc")
        .flag_if_supported("/std:c++17")
        .static_crt(true)
        .warnings_into_errors(true)
        .compile("waterui_cef_windows_sandbox");
    println!(
        "cargo::rustc-link-search=native={}",
        cef_directory.display()
    );
    println!("cargo::rustc-link-lib=static=cef_sandbox");
}
