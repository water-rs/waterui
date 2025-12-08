use std::convert::Infallible;

use serde::{Deserialize, Serialize};

use crate::{
    toolchain::{Toolchain, ToolchainError},
    utils::{run_command, which},
};

/// Represents the complete Apple toolchain consisting of Xcode and an Apple SDK
pub type AppleToolchain = (Xcode, AppleSdk);

/// Represents the Xcode toolchain
#[derive(Debug, Clone, Default)]
pub struct Xcode;

impl Toolchain for Xcode {
    type Installation = Infallible;
    async fn check(&self) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        // Check if Xcode is installed and available
        if which("xcodebuild").await.is_ok() && which("xcode-select").await.is_ok() {
            Ok(())
        } else {
            Err(ToolchainError::unfixable(
                "Xcode is not installed or not found in PATH",
                "Please install Xcode from the App Store or the Apple Developer website and ensure it's available in your PATH.",
            ))
        }
    }
}

/// Represents an Apple SDK (e.g., iOS, macOS)
#[derive(Debug, Deserialize, Serialize)]
pub enum AppleSdk {
    /// iOS SDK
    #[serde(rename = "iOS")]
    Ios,
    /// macOS SDK
    #[serde(rename = "MacOS")]
    Macos,
    // TODO: more SDKs
}

impl AppleSdk {
    /// Get the SDK name as used by `xcrun`
    #[must_use]
    pub const fn sdk_name(&self) -> &str {
        match self {
            Self::Ios => "iphoneos",
            Self::Macos => "macosx",
        }
    }
}

impl std::fmt::Display for AppleSdk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        serde_json::to_value(self).unwrap().as_str().unwrap().fmt(f)
    }
}

impl Toolchain for AppleSdk {
    type Installation = Infallible;
    async fn check(&self) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        // Check if the required Apple SDK is available
        let result = run_command("xcrun", ["--sdk", self.sdk_name(), "--show-sdk-path"]).await;

        if result.is_err() {
            return Err(ToolchainError::unfixable(
                format!("{self} SDK is not installed or not available"),
                format!(
                    "Please install {self} SDK through Xcode or use xcode-select to configure the active developer directory."
                ),
            ));
        }

        Ok(())
    }
}
