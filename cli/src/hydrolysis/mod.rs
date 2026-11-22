//! `Hydrolysis` backend support for `WaterUI` CLI.

use crate::toolchain::{Host, ToolchainError, windows_arm64_llvm::WindowsArm64LlvmToolchain};

pub mod android;
pub mod backend;
pub mod platform;
mod web_launch;

async fn windows_arm64_llvm_envs(host: &Host) -> eyre::Result<Vec<(String, std::ffi::OsString)>> {
    WindowsArm64LlvmToolchain
        .cargo_envs(host)
        .await
        .map_err(|error| match error {
            ToolchainError::Fixable(_) => eyre::eyre!(
                "Windows ARM64 LLVM toolchain is missing. Run `water doctor --fix` to install it automatically."
            ),
            ToolchainError::Unfixable(unfixable) => {
                eyre::eyre!("Windows ARM64 LLVM toolchain check failed: {unfixable}")
            }
        })
}

/// Enables `erased-dispatch` when requested and declared by the launcher manifest.
fn erased_dispatch(
    manifest: &std::path::Path,
    enabled: bool,
) -> std::io::Result<Option<&'static str>> {
    if enabled && crate::templates::generated_manifest_declares(manifest, "erased-dispatch")? {
        Ok(Some("erased-dispatch"))
    } else {
        Ok(None)
    }
}

/// Development builds enable inspection when the resolved framework exposes it.
fn development_inspector(
    manifest: &std::path::Path,
    release: bool,
) -> std::io::Result<Option<&'static str>> {
    if !release && crate::templates::generated_manifest_declares(manifest, "inspector")? {
        Ok(Some("inspector"))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn inspection_defaults_only_to_development_builds() {
        let dir = tempfile::tempdir().expect("manifest directory");
        let path = dir.path().join("Cargo.toml");
        for declared in [false, true] {
            let mut manifest = cargo_toml::Manifest::<()>::default();
            if declared {
                manifest.features.insert("inspector".to_owned(), Vec::new());
            }
            std::fs::write(
                &path,
                toml::to_string(&manifest).expect("serialize manifest"),
            )
            .expect("write manifest");
            assert_eq!(
                super::development_inspector(&path, false).expect("development features"),
                declared.then_some("inspector")
            );
            assert_eq!(
                super::development_inspector(&path, true).expect("release features"),
                None
            );
        }
    }
}
