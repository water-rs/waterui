//! Refresh-rate vote types shared across core, the Cherenkov engine, and backends.

use nami::{Computed, signal::IntoComputed};
pub use waterui_frame_rate::FrameRate;

use crate::metadata::MetadataKey;

/// Metadata key carrying a view subtree's explicit [`FrameRate`] vote while
/// mounted and visible.
#[derive(Clone, Debug)]
pub struct FrameRateVote(Computed<Option<FrameRate>>);

impl MetadataKey for FrameRateVote {}

impl FrameRateVote {
    /// Creates a new frame-rate vote from a constant or reactive signal.
    #[must_use]
    pub fn new(rate: impl IntoComputed<Option<FrameRate>>) -> Self {
        Self(rate.into_computed())
    }

    /// Returns the reactive frame-rate vote signal.
    #[must_use]
    pub const fn signal(&self) -> &Computed<Option<FrameRate>> {
        &self.0
    }
}
