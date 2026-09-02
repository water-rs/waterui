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
use waterui_text::text;

use crate::layout::{Emphasis, Label};

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
        let view = text(self.label.text).size(match self.label.emphasis {
            // A fragment keyword or subgraph title is drawn a step down from
            // the body text, as Mermaid draws it.
            Emphasis::Title | Emphasis::Muted => self.font_size * 0.875,
            Emphasis::Normal => self.font_size,
        });
        match self.label.emphasis {
            Emphasis::Title => view.bold(),
            Emphasis::Normal | Emphasis::Muted => view,
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }
}
