//! Regenerates `ffi/waterui.h` from the `waterui-ffi` crate via cbindgen and
//! propagates the header to the native backend submodules.
//!
//! The generator lives in its own crate so that building it does not build the
//! framework: its only dependency is cbindgen, and the single compile of the
//! `waterui-ffi` graph left is the check-profile expansion cbindgen drives
//! itself. Both the cbindgen configuration and the feature list stay where they
//! were — `ffi/cbindgen.toml` and the `header` feature of `ffi/Cargo.toml` —
//! and are read from the sibling crate directory.

use std::{env, fs, path::Path, path::PathBuf};

use cbindgen::{Builder, Config};

fn main() {
    // cbindgen expands macros by running `cargo rustc -- -Zunpretty=expanded`,
    // which prints expanded source instead of writing the declared artifacts.
    // Artifact-caching rustc wrappers (sccache) fail while collecting those
    // missing outputs, so the expansion subprocess must run unwrapped. The
    // expansion produces nothing cacheable; every real build keeps its wrapper.
    // SAFETY: called at the start of `main`, before any other thread exists.
    unsafe { env::set_var("RUSTC_WRAPPER", "") };
    let crate_dir = ffi_crate_dir();
    let mut config =
        Config::from_file(crate_dir.join("cbindgen.toml")).expect("failed to load cbindgen.toml");
    config
        .parse
        .expand
        .crates
        .retain(|crate_name| crate_name == "waterui-ffi");
    // `header` aggregates every optional C surface; see `ffi/Cargo.toml`.
    config.parse.expand.features = Some(vec![String::from("header")]);
    let bindings = Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .expect("Unable to generate bindings");
    let mut header_bytes = Vec::new();
    bindings.write(&mut header_bytes);
    let header_path = crate_dir.join("waterui.h");
    fs::write(&header_path, header_bytes).expect("failed to write generated header");
    propagate_to_backends(&header_path);
}

/// Directory of the `waterui-ffi` crate this generator binds, which is the
/// parent of this crate's own directory.
fn ffi_crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("failed to determine the FFI crate directory from the generator manifest path")
        .to_path_buf()
}

/// Copies the freshly generated header over the native backends' checked-in
/// copies, which CI compares against this one.
fn propagate_to_backends(header_path: &Path) {
    let workspace_root = header_path
        .parent()
        .and_then(Path::parent)
        .expect("failed to determine workspace root from FFI header path");

    let destinations = [
        workspace_root.join("backends/apple/Sources/CWaterUI/include/waterui.h"),
        workspace_root.join("backends/android/runtime/src/main/cpp/waterui.h"),
    ];

    for dest in destinations {
        fs::copy(header_path, &dest)
            .unwrap_or_else(|error| panic!("failed to copy header to {}: {error}", dest.display()));
    }
}
