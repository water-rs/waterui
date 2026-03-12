//! Toolchain diagnostics for the `water doctor` command.

use std::future::Future;
use std::pin::Pin;

use color_eyre::eyre;

use crate::{
    android::{
        device::AndroidDevice,
        platform::AndroidPlatform,
        toolchain::{
            AndroidBuildTools, AndroidNdk, AndroidPlatformTools, AndroidRustTargets, AndroidSdk,
            AndroidSdkPlatforms, Java, Kotlin,
        },
    },
    apple::{
        device::AppleSimulator,
        toolchain::{AppleSdk, Xcode},
    },
    device::Device,
    gtk4::toolchain::Gtk4Toolchain,
    toolchain::{
        Installation, Toolchain, ToolchainError, UnfixableToolchain, cmake::Cmake,
        linux::LinuxSystemToolchain, rust::RustToolchain, sccache::Sccache,
        windows_arm64_llvm::WindowsArm64LlvmToolchain,
    },
};

/// Status of a toolchain check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    /// Toolchain is available and working.
    Ok,
    /// Toolchain is missing or misconfigured.
    Missing,
    /// Toolchain check was skipped (e.g., not applicable on this platform).
    Skipped,
}

/// A boxed async function that performs an installation.
pub type BoxedInstallFn =
    Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = eyre::Result<()>> + Send>> + Send>;

/// A single item in the doctor report.
pub struct DoctorItem {
    /// Name of the toolchain or component.
    pub name: &'static str,
    /// Status of the check.
    pub status: CheckStatus,
    /// Optional message with details or suggestions.
    pub message: Option<String>,
    /// Optional installation function if the issue can be fixed automatically.
    pub install_fn: Option<BoxedInstallFn>,
}

impl std::fmt::Debug for DoctorItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DoctorItem")
            .field("name", &self.name)
            .field("status", &self.status)
            .field("message", &self.message)
            .field("install_fn", &self.install_fn.as_ref().map(|_| "..."))
            .finish()
    }
}

impl DoctorItem {
    const fn ok(name: &'static str) -> Self {
        Self {
            name,
            status: CheckStatus::Ok,
            message: None,
            install_fn: None,
        }
    }

    fn missing(name: &'static str, message: impl Into<String>) -> Self {
        Self {
            name,
            status: CheckStatus::Missing,
            message: Some(message.into()),
            install_fn: None,
        }
    }

    fn fixable<I: Installation + Send + 'static>(
        name: &'static str,
        message: impl Into<String>,
        installation: I,
    ) -> Self {
        Self {
            name,
            status: CheckStatus::Missing,
            message: Some(message.into()),
            install_fn: Some(Box::new(move || {
                Box::pin(async move { installation.install().await.map_err(Into::into) })
            })),
        }
    }

    const fn skipped(name: &'static str) -> Self {
        Self {
            name,
            status: CheckStatus::Skipped,
            message: None,
            install_fn: None,
        }
    }

    fn skipped_with_message(name: &'static str, message: impl Into<String>) -> Self {
        Self {
            name,
            status: CheckStatus::Skipped,
            message: Some(message.into()),
            install_fn: None,
        }
    }

    /// Returns `true` if the issue can be fixed automatically.
    #[must_use]
    pub const fn is_fixable(&self) -> bool {
        self.install_fn.is_some()
    }
}

fn unfixable_message(error: &UnfixableToolchain) -> String {
    format!(
        "Cannot auto-fix: {}. Next step: {}",
        error.message(),
        error.suggestion()
    )
}

