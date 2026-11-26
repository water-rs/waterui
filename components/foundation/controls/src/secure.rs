//! Secure and dynamic-range metadata.

use nami::{Computed, signal::IntoComputed};
use waterui_core::metadata::MetadataKey;

/// Secure metadata for secure fields.
///
/// User would be forbidden to take a screenshot of the view that has this metadata.
#[derive(Debug)]
pub struct Secure;

impl MetadataKey for Secure {}

impl Default for Secure {
    fn default() -> Self {
        Self::new()
    }
}

impl Secure {
    /// Creates a new Secure metadata.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Selects the dynamic range for a subtree.
///
/// Use `ViewExt::dynamic_range` (or `Avatar::dynamic_range` on an avatar) to
/// apply it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum DynamicRange {
    /// Standard dynamic range. Use for content that should not exceed
    /// `1.0` per channel — avatars, screenshots, UI chrome.
    Standard,
    /// High dynamic range. Honors per-color `headroom` so highlights can
    /// extend beyond the display's nominal white point.
    #[default]
    High,
}

nami::impl_constant!(DynamicRange);

/// Dynamic-range metadata attached by `ViewExt::dynamic_range`.
///
/// By default, `WaterUI` renders subtrees at [`DynamicRange::High`]. Attach
/// this metadata to pin a subtree (such as an avatar) to
/// [`DynamicRange::Standard`] or switch it dynamically through a signal.
#[derive(Debug)]
#[non_exhaustive]
pub struct DynamicRangeMetadata {
    range: Computed<DynamicRange>,
}

impl MetadataKey for DynamicRangeMetadata {}

impl DynamicRangeMetadata {
    /// Creates dynamic-range metadata from a constant or reactive value.
    #[must_use]
    pub fn new(range: impl IntoComputed<DynamicRange>) -> Self {
        Self {
            range: range.into_computed(),
        }
    }

    /// Returns the requested dynamic range signal for this subtree.
    #[must_use]
    pub const fn range(&self) -> &Computed<DynamicRange> {
        &self.range
    }
}
