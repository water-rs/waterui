//! `WaterUI` CLI library
//!
//! This crate provides core functionality for the `WaterUI` command-line interface (CLI) tool.
//! It includes modules for managing projects, installing dependencies, and interacting with different platforms.
//!
//! # Architecture
//!
//! The CLI is designed as a library with a terminal frontend:
//!
//! - **Library modules** (`backend`, `platform`, `device`, `project`, `build`) contain the core logic
//! - **Terminal frontend** (`cli/src/terminal/`) provides the user interface
//!
//! ## Key Concepts
//!
//! - **`BuildContext`** - Unified build configuration (release mode, hot reload, speedups)
//! - **`BuildCoordinator`** - Tracks build state to avoid redundant builds
//! - **`Platform`** - Abstracts platform-specific build and packaging
//! - **`Device`** - Abstracts device-specific deployment and running
//! - **`Backend`** - Handles project initialization and requirements checking
//! - **`ToolchainIssue`** - Represents problems that `water doctor` can diagnose/fix

#![allow(missing_docs)]

pub mod platform;

pub mod backend;
pub mod build;
pub mod crash;
pub mod device;
pub mod doctor;
pub mod installer;
pub mod output;
pub mod package;
pub mod project;
pub mod util;

pub const WATERUI_VERSION: &str = env!("WATERUI_VERSION");
pub const WATERUI_SWIFT_BACKEND_VERSION: &str = env!("WATERUI_BACKEND_SWIFT_VERSION");
pub const WATERUI_ANDROID_BACKEND_VERSION: &str = env!("WATERUI_BACKEND_ANDROID_VERSION");
pub const WATERUI_TRACING_PREFIX: &str = "[waterui::tracing]";

#[macro_use]
mod macros {
    #[macro_export]
    macro_rules! impl_display {
        ($ty:ty, $value:expr) => {
            impl core::fmt::Display for $ty {
                fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                    write!(f, $value)
                }
            }
        };
    }
}
