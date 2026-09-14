//! Apple toolchain module

use std::convert::Infallible;

use serde::{Deserialize, Serialize};

use crate::toolchain::{Host, Toolchain, ToolchainError};

/// Represents the complete Apple toolchain consisting of Xcode and an Apple SDK
pub type AppleToolchain = (Xcode, AppleSdk);

/// Represents the Xcode toolchain
#[derive(Debug, Clone, Default)]
pub struct Xcode;

impl Toolchain for Xcode {
    type Installation = Infallible;
    async fn check(
        &self,
        host: &Host,
    ) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        // Check if Xcode is installed and available
        if host.which("xcodebuild").await.is_ok() && host.which("xcode-select").await.is_ok() {
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
#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
pub enum AppleSdk {
    /// iOS SDK
    #[serde(rename = "iOS")]
    Ios,
    /// iOS Simulator SDK
    #[serde(rename = "iOS Simulator")]
    IosSimulator,
    /// macOS SDK
    #[serde(rename = "macOS")]
    Macos,
    /// tvOS SDK
    #[serde(rename = "tvOS")]
    TvOs,
    /// watchOS SDK
    #[serde(rename = "watchOS")]
    WatchOs,
    /// visionOS SDK
    #[serde(rename = "visionOS")]
    VisionOs,
}

impl AppleSdk {
    /// Get the SDK name as used by `xcrun`
    #[must_use]
    pub const fn sdk_name(&self) -> &str {
        match self {
            Self::Ios => "iphoneos",
            Self::IosSimulator => "iphonesimulator",
            Self::Macos => "macosx",
            Self::TvOs => "appletvos",
            Self::WatchOs => "watchos",
            Self::VisionOs => "xros",
        }
    }
}

impl std::fmt::Display for AppleSdk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Avoid panics in Display; serde is for config IO, not formatting.
        match self {
            Self::Ios => "iOS",
            Self::IosSimulator => "iOS Simulator",
            Self::Macos => "macOS",
            Self::TvOs => "tvOS",
            Self::WatchOs => "watchOS",
            Self::VisionOs => "visionOS",
        }
        .fmt(f)
    }
}

impl Toolchain for AppleSdk {
    type Installation = Infallible;
    async fn check(
        &self,
        host: &Host,
    ) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        // Check if the required Apple SDK is available
        let result = host
            .run("xcrun", ["--sdk", self.sdk_name(), "--show-sdk-path"])
            .await;

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

#[cfg(test)]
mod tests {
    use super::{AppleSdk, Xcode};
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Toolchain, ToolchainError};

    #[test]
    fn xcode_ok_when_tools_on_path() {
        let machine = TestMachine::new();
        machine.install("xcodebuild");
        machine.install("xcode-select");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(Xcode.check(&host)).expect("xcodebuild + xcode-select on PATH must be ok");
    }

    #[test]
    fn xcode_missing_is_unfixable() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(Xcode.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "Xcode requires a manual App Store install: {result:?}"
        );
    }

    #[test]
    fn apple_sdk_ok_when_xcrun_reports_path() {
        let machine = TestMachine::new();
        machine.install("xcrun");
        machine.respond("XCRUN_SDK_PATH", "/fake/SDKs/iPhoneOS.sdk\n");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(AppleSdk::Ios.check(&host))
            .expect("an SDK path from xcrun must satisfy the check");
    }

    #[test]
    fn apple_sdk_missing_is_unfixable() {
        let machine = TestMachine::new();
        machine.install("xcrun");
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(AppleSdk::Ios.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "xcrun without an SDK path must be unfixable: {result:?}"
        );
    }
}
