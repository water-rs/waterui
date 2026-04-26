//! Build metadata injection for the `WaterUI` inspector protocol crate.

use std::{env, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=WATERUI_INSPECTOR_PROTOCOL_COMMIT");
    println!("cargo:rerun-if-env-changed=WATERUI_CLI_COMMIT");

    let commit = env::var("WATERUI_INSPECTOR_PROTOCOL_COMMIT")
        .ok()
        .or_else(|| env::var("WATERUI_CLI_COMMIT").ok())
        .or_else(resolve_git_commit)
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=WATERUI_INSPECTOR_PROTOCOL_COMMIT={commit}");
}

fn resolve_git_commit() -> Option<String> {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").ok()?;
    let output = Command::new("git")
        .arg("-C")
        .arg(crate_dir)
        .arg("rev-parse")
        .arg("--short=12")
        .arg("HEAD")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let commit = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!commit.is_empty()).then_some(commit)
}
