//! Metadata definitions for `WaterUI`.
//!
//! `Metadata`s are some extra information that can be attached to `View`s to modify their behavior
//! or appearance, but not affect their layout.
//!
//! They are defined as types that implement the `MetadataKey` trait.

/// The metadata mechanism itself: the wrappers that attach a value to a view
/// and the marker trait that makes a type attachable. Re-exported here so the
/// mechanism and the concrete keys below live at one path.
pub use waterui_core::metadata::{Metadata, MetadataKey, Retain};

/// Anchored overlay metadata module.
pub use waterui_controls::anchored_overlay;

/// Context menu metadata module.
#[cfg(feature = "menu")]
pub use waterui_controls::context_menu;

/// Secure and dynamic-range metadata module.
pub use waterui_controls::secure;
