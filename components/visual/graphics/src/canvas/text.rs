//! Semantic text a canvas draws.
//!
//! A canvas names the text and where it is anchored; the render target lays
//! it out and draws it with its own text stack, so it matches every other
//! `Text` on the screen. The canvas never measures: text whose box must be
//! known before layout stays a `Text` view placed over the canvas.

use alloc::rc::Rc;
use core::any::Any;
use core::fmt;

use nami::impl_constant;
use waterui_core::layout::{HorizontalAlignment, Point};

/// Styled text a canvas draws, in the form the crate that owns styled text
/// describes it.
///
/// `waterui-graphics` sits below the text crate, so it carries the text
/// without naming its type: the text crate builds one from its styled
/// string, and the render target that lays it out reads that string back
/// with [`get`](Self::get).
#[derive(Clone)]
pub struct CanvasText(Rc<dyn Any>);

impl fmt::Debug for CanvasText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CanvasText(..)")
    }
}

impl PartialEq for CanvasText {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl CanvasText {
    /// Text described by `text`, the owning crate's styled string.
    #[must_use]
    pub fn new<T: 'static>(text: T) -> Self {
        Self(Rc::new(text))
    }

    /// The text as the owning crate's `T`, or `None` when it was made from
    /// another type.
    #[must_use]
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }
}

/// Where a canvas anchors its text, and how it breaks and aligns it.
///
/// The placement's horizontal alignment decides both how its lines align inside
/// the text box and which edge of that box sits at [`origin`](Self::origin):
/// the leading edge, the centre or the trailing edge. The box is
/// [`width`](Self::width) wide, or as wide as its longest line when there is
/// none, and its top is at the origin's `y`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextPlacement {
    origin: Point,
    alignment: HorizontalAlignment,
    width: Option<f32>,
}

impl TextPlacement {
    /// Text anchored at `origin`, leading-aligned and unwrapped.
    #[must_use]
    pub const fn at(origin: Point) -> Self {
        Self {
            origin,
            alignment: HorizontalAlignment::Leading,
            width: None,
        }
    }

    /// Aligns the lines, and anchors the box's matching edge.
    #[must_use]
    pub const fn aligned(mut self, alignment: HorizontalAlignment) -> Self {
        self.alignment = alignment;
        self
    }

    /// Breaks lines at `width` points.
    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// The anchor, in canvas space.
    #[must_use]
    pub const fn origin(&self) -> Point {
        self.origin
    }

    /// The alignment of the lines and of the anchored edge.
    #[must_use]
    pub const fn alignment(&self) -> HorizontalAlignment {
        self.alignment
    }

    /// The width lines break at, if any.
    #[must_use]
    pub const fn get_width(&self) -> Option<f32> {
        self.width
    }

    /// Where the top-left corner of a text box `box_width` wide lands: the
    /// origin, moved left by the share of the box its alignment anchors.
    pub(crate) fn corner(&self, box_width: f32) -> Point {
        let shift = if self.alignment == HorizontalAlignment::Center {
            box_width / 2.0
        } else if self.alignment == HorizontalAlignment::Trailing {
            box_width
        } else {
            0.0
        };
        Point::new(self.origin.x - shift, self.origin.y)
    }

    /// Whether `other` lays the same text out the same way: only the
    /// origin may differ, which moves a laid-out text without laying it
    /// out again.
    pub(crate) fn same_layout(&self, other: &Self) -> bool {
        self.alignment == other.alignment && self.width == other.width
    }
}

impl_constant!(CanvasText, TextPlacement);
