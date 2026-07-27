//! Android platform support.

/// Oldest Android API level supported by the native runtime and its `AAudio` backend.
pub(crate) const ANDROID_MIN_API_LEVEL: u32 = 26;

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
