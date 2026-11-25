//! Records where the crate's `SwiftUI`/`WidgetKit` extension sources live and
//! compiles the Apple `WidgetKit` reload / App Group container bridge on Apple
//! targets.
//!
//! The `water` CLI compiles the generated `WidgetKit` extension from the
//! `swift/` sources shipped with this crate — on a path checkout, a git
//! dependency, or a crates.io package alike — so the path is baked into the
//! crate's compilation environment and from there into the
//! `waterui_meta_widgets_swift_sources` metadata record.

use std::env;
use std::path::PathBuf;
use waterkit_build::SwiftModule;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    println!(
        "cargo:rustc-env=WATERUI_WIDGETS_SWIFT_SOURCES={}",
        manifest_dir.join("swift").display()
    );
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=swift");
    println!("cargo:rerun-if-changed=swift_bridge/AppleBridge.swift");
    println!("cargo:rerun-if-changed=kotlin/WaterUIWidgetHost.kt");

    compile_apple_bridge();
}

/// Compiles `swift_bridge/AppleBridge.swift` when building for an Apple target
/// (`macos` or `ios`), exposing `WidgetCenter` timeline reload and App Group
/// shared-container resolution to Rust.
fn compile_apple_bridge() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" || target_os == "ios" {
        SwiftModule::new("WaterUIWidgetsBridge")
            .swift_source("swift_bridge/AppleBridge.swift")
            .framework("Foundation")
            .framework("WidgetKit")
            .compiler_arg("-application-extension")
            .compile();
    }
}
