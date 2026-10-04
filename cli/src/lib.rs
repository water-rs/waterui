#![doc = include_str!("../README.md")]
//! `WaterUI` CLI library for managing cross-platform builds and development workflows.
pub mod android;
pub mod apple;
pub mod artifact_symbols;
pub mod bench;
mod browser_runtime;
mod dependencies;
pub mod esp32;
pub mod gtk4;
pub mod hydrolysis;
pub mod mcp;
#[cfg(test)]
pub(crate) mod pinned_framework;
mod platforming;
pub mod preview;
mod project_model;
mod runtime;
pub mod self_update;
pub mod toolchain;
pub mod toolchain_checks;
pub mod tui;
pub mod winui;
mod workflows;

pub use dependencies::brew;
pub use platforming::{backend, macos_bundle, package_output, platform};
pub use project_model::assets::{FetchOutcome, seed_font_cache, seed_font_cache_for_backend};
pub(crate) use project_model::{assets, support_app, templates};
pub use project_model::{framework, project, project_types, water_dir, web};
pub use runtime::{build_info, utils};
pub(crate) use runtime::{runtime_compat, runtime_fingerprint};
pub use workflows::{build, capture, debug, device, diff, elf, gesture, inspector, rustc_wrapper};
