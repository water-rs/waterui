//! Windows ARM64 LLVM toolchain support for native assembly dependencies.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::{
    toolchain::winget::{WingetInstallError, ensure_package_installed},
    toolchain::{Installation, Toolchain, ToolchainError},
    utils::which,
};

const LLVM_WINGET_PACKAGE_ID: &str = "LLVM.LLVM";
const DEFAULT_CLANG_CL_PATH: &str = r"C:\Program Files\LLVM\bin\clang-cl.exe";
const DEFAULT_LLVM_LIB_PATH: &str = r"C:\Program Files\LLVM\bin\llvm-lib.exe";
const TARGET_UNDERSCORE: &str = "aarch64_pc_windows_msvc";
const TARGET_DASHED: &str = "aarch64-pc-windows-msvc";

/// Toolchain for Windows ARM64 LLVM C/ASM build support.
///
/// This is required by native Rust dependencies that ship `.S` sources
/// (for example `aws-lc-sys` and `rav1e`) when building on `aarch64-pc-windows-msvc`.
#[derive(Debug, Clone, Default)]
pub struct WindowsArm64LlvmToolchain;

impl WindowsArm64LlvmToolchain {
    /// Whether this host requires explicit LLVM tooling for native assembly builds.
    #[must_use]
    pub const fn required_on_host() -> bool {
        cfg!(all(target_os = "windows", target_arch = "aarch64"))
    }

    /// Build target-scoped cargo environment overrides that force LLVM tools
    /// for Windows ARM64 C/C++/ASM compilation.
    ///
    /// Returns an empty list on hosts where this toolchain is not required.
    pub async fn cargo_envs(
        &self,
    ) -> Result<Vec<(String, OsString)>, ToolchainError<WindowsArm64LlvmInstallation>> {
        if !Self::required_on_host() {
            return Ok(Vec::new());
        }

        let tools = ensure_llvm_tools_available().await?;
        Ok(vec![
            (
                format!("CC_{TARGET_UNDERSCORE}"),
                tools.clang_cl.clone().into_os_string(),
            ),
            (
                format!("CXX_{TARGET_UNDERSCORE}"),
                tools.clang_cl.clone().into_os_string(),
            ),
            (
                format!("AR_{TARGET_UNDERSCORE}"),
                tools.llvm_lib.clone().into_os_string(),
            ),
            (
                format!("CC_{TARGET_DASHED}"),
                tools.clang_cl.clone().into_os_string(),
            ),
            (
                format!("CXX_{TARGET_DASHED}"),
                tools.clang_cl.clone().into_os_string(),
            ),
            (
                format!("AR_{TARGET_DASHED}"),
                tools.llvm_lib.into_os_string(),
            ),
        ])
    }
}

impl Toolchain for WindowsArm64LlvmToolchain {
    type Installation = WindowsArm64LlvmInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if !Self::required_on_host() {
            return Ok(());
        }

        ensure_llvm_tools_available().await.map(|_| ())
    }
}

/// Installation plan for Windows ARM64 LLVM tooling.
#[derive(Debug, Clone)]
pub struct WindowsArm64LlvmInstallation;

/// Errors that can occur when installing Windows ARM64 LLVM tooling.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallWindowsArm64Llvm {
    /// winget is required for automatic installation.
    #[error(
        "winget is required for automatic LLVM installation on Windows. Install App Installer and retry."
    )]
    WingetNotFound,
    /// winget installation failed.
    #[error("Failed to install LLVM via winget: {0}")]
    WingetInstallFailed(String),
    /// LLVM package installed but required binaries are still unavailable.
    #[error(
        "LLVM was installed, but required binaries are still missing ({missing}). Ensure `{}` is accessible and restart shell/terminal.",
        DEFAULT_CLANG_CL_PATH
    )]
    ToolsNotDetected {
        /// Missing binary list.
        missing: String,
    },
}

impl Installation for WindowsArm64LlvmInstallation {
    type Error = FailToInstallWindowsArm64Llvm;

    async fn install(&self) -> Result<(), Self::Error> {
        ensure_package_installed(LLVM_WINGET_PACKAGE_ID)
            .await
            .map_err(map_winget_error_for_windows_arm64_llvm)?;

        let tools = resolve_llvm_tools().await;
        if tools.is_complete() {
            Ok(())
        } else {
            Err(FailToInstallWindowsArm64Llvm::ToolsNotDetected {
                missing: tools.missing_components().join(", "),
            })
        }
    }
}

