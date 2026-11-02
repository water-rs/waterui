//! The platform half of the text engine, as the layout calls it.

use crate::hwui::HwuiError;

use super::wire::PackedRequest;

/// The calls [`HwuiTextLayout`](super::HwuiTextLayout) makes on the Kotlin
/// `HwuiTextProvider`, one per method there, in its UTF-16 offsets. On
/// Android, `JniTextProvider` implements it over JNI.
pub trait PlatformText: Send + Sync + 'static {
    /// Builds platform layout `id` from `text` and returns the packed reply
    /// [`decode_reply`](super::wire::decode_reply) reads. Nothing is registered
    /// when this fails.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Text`] when the platform refuses the request.
    fn shape(&self, id: u32, text: &str, request: &PackedRequest) -> Result<Vec<f32>, HwuiError>;

    /// The caret rectangle at `offset` as left, top, right, bottom.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Text`] when the platform refuses the query.
    fn caret_rect(&self, id: u32, offset: i32, upstream: bool) -> Result<[f32; 4], HwuiError>;

    /// The offset a point hits, and whether it attaches upstream.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Text`] when the platform refuses the query.
    fn hit_test(&self, id: u32, x: f32, y: f32) -> Result<(i32, bool), HwuiError>;

    /// The word under a point as start, end.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Text`] when the platform refuses the query.
    fn word_at(&self, id: u32, x: f32, y: f32) -> Result<(i32, i32), HwuiError>;

    /// The line under a point as start, end.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Text`] when the platform refuses the query.
    fn line_at(&self, id: u32, x: f32, y: f32) -> Result<(i32, i32), HwuiError>;

    /// The cluster boundary at or before `offset`.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Text`] when the platform refuses the query.
    fn snap(&self, id: u32, offset: i32) -> Result<i32, HwuiError>;

    /// The rectangles covering `start` until `end`, as left, top, right,
    /// bottom.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Text`] when the platform refuses the query.
    fn selection_rects(&self, id: u32, start: i32, end: i32) -> Result<Vec<[f32; 4]>, HwuiError>;

    /// The offset one cluster to the left of `offset`, the way the arrow
    /// key moves.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Text`] when the platform refuses the query.
    fn previous_visual(&self, id: u32, offset: i32) -> Result<i32, HwuiError>;

    /// The offset one cluster to the right of `offset`.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Text`] when the platform refuses the query.
    fn next_visual(&self, id: u32, offset: i32) -> Result<i32, HwuiError>;
}
