//! `WaterUI` CLI library
//! This crate provides core functionality for the `WaterUI` command-line interface (CLI) tool.
//! It includes modules for managing projects, installing dependencies, and interacting with different platforms.
#![allow(missing_docs)]

pub mod platform;

pub mod backend;
pub mod device;
pub mod doctor;
pub(crate) mod installer;
pub mod output;
pub mod package;
pub mod project;
pub mod util;

pub const WATERUI_VERSION: &str = env!("WATERUI_VERSION");
pub const WATERUI_SWIFT_BACKEND_VERSION: &str = env!("WATERUI_BACKEND_SWIFT_VERSION");

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
