//! The text-engine seam: the traits and seam types every text operation in
//! the backend routes through.
//!
//! [`TextEngine`] is the backend's contract for shaping, ink measurement and
//! recording, and [`TextLayout`] its contract for everything a laid-out text
//! answers — metrics, hit testing, and the visual moves editing makes. The
//! session binds exactly one engine (`SessionTextEngine`), so there is no
//! `dyn` dispatch and no runtime choice: the seam exists so the platform
//! backends each name their own shaper in one place.
//!
//! Coordinates are in layout space before the ink shift, the way the shaping
//! backend reports them today.

use core::num::NonZeroUsize;

use crate::renderer::{FrameWorkCounters, Recording};

use super::input::ResolvedTextLayoutInput;

pub use super::types::{Affinity, CARET_WIDTH, LineMetrics, TextPosition, TextSelection};

/// Everything a laid-out text answers: metrics, hit testing and the visual
/// moves editing makes.
pub trait TextLayout: Clone + Send + Sync + 'static {
    /// The number of laid-out lines.
    fn line_count(&self) -> usize;
    /// One line's metrics; panics when `line >= line_count`.
    fn line_metrics(&self, line: usize) -> LineMetrics;
    /// The laid-out height.
    fn height(&self) -> f32;
    /// The selection an anchor/focus pair forms, snapped to clusters.
    fn selection(&self, anchor: TextPosition, focus: TextPosition) -> TextSelection;
    /// The position a point hits.
    fn hit_test(&self, x: f32, y: f32) -> TextPosition;
    /// The word under a point.
    fn word_at(&self, x: f32, y: f32) -> TextSelection;
    /// The line under a point.
    fn line_at(&self, x: f32, y: f32) -> TextSelection;
    /// The caret rectangle at `at`, [`CARET_WIDTH`] wide.
    fn caret_rect(&self, at: TextPosition) -> kurbo::Rect;
    /// Runs `each` on every rectangle `selection` covers.
    fn selection_rects(&self, selection: TextSelection, each: impl FnMut(kurbo::Rect));
    /// The selection with its focus at the previous cluster in visual order.
    fn previous_visual(&self, selection: TextSelection, extend: bool) -> TextSelection;
    /// The selection with its focus at the next cluster in visual order.
    fn next_visual(&self, selection: TextSelection, extend: bool) -> TextSelection;
}

/// A text shaping, measurement and recording engine.
pub trait TextEngine: Send + Sync + 'static {
    /// The layout this engine produces.
    type Layout: TextLayout;

    /// A layout carrying no text: zero lines.
    fn empty_layout(&self) -> Self::Layout;

    /// Shape `input` at `max_width`; never called with empty input.
    fn shape(&self, input: &ResolvedTextLayoutInput, max_width: Option<f32>) -> Self::Layout;

    /// At most `max_lines`, the last ending in an ellipsis when the text
    /// overflows; otherwise `layout`.
    ///
    /// `shape` is the service's cached shaper; every reshape goes through it.
    fn truncate_tail(
        &self,
        layout: Self::Layout,
        input: &ResolvedTextLayoutInput,
        max_width: Option<f32>,
        max_lines: NonZeroUsize,
        shape: impl FnMut(&ResolvedTextLayoutInput, Option<f32>) -> Self::Layout,
    ) -> Self::Layout;

    /// Horizontal ink extent of the measured-frame contract
    /// (water-rs/hydrolysis#237).
    fn ink_extent(&self, layout: &Self::Layout, max_lines: Option<usize>) -> Option<(f32, f32)>;

    /// Records `layout` at the local origin, shifted right by `x_shift`, into
    /// the node's recording.
    fn record(
        &self,
        layout: &Self::Layout,
        max_lines: Option<usize>,
        x_shift: f32,
        scene: &mut Recording,
        counters: &mut FrameWorkCounters,
    );
}

/// How a named font family the collection cannot resolve is treated.
///
/// An application resolves leniently: a named family that is not installed is
/// skipped for the rest of its CSS list and the generic family answers, the
/// way a browser resolves `font-family` against whatever the host happens to
/// carry. A test host resolves strictly: the style package a test mounts
/// names the families its design assumes, so a missing one means the host
/// never ran the package's font install script — the shape panics naming the
/// family instead of silently measuring a substitute face.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FontFamilyResolution {
    /// An unresolved named family falls through to the next list entry.
    #[default]
    Lenient,
    /// An unresolved named family panics, naming the family.
    Strict,
}

impl FontFamilyResolution {
    pub(in crate::text) const fn is_strict(self) -> bool {
        matches!(self, Self::Strict)
    }
}

/// How a drawn text treats the tail that a line limit cuts off.
#[derive(Clone, Copy)]
pub enum TailMark {
    /// No line limit — every laid-out line draws.
    None,
    /// At most the given lines draw; the rest are clipped.
    Clip(usize),
    /// At most the given lines draw and the last carries a trailing
    /// ellipsis — what a `Text` leaf's `line_limit` means.
    Ellipsis(usize),
}

impl TailMark {
    pub(crate) const fn parts(self) -> (Option<usize>, bool) {
        match self {
            Self::None => (None, false),
            Self::Clip(limit) => (Some(limit), false),
            Self::Ellipsis(limit) => (Some(limit), true),
        }
    }
}
