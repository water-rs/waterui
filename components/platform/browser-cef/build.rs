//! Builds the platform CEF application and sandbox bridges.

use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-changed=native/macos_application.mm");
    println!("cargo::rerun-if-changed=native/windows_sandbox.cc");
    if std::env::var_os("CARGO_FEATURE_CEF_RUNTIME").is_none() {
        return;
    }

    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => build_macos_application(),
        Ok("windows") => build_windows_sandbox(),
        _ => {}
    }
}

fn cef_directory() -> PathBuf {
    PathBuf::from(
        std::env::var("DEP_CEF_DLL_WRAPPER_CEF_DIR")
            .expect("cef-dll-sys did not expose its CEF distribution directory"),
    )
}

fn build_macos_application() {
    let cef_directory = cef_directory();
    cc::Build::new()
        .cpp(true)
        .file("native/macos_application.mm")
        .flag("-isystem")
        .flag(cef_directory)
        .flag_if_supported("-std=c++20")
        .warnings_into_errors(true)
        .compile("waterui_cef_macos_application");
    println!("cargo::rustc-link-lib=framework=AppKit");
}

fn build_windows_sandbox() {
    let cef_directory = cef_directory();
    cc::Build::new()
        .cpp(true)
        .include(&cef_directory)
        .file("native/windows_sandbox.cc")
        .flag_if_supported("/std:c++20")
        .static_crt(true)
        .warnings_into_errors(true)
        .compile("waterui_cef_windows_sandbox");
    println!(
        "cargo::rustc-link-search=native={}",
        cef_directory.display()
    );
    println!("cargo::rustc-link-lib=static=cef_sandbox");
}
