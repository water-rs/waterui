//! Windows ARM64 LLVM toolchain support for native assembly dependencies.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::toolchain::{
    Host, Installation, Toolchain, ToolchainError,
    managed_tool::{LLVM_ARM64_MSI_SHA256, LLVM_ARM64_MSI_URL, ManagedToolError, fetch_pinned},
    winget::{WingetInstallError, ensure_package_installed},
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
    ///
    /// # Errors
    /// Returns an error if this host requires LLVM tools and they cannot be located.
    pub async fn cargo_envs(
        &self,
        host: &Host,
    ) -> Result<Vec<(String, OsString)>, ToolchainError<WindowsArm64LlvmInstallation>> {
        if !Self::required_on_host() {
            return Ok(Vec::new());
        }

        let tools = ensure_llvm_tools_available(host).await?;
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

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if !Self::required_on_host() {
            return Ok(());
        }

        ensure_llvm_tools_available(host).await.map(|_| ())
    }
}

/// Installation plan for Windows ARM64 LLVM tooling — the strategy `check`
/// selected for this host.
#[derive(Debug, Clone)]
pub enum WindowsArm64LlvmInstallation {
    /// `winget install LLVM.LLVM`.
    Winget,
    /// The pinned `llvm/llvm-project` Windows ARM64 MSI run through
    /// `msiexec` — no package manager required. Installs to
    /// `C:\Program Files\LLVM`, which the resolution already probes.
    Msi,
}

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
    /// The pinned MSI could not be downloaded or verified.
    #[error(transparent)]
    Managed(#[from] ManagedToolError),
    /// An installation command failed to spawn.
    #[error(transparent)]
    Command(#[from] crate::utils::CommandError),
    /// An I/O operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// `msiexec` exited with a failure.
    #[error("`msiexec` for the LLVM installer exited with {0}")]
    MsiexecFailed(std::process::ExitStatus),
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

    /// `msiexec` writes outside `~/.water` (`C:\Program Files\LLVM`), so the
    /// doctor fix loop confirms it first.
    fn modifies_system(&self) -> bool {
        matches!(self, Self::Msi)
    }

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        match self {
            Self::Winget => {
                ensure_package_installed(host, LLVM_WINGET_PACKAGE_ID)
                    .await
                    .map_err(map_winget_error_for_windows_arm64_llvm)?;
            }
            Self::Msi => {
                use std::ffi::OsStr;
                let staging = smol::unblock(tempfile::tempdir).await?;
                let msi = staging.path().join("LLVM-23.1.1-woa64.msi");
                fetch_pinned(LLVM_ARM64_MSI_URL, LLVM_ARM64_MSI_SHA256, &msi).await?;
                let output = host
                    .output(
                        "msiexec",
                        [
                            OsStr::new("/i"),
                            msi.as_os_str(),
                            OsStr::new("/quiet"),
                            OsStr::new("/norestart"),
                        ],
                    )
                    .await?;
                if !output.status.success() {
                    return Err(FailToInstallWindowsArm64Llvm::MsiexecFailed(output.status));
                }
            }
        }

        let tools = resolve_llvm_tools(host).await;
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

async fn ensure_llvm_tools_available(
    host: &Host,
) -> Result<CompleteLlvmTools, ToolchainError<WindowsArm64LlvmInstallation>> {
    let resolved = resolve_llvm_tools(host).await;
    if let Some(complete) = resolved.clone().into_complete() {
        return Ok(complete);
    }

    if host.which("winget").await.is_ok() {
        Err(ToolchainError::fixable(
            WindowsArm64LlvmInstallation::Winget,
        ))
    } else {
        // The pinned llvm-project Windows ARM64 MSI covers hosts without
        // winget (Windows Server images ship without App Installer).
        Err(ToolchainError::fixable(WindowsArm64LlvmInstallation::Msi))
    }
}

async fn resolve_llvm_tools(host: &Host) -> ResolvedLlvmTools {
    let clang_cl = find_executable(host, "clang-cl", DEFAULT_CLANG_CL_PATH).await;
    let llvm_lib = find_executable(host, "llvm-lib", DEFAULT_LLVM_LIB_PATH).await;
    ResolvedLlvmTools { clang_cl, llvm_lib }
}

async fn find_executable(
    host: &Host,
    binary_name: &'static str,
    fallback_path: &'static str,
) -> Option<PathBuf> {
    if let Ok(path) = host.which(binary_name).await {
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

#[cfg(test)]
mod host_tests {
    use super::WindowsArm64LlvmToolchain;
    use crate::toolchain::Toolchain;
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    use crate::toolchain::ToolchainError;
    use crate::toolchain::testing::TestMachine;

    #[test]
    #[cfg(not(all(target_os = "windows", target_arch = "aarch64")))]
    fn not_required_outside_windows_arm64() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(WindowsArm64LlvmToolchain.check(&host))
            .expect("LLVM tooling is only required on Windows ARM64");
        let envs = smol::block_on(WindowsArm64LlvmToolchain.cargo_envs(&host))
            .expect("cargo envs off Windows ARM64 must not probe tools");
        assert_eq!(envs, [] as [(String, std::ffi::OsString); 0]);
    }

    #[test]
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    fn ok_when_llvm_tools_on_path() {
        let machine = TestMachine::new();
        machine.install("clang-cl");
        machine.install("llvm-lib");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(WindowsArm64LlvmToolchain.check(&host))
            .expect("clang-cl and llvm-lib on PATH must satisfy the check");
    }

    #[test]
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    fn missing_tools_classify_by_winget_presence() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(WindowsArm64LlvmToolchain.check(&host));
        assert!(
            matches!(
                result,
                Err(ToolchainError::Fixable(
                    super::WindowsArm64LlvmInstallation::Msi
                ))
            ),
            "missing LLVM tools without winget falls back to the pinned MSI: {result:?}"
        );
        machine.install("winget");
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(WindowsArm64LlvmToolchain.check(&host));
        assert!(
            matches!(
                result,
                Err(ToolchainError::Fixable(
                    super::WindowsArm64LlvmInstallation::Winget
                ))
            ),
            "missing LLVM tools with winget must stay on winget: {result:?}"
        );
    }
}
