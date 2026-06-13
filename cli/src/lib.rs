//! `WaterUI` CLI library for managing cross-platform builds and development workflows.
pub mod android;
pub mod apple;
mod dependencies;
pub mod esp32;
pub mod gtk4;
pub mod hydrolysis;
mod platforming;
pub mod preview;
mod project_model;
mod runtime;
pub mod toolchain;
mod workflows;

pub use dependencies::brew;
pub use platforming::{backend, macos_bundle, platform};
pub(crate) use project_model::{assets, support_app, templates};
pub use project_model::{project, project_types, water_dir};
pub use runtime::{build_info, utils};
pub(crate) use runtime::{runtime_compat, runtime_fingerprint};
pub use workflows::{build, capture, debug, device, diff, gesture, inspector};