#[derive(Debug, Clone)]
struct CompleteLlvmTools {
    clang_cl: PathBuf,
    llvm_lib: PathBuf,
}

#[derive(Debug, Clone, Default)]
struct ResolvedLlvmTools {
    clang_cl: Option<PathBuf>,
    llvm_lib: Option<PathBuf>,
}

impl ResolvedLlvmTools {
    const fn is_complete(&self) -> bool {
        self.clang_cl.is_some() && self.llvm_lib.is_some()
    }

    fn missing_components(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.clang_cl.is_none() {
            missing.push("clang-cl");
        }
        if self.llvm_lib.is_none() {
            missing.push("llvm-lib");
        }
        missing
    }

    fn into_complete(self) -> Option<CompleteLlvmTools> {
        Some(CompleteLlvmTools {
            clang_cl: self.clang_cl?,
            llvm_lib: self.llvm_lib?,
        })
    }
}

async fn ensure_llvm_tools_available()
-> Result<CompleteLlvmTools, ToolchainError<WindowsArm64LlvmInstallation>> {
    let resolved = resolve_llvm_tools().await;
    if let Some(complete) = resolved.clone().into_complete() {
        return Ok(complete);
    }

    if which("winget").await.is_ok() {
        Err(ToolchainError::fixable(WindowsArm64LlvmInstallation))
    } else {
        let missing = resolved.missing_components().join(", ");
        Err(ToolchainError::unfixable(
            format!("Windows ARM64 LLVM tooling is missing: {missing}"),
            format!(
                "Install Microsoft App Installer to enable `winget`, or install LLVM manually and ensure both `{}` and `{}` are available.",
                DEFAULT_CLANG_CL_PATH, DEFAULT_LLVM_LIB_PATH
            ),
        ))
    }
}

async fn resolve_llvm_tools() -> ResolvedLlvmTools {
    let clang_cl = find_executable("clang-cl", DEFAULT_CLANG_CL_PATH).await;
    let llvm_lib = find_executable("llvm-lib", DEFAULT_LLVM_LIB_PATH).await;
    ResolvedLlvmTools { clang_cl, llvm_lib }
}

async fn find_executable(
    binary_name: &'static str,
    fallback_path: &'static str,
) -> Option<PathBuf> {
    if let Ok(path) = which(binary_name).await {
        return Some(path);
    }

    let fallback = PathBuf::from(fallback_path);
    if fallback.exists() {
        Some(fallback)
    } else {
        None
    }
}

fn map_winget_error_for_windows_arm64_llvm(
    error: WingetInstallError,
) -> FailToInstallWindowsArm64Llvm {
    match error {
        WingetInstallError::WingetNotFound => FailToInstallWindowsArm64Llvm::WingetNotFound,
        WingetInstallError::CommandFailed(err) => {
            FailToInstallWindowsArm64Llvm::WingetInstallFailed(err.to_string())
        }
        WingetInstallError::NotInstalled { package_id } => {
            FailToInstallWindowsArm64Llvm::WingetInstallFailed(format!(
                "Package `{package_id}` is still missing after winget install; verify winget sources and retry."
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FailToInstallWindowsArm64Llvm, ResolvedLlvmTools, map_winget_error_for_windows_arm64_llvm,
    };
    use crate::toolchain::winget::WingetInstallError;

    #[test]
    fn maps_winget_not_found_to_specific_error() {
        let mapped = map_winget_error_for_windows_arm64_llvm(WingetInstallError::WingetNotFound);
        assert!(matches!(
            mapped,
            FailToInstallWindowsArm64Llvm::WingetNotFound
        ));
    }

    #[test]
    fn maps_not_installed_error_with_package_context() {
        let mapped = map_winget_error_for_windows_arm64_llvm(WingetInstallError::NotInstalled {
            package_id: "LLVM.LLVM",
        });
        let message = mapped.to_string();
        assert!(message.contains("LLVM.LLVM"));
        assert!(message.contains("still missing"));
    }

    #[test]
    fn missing_components_reports_expected_tools() {
        let missing_both = ResolvedLlvmTools::default();
        assert_eq!(
            missing_both.missing_components(),
            vec!["clang-cl", "llvm-lib"]
        );

        let missing_llvm_lib = ResolvedLlvmTools {
            clang_cl: Some("clang-cl".into()),
            llvm_lib: None,
        };
        assert_eq!(missing_llvm_lib.missing_components(), vec!["llvm-lib"]);
    }
}
