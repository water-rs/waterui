//! Diagram text, as real views.
//!
//! Labels are the reason this crate lays diagrams out with `WaterUI`'s own text
//! engine instead of accepting `merman`'s built-in metrics. Painting them into
//! the scene as glyphs would throw that away again: the diagram would be one
//! opaque picture to the accessibility tree, would not honour the platform's
//! text rendering, and could not be selected. So every label is a `text()` view,
//! placed into the box layout reserved for it.

use waterui_core::layout::{Layout, Point, ProposalSize, Rect, Size, StretchAxis, SubView};
use waterui_core::{Environment, View};
use waterui_graphics::color::{Color, ForegroundColor, MutedForegroundColor};
use waterui_text::text;

use crate::layout::{Emphasis, Label};
use crate::measure::FromF64Lossless as _;

/// Places one label centred in the box the diagram reserved for it.
///
/// The reserved box and the measured text agree by construction — the same
/// engine produced both — so the centring here only ever absorbs the
/// sub-pixel difference between a measurement taken during layout and one taken
/// during placement.
#[derive(Debug)]
pub struct Placement {
    frame: Rect,
}

impl Placement {
    /// Places a label in `frame`, in diagram coordinates.
    #[must_use]
    pub const fn new(frame: Rect) -> Self {
        Self { frame }
    }
}

impl Layout for Placement {
    fn size_that_fits(&self, _proposal: ProposalSize, _children: &[&dyn SubView]) -> Size {
        *self.frame.size()
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        let [child] = children else {
            panic!("a diagram label must contain exactly one text view");
        };
        let size = child.measure(ProposalSize::UNSPECIFIED).size;
        vec![Rect::new(
            Point::new(
                bounds.mid_x() - size.width / 2.0,
                bounds.mid_y() - size.height / 2.0,
            ),
            size,
        )]
    }

    fn stretch_axis(&self, _children: &[StretchAxis]) -> StretchAxis {
        StretchAxis::None
    }
}

/// The view for one label.
#[derive(Debug)]
pub struct LabelView {
    label: Label,
    font_size: f32,
}

impl LabelView {
    /// Builds the view for `label`, drawn at the diagram's font size.
    #[must_use]
    pub const fn new(label: Label, font_size: f32) -> Self {
        Self { label, font_size }
    }
}

impl View for LabelView {
    fn body(self, _env: &Environment) -> impl View {
        // Drawn at the size it was measured at. See `measure::label_style` for
        // why prominence may not touch that size: the box the diagram reserved
        // holds the text that measurement described and nothing wider, so
        // prominence is a colour here.
        let style = crate::measure::label_style(self.font_size);
        let colour = match self.label.emphasis {
            // Secondary text, such as a fragment's guard condition.
            Emphasis::Muted => Color::new(MutedForegroundColor),
            Emphasis::Normal | Emphasis::Title => Color::new(ForegroundColor),
        };
        text(self.label.text)
            .size(f32::from_f64_lossless(style.font_size))
            .color(colour)
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }
}
