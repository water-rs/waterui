//! Forwards `DEP_WATERUI_FEATURES` from the `waterui` facade into
//! `waterui_feature` cfgs so optional component `WidgetTheme` methods stay
//! aligned with `backends/hydrolysis`.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo::rustc-check-cfg=cfg(waterui_feature, values(any()))");
    let features = std::env::var("DEP_WATERUI_FEATURES")
        .expect("the `waterui` facade's build script exports DEP_WATERUI_FEATURES");
    for feature in features.split(',').filter(|feature| !feature.is_empty()) {
        println!("cargo::rustc-cfg=waterui_feature=\"{feature}\"");
    }
}
