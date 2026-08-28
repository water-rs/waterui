use alloc::boxed::Box;
use alloc::vec::Vec;

use kurbo::{Affine, BezPath, Stroke};
use peniko::{BlendMode, Brush, Fill, FontData, ImageBrush, Style, StyleRef};

/// One positioned glyph within a run.
///
/// Offsets are relative to the run's transform, as they come out of shaping.
#[derive(Debug, Clone, Copy)]
pub struct Glyph {
    /// Glyph identifier within its font.
    pub id: u32,
    /// X offset within the run.
    pub x: f32,
    /// Y offset within the run.
    pub y: f32,
}

/// A run of glyphs from one font that share every drawing attribute.
///
/// Text is the one thing a scene draws that is not a path, so it is the one
/// thing that cannot be expressed through [`Scene2D::fill`]: each engine turns
/// glyph ids into outlines its own way, with its own caches.
#[derive(Debug)]
pub struct GlyphRun<'a> {
    /// The font these glyphs are indexed in.
    pub font: &'a FontData,
    /// Em size in pixels.
    pub font_size: f32,
    /// Variable-font axis positions, normalized, as shaping produced them.
    pub normalized_coords: &'a [i16],
    /// Transform applied to the whole run.
    pub transform: Affine,
    /// Paint for the glyphs, and the "foreground colour" for colour fonts.
    pub brush: &'a Brush,
    /// Extra alpha multiplier applied to `brush`.
    pub brush_alpha: f32,
    /// Whether the glyphs are filled or stroked.
    pub style: StyleRef<'a>,
    /// The glyphs, in run order.
    pub glyphs: &'a [Glyph],
}

/// Rendering-engine-independent 2D scene builder interface.
pub trait Scene2D {
    /// Fills a shape with the given brush and transform.
    ///
    /// `brush_transform` is applied to the brush on top of `transform`, which is
    /// what positions a gradient independently of the shape it paints.
    fn fill(
        &mut self,
        fill: Fill,
        transform: Affine,
        brush: &Brush,
        brush_transform: Option<Affine>,
        shape: &BezPath,
    );

    /// Strokes a shape with the given brush and transform.
    ///
    /// `brush_transform` is applied to the brush on top of `transform`.
    fn stroke(
        &mut self,
        stroke: &Stroke,
        transform: Affine,
        brush: &Brush,
        brush_transform: Option<Affine>,
        shape: &BezPath,
    );

    /// Pushes a clipped compositing layer.
    fn push_layer(
        &mut self,
        fill: Fill,
        blend: BlendMode,
        alpha: f32,
        transform: Affine,
        clip: &BezPath,
    );

    /// Pushes a clip-only layer.
    fn push_clip_layer(&mut self, fill: Fill, transform: Affine, clip: &BezPath);

    /// Pops the current layer.
    fn pop_layer(&mut self);

    /// Draws an image with transform.
    fn draw_image(&mut self, image: &ImageBrush, transform: Affine);

    /// Draws a run of glyphs.
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_>);

    /// Clears all recorded scene commands.
    fn reset(&mut self);
}

/// A glyph run with every borrow it was drawn from turned into an owned value,
/// so it survives the call that recorded it.
#[derive(Debug)]
struct RecordedGlyphRun {
    font: FontData,
    font_size: f32,
    normalized_coords: Vec<i16>,
    transform: Affine,
    brush: Brush,
    brush_alpha: f32,
    style: Style,
    glyphs: Vec<Glyph>,
}

/// One recorded drawing command, in the same terms [`Scene2D`] takes them.
#[derive(Debug)]
enum SceneCommand {
    Fill {
        fill: Fill,
        transform: Affine,
        brush: Brush,
        brush_transform: Option<Affine>,
        shape: BezPath,
    },
    Stroke {
        stroke: Stroke,
        transform: Affine,
        brush: Brush,
        brush_transform: Option<Affine>,
        shape: BezPath,
    },
    PushLayer {
        fill: Fill,
        blend: BlendMode,
        alpha: f32,
        transform: Affine,
        clip: BezPath,
    },
    PushClipLayer {
        fill: Fill,
        transform: Affine,
        clip: BezPath,
    },
    PopLayer,
    DrawImage {
        image: ImageBrush,
        transform: Affine,
    },
    DrawGlyphRun(Box<RecordedGlyphRun>),
}

