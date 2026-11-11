//! The text-engine seam: the traits and seam types every text operation in
//! the backend routes through.
//!
//! [`TextEngine`] is the backend's contract for shaping, ink measurement and
//! recording, and [`TextLayout`] its contract for everything a laid-out text
//! answers — metrics, hit testing, and the visual moves editing makes. The
//! renderer carries its concrete engine as a type parameter, and each painter
//! constructs that engine at its entry point. Shaping and editing use static
//! dispatch throughout.
//!
//! Coordinates are in layout space before the ink shift, the way the shaping
//! backend reports them today.

use core::num::NonZeroUsize;

use cherenkov_record::Paint;
use nami::Computed;

use crate::renderer::{FrameWorkCounters, Recording};

use super::input::ResolvedTextLayoutInput;
#[cfg(hydrolysis_cherenkov)]
pub use super::types::{Affinity, CARET_WIDTH};
pub use super::types::{LineMetrics, TextPosition, TextSelection};

/// Everything a laid-out text answers: metrics, hit testing and the visual
/// moves editing makes.
pub trait TextLayout: Clone + Send + Sync + 'static {
    /// The number of laid-out lines.
    fn line_count(&self) -> usize;
    /// One line's metrics; panics when `line >= line_count`.
    fn line_metrics(&self, line: usize) -> LineMetrics;
    /// The laid-out height.
    fn height(&self) -> f32;
    /// The selection an anchor/focus pair forms, each end moved back to the
    /// start of the extended grapheme cluster it falls inside.
    fn selection(&self, anchor: TextPosition, focus: TextPosition) -> TextSelection;
    /// The grapheme boundary a point hits.
    fn hit_test(&self, x: f32, y: f32) -> TextPosition;
    /// The word under a point.
    fn word_at(&self, x: f32, y: f32) -> TextSelection;
    /// The line under a point.
    fn line_at(&self, x: f32, y: f32) -> TextSelection;
    /// The caret rectangle at `at`, [`CARET_WIDTH`] wide.
    fn caret_rect(&self, at: TextPosition) -> kurbo::Rect;
    /// Runs `each` on every rectangle `selection` covers.
    fn selection_rects(&self, selection: TextSelection, each: impl FnMut(kurbo::Rect));
    /// The selection with its focus at the previous grapheme boundary in
    /// visual order.
    fn previous_visual(&self, selection: TextSelection, extend: bool) -> TextSelection;
    /// The selection with its focus at the next grapheme boundary in visual
    /// order.
    fn next_visual(&self, selection: TextSelection, extend: bool) -> TextSelection;
}

/// A text shaping, measurement and recording engine.
pub trait TextEngine: Send + Sync + 'static {
    /// The layout this engine produces.
    type Layout: TextLayout;

    /// A layout carrying no text: zero lines.
    fn empty_layout(&self) -> Self::Layout;

    /// Shape `input` at `max_width`. An empty input lays out one empty line
    /// in its default style — the line a caret in an empty field occupies.
    fn shape(&self, input: &ResolvedTextLayoutInput, max_width: Option<f32>) -> Self::Layout;

    /// At most `max_lines` lines, the last ending in an ellipsis when the
    /// text overflows; a platform that ellipsizes itself does so in one
    /// call, otherwise the callback re-shapes respelled input.
    ///
    /// `shape` is the service's cached shaper; every reshape goes through it.
    fn shape_limited(
        &self,
        input: &ResolvedTextLayoutInput,
        max_width: Option<f32>,
        max_lines: NonZeroUsize,
        shape: impl FnMut(&ResolvedTextLayoutInput, Option<f32>) -> Self::Layout,
    ) -> Self::Layout;

    /// Horizontal ink extent of the measured-frame contract
    /// (water-rs/hydrolysis#237), measured from each line's aligned start:
    /// `0` is where the line's pen begins, so alignment never widens it.
    fn ink_extent(&self, layout: &Self::Layout, max_lines: Option<usize>) -> Option<(f32, f32)>;

    /// Records `layout` at the local origin, shifted right by `x_shift`, into
    /// the node's recording. With `paint`, the runs in the layout's default
    /// colour draw with that live paint instead, so a colour change reaches
    /// the engine without a recording; runs the text styled with their own
    /// colour keep it.
    fn record(
        &self,
        layout: &Self::Layout,
        max_lines: Option<usize>,
        x_shift: f32,
        paint: Option<&Computed<Paint>>,
        scene: &mut Recording,
        counters: &mut FrameWorkCounters,
    );

    /// `layout` as Cherenkov text, each font its runs draw with registered
    /// through `font`: how the GPU target draws the text a canvas names.
    ///
    /// # Errors
    ///
    /// The error `font` returns.
    ///
    /// An engine that does not lay text out for the Cherenkov targets is
    /// used on them against its contract: that is reported and the text
    /// registers as [`cherenkov::ResourceError::Unsupported`], so the canvas
    /// draws without it.
    #[cfg(hydrolysis_cherenkov)]
    fn cherenkov_text(
        &self,
        layout: &Self::Layout,
        font: &mut dyn FnMut(
            &parley::FontData,
        ) -> Result<cherenkov::Font, cherenkov::ResourceError>,
    ) -> Result<cherenkov::TextLayout, cherenkov::ResourceError> {
        let _ = (layout, font);
        waterui_core::contract_violation!(
            "this text engine does not lay text out for the Cherenkov targets; the text is not registered"
        );
        Err(cherenkov::ResourceError::Unsupported(
            "text layout on the Cherenkov targets",
        ))
    }

    /// The lease on the platform layout `layout` is: how the HWUI target
    /// draws the text a canvas names. `None` for a layout of no text.
    ///
    /// An engine that does not lay text out on the platform is used on it
    /// against its contract: that is reported and the text draws as none.
    #[cfg(any(test, hydrolysis_hwui))]
    fn platform_text(
        &self,
        layout: &Self::Layout,
    ) -> Option<std::sync::Arc<crate::hwui::text::TextLayoutResource>> {
        let _ = layout;
        waterui_core::contract_violation!(
            "this text engine does not lay text out on the HWUI platform; the text draws as none"
        );
        None
    }

    /// The collection the engine shapes with gained faces: drops every
    /// result the engine keeps that was made against the old set.
    #[cfg(all(target_arch = "wasm32", feature = "web"))]
    fn fonts_changed(&self);
}

/// How a named font family the collection cannot resolve is treated.
///
/// An application resolves leniently: a named family that is not installed is
/// skipped for the rest of its CSS list and the generic family answers, the
/// way a browser resolves `font-family` against whatever the host happens to
/// carry. A test host resolves strictly: the style package a test mounts
/// names the families its design assumes, so a family that is neither
/// installed on the host nor provided by a declared font file
/// ([`crate::DeclaredFonts`]) is a missing declaration — the shape reports
/// it as a contract violation naming the family (a debug build panics, a
/// release build logs and resolves the entry leniently) instead of silently
/// measuring a substitute face.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FontFamilyResolution {
    /// An unresolved named family falls through to the next list entry.
    #[default]
    Lenient,
    /// An unresolved named family is a contract violation, naming the
    /// family.
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
