//! The text seam's value types: positions, selections and line metrics.
//!
//! They depend on no renderer, so a target whose renderer is not built yet
//! answers the seam's queries in these same types.

/// The caret width every caret rectangle carries.
pub const CARET_WIDTH: f32 = 1.0;

/// Which side of a cluster boundary a position attaches to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Affinity {
    /// The position attaches to the cluster that follows it.
    Downstream,
    /// The position attaches to the cluster that precedes it.
    Upstream,
}

/// A caret position inside a layout: a byte index into the layout's text and
/// the side it attaches to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextPosition {
    /// Byte index into the layout's text.
    pub(crate) index: usize,
    /// The side the position attaches to.
    pub(crate) affinity: Affinity,
}

impl TextPosition {
    /// The rule every editing caller uses today: at or past the end attaches
    /// upstream.
    pub(crate) const fn in_text(index: usize, text_len: usize) -> Self {
        Self {
            index,
            affinity: if index >= text_len {
                Affinity::Upstream
            } else {
                Affinity::Downstream
            },
        }
    }
}

/// An anchor/focus pair inside a layout, each end already snapped to the
/// layout's cluster boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextSelection {
    /// The end the selection was initiated at.
    pub(crate) anchor: TextPosition,
    /// The end the selection currently extends to.
    pub(crate) focus: TextPosition,
}

impl TextSelection {
    /// A caret at `at`.
    pub(crate) const fn collapsed(at: TextPosition) -> Self {
        Self {
            anchor: at,
            focus: at,
        }
    }

    /// Whether the selection is a single position rather than a range:
    /// anchor and focus carry the same index.
    pub(crate) const fn is_collapsed(&self) -> bool {
        self.anchor.index == self.focus.index
    }
}

/// One line's advance, line height and baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineMetrics {
    /// The line's laid-out width.
    pub(crate) advance: f32,
    /// The height the line occupies.
    pub(crate) line_height: f32,
    /// The line's baseline offset.
    pub(crate) baseline: f32,
}
