//! Toolchain diagnostics for the `water doctor` command.

use std::future::Future;
use std::pin::Pin;

use color_eyre::eyre;

use crate::{
    android::toolchain::{AndroidNdk, AndroidSdk, Java, Kotlin},
    apple::toolchain::{AppleSdk, Xcode},
    gtk4::toolchain::Gtk4Toolchain,
    toolchain::{
        Installation, Toolchain, ToolchainError, UnfixableToolchain, linux::LinuxSystemToolchain,
        rust::RustToolchain, sccache::Sccache,
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

        // Check macOS SDK
        match AppleSdk::Macos.check().await {
            Ok(()) => items.push(DoctorItem::ok("macOS SDK")),
            Err(e) => items.push(DoctorItem::missing("macOS SDK", e.to_string())),
        }
    } else {
        items.push(DoctorItem::skipped("Xcode"));
        items.push(DoctorItem::skipped("iOS SDK"));
        items.push(DoctorItem::skipped("iOS Simulator SDK"));
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
                "Android SDK is missing but has an automatic install plan.",
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

    // Check Android NDK
    match AndroidNdk.check().await {
        Ok(()) => items.push(DoctorItem::ok("Android NDK")),
        Err(ToolchainError::Fixable(installation)) => {
            items.push(DoctorItem::fixable(
                "Android NDK",
                "Android NDK is missing but has an automatic install plan.",
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

    // Check Java
    match Java::detect_path().await {
        Some(_) => items.push(DoctorItem::ok("Java")),
        None => items.push(DoctorItem::missing(
            "Java",
            "Install JDK or set JAVA_HOME. Android Studio includes a bundled JDK.",
        )),
    }

    // Check Kotlin
    match Kotlin.check().await {
        Ok(()) => items.push(DoctorItem::ok("Kotlin")),
        Err(ToolchainError::Fixable(installation)) => {
            items.push(DoctorItem::fixable(
                "Kotlin",
                "Kotlin compiler is missing but has an automatic install plan.",
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
            Err(ToolchainError::Unfixable(e)) => {
                items.push(DoctorItem::missing(
                    "Linux system packages",
                    unfixable_message(&e),
                ));
            }
        }
    } else {
        items.push(DoctorItem::skipped("Linux system packages"));
    }

    // Check GTK4 toolchain
    match Gtk4Toolchain.check().await {
        Ok(()) => items.push(DoctorItem::ok("GTK4")),
        Err(ToolchainError::Fixable(installation)) => {
            let msg = match &installation {
                crate::gtk4::toolchain::Gtk4Installation::PkgConfig => {
                    "pkg-config not found (can be installed via Homebrew)"
                }
                crate::gtk4::toolchain::Gtk4Installation::Gtk4 => {
                    "GTK4 not found (can be installed via Homebrew)"
                }
                crate::gtk4::toolchain::Gtk4Installation::Both => {
                    "pkg-config and GTK4 not found (can be installed via Homebrew)"
                }
            };
            items.push(DoctorItem::fixable("GTK4", msg, installation));
        }
        Err(ToolchainError::Unfixable(error)) => {
            if cfg!(target_os = "linux") && linux_packages_fixable {
                items.push(DoctorItem::missing(
                    "GTK4",
                    "GTK4 probe failed because required Linux packages are missing. Run `water doctor --fix` to install Linux system packages, then re-run `water doctor`.",
                ));
            } else {
                items.push(DoctorItem::missing("GTK4", unfixable_message(&error)));
            }
        }
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
        Err(ToolchainError::Unfixable(e)) => {
            items.push(DoctorItem::missing("sccache", unfixable_message(&e)));
        }
    }

    items
}
