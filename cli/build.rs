use std::{path::Path, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../Cargo.toml");
    println!("cargo:rerun-if-changed=../backends/apple/Package.swift");

    emit_waterui_version();
    emit_swift_backend_version();
}

fn emit_waterui_version() {
    let repo_path = Path::new("..");
    let version = git_tag_version(repo_path, "waterui", "waterui-v*");

    match version {
        Some(version) => {
            println!("cargo:rustc-env=WATERUI_VERSION={}", version);
        }
        None => {
            eprintln!(
                "cargo:warning=No stable version tag found for 'waterui'. Use `water create --dev` until a release is cut."
            );
            println!("cargo:rustc-env=WATERUI_VERSION=");
        }
    }
}

fn emit_swift_backend_version() {
    let repo_path = Path::new("../backends/apple");
    let version = git_tag_version(
        Path::new(repo_path),
        "waterui-backend-swift",
        "apple-backend-v*",
    );

    match version {
        Some(version) => {
            println!("cargo:rustc-env=WATERUI_BACKEND_SWIFT_VERSION={}", version);
        }
        None => {
            eprintln!(
                "cargo:warning=No stable version tag found for 'waterui-backend-swift'. Use `water create --dev` until a release is cut."
            );
            println!("cargo:rustc-env=WATERUI_BACKEND_SWIFT_VERSION=");
        }
    }
}

fn git_tag_version(repo_path: &Path, name: &str, pattern: &str) -> Option<String> {
    let mut command = Command::new("git");
    command.args(["describe", "--tags", "--abbrev=0", "--match", pattern]);
    command.current_dir(repo_path);

    let output = match command.output() {
        Ok(output) => output,
        Err(err) => {
            eprintln!(
                "cargo:warning=Failed to query git tags for '{}': {}",
                name, err
            );
            return None;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("No names found") {
            eprintln!("cargo:warning=No matching tags found for '{}'.", name);
        } else {
            eprintln!(
                "cargo:warning=git describe for '{}' failed: {}",
                name,
                stderr.trim()
            );
        }
        return None;
    }

    let version_tag = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let prefix = format!("{}-v", name);

    match version_tag.strip_prefix(&prefix) {
        Some(version) => Some(version.to_string()),
        None => {
            eprintln!(
                "cargo:warning=Unexpected tag format '{}' for '{}'.",
                version_tag, name
            );
            None
        }
    }
}
