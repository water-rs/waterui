//! Mermaid diagrams, drawn by `WaterUI`.
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui_mermaid::mermaid;
//!
//! # fn diagram() -> impl View {
//! mermaid(
//!     "flowchart TD\n  A[Start] --> B{Ready?}\n  B -->|yes| C[Go]\n  B -->|no| A",
//! )
//! # }
//! ```
//!
//! # How a diagram gets on screen
//!
//! Mermaid source is parsed and laid out by [`merman`](https://github.com/Latias94/merman),
//! which tracks upstream Mermaid's own grammar and geometry. Everything after
//! that is this crate's:
//!
//! - **Layout is measured with `WaterUI`'s text engine.** `merman` sizes every
//!   node and every label through a host-supplied measurer, and this crate
//!   supplies one backed by the same `parley` engine and the same system fonts
//!   that draw the labels. Without it, boxes would be sized from a browser
//!   compatibility profile and the glyphs inside them drawn from ours, and the
//!   text would not fit.
//! - **Geometry is drawn through `Scene2D`.** Node outlines, subgraph frames and
//!   routed connectors are vector paths on the shared scene contract, so one
//!   drawing path serves every backend.
//! - **Text is not drawn into the scene.** Labels are real `text()` views placed
//!   into the boxes layout reserved for them, which is what gives a diagram a
//!   meaningful accessibility tree and the platform's own text rendering.
//! - **Colours come from theme tokens.** A diagram follows a light/dark switch
//!   and a custom accent because it reads the same tokens every other component
//!   reads, never Mermaid's CSS themes.
//!
//! A diagram is drawn at its natural size. Scaling it to fit would break the
//! agreement between a reserved box and the glyphs in it, so a diagram larger
//! than its container is the container's to scroll.

#![doc(html_logo_url = "https://raw.githubusercontent.com/water-rs/waterui/main/assets/logo.svg")]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;

use nami::SignalExt as _;
use waterui_canvas::Canvas;
use waterui_core::layout::{Layout, Point, ProposalSize, Rect, Size, StretchAxis, SubView};
use waterui_core::{AnyView, Environment, View, resolve::Resolvable as _};
use waterui_layout::container::FixedContainer;
use waterui_str::Str;
use waterui_text::text;

mod draw;
mod engine;
mod label;
mod layout;
mod measure;
mod shape;
mod theme;

pub use engine::MermaidError;
pub use layout::{
    Cluster, DiagramLayout, Edge, EdgeMarker, EdgeStroke, Emphasis, Fragment, Label, Lifeline,
    Node, NodeShape, UnsupportedShape,
};
pub use theme::{DiagramPalette, Palette};

/// A Mermaid diagram.
///
/// See the [crate documentation](crate) for what the rendering pipeline does
/// and does not do.
#[derive(Debug, Clone)]
pub struct Mermaid {
    source: Str,
}

impl Mermaid {
    /// Creates a diagram from Mermaid source.
    ///
    /// The source carries its own diagram type in its first line, so a
    /// `flowchart` and a `sequenceDiagram` are both just source here.
    #[must_use]
    pub fn new(source: impl Into<Str>) -> Self {
        Self {
            source: source.into(),
        }
    }
}

/// Convenience constructor for [`Mermaid`]. Equivalent to [`Mermaid::new`].
#[must_use]
pub fn mermaid(source: impl Into<Str>) -> Mermaid {
    Mermaid::new(source)
}

impl View for Mermaid {
    fn body(self, env: &Environment) -> impl View {
        match engine::render(&self.source) {
            Ok(diagram) => AnyView::new(drawn(&diagram, env)),
            Err(error) => AnyView::new(undrawable(&error)),
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }
}

/// The scene and the labels of a diagram that laid out successfully.
fn drawn(diagram: &DiagramLayout, env: &Environment) -> impl View + use<> {
    let palette = DiagramPalette.resolve(env).computed();
    let scene = {
        let diagram = diagram.clone();
        Canvas::with_signal(palette, move |ctx, palette| {
            draw::diagram(ctx, &diagram, &palette, Point::zero());
        })
    };

    let mut cells: Vec<AnyView> = Vec::with_capacity(diagram.nodes.len() + 1);
    cells.push(AnyView::new(scene));
    cells.extend(diagram.labels().map(|label| {
        AnyView::new(FixedContainer::new(
            label::Placement::new(label.frame),
            (label::LabelView::new(label.clone(), diagram.font_size),),
        ))
    }));

    FixedContainer::new(Placement::new(diagram), cells)
}

/// What is shown when a diagram cannot be drawn.
///
/// Mermaid itself renders a broken diagram as a visible error, and so does this:
/// a fence that silently disappears is a worse outcome than one that says what
/// is wrong with it.
fn undrawable(error: &MermaidError) -> impl View + use<> {
    text(Str::from(format!("Mermaid: {error}")))
}

/// Places a diagram's scene and its labels.
///
/// The scene fills the diagram's natural size, and each label sits in the box
/// the diagram reserved for it. Both are positioned in the same coordinates the
/// geometry was laid out in, so nothing has to be scaled or corrected.
#[derive(Debug)]
struct Placement {
    size: Size,
    labels: Vec<Rect>,
}

impl Placement {
    fn new(diagram: &DiagramLayout) -> Self {
        Self {
            size: diagram.size,
            labels: diagram.labels().map(|label| label.frame).collect(),
        }
    }
}

impl Layout for Placement {
    fn size_that_fits(&self, _proposal: ProposalSize, _children: &[&dyn SubView]) -> Size {
        self.size
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        let origin = bounds.origin();
        let mut frames = Vec::with_capacity(children.len());
        // The scene, covering the whole diagram.
        frames.push(Rect::new(origin, self.size));
        frames.extend(self.labels.iter().map(|frame| {
            Rect::new(
                Point::new(frame.x() + origin.x, frame.y() + origin.y),
                *frame.size(),
            )
        }));
        frames
    }

    fn stretch_axis(&self, _children: &[StretchAxis]) -> StretchAxis {
        StretchAxis::None
    }
}