/// Run diagnostics on all toolchains and return a report.
pub async fn doctor() -> Vec<DoctorItem> {
    let mut items = Vec::new();

    // Check Xcode (macOS only)
    if cfg!(target_os = "macos") {
        match Xcode.check().await {
            Ok(()) => items.push(DoctorItem::ok("Xcode")),
            Err(e) => items.push(DoctorItem::missing("Xcode", e.to_string())),
        }

        // Check iOS SDK
        match AppleSdk::Ios.check().await {
            Ok(()) => items.push(DoctorItem::ok("iOS SDK")),
            Err(e) => items.push(DoctorItem::missing("iOS SDK", e.to_string())),
        }

        // Check iOS Simulator SDK
        match AppleSdk::IosSimulator.check().await {
            Ok(()) => items.push(DoctorItem::ok("iOS Simulator SDK")),
            Err(e) => items.push(DoctorItem::missing("iOS Simulator SDK", e.to_string())),
        }

        // Check iOS simulator runtime/device availability for `water run --platform ios`.
        match AppleSimulator::scan_ios().await {
            Ok(simulators) if simulators.is_empty() => items.push(DoctorItem::missing(
                "iOS Simulators",
                "No iOS simulators available. Install a simulator runtime in Xcode Settings > Platforms.",
            )),
            Ok(_) => items.push(DoctorItem::ok("iOS Simulators")),
            Err(error) => items.push(DoctorItem::missing(
                "iOS Simulators",
                format!("Failed to list iOS simulators: {error}"),
            )),
        }

        // Check macOS SDK
        match AppleSdk::Macos.check().await {
            Ok(()) => items.push(DoctorItem::ok("macOS SDK")),
            Err(e) => items.push(DoctorItem::missing("macOS SDK", e.to_string())),
        }
    } else {
        items.push(DoctorItem::skipped("Xcode"));
        items.push(DoctorItem::skipped("iOS SDK"));
        items.push(DoctorItem::skipped("iOS Simulator SDK"));
        items.push(DoctorItem::skipped("iOS Simulators"));
        items.push(DoctorItem::skipped("macOS SDK"));
    }

    // Check Rust toolchain
    match RustToolchain.check().await {
        Ok(()) => items.push(DoctorItem::ok("Rust toolchain")),
        Err(ToolchainError::Fixable(installation)) => {
            items.push(DoctorItem::fixable(
                "Rust toolchain",
                format!(
                    "Rust toolchain is missing, outdated, or incomplete. Planned automatic fixes: {}",
                    installation.summary()
                ),
                installation,
            ));
        }
        Err(ToolchainError::Unfixable(error)) => {
            items.push(DoctorItem::missing(
                "Rust toolchain",
                unfixable_message(&error),
            ));
        }
    }

    // Check Android SDK
    match AndroidSdk.check().await {
        Ok(()) => items.push(DoctorItem::ok("Android SDK")),
        Err(ToolchainError::Fixable(installation)) => {
            items.push(DoctorItem::fixable(
                "Android SDK",
                "Android SDK is missing (automatic install is supported on this host)",
                installation,
            ));
        }
        Err(ToolchainError::Unfixable(error)) => {
            items.push(DoctorItem::missing(
                "Android SDK",
                unfixable_message(&error),
            ));
        }
    }

    let sdk_ready_for_component_checks =
        AndroidSdk::detect_path().is_some() || AndroidSdk::sdkmanager_path().await.is_some();

    if sdk_ready_for_component_checks {
        // Check Android Platform-Tools (`adb`) for run flows.
        match AndroidPlatformTools.check().await {
            Ok(()) => items.push(DoctorItem::ok("Android Platform-Tools (adb)")),
            Err(ToolchainError::Fixable(installation)) => {
                items.push(DoctorItem::fixable(
                    "Android Platform-Tools (adb)",
                    "Required for `water run --platform android`",
                    installation,
                ));
            }
            Err(ToolchainError::Unfixable(error)) => {
                items.push(DoctorItem::missing(
                    "Android Platform-Tools (adb)",
                    unfixable_message(&error),
                ));
            }
        }

        // Check Android NDK for build/package flows.
        match AndroidSdkPlatforms.check().await {
            Ok(()) => items.push(DoctorItem::ok("Android SDK Platforms")),
            Err(ToolchainError::Fixable(installation)) => {
                items.push(DoctorItem::fixable(
                    "Android SDK Platforms",
                    "Required for Android build/package workflows",
                    installation,
                ));
            }
            Err(ToolchainError::Unfixable(error)) => {
                items.push(DoctorItem::missing(
                    "Android SDK Platforms",
                    unfixable_message(&error),
                ));
            }
        }

        // Check Android SDK build-tools (d8) for build/package flows.
        match AndroidBuildTools.check().await {
            Ok(()) => items.push(DoctorItem::ok("Android SDK Build-Tools (d8)")),
            Err(ToolchainError::Fixable(installation)) => {
                items.push(DoctorItem::fixable(
                    "Android SDK Build-Tools (d8)",
                    "Required for Android build/package workflows",
                    installation,
                ));
            }
            Err(ToolchainError::Unfixable(error)) => {
                items.push(DoctorItem::missing(
                    "Android SDK Build-Tools (d8)",
                    unfixable_message(&error),
                ));
            }
        }

        // Check Android NDK for build/package flows.
        match AndroidNdk.check().await {
            Ok(()) => items.push(DoctorItem::ok("Android NDK")),
            Err(ToolchainError::Fixable(installation)) => {
                items.push(DoctorItem::fixable(
                    "Android NDK",
                    "Required for Android build/package workflows",
                    installation,
                ));
            }
            Err(ToolchainError::Unfixable(error)) => {
                items.push(DoctorItem::missing(
                    "Android NDK",
                    unfixable_message(&error),
                ));
            }
        }

        // Check Rust Android targets for Rust cross-compilation.
        match AndroidRustTargets.check().await {
            Ok(()) => items.push(DoctorItem::ok("Android Rust Targets")),
            Err(ToolchainError::Fixable(installation)) => {
                items.push(DoctorItem::fixable(
                    "Android Rust Targets",
                    "Required for Android Rust cross-compilation",
                    installation,
                ));
            }
            Err(ToolchainError::Unfixable(error)) => {
                items.push(DoctorItem::missing(
                    "Android Rust Targets",
                    unfixable_message(&error),
                ));
            }
        }
    } else {
        items.push(DoctorItem::missing(
            "Android Platform-Tools (adb)",
            "Blocked: Android SDK / `sdkmanager` is not ready yet. Fix Android SDK first.",
        ));
        items.push(DoctorItem::missing(
            "Android NDK",
            "Blocked: Android SDK / `sdkmanager` is not ready yet. Fix Android SDK first.",
        ));
        items.push(DoctorItem::missing(
            "Android SDK Platforms",
            "Blocked: Android SDK / `sdkmanager` is not ready yet. Fix Android SDK first.",
        ));
        items.push(DoctorItem::missing(
            "Android SDK Build-Tools (d8)",
            "Blocked: Android SDK / `sdkmanager` is not ready yet. Fix Android SDK first.",
        ));
        items.push(DoctorItem::missing(
            "Android Rust Targets",
            "Blocked: Android SDK / `sdkmanager` is not ready yet. Fix Android SDK first.",
        ));
    }

    // Check Android run target availability for `water run --platform android`.
    if AndroidSdk::adb_path().is_some() {
        match AndroidDevice::scan().await {
            Ok(devices) if !devices.is_empty() => {
                items.push(DoctorItem::ok("Android Run Targets"));
            }
            Ok(_) => match AndroidPlatform::list_avds().await {
                Ok(avds) if !avds.is_empty() => {
                    items.push(DoctorItem::ok("Android Run Targets"));
                }
                Ok(_) => items.push(DoctorItem::missing(
                    "Android Run Targets",
                    "No connected Android devices and no emulator AVDs were found. Connect a device or create an AVD.",
                )),
                Err(error) => items.push(DoctorItem::missing(
                    "Android Run Targets",
                    format!(
                        "No connected Android devices and failed to list AVDs: {error}. Install Android emulator components or connect a device."
                    ),
                )),
            },
            Err(error) => items.push(DoctorItem::missing(
                "Android Run Targets",
                format!("Failed to query Android devices via adb: {error}"),
            )),
        }
    } else {
        items.push(DoctorItem::missing(
            "Android Run Targets",
            "Blocked: Android Platform-Tools (`adb`) is not ready yet.",
        ));
    }

    // Check CMake used by native Rust dependencies during Android builds.
    match Cmake::default().check().await {
        Ok(()) => items.push(DoctorItem::ok("Host CMake")),
        Err(ToolchainError::Fixable(installation)) => {
            items.push(DoctorItem::fixable(
                "Host CMake",
                "Required for native Rust dependencies in Android builds",
                installation,
            ));
        }
        Err(ToolchainError::Unfixable(error)) => {
            items.push(DoctorItem::missing("Host CMake", unfixable_message(&error)));
        }
    }

    // Check Windows ARM64 LLVM tooling required by native `.S` dependencies
    // in hydrolysis/windows builds (e.g. aws-lc-sys, rav1e).
    if WindowsArm64LlvmToolchain::required_on_host() {
        match WindowsArm64LlvmToolchain.check().await {
            Ok(()) => items.push(DoctorItem::ok("Windows ARM64 LLVM toolchain")),
            Err(ToolchainError::Fixable(installation)) => {
                items.push(DoctorItem::fixable(
                    "Windows ARM64 LLVM toolchain",
                    "Required by native assembly dependencies in Windows ARM64 hydrolysis builds",
                    installation,
                ));
            }
            Err(ToolchainError::Unfixable(error)) => {
                items.push(DoctorItem::missing(
                    "Windows ARM64 LLVM toolchain",
                    unfixable_message(&error),
                ));
            }
        }
    } else {
        items.push(DoctorItem::skipped_with_message(
            "Windows ARM64 LLVM toolchain",
            "Only required on Windows ARM64 hosts for native assembly dependencies.",
        ));
    }

    // Check Java runtime for Android Gradle builds.
    match Java.check().await {
        Ok(()) => items.push(DoctorItem::ok("Java")),
        Err(ToolchainError::Fixable(installation)) => {
            items.push(DoctorItem::fixable(
                "Java",
                "Required for Android Gradle builds",
                installation,
            ));
        }
        Err(ToolchainError::Unfixable(error)) => {
            items.push(DoctorItem::missing("Java", unfixable_message(&error)));
        }
    }

    // Kotlin compiler is required for Android helper sources used by build scripts.
    match Kotlin.check().await {
        Ok(()) => items.push(DoctorItem::ok("Kotlin")),
        Err(ToolchainError::Fixable(installation)) => {
            items.push(DoctorItem::fixable(
                "Kotlin",
                "Required for Android Kotlin helper compilation",
                installation,
            ));
        }
        Err(ToolchainError::Unfixable(error)) => {
            items.push(DoctorItem::missing("Kotlin", unfixable_message(&error)));
        }
    }

    // Check Linux system package toolchain
    let mut linux_packages_fixable = false;
    if cfg!(target_os = "linux") {
        match LinuxSystemToolchain.check().await {
            Ok(()) => items.push(DoctorItem::ok("Linux system packages")),
            Err(ToolchainError::Fixable(installation)) => {
                linux_packages_fixable = true;
                let msg = format!(
                    "Missing packages for {}: {}. Install command: {}",
                    installation.package_manager_name(),
                    installation.missing_packages().join(", "),
                    installation.install_command_hint(),
                );
                items.push(DoctorItem::fixable(
                    "Linux system packages",
                    msg,
                    installation,
                ));
            }
            Err(ToolchainError::Unfixable(error)) => {
                items.push(DoctorItem::missing(
                    "Linux system packages",
                    unfixable_message(&error),
                ));
            }
        }
    } else {
        items.push(DoctorItem::skipped("Linux system packages"));
    }

    // Check GTK4 toolchain
    if cfg!(target_os = "linux") {
        match Gtk4Toolchain.check().await {
            Ok(()) => items.push(DoctorItem::ok("GTK4")),
            Err(ToolchainError::Fixable(installation)) => {
                items.push(DoctorItem::fixable(
                    "GTK4",
                    "GTK4 dependencies are missing",
                    installation,
                ));
            }
            Err(ToolchainError::Unfixable(error)) => {
                if linux_packages_fixable {
                    items.push(DoctorItem::missing(
                        "GTK4",
                        "GTK4 probe failed because required Linux packages are missing. Run `water doctor --fix` to install Linux system packages, then re-run `water doctor`.",
                    ));
                } else {
                    items.push(DoctorItem::missing("GTK4", unfixable_message(&error)));
                }
            }
        }
    } else {
        items.push(DoctorItem::skipped("GTK4"));
    }

    // Check sccache (optional but recommended for faster builds)
    match Sccache.check().await {
        Ok(()) => items.push(DoctorItem::ok("sccache")),
        Err(ToolchainError::Fixable(installation)) => {
            items.push(DoctorItem::fixable(
                "sccache",
                "sccache not found (recommended for faster builds)",
                installation,
            ));
        }
        Err(ToolchainError::Unfixable(error)) => {
            items.push(DoctorItem::missing("sccache", unfixable_message(&error)));
        }
    }

    items
}
