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
        Installation, Toolchain, ToolchainError, UnfixableToolchain,
        cmake::Cmake,
        linux::LinuxSystemToolchain,
        rust::RustToolchain,
        sccache::Sccache,
        web::{Wasm32UnknownUnknownTarget, WasmPack},
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

async fn push_toolchain_check<T>(
    items: &mut Vec<DoctorItem>,
    name: &'static str,
    fixable_message: &'static str,
    toolchain: T,
) where
    T: Toolchain,
    T::Installation: Send + 'static,
{
    match toolchain.check().await {
        Ok(()) => items.push(DoctorItem::ok(name)),
        Err(ToolchainError::Fixable(installation)) => {
            items.push(DoctorItem::fixable(name, fixable_message, installation));
        }
        Err(ToolchainError::Unfixable(error)) => {
            items.push(DoctorItem::missing(name, unfixable_message(&error)));
        }
    }
}

async fn push_toolchain_check_with_unfixable<T, F>(
    items: &mut Vec<DoctorItem>,
    name: &'static str,
    fixable_message: &'static str,
    toolchain: T,
    unfixable_message_fn: F,
) where
    T: Toolchain,
    T::Installation: Send + 'static,
    F: FnOnce(&UnfixableToolchain) -> String,
{
    match toolchain.check().await {
        Ok(()) => items.push(DoctorItem::ok(name)),
        Err(ToolchainError::Fixable(installation)) => {
            items.push(DoctorItem::fixable(name, fixable_message, installation));
        }
        Err(ToolchainError::Unfixable(error)) => {
            items.push(DoctorItem::missing(name, unfixable_message_fn(&error)));
        }
    }
}

async fn push_apple_checks(items: &mut Vec<DoctorItem>) {
    if !cfg!(target_os = "macos") {
        items.push(DoctorItem::skipped("Xcode"));
        items.push(DoctorItem::skipped("iOS SDK"));
        items.push(DoctorItem::skipped("iOS Simulator SDK"));
        items.push(DoctorItem::skipped("iOS Simulators"));
        items.push(DoctorItem::skipped("macOS SDK"));
        return;
    }

    push_simple_check(items, "Xcode", Xcode.check().await);
    push_simple_check(items, "iOS SDK", AppleSdk::Ios.check().await);
    push_simple_check(
        items,
        "iOS Simulator SDK",
        AppleSdk::IosSimulator.check().await,
    );
    push_ios_simulator_check(items).await;
    push_simple_check(items, "macOS SDK", AppleSdk::Macos.check().await);
}

fn push_simple_check(
    items: &mut Vec<DoctorItem>,
    name: &'static str,
    result: Result<(), impl std::fmt::Display>,
) {
    match result {
        Ok(()) => items.push(DoctorItem::ok(name)),
        Err(error) => items.push(DoctorItem::missing(name, error.to_string())),
    }
}

async fn push_ios_simulator_check(items: &mut Vec<DoctorItem>) {
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
}

async fn push_android_sdk_checks(items: &mut Vec<DoctorItem>) -> bool {
    push_toolchain_check(
        items,
        "Android SDK",
        "Android SDK is missing (automatic install is supported on this host)",
        AndroidSdk,
    )
    .await;

    AndroidSdk::detect_path().is_some() || AndroidSdk::sdkmanager_path().await.is_some()
}

async fn push_android_component_checks(items: &mut Vec<DoctorItem>, sdk_ready: bool) {
    if !sdk_ready {
        push_blocked_android_component_checks(items);
        return;
    }

    push_toolchain_check(
        items,
        "Android Platform-Tools (adb)",
        "Required for `water run --platform android`",
        AndroidPlatformTools,
    )
    .await;
    push_toolchain_check(
        items,
        "Android SDK Platforms",
        "Required for Android build/package workflows",
        AndroidSdkPlatforms,
    )
    .await;
    push_toolchain_check(
        items,
        "Android SDK Build-Tools (d8)",
        "Required for Android build/package workflows",
        AndroidBuildTools,
    )
    .await;
    push_toolchain_check(
        items,
        "Android NDK",
        "Required for Android build/package workflows",
        AndroidNdk,
    )
    .await;
    push_toolchain_check(
        items,
        "Android Rust Targets",
        "Required for Android Rust cross-compilation",
        AndroidRustTargets,
    )
    .await;
}

