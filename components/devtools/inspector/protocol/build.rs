//! Build metadata injection for the `WaterUI` inspector protocol crate.

use std::{env, path::Path};

/// Hex digits of the abbreviated commit the handshake carries.
const COMMIT_ABBREVIATION: u8 = 12;

fn main() {
    println!("cargo:rerun-if-env-changed=WATERUI_INSPECTOR_PROTOCOL_COMMIT");
    println!("cargo:rerun-if-env-changed=WATERUI_CLI_COMMIT");

    let commit = env::var("WATERUI_INSPECTOR_PROTOCOL_COMMIT")
        .ok()
        .or_else(|| env::var("WATERUI_CLI_COMMIT").ok())
        .or_else(|| {
            let crate_dir = env::var_os("CARGO_MANIFEST_DIR")
                .expect("Cargo sets CARGO_MANIFEST_DIR for build scripts");
            waterui_git_head::head_commit(Path::new(&crate_dir), Some(COMMIT_ABBREVIATION))
        })
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=WATERUI_INSPECTOR_PROTOCOL_COMMIT={commit}");
}
