//! Core foundation utilities for the `WaterUI` framework.
//!
//! This crate provides platform-specific utilities for accessing bundle resources
//! and asset paths on macOS and iOS platforms.

use core_foundation::bundle::CFBundle;
use std::path::PathBuf;

/// Returns the path to the main application bundle's resources directory.
///
/// This function uses Core Foundation to locate the main bundle and extract
/// the path to its resources directory.
///
/// # Panics
///
/// Panics if the main bundle cannot be found or if the bundle URL cannot
/// be converted to a valid file system path.
#[must_use]
pub fn bundle_path() -> PathBuf {
    let bundle = CFBundle::main_bundle();

    bundle.bundle_resources_url().unwrap().to_path().unwrap()
}

/// Returns the path to the assets directory within the main bundle.
///
/// This is a convenience function that combines [`bundle_path()`] with
/// the "assets" subdirectory.
#[must_use]
pub fn assets_path() -> PathBuf {
    bundle_path().join("assets")
}
