//! Build script configuring compile-time options for WaterUI.

use std::env;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(waterui_disable_hot_reload)");
    let disable = env::var("WATERUI_DISABLE_HOT_RELOAD")
        .map(|val| val != "0")
        .unwrap_or(false);
    if disable {
        println!("cargo:rustc-cfg=waterui_disable_hot_reload");
    }
}
