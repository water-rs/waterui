//! Reusable recordings of constants.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use waterui_core::layout::{Affine2, Rect, Size};

use crate::canvas::geometry::{Shape, lower_rect};
use crate::canvas::names::Names;
use crate::canvas::paint::{Layer, Paint, Sampling, Shadow, StrokeStyle, affine};
use crate::canvas::resource::{Font, GlyphRun, ImageSource};
use crate::draw::{self, Draw as _, Fixed, ShapeData, StaticRecorder, kurbo};

/// One recorded command of a [`Drawing`]: constants, in canvas vocabulary.
#[derive(Debug)]
enum Node {
    Fill {
        shape: ShapeData,
        paint: Paint,
    },
    Stroke {
        shape: ShapeData,
        style: kurbo::Stroke,
        paint: Paint,
    },
    Shadow {
        shape: ShapeData,
        shadow: draw::Shadow,
    },
    Image {
        image: ImageSource,
        dst: kurbo::Rect,
        sampling: Sampling,
    },
    Glyphs {
        run: GlyphRun,
        paint: Paint,
    },
    Draw {
        drawing: Drawing,
        transform: kurbo::Affine,
    },
    Clip {
        shape: ShapeData,
        body: Vec<Self>,
    },
    Transform {
        transform: kurbo::Affine,
        body: Vec<Self>,
    },
    Layer {
        group: draw::Group,
        body: Vec<Self>,
    },
}

#[derive(Debug)]
struct Inner {
    size: Size,
    nodes: Vec<Node>,
    images: Vec<ImageSource>,
    fonts: Vec<Font>,
}

/// A recording of constants that a canvas draws any number of times.
///
/// Recorded once with [`Drawing::record`], on any thread — a large document
/// can be parsed and recorded off the main thread — and cheap to clone:
/// clones share the recording. It holds the images and fonts it draws, not
/// any render target's registration of them, so every target draws it, and a
/// target whose engine is replaced draws it again without the author doing
/// anything.
///
/// A canvas places it with [`Cx::draw`](crate::canvas::Cx::draw), and
/// [`Canvas::from`](crate::canvas::Canvas) shows one as a view of its own
/// size.
#[derive(Clone)]
pub struct Drawing(Arc<Inner>);

impl fmt::Debug for Drawing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Drawing")
            .field("size", &self.0.size)
            .field("commands", &self.0.nodes.len())
            .finish_non_exhaustive()
    }
}

impl PartialEq for Drawing {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Drawing {
    /// Records `body` into a drawing whose content occupies `size` points.
    ///
    /// # Panics
    ///
    /// When `size` is not finite and positive.
    #[must_use]
    pub fn record(size: Size, body: impl FnOnce(&mut DrawingCx)) -> Self {
        assert!(
            size.width.is_finite()
                && size.height.is_finite()
                && size.width > 0.0
                && size.height > 0.0,
            "a Drawing needs a finite, positive size, got {}x{}",
            size.width,
            size.height
        );
        let mut cx = DrawingCx::default();
        body(&mut cx);
        Self(Arc::new(Inner {
            size,
            nodes: cx.nodes,
            images: cx.images,
            fonts: cx.fonts,
        }))
    }

    /// The size the drawing's content occupies, in points.
    #[must_use]
    pub fn size(&self) -> Size {
        self.0.size
    }

    /// Every image the drawing samples, nested drawings included.
    pub(crate) fn images(&self) -> &[ImageSource] {
        &self.0.images
    }

    /// Every font the drawing's glyph runs use, nested drawings included.
    pub(crate) fn fonts(&self) -> &[Font] {
        &self.0.fonts
    }

    /// The identity a target caches the drawing's recorded form by.
    pub(crate) fn key(&self) -> usize {
        Arc::as_ptr(&self.0).addr()
    }

