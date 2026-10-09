//! Android platform support.

/// The `adb` client with its server running.
pub mod adb;
/// Android backend implementation.
pub mod backend;
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
    AndroidSdkPlatforms, Java, Kotlin, KotlinToolchain,
};
