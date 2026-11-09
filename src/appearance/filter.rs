//! View metadata for compositor-native visual properties.
//!
//! `Opacity` is intentionally separate from GPU filter effects.
//! Backends should map it to native alpha/compositor properties when possible.

use nami::{Computed, SignalExt};
use waterui_core::{IntoSignalF32, metadata::MetadataKey};

/// Compositor opacity metadata.
///
/// This is not a GPU filter: it should avoid offscreen rendering and map to
/// native alpha properties whenever the backend supports it.
#[derive(Debug)]
#[non_exhaustive]
pub struct Opacity {
    /// Opacity value where 0.0 is fully transparent and 1.0 is fully opaque.
    value: Computed<f32>,
}

impl Opacity {
    /// Opacity value where 0.0 is fully transparent and 1.0 is fully opaque.
    #[must_use]
    pub const fn value(&self) -> &Computed<f32> {
        &self.value
    }
}

impl MetadataKey for Opacity {}

impl Opacity {
    /// Creates opacity metadata from a reactive or constant value.
    #[must_use]
    pub fn new(value: impl IntoSignalF32) -> Self {
        Self {
            value: value.into_signal_f32().computed(),
        }
    }
}
