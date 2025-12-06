//! Build script for WaterUI projects.
//!
//! This script forwards hot reload configuration from CLI environment
//! variables to compile-time constants accessible via `option_env!`.

fn main() {
    // Forward hot reload configuration from CLI to compile-time env
    println!("cargo:rerun-if-env-changed=WATERUI_HOT_RELOAD_HOST");
    println!("cargo:rerun-if-env-changed=WATERUI_HOT_RELOAD_PORT");

    if let Ok(host) = std::env::var("WATERUI_HOT_RELOAD_HOST") {
        println!("cargo:rustc-env=WATERUI_HOT_RELOAD_HOST={host}");
    }
    if let Ok(port) = std::env::var("WATERUI_HOT_RELOAD_PORT") {
        println!("cargo:rustc-env=WATERUI_HOT_RELOAD_PORT={port}");
    }
}
