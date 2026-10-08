//! Android platform support.

/// The `adb` client with its server running.
pub mod adb;
/// Android device detection and management.
pub mod device;
/// Embedded Android NDK version and its runtime-Gradle parser.
pub mod ndk_version;
/// Gradle package task output discovery.
pub(crate) mod output_metadata;
/// Android platform configuration.
pub mod platform;
/// Android release signing (`[signing.android]`).
pub mod signing;
pub(crate) mod toolchain;

pub use self::toolchain::{
    AndroidBuildTools, AndroidNdk, AndroidPlatformTools, AndroidRustTargets, AndroidSdk,
    AndroidSdkPlatforms, Java, Kotlin,
};

/// The `<uses-permission>` entries the project manifest enables, for the
/// backend that scaffolds an `AndroidManifest.xml` — the Hydrolysis host.
pub(crate) fn manifest_permissions(
    manifest: &crate::project::Manifest,
) -> Vec<crate::project_types::AndroidPermissionName> {
    manifest
        .permissions
        .iter()
        .filter(|(_, entry)| entry.is_enabled())
        .filter_map(|(key, _)| key.android_permission_name())
        .collect()
}
