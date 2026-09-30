//! Views whose pixels are drawn rather than bridged: GPU content a native host
//! presents, and pictures rasterized for it.

// Gated on `gpu` alone: Android drives these through the JNI bindings, which
// are compiled with `android-jni` and without `c-api`.
#[cfg(feature = "gpu")]
pub mod gpu_content;
#[cfg(feature = "gpu")]
pub mod gpu_content_input;
#[cfg(feature = "gpu")]
pub mod gpu_runtime;
pub mod picture;
pub mod view_renderer;
