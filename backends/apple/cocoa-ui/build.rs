//! Precompiles `src/capture_composite.metal` into a metallib the capture
//! compositor loads with `MTLDevice::newLibraryWithData` (waterui#2827).
//!
//! A shader compiled from source at run time always gets the platform's
//! default texture-store rounding — `MTLCompileOptions` has no rounding
//! switch, the mode is a `metal` driver flag only — and Apple GPUs default
//! to toward-zero, which lands every `RGBA16Float` channel up to one f16
//! step below the nearest value. Compiling at build time with
//! `-ftexture-write-rounding-mode=rte` rounds to nearest even, matching
//! the Cherenkov engine passes the composite feeds.
//!
//! The Apple SDK/`-target` selection and the `metal -c`/`metallib` driver
//! come from the module `cherenkov-gpu`'s `build.rs` declares, included
//! here by path so the two never drift.

#[path = "../../../graphics/cherenkov/gpu/build_metal.rs"]
mod metal;

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let source = manifest.join("src/capture_composite.metal");
    println!("cargo::rerun-if-changed={}", source.display());
    let Some(apple) = metal::apple_target() else {
        // The crate compiles only on Apple targets, but a docs/cargo run
        // on another host still runs this script — nothing to emit there.
        return;
    };
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    // The compositor's MSL is written for Metal 2.3 — the first language
    // version whose toolchain takes `-ftexture-write-rounding-mode` —
    // within the macOS/iOS 26 deployment floor `apple_target` enforces.
    metal::compile_metal(
        &out_dir,
        "capture_composite",
        &source,
        &apple,
        (2, 3),
        None,
        &[],
    );
}
