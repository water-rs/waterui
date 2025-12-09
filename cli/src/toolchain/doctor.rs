//! Toolchain diagnostics for the `water doctor` command.

use crate::{
    android::toolchain::{AndroidNdk, AndroidSdk, Java},
    apple::toolchain::{AppleSdk, Xcode},
    toolchain::Toolchain,
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

/// A single item in the doctor report.
#[derive(Debug)]
pub struct DoctorItem {
    /// Name of the toolchain or component.
    pub name: &'static str,
    /// Status of the check.
    pub status: CheckStatus,
    /// Optional message with details or suggestions.
    pub message: Option<String>,
}

impl DoctorItem {
    const fn ok(name: &'static str) -> Self {
        Self {
            name,
            status: CheckStatus::Ok,
            message: None,
        }
    }

    fn missing(name: &'static str, message: impl Into<String>) -> Self {
        Self {
            name,
            status: CheckStatus::Missing,
            message: Some(message.into()),
        }
    }

    const fn skipped(name: &'static str) -> Self {
        Self {
            name,
            status: CheckStatus::Skipped,
            message: None,
        }
    }
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

        // Check macOS SDK
        match AppleSdk::Macos.check().await {
            Ok(()) => items.push(DoctorItem::ok("macOS SDK")),
            Err(e) => items.push(DoctorItem::missing("macOS SDK", e.to_string())),
        }
    } else {
        items.push(DoctorItem::skipped("Xcode"));
        items.push(DoctorItem::skipped("iOS SDK"));
        items.push(DoctorItem::skipped("macOS SDK"));
    }

    // Check Android SDK
    match AndroidSdk.check().await {
        Ok(()) => items.push(DoctorItem::ok("Android SDK")),
        Err(e) => items.push(DoctorItem::missing("Android SDK", e.to_string())),
    }

    // Check Android NDK
    match AndroidNdk.check().await {
        Ok(()) => items.push(DoctorItem::ok("Android NDK")),
        Err(e) => items.push(DoctorItem::missing("Android NDK", e.to_string())),
    }

    // Check Java
    match Java::detect_path().await {
        Some(_) => items.push(DoctorItem::ok("Java")),
        None => items.push(DoctorItem::missing(
            "Java",
            "Install JDK or set JAVA_HOME. Android Studio includes a bundled JDK.",
        )),
    }

    items
}
