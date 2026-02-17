use std::env;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=WATERUI_GRAPHICS_COMMIT");

    if let Ok(commit) = env::var("WATERUI_GRAPHICS_COMMIT")
        && !commit.trim().is_empty()
    {
        println!("cargo:rustc-env=WATERUI_GRAPHICS_COMMIT={}", commit.trim());
        return;
    }

    let commit = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());

    println!("cargo:rustc-env=WATERUI_GRAPHICS_COMMIT={commit}");
}