fn push_blocked_android_component_checks(items: &mut Vec<DoctorItem>) {
    for name in [
        "Android Platform-Tools (adb)",
        "Android NDK",
        "Android SDK Platforms",
        "Android SDK Build-Tools (d8)",
        "Android Rust Targets",
    ] {
        items.push(DoctorItem::missing(
            name,
            "Blocked: Android SDK / `sdkmanager` is not ready yet. Fix Android SDK first.",
        ));
    }
}

async fn push_android_run_target_check(items: &mut Vec<DoctorItem>) {
    if AndroidSdk::adb_path().is_none() {
        items.push(DoctorItem::missing(
            "Android Run Targets",
            "Blocked: Android Platform-Tools (`adb`) is not ready yet.",
        ));
        return;
    }

    match AndroidDevice::scan().await {
        Ok(devices) if !devices.is_empty() => items.push(DoctorItem::ok("Android Run Targets")),
        Ok(_) => match AndroidPlatform::list_avds().await {
            Ok(avds) if !avds.is_empty() => items.push(DoctorItem::ok("Android Run Targets")),
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
}

async fn push_desktop_and_web_checks(items: &mut Vec<DoctorItem>) {
    push_toolchain_check(
        items,
        "Host CMake",
        "Required for native Rust dependencies in Android builds",
        Cmake::default(),
    )
    .await;

    if WindowsArm64LlvmToolchain::required_on_host() {
        push_toolchain_check(
            items,
            "Windows ARM64 LLVM toolchain",
            "Required by native assembly dependencies in Windows ARM64 hydrolysis builds",
            WindowsArm64LlvmToolchain,
        )
        .await;
    } else {
        items.push(DoctorItem::skipped_with_message(
            "Windows ARM64 LLVM toolchain",
            "Only required on Windows ARM64 hosts for native assembly dependencies.",
        ));
    }

    push_toolchain_check(items, "Java", "Required for Android Gradle builds", Java).await;
    push_toolchain_check(
        items,
        "Kotlin",
        "Required for Android Kotlin helper compilation",
        Kotlin,
    )
    .await;
    push_toolchain_check_with_unfixable(
        items,
        "Rust wasm32 target",
        "wasm32-unknown-unknown target not installed",
        Wasm32UnknownUnknownTarget,
        ToString::to_string,
    )
    .await;
    push_toolchain_check_with_unfixable(
        items,
        "wasm-pack",
        "wasm-pack not found (required for web packaging)",
        WasmPack,
        ToString::to_string,
    )
    .await;
}

async fn push_linux_checks(items: &mut Vec<DoctorItem>) {
    if !cfg!(target_os = "linux") {
        items.push(DoctorItem::skipped("Linux system packages"));
        items.push(DoctorItem::skipped("GTK4"));
        return;
    }

    let linux_packages_fixable = match LinuxSystemToolchain.check().await {
        Ok(()) => {
            items.push(DoctorItem::ok("Linux system packages"));
            false
        }
        Err(ToolchainError::Fixable(installation)) => {
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
            true
        }
        Err(ToolchainError::Unfixable(error)) => {
            items.push(DoctorItem::missing(
                "Linux system packages",
                unfixable_message(&error),
            ));
            false
        }
    };

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
}

/// Run diagnostics on all toolchains and return a report.
pub async fn doctor() -> Vec<DoctorItem> {
    let mut items = Vec::new();
    push_apple_checks(&mut items).await;
    push_rust_toolchain_check(&mut items).await;
    let sdk_ready = push_android_sdk_checks(&mut items).await;
    push_android_component_checks(&mut items, sdk_ready).await;
    push_android_run_target_check(&mut items).await;
    push_desktop_and_web_checks(&mut items).await;
    push_linux_checks(&mut items).await;
    push_toolchain_check(
        &mut items,
        "sccache",
        "sccache not found (recommended for faster builds)",
        Sccache,
    )
    .await;

    items
}

async fn push_rust_toolchain_check(items: &mut Vec<DoctorItem>) {
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
        Err(ToolchainError::Unfixable(error)) => items.push(DoctorItem::missing(
            "Rust toolchain",
            unfixable_message(&error),
        )),
    }
}
