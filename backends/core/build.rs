//! Turns the framework's enabled features into `waterui_feature` cfgs.
//!
//! `waterui-core` exports them as `DEP_WATERUI_FEATURES` (its `build.rs`), and
//! `NativeBackend` declares a method for a component only when the feature
//! providing it is enabled.

use std::env;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rustc-check-cfg=cfg(waterui_feature, values(any()))");
    let features = env::var("DEP_WATERUI_FEATURES")
        .expect("`waterui-core`'s build script exports DEP_WATERUI_FEATURES");
    for feature in features.split(',').filter(|feature| !feature.is_empty()) {
        println!("cargo::rustc-cfg=waterui_feature=\"{feature}\"");
    }
}