    /// Records the drawing for a target that registered its resources under
    /// `names`, drawing each nested drawing through `nested`. A command whose
    /// resource the target refused is not recorded: the refusal is reported
    /// through [`ImageSource::status`].
    pub(crate) fn lower(
        &self,
        names: &Names,
        nested: &mut dyn FnMut(&Self) -> draw::Picture,
    ) -> draw::Picture {
        draw::Picture::record(|recorder| lower_nodes(&self.0.nodes, recorder, names, nested))
    }
}

fn lower_nodes(
    nodes: &[Node],
    recorder: &mut StaticRecorder,
    names: &Names,
    nested: &mut dyn FnMut(&Drawing) -> draw::Picture,
) {
    for node in nodes {
        match node {
            Node::Fill { shape, paint } => {
                if let Some(paint) = paint.lower(names) {
                    recorder.fill::<ShapeData, draw::Paint>(Fixed(shape.clone()), Fixed(paint));
                }
            }
            Node::Stroke {
                shape,
                style,
                paint,
            } => {
                if let Some(paint) = paint.lower(names) {
                    recorder.stroke::<ShapeData, draw::Paint>(
                        Fixed(shape.clone()),
                        Fixed(style.clone()),
                        Fixed(paint),
                    );
                }
            }
            Node::Shadow { shape, shadow } => {
                recorder.shadow::<ShapeData>(Fixed(shape.clone()), Fixed(*shadow));
            }
            Node::Image {
                image,
                dst,
                sampling,
            } => {
                if let Some(id) = names.image(image) {
                    recorder.image(id, Fixed(*dst), sampling.lower());
                }
            }
            Node::Glyphs { run, paint } => {
                if let (Some(font), Some(paint)) = (names.font(&run.font), paint.lower(names)) {
                    recorder.glyphs::<draw::Paint>(Fixed(run.lower(font)), Fixed(paint));
                }
            }
            Node::Draw { drawing, transform } => {
                let picture = nested(drawing);
                recorder.picture(&picture, Fixed(*transform));
            }
            Node::Clip { shape, body } => {
                recorder.clip::<ShapeData>(Fixed(shape.clone()), |recorder| {
                    lower_nodes(body, recorder, names, nested);
                });
            }
            Node::Transform { transform, body } => {
                recorder.transform(Fixed(*transform), |recorder| {
                    lower_nodes(body, recorder, names, nested);
                });
            }
            Node::Layer { group, body } => {
                recorder.group(Fixed(*group), |recorder| {
                    lower_nodes(body, recorder, names, nested);
                });
            }
        }
    }
}

/// What [`Drawing::record`] draws into: the canvas commands, taking
/// constants.
#[derive(Debug, Default)]
pub struct DrawingCx {
    nodes: Vec<Node>,
    images: Vec<ImageSource>,
    fonts: Vec<Font>,
}

impl DrawingCx {
    fn note_paint(&mut self, paint: &Paint) {
        if let Some(image) = paint.image() {
            self.note_image(image);
        }
    }

    fn note_image(&mut self, image: &ImageSource) {
        if !self.images.contains(image) {
            self.images.push(image.clone());
        }
    }

    fn note_font(&mut self, font: &Font) {
        if !self.fonts.contains(font) {
            self.fonts.push(font.clone());
        }
    }

    fn scoped(&mut self, body: impl FnOnce(&mut Self)) -> Vec<Node> {
        let outer = core::mem::take(&mut self.nodes);
        body(self);
        core::mem::replace(&mut self.nodes, outer)
    }

    /// Fills `shape` with `paint`.
    pub fn fill(&mut self, shape: impl Shape, paint: impl Into<Paint>) {
        let paint = paint.into();
        self.note_paint(&paint);
        self.nodes.push(Node::Fill {
            shape: shape.lower(),
            paint,
        });
    }

    /// Strokes `shape`'s outline in `style` with `paint`.
    pub fn stroke(&mut self, shape: impl Shape, style: &StrokeStyle, paint: impl Into<Paint>) {
        let paint = paint.into();
        self.note_paint(&paint);
        self.nodes.push(Node::Stroke {
            shape: shape.lower(),
            style: style.lower(),
            paint,
        });
    }

    /// Casts `shadow` from `shape`.
    pub fn shadow(&mut self, shape: impl Shape, shadow: Shadow) {
        self.nodes.push(Node::Shadow {
            shape: shape.lower(),
            shadow: shadow.lower(),
        });
    }

    /// Draws `image` scaled into `rect`.
    pub fn image(&mut self, image: &ImageSource, rect: Rect, sampling: Sampling) {
        self.note_image(image);
        self.nodes.push(Node::Image {
            image: image.clone(),
            dst: lower_rect(rect),
            sampling,
        });
    }

    /// Draws `run`'s glyphs, exactly as positioned, with `paint`.
    pub fn glyphs(&mut self, run: &GlyphRun, paint: impl Into<Paint>) {
        let paint = paint.into();
        self.note_paint(&paint);
        self.note_font(&run.font);
        self.nodes.push(Node::Glyphs {
            run: run.clone(),
            paint,
        });
    }

    /// Draws another drawing under `transform`.
    pub fn draw(&mut self, drawing: &Drawing, transform: Affine2) {
        for image in drawing.images() {
            self.note_image(image);
        }
        for font in drawing.fonts() {
            self.note_font(font);
        }
        self.nodes.push(Node::Draw {
            drawing: drawing.clone(),
            transform: affine(transform),
        });
    }

    /// Draws `body` clipped to `shape`.
    pub fn clip(&mut self, shape: impl Shape, body: impl FnOnce(&mut Self)) {
        let shape = shape.lower();
        let body = self.scoped(body);
        self.nodes.push(Node::Clip { shape, body });
    }

    /// Draws `body` under `transform`.
    pub fn transform(&mut self, transform: Affine2, body: impl FnOnce(&mut Self)) {
        let body = self.scoped(body);
        self.nodes.push(Node::Transform {
            transform: affine(transform),
            body,
        });
    }

    /// Draws `body` isolated in `layer`, which composites it as one image.
    pub fn layer(&mut self, layer: Layer, body: impl FnOnce(&mut Self)) {
        let body = self.scoped(body);
        self.nodes.push(Node::Layer {
            group: layer.lower(),
            body,
        });
    }
}
