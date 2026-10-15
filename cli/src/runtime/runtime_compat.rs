//! Shared runtime compatibility rules used by generated crates and the
//! inspector support application.

/// Incremental-compilation setting every generated crate builds under.
///
/// This is part of Cargo's `-C metadata` and therefore of every mangled
/// symbol, so a generated crate and the shared runtime it links must agree
/// on it. Recording it in the runtime fingerprint means changing it
/// invalidates a generated scaffold rather than producing units whose
/// symbols cannot resolve against the runtime already built.
#[expect(
    clippy::redundant_pub_crate,
    reason = "fingerprint and generated-crate modules share this crate-private runtime contract"
)]
pub(crate) const GENERATED_CRATE_INCREMENTAL: &str = "0";

const RUNTIME_FINGERPRINT_ROOT_FILES: [&str; 5] = [
    "Cargo.toml",
    "Cargo.lock",
    ".gitmodules",
    "rust-toolchain",
    "rust-toolchain.toml",
];

const RUNTIME_FINGERPRINT_ROOT_DIRS: [&str; 8] = [
    "core",
    "components",
    "utils",
    "src",
    "macros",
    "backends",
    "kit",
    "icon",
];

/// Return a stable runtime profile tag used to version runtime compatibility.
#[must_use]
#[expect(
    clippy::redundant_pub_crate,
    reason = "the inspector shares this crate-private compatibility tag"
)]
pub(crate) fn runtime_profile_tag() -> String {
    format!("incremental={GENERATED_CRATE_INCREMENTAL}")
}

/// Return top-level files that participate in runtime fingerprinting.
#[must_use]
#[expect(
    clippy::redundant_pub_crate,
    reason = "runtime fingerprinting consumes this crate-private input contract"
)]
pub(crate) const fn runtime_fingerprint_root_files() -> &'static [&'static str] {
    &RUNTIME_FINGERPRINT_ROOT_FILES
}

/// Return top-level directories that participate in runtime fingerprinting.
#[must_use]
#[expect(
    clippy::redundant_pub_crate,
    reason = "runtime fingerprinting consumes this crate-private input contract"
)]
pub(crate) const fn runtime_fingerprint_root_dirs() -> &'static [&'static str] {
    &RUNTIME_FINGERPRINT_ROOT_DIRS
}
