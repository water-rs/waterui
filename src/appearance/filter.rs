//! View metadata for compositor-native visual properties.
//!
//! `Opacity` is intentionally separate from GPU filter effects.
//! Backends should map it to native alpha/compositor properties when possible.

pub use waterui_shape::filter::Opacity;
