//! Build script for waterui-cli.
//!
//! Embeds the git commit hash as an environment variable for runtime detection.

use std::process::Command;

fn main() {
    // Get the current git commit hash
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok()).map_or_else(|| "unknown".to_string(), |s| s.trim().to_string());

    println!("cargo:rustc-env=WATERUI_CLI_COMMIT={output}");

    // Rerun if git HEAD changes
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs/heads/");
}
