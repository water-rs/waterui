//! On-device verification harness for cherenkov's Android
//! system-compositor planes (issue #90).

pub mod pattern;
#[path = "../../planes_common/recorded.rs"]
pub mod recorded;
pub mod scenario;

#[cfg(target_os = "android")]
mod ahb;
#[cfg(target_os = "android")]
mod app;
mod logcat;
pub mod observe;
#[cfg(target_os = "android")]
mod producer;
#[cfg(target_os = "android")]
mod text;
