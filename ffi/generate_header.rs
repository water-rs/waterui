//! Regenerates `ffi/waterui.h` from the `waterui-ffi` crate via cbindgen and
//! propagates the header to the native backend submodules.
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
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
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

fn propagate_to_backends(header_path: &std::path::Path) {
    let workspace_root = header_path
        .parent()
        .and_then(|p| p.parent())
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
