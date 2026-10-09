//! The Android text engine's Rust half: a layout backed by a platform
//! `StaticLayout` that the Kotlin `HwuiTextProvider` builds, measures and
//! draws once into its own `RenderNode`.
//!
//! [`HwuiTextLayout`]'s inherent methods answer what the seam's
//! `TextLayout` asks, one for one, against the live platform layout. The
//! seam's `impl TextEngine` and its `record`, which emits `Draw::text`, join
//! once the session mounts the HWUI target (water-rs/waterui#1809).
//!
//! The Rust side holds the string, so every offset crossing the boundary is
//! converted here between the seam's byte indices and the platform's UTF-16
//! indices ([`Utf16Index`]). A layout's platform id is a dense
//! [`TextLayoutIds`] id; dropping its last clone queues a
//! `ReleaseTextLayout` into the next command buffer.

mod ids;
mod index;
#[cfg(hydrolysis_hwui)]
mod jni;
mod layout;
mod provider;
#[cfg(test)]
mod tests;
pub(super) mod wire;

pub use ids::TextLayoutIds;
#[cfg(test)]
pub(in crate::hwui) use index::Utf16Index;
#[cfg(hydrolysis_hwui)]
pub use jni::JniTextProvider;
pub use layout::HwuiTextLayout;
pub use provider::PlatformText;
pub use wire::{PackedRequest, ShapeRequest, TextAlignment, TextRun};
#[cfg(test)]
pub(in crate::hwui) use wire::{
    decode_reply, describe_paragraph, describe_run, unpack_position, unpack_range,
};
