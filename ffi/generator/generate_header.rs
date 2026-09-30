//! Regenerates `ffi/waterui.h` from the `waterui-ffi` crate via cbindgen and
//! propagates the header to the native backend submodules.
//!
//! The generator lives in its own crate so that building it does not build the
//! framework: its only dependency is cbindgen, and the single compile of the
//! `waterui-ffi` graph left is the check-profile expansion cbindgen drives
//! itself. Both the cbindgen configuration and the feature list stay where they
//! were — `ffi/cbindgen.toml` and the `header` feature of `ffi/Cargo.toml` —
//! and are read from the sibling crate directory.

use std::{env, fs, path::PathBuf};

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
    // Every native backend syncs its copy from `ffi/waterui.h` in its own
    // CI; none rides a gitlink in this tree any more.
    fs::write(&header_path, header_bytes).expect("failed to write generated header");
}

/// Directory of the `waterui-ffi` crate this generator binds, which is the
/// parent of this crate's own directory.
fn ffi_crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("failed to determine the FFI crate directory from the generator manifest path")
        .to_path_buf()
}
