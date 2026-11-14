//! Precompiles `src/capture_composite.metal` and `src/mesh_gradient.metal`
//! into metallibs loaded with `MTLDevice::newLibraryWithData` (the capture
//! compositor, waterui#2827, and the canvas mesh gradient pass).
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

/// The shaders the crate precompiles, each into `<name>.metallib` under
/// `OUT_DIR`: the capture compositor and the canvas's mesh gradient pass.
const SHADERS: [&str; 2] = ["capture_composite", "mesh_gradient"];

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    for name in SHADERS {
        println!(
            "cargo::rerun-if-changed={}",
            manifest.join(format!("src/{name}.metal")).display()
        );
    }
    let Some(apple) = metal::apple_target() else {
        // The crate compiles only on Apple targets, but a docs/cargo run
        // on another host still runs this script — nothing to emit there.
        return;
    };
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    // The MSL is written for Metal 2.3 — the first language version whose
    // toolchain takes `-ftexture-write-rounding-mode` — within the
    // macOS/iOS 26 deployment floor `apple_target` enforces.
    for name in SHADERS {
        metal::compile_metal(
            &out_dir,
            name,
            &manifest.join(format!("src/{name}.metal")),
            &apple,
            (2, 3),
            None,
            &[],
        );
    }
}
