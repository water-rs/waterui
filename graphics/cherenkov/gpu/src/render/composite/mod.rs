//! Shared pixel-local composition planning and backend execution.
//!
//! Planning is platform-neutral: [`plan`] derives image lifetimes and read
//! dependencies on every backend. A native executor supplies tile limits;
//! otherwise the same graph uses materialized images. [`demand`] walks that
//! graph backward from the persistent output regions that need repainting.

pub mod cache;
pub mod demand;
pub mod plan;

#[cfg(target_vendor = "apple")]
pub mod metal;

#[cfg(all(unix, not(target_vendor = "apple"), not(target_arch = "wasm32")))]
pub mod vulkan;
