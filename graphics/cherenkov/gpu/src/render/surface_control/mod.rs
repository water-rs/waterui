//! Android system-compositor planes through `SurfaceControl` (#90).
//!
//! A surface created on an
//! [`interop::android::SurfaceControlTarget`](crate::interop::android::SurfaceControlTarget)
//! is realized as child surface controls of the host's parent: the engine's
//! own composited content on plane buffers it renders into, and promoted
//! external frames on planes fed their `AHardwareBuffer` directly, with the
//! frame's dataspace and HDR metadata, so hardware overlays and the
//! system's tone mapping apply. [`plan`] holds what a transaction sets;
//! [`planes`] builds and applies it.

#[cfg(target_os = "android")]
mod buffer;
#[cfg(target_os = "android")]
pub mod ffi;
#[cfg(any(target_os = "android", test))]
pub mod plan;
#[cfg(target_os = "android")]
pub mod planes;
