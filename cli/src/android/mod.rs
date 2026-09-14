//! Android platform support.

/// Android backend implementation.
pub mod backend;
/// Android device detection and management.
pub mod device;
/// Gradle package task output discovery.
pub(crate) mod output_metadata;
/// Android platform configuration.
pub mod platform;
pub(crate) mod toolchain;

pub use self::toolchain::{
    AndroidBuildTools, AndroidNdk, AndroidPlatformTools, AndroidRustTargets, AndroidSdk,
    AndroidSdkPlatforms, Java, Kotlin,
};
