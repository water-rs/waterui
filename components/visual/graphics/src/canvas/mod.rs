//! Canvas: target-neutral vector drawing.
//!
//! A [`Canvas`] is a view whose content is drawn by commands — fills,
//! strokes, shadows, images, glyph runs and reusable [`Drawing`]s, under
//! clips, transforms and compositing layers. Whichever renderer draws the
//! screen around it draws it, in the same retained recording.
//!
//! ```
//! use waterui_core::layout::{Point, Rect, Size};
//! use waterui_graphics::canvas::{Canvas, Circle, Layer, StrokeStyle, Cap};
//! use waterui_graphics::color::WorkingColor;
//! use waterui_core::reactive::{SignalExt as _, binding};
//!
//! let progress = binding::<f32>(0.25);
//! let canvas = Canvas::new(move |cx| {
//!     let size = cx.size();
//!     cx.fill(size.map(Rect::from_size), WorkingColor::BLACK);
//!     // A live operand: moving the binding updates this one circle.
//!     cx.fill(
//!         progress.map(|t| Circle::new(Point::new(20.0 + 80.0 * t, 20.0), 8.0)),
//!         WorkingColor::WHITE,
//!     );
//!     cx.layer(Layer::opacity(0.5), |cx| {
//!         cx.stroke(
//!             Circle::new(Point::new(60.0, 60.0), 30.0),
//!             StrokeStyle::new(4.0).cap(Cap::Round),
//!             WorkingColor::WHITE,
//!         );
//!     });
//! })
//! .intrinsic_size(Size::new(120.0, 120.0));
//! # let _ = canvas;
//! ```
//!
//! # The closure runs once
//!
//! [`Canvas::new`] runs its closure once per mounted instance, and what it
//! records lives until the canvas unmounts. Every operand takes a value or a
//! signal; a signal stays bound, so its change updates that one operand in
//! place, animated when the change carries an animation. Structure that
//! depends on a value re-records locally, through [`Cx::each`] and
//! [`Cx::scope`]; nothing else ever records again.
//!
//! # Resources are semantic
//!
//! Images ([`ImageSource`]) and fonts ([`Font`]) are described, never
//! registered: the target that draws the canvas registers each on first
//! use, holds it while the canvas draws it, and registers it again after
//! its engine is replaced. What it made of an image is the image's
//! [`status`](ImageSource::status).

mod cell;
mod cx;
mod drawing;
mod geometry;
mod names;
mod operand;
mod paint;
mod resource;
mod scene;
mod view;

use alloc::boxed::Box;
use core::fmt;

use nami::{Computed, SignalExt as _};
use waterui_core::layout::{Affine2, Size};
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Environment, Str, View};

use cell::SignalCell;

#[cfg(any(feature = "gpu", feature = "cpu"))]
pub(crate) use scene::lower_drawing;

pub use cx::Cx;
pub use drawing::{Drawing, DrawingCx};
pub use geometry::{Circle, Ellipse, FillRule, Line, Path, PathBuilder, RoundedRect, Shape};
pub use operand::Operand;
pub use paint::{
    BlendMode, BlendSpace, Cap, ColorStop, ConicGradient, Extend, ImagePattern, Interpolation,
    IntoPaint, Join, Layer, LinearGradient, MeshGradient, Paint, RadialGradient, Sampling, Shadow,
    StrokeStyle, TransformedPaint,
};
pub use resource::{
    Font, Glyph, GlyphRun, GlyphStyle, ImageColorSpace, ImageSource, ImageStatus, PixelBuffer,
    PixelFormat,
};

pub use view::CanvasView;

/// A view drawn by vector commands.
///
/// See the [module documentation](self) for the model. A canvas with no
/// [`intrinsic_size`](Self::intrinsic_size) stretches on both axes and
/// draws into whatever box layout gives it, which [`Cx::size`] reports.
pub struct Canvas {
    body: Box<dyn FnOnce(&mut Cx)>,
    intrinsic: Computed<Option<Size>>,
    label: Computed<Option<Str>>,
    value: Computed<Option<Str>>,
    /// The box in points, set by the mounted canvas as layout moves it.
    size: SignalCell<Size>,
}

impl fmt::Debug for Canvas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Canvas").finish_non_exhaustive()
    }
}

impl Canvas {
    /// A canvas drawn by `body`, which runs once per mounted instance.
    pub fn new(body: impl FnOnce(&mut Cx) + 'static) -> Self {
        Self {
            body: Box::new(body),
            intrinsic: Computed::constant(None),
            label: Computed::constant(None),
            value: Computed::constant(None),
            size: SignalCell::new(Size::zero()),
        }
    }

    /// The canvas's box, in points: a signal that follows layout.
    ///
    /// It is the signal [`Cx::size`] returns inside the drawing closure, and
    /// it exists before the canvas is built and mounted, so surrounding views
    /// can read it. Until the first layout pass gives the canvas a box it
    /// reads [`Size::zero`]; the drawing closure runs at mount, before that
    /// first pass, and sees the same zero size until the signal updates.
    #[must_use]
    pub fn size(&self) -> Computed<Size> {
        Computed::new(self.size.clone())
    }

    /// The size the drawing naturally is, in points: an image's pixel size,
    /// a formula's typeset box. Layout measures again when it changes. With
    /// a size the canvas is content-sized and keeps its aspect ratio when a
    /// container names only one axis; with `None`, the default, it stretches
    /// on both axes.
    ///
    /// # Panics
    ///
    /// Layout panics on a size that is not finite and positive on both axes.
    #[must_use]
    pub fn intrinsic_size(mut self, size: impl IntoComputed<Option<Size>>) -> Self {
        self.intrinsic = size.into_computed();
        self
    }

    /// The accessible name the drawing offers — an SVG's title, a chart's
    /// caption. An app's own `.a11y_label(…)` overrides it.
    #[must_use]
    pub fn offered_label(mut self, label: impl IntoComputed<Option<Str>>) -> Self {
        self.label = label.into_computed();
        self
    }

    /// The accessible value the drawing offers — a formula's spoken
    /// mathematics. An app's own `.a11y_value(…)` overrides it.
    #[must_use]
    pub fn offered_value(mut self, value: impl IntoComputed<Option<Str>>) -> Self {
        self.value = value.into_computed();
        self
    }
}

impl From<Drawing> for Canvas {
    /// The drawing as a view of its own size, scaled to fill whatever box
    /// layout gives it.
    fn from(drawing: Drawing) -> Self {
        let natural = drawing.size();
        Self::new(move |cx| {
            let scale = cx.size().map(move |size: Size| {
                Affine2::scale(size.width / natural.width, size.height / natural.height)
            });
            cx.draw(&drawing, scale);
        })
        .intrinsic_size(Computed::constant(Some(natural)))
    }
}

impl View for Canvas {
    fn body(self, env: &Environment) -> impl View {
        CanvasView::mount(self, env)
    }
}

#[cfg(test)]
mod tests;
