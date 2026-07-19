//! Build metadata injection for the `WaterUI` preview protocol crate.

use std::{env, path::Path, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=WATERUI_PREVIEW_PROTOCOL_COMMIT");
    println!("cargo:rerun-if-env-changed=WATERUI_CLI_COMMIT");

    let crate_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    track_git_commit_inputs(Path::new(&crate_dir));

    let commit = env::var("WATERUI_PREVIEW_PROTOCOL_COMMIT")
        .ok()
        .or_else(|| env::var("WATERUI_CLI_COMMIT").ok())
        .or_else(|| resolve_git_commit(Path::new(&crate_dir)))
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=WATERUI_PREVIEW_PROTOCOL_COMMIT={commit}");
}

fn track_git_commit_inputs(crate_dir: &Path) {
    for name in ["HEAD", "packed-refs"] {
        if let Some(path) = resolve_git_path(crate_dir, name) {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    if let Some(reference) = git_output(crate_dir, &["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = resolve_git_path(crate_dir, &reference)
    {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn resolve_git_commit(crate_dir: &Path) -> Option<String> {
    git_output(crate_dir, &["rev-parse", "--short=12", "HEAD"])
}

fn resolve_git_path(crate_dir: &Path, name: &str) -> Option<std::path::PathBuf> {
    git_output(
        crate_dir,
        &["rev-parse", "--path-format=absolute", "--git-path", name],
    )
    .map(Into::into)
}

fn git_output(crate_dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(crate_dir)
        .args(args)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!value.is_empty()).then_some(value)
}
