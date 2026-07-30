use std::{env, fs, path::PathBuf};

use cbindgen::{Builder, Config};

fn main() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut config =
        Config::from_file(crate_dir.join("cbindgen.toml")).expect("failed to load cbindgen.toml");
    config
        .parse
        .expand
        .crates
        .retain(|crate_name| crate_name == "waterui-ffi");
    config.parse.expand.features = Some(vec![String::from("cef-header")]);
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