/// An engine-neutral recording of scene commands.
///
/// A recording is itself a [`Scene2D`], so anything that draws through the
/// contract can draw into one, and [`SceneRecording::replay`] plays it back into
/// any other scene with an optional transform applied on top of every command.
/// That is what lets a display list be built once and reused across frames
/// without binding the cache — or the code that filled it — to one rendering
/// engine.
///
/// Replay applies `transform` to each command's own transform, exactly as a
/// scene appended into another one would be positioned. A command's
/// `brush_transform` is relative to its transform and is replayed untouched.
#[derive(Debug, Default)]
pub struct SceneRecording {
    commands: Vec<SceneCommand>,
}

impl SceneRecording {
    /// Creates an empty recording.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    /// Returns the number of recorded commands.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.commands.len()
    }

    /// Returns whether nothing has been recorded.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Replays every recorded command into `target`, positioned by `transform`.
    pub fn replay(&self, target: &mut dyn Scene2D, transform: Option<Affine>) {
        let base = transform.unwrap_or(Affine::IDENTITY);
        for command in &self.commands {
            match command {
                SceneCommand::Fill {
                    fill,
                    transform,
                    brush,
                    brush_transform,
                    shape,
                } => target.fill(*fill, base * *transform, brush, *brush_transform, shape),
                SceneCommand::Stroke {
                    stroke,
                    transform,
                    brush,
                    brush_transform,
                    shape,
                } => target.stroke(stroke, base * *transform, brush, *brush_transform, shape),
                SceneCommand::PushLayer {
                    fill,
                    blend,
                    alpha,
                    transform,
                    clip,
                } => target.push_layer(*fill, *blend, *alpha, base * *transform, clip),
                SceneCommand::PushClipLayer {
                    fill,
                    transform,
                    clip,
                } => target.push_clip_layer(*fill, base * *transform, clip),
                SceneCommand::PopLayer => target.pop_layer(),
                SceneCommand::DrawImage { image, transform } => {
                    target.draw_image(image, base * *transform);
                }
                SceneCommand::DrawGlyphRun(run) => target.draw_glyph_run(&GlyphRun {
                    font: &run.font,
                    font_size: run.font_size,
                    normalized_coords: &run.normalized_coords,
                    transform: base * run.transform,
                    brush: &run.brush,
                    brush_alpha: run.brush_alpha,
                    style: (&run.style).into(),
                    glyphs: &run.glyphs,
                }),
            }
        }
    }
}

impl Scene2D for SceneRecording {
    fn fill(
        &mut self,
        fill: Fill,
        transform: Affine,
        brush: &Brush,
        brush_transform: Option<Affine>,
        shape: &BezPath,
    ) {
        self.commands.push(SceneCommand::Fill {
            fill,
            transform,
            brush: brush.clone(),
            brush_transform,
            shape: shape.clone(),
        });
    }

    fn stroke(
        &mut self,
        stroke: &Stroke,
        transform: Affine,
        brush: &Brush,
        brush_transform: Option<Affine>,
        shape: &BezPath,
    ) {
        self.commands.push(SceneCommand::Stroke {
            stroke: stroke.clone(),
            transform,
            brush: brush.clone(),
            brush_transform,
            shape: shape.clone(),
        });
    }

    fn push_layer(
        &mut self,
        fill: Fill,
        blend: BlendMode,
        alpha: f32,
        transform: Affine,
        clip: &BezPath,
    ) {
        self.commands.push(SceneCommand::PushLayer {
            fill,
            blend,
            alpha,
            transform,
            clip: clip.clone(),
        });
    }

    fn push_clip_layer(&mut self, fill: Fill, transform: Affine, clip: &BezPath) {
        self.commands.push(SceneCommand::PushClipLayer {
            fill,
            transform,
            clip: clip.clone(),
        });
    }

    fn pop_layer(&mut self) {
        self.commands.push(SceneCommand::PopLayer);
    }

    fn draw_image(&mut self, image: &ImageBrush, transform: Affine) {
        self.commands.push(SceneCommand::DrawImage {
            image: image.clone(),
            transform,
        });
    }

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_>) {
        self.commands
            .push(SceneCommand::DrawGlyphRun(Box::new(RecordedGlyphRun {
                font: run.font.clone(),
                font_size: run.font_size,
                normalized_coords: run.normalized_coords.to_vec(),
                transform: run.transform,
                brush: run.brush.clone(),
                brush_alpha: run.brush_alpha,
                style: run.style.to_owned(),
                glyphs: run.glyphs.to_vec(),
            })));
    }

    fn reset(&mut self) {
        self.commands.clear();
    }
}
