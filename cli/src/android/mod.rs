//! Android platform support.

/// Android backend implementation.
pub mod backend;
/// Android device detection and management.
pub mod device;
/// Android platform configuration.
pub mod platform;
pub(crate) mod toolchain;

pub use self::toolchain::{
    AndroidBuildTools, AndroidNdk, AndroidPlatformTools, AndroidRustTargets, AndroidSdk,
    AndroidSdkPlatforms, Java, Kotlin,
};
