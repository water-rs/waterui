//! Records where the crate's `SwiftUI`/`WidgetKit` extension sources live.
//!
//! The `water` CLI compiles the generated `WidgetKit` extension from the
//! `swift/` sources shipped with this crate — on a path checkout, a git
//! dependency, or a crates.io package alike — so the path is baked into the
//! crate's compilation environment and from there into the
//! `waterui_meta_widgets_swift_sources` metadata record.

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    println!(
        "cargo:rustc-env=WATERUI_WIDGETS_SWIFT_SOURCES={}",
        manifest_dir.join("swift").display()
    );
    println!("cargo:rerun-if-changed=swift");
}
