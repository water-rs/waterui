//! Shared runtime compatibility rules used by preview and inspector.

use std::ffi::OsStr;
use std::path::{Component, Path};

/// Incremental-compilation setting every generated crate builds under.
///
/// This is part of Cargo's `-C metadata` and therefore of every mangled symbol, so a
/// preview module and the support app it loads into must agree on it. Recording it in the
/// runtime fingerprint means changing it invalidates the support-app scaffold rather than
/// producing a module that cannot resolve against the runtime the app already loaded.
#[expect(
    clippy::redundant_pub_crate,
    reason = "preview, inspector, and fingerprint modules share this crate-private runtime contract"
)]
pub(crate) const GENERATED_CRATE_INCREMENTAL: &str = "0";

/// Environment variables required for the preview support app runtime profile.
#[expect(
    clippy::redundant_pub_crate,
    reason = "preview, inspector, and fingerprint modules share this crate-private runtime contract"
)]
pub(crate) const PREVIEW_RUNTIME_ENV_VARS: [(&str, &str); 1] = [("WATERUI_PREVIEW_MODE", "1")];

const PREVIEW_ROOT_INPUT_FILES: [&str; 4] = ["Cargo.toml", "Cargo.lock", "Water.toml", "build.rs"];

const RUNTIME_FINGERPRINT_ROOT_FILES: [&str; 5] = [
    "Cargo.toml",
    "Cargo.lock",
    ".gitmodules",
    "rust-toolchain",
    "rust-toolchain.toml",
];

const RUNTIME_FINGERPRINT_ROOT_DIRS: [&str; 9] = [
    "core",
    "components",
    "utils",
    "src",
    "ffi",
    "macros",
    "backends",
    "kit",
    "icon",
];

/// Build input extensions relevant to runtime compatibility.
const PREVIEW_INPUT_EXTENSIONS: [&str; 19] = [
    "rs", "swift", "m", "mm", "h", "hpp", "c", "cc", "cpp", "metal", "wgsl", "kt", "kts", "java",
    "xml", "plist", "toml", "json", "yaml",
];

/// Return a stable runtime profile tag used to version runtime compatibility.
#[must_use]
#[expect(
    clippy::redundant_pub_crate,
    reason = "preview and inspector share this crate-private compatibility tag"
)]
pub(crate) fn runtime_profile_tag() -> String {
    PREVIEW_RUNTIME_ENV_VARS
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .chain(std::iter::once(format!(
            "incremental={GENERATED_CRATE_INCREMENTAL}"
        )))
        .collect::<Vec<_>>()
        .join(";")
}

/// Return whether a directory should be skipped while scanning runtime inputs.
#[must_use]
#[expect(
    clippy::redundant_pub_crate,
    reason = "preview input and runtime fingerprinting share this crate-private rule"
)]
pub(crate) fn should_skip_scan_dir(name: &OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(
            ".git" | ".jj" | ".water" | "target" | "node_modules" | ".gradle" | ".idea" | ".vscode"
        )
    )
}

/// Return top-level files that participate in preview runtime fingerprinting.
#[must_use]
#[expect(
    clippy::redundant_pub_crate,
    reason = "runtime fingerprinting consumes this crate-private input contract"
)]
pub(crate) const fn runtime_fingerprint_root_files() -> &'static [&'static str] {
    &RUNTIME_FINGERPRINT_ROOT_FILES
}

/// Return top-level directories that participate in preview runtime fingerprinting.
#[must_use]
#[expect(
    clippy::redundant_pub_crate,
    reason = "runtime fingerprinting consumes this crate-private input contract"
)]
pub(crate) const fn runtime_fingerprint_root_dirs() -> &'static [&'static str] {
    &RUNTIME_FINGERPRINT_ROOT_DIRS
}

/// Return whether a file participates in build/runtime compatibility.
#[must_use]
#[expect(
    clippy::redundant_pub_crate,
    reason = "preview input and runtime fingerprinting share this crate-private rule"
)]
pub(crate) fn is_preview_build_input_file(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };

    if PREVIEW_ROOT_INPUT_FILES.contains(&file_name) {
        return true;
    }

    // Any file under src/ or assets/ can be consumed by include_* at compile time.
    if path_contains_component(path, "src") || path_contains_component(path, "assets") {
        return true;
    }

    let Some(ext) = path.extension().and_then(OsStr::to_str) else {
        return false;
    };
    PREVIEW_INPUT_EXTENSIONS
        .iter()
        .any(|candidate| ext.eq_ignore_ascii_case(candidate))
}

fn path_contains_component(path: &Path, needle: &str) -> bool {
    path.components().any(|component| match component {
        Component::Normal(name) => name == OsStr::new(needle),
        _ => false,
    })
}
