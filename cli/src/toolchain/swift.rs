//! Swift/Xcode toolchain implementation.

use std::{
    fmt::{self, Display},
    future::Future,
    process::Command,
};

use tokio::process::Command as AsyncCommand;
use which::which;

use super::{
    Toolchain, ToolchainError,
    installation::{Empty, Installation, InstallationReport, Progress},
};

// ============================================================================
// Toolchain
// ============================================================================

/// Swift/Xcode toolchain configuration.
#[derive(Debug, Clone, Default)]
pub struct Swift {
    require_ios: bool,
    require_macos: bool,
}

impl Swift {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn with_ios(mut self) -> Self {
        self.require_ios = true;
        self
    }

    #[must_use]
    pub const fn with_macos(mut self) -> Self {
        self.require_macos = true;
        self
    }

    fn has_xcode_cli_tools() -> bool {
        Command::new("xcode-select")
            .args(["--print-path"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn has_swift() -> bool {
        which("swift").is_ok()
    }

    fn has_xcodebuild() -> bool {
        which("xcodebuild").is_ok()
    }

    async fn check_sdk(sdk: &str) -> bool {
        AsyncCommand::new("xcrun")
            .args(["--sdk", sdk, "--show-sdk-path"])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

/// Installation type for Swift toolchain.
#[derive(Debug)]
pub enum SwiftInstallation {
    Empty(Empty),
    XcodeCliTools(XcodeCliTools),
}

impl Display for SwiftInstallation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(e) => write!(f, "{e}"),
            Self::XcodeCliTools(x) => write!(f, "{x}"),
        }
    }
}

impl Installation for SwiftInstallation {
    type Future = impl Future<Output = Result<InstallationReport, ToolchainError>> + Send;

    fn description(&self) -> &str {
        match self {
            Self::Empty(_) => "nothing",
            Self::XcodeCliTools(_) => "Xcode Command Line Tools",
        }
    }

    fn install(self, progress: Progress) -> Self::Future {
        async move {
            match self {
                Self::Empty(e) => e.install(progress).await,
                Self::XcodeCliTools(x) => x.install(progress).await,
            }
        }
    }
}

impl Toolchain for Swift {
    type Installation = SwiftInstallation;

    fn name(&self) -> &'static str {
        "Swift"
    }

    async fn check(&self) -> Result<(), ToolchainError> {
        if !cfg!(target_os = "macos") {
            return Err(
                ToolchainError::unfixable("Swift development requires macOS")
                    .with_suggestion("Use a Mac or set up a macOS CI environment"),
            );
        }

        if !Self::has_xcode_cli_tools() {
            return Err(
                ToolchainError::unfixable("Xcode Command Line Tools are not installed")
                    .with_suggestion("Run: xcode-select --install"),
            );
        }

        if !Self::has_swift() {
            return Err(ToolchainError::unfixable("Swift compiler not found")
                .with_suggestion("Install Xcode from the Mac App Store"));
        }

        if !Self::has_xcodebuild() {
            return Err(ToolchainError::unfixable("xcodebuild not found")
                .with_suggestion("Install Xcode from the Mac App Store"));
        }

        if self.require_ios && !Self::check_sdk("iphoneos").await {
            return Err(ToolchainError::unfixable("iOS SDK not found")
                .with_suggestion("Install Xcode with iOS platform support"));
        }

        if self.require_macos && !Self::check_sdk("macosx").await {
            return Err(ToolchainError::unfixable("macOS SDK not found")
                .with_suggestion("Install Xcode from the Mac App Store"));
        }

        Ok(())
    }

    fn fix(&self) -> Result<Self::Installation, ToolchainError> {
        if !cfg!(target_os = "macos") {
            return Err(ToolchainError::unfixable(
                "Swift development requires macOS",
            ));
        }

        if !Self::has_xcode_cli_tools() {
            return Ok(SwiftInstallation::XcodeCliTools(XcodeCliTools));
        }

        if !Self::has_xcodebuild() {
            return Err(ToolchainError::unfixable("Full Xcode.app is required")
                .with_suggestion("Install Xcode from the Mac App Store"));
        }

        Ok(SwiftInstallation::Empty(Empty::new()))
    }
}

// ============================================================================
// Atomic Installers
// ============================================================================

/// Install Xcode Command Line Tools.
#[derive(Debug, Clone, Copy)]
pub struct XcodeCliTools;

impl Display for XcodeCliTools {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "xcode-select --install")
    }
}

impl Installation for XcodeCliTools {
    type Future = impl Future<Output = Result<InstallationReport, ToolchainError>> + Send;

    fn description(&self) -> &'static str {
        "Xcode Command Line Tools"
    }

    fn install(self, progress: Progress) -> Self::Future {
        async move {
            progress.start("xcode-cli-tools");
            progress.update("xcode-cli-tools", 0, "launching installer");

            let status = AsyncCommand::new("xcode-select")
                .args(["--install"])
                .status()
                .await
                .map_err(|e| {
                    progress.fail("xcode-cli-tools", format!("{e}"));
                    ToolchainError::install_failed(format!("Failed to run xcode-select: {e}"))
                })?;

            let mut report = InstallationReport::empty();

            if status.success() {
                report.add_completed("Xcode Command Line Tools installation started");
                report.add_warning("Complete the dialog and run this command again");
                progress.done("xcode-cli-tools", "dialog opened");
            } else {
                report.add_warning("Tools may already be installed");
                progress.done("xcode-cli-tools", "may be installed");
            }

            Ok(report)
        }
    }
}
