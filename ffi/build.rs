use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=WATERUI_ENABLE_HOT_RELOAD");
    println!("cargo:rustc-check-cfg=cfg(waterui_enable_hot_reload)");
    let enable = env::var("WATERUI_ENABLE_HOT_RELOAD")
        .map(|val| val != "0")
        .unwrap_or(false);
    if enable {
        println!("cargo:rustc-cfg=waterui_enable_hot_reload");
    }
}
