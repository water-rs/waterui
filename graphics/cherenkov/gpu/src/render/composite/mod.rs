//! Shared pixel-local composition planning and backend execution.
//!
//! Planning is platform-neutral (issue #2549): [`plan`] derives logical
//! image lifetimes and tile epochs from the canonical frame alone, and a
//! backend supplies an executor plus the [`plan::Limits`] it works
//! within. Logical lifetimes and materialized storage are planned on every
//! backend. Without a native executor, images remain in device memory with
//! [`crate::render::composition::Materialization::NoExecutor`].

pub mod cache;
pub mod plan;

#[cfg(target_vendor = "apple")]
pub mod metal;

#[cfg(all(unix, not(target_vendor = "apple"), not(target_arch = "wasm32")))]
pub mod vulkan;
