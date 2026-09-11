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

/// The last commit that changed this crate, not the last commit in the tree.
///
/// This value is a *protocol* version: the CLI refuses to talk to a support app
/// whose value differs, because the two would be speaking different wire
/// formats. Taking the workspace `HEAD` instead made every commit anywhere
/// declare a new protocol, so a `water` binary built before your last commit
/// could never preview anything again — the handshake rejected the app it had
/// just built, retried until the startup backstop, and reported the failure as
/// a TCP server that would not start.
///
/// Runtime compatibility is a separate question and already has an answer:
/// `waterui_core_fingerprint` carries the package version, the feature set, the
/// dependency-graph hash and the profile, and the CLI computes what it expects
/// from the tree at run time rather than baking it in. That is what catches an
/// app built against a different runtime. This value only has to change when
/// the protocol does.
fn resolve_git_commit(crate_dir: &Path) -> Option<String> {
    git_output(
        crate_dir,
        &["log", "-1", "--format=%h", "--abbrev=12", "--", "."],
    )
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
