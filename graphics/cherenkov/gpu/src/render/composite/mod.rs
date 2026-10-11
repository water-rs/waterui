//! Shared pixel-local composition planning and backend execution.

pub mod cache;
pub mod plan;

#[cfg(target_vendor = "apple")]
pub mod metal;
