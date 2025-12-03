//! Build script for WaterUI projects.
//!
//! This script configures the `waterui_enable_hot_reload` cfg flag
//! which is used by the `waterui_ffi::export!()` macro.

use std::env;

fn main() {
    // Declare the custom cfg so Rust doesn't warn about it
    println!("cargo:rerun-if-env-changed=WATERUI_ENABLE_HOT_RELOAD");
    println!("cargo:rustc-check-cfg=cfg(waterui_enable_hot_reload)");

    // Enable hot reload cfg when requested
    let enable = env::var("WATERUI_ENABLE_HOT_RELOAD")
        .map(|val| val != "0")
        .unwrap_or(false);
    if enable {
        println!("cargo:rustc-cfg=waterui_enable_hot_reload");
    }
}
