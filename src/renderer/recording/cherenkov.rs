//! The Cherenkov implementation of the recording boundary (water-rs/hydrolysis#205, H1).
//!
//! [`Recording`] keeps the fixed imperative API drawing code records through;
//! internally it is an ordered op list. The flush lowers each contiguous run
//! into a [`cherenkov::Content`] the mounted engine layer shows, so engine
//! mounts and hierarchy persist across frames while only the content payload
//! is replaced.
//!
//! Ops name peniko types because that is the currency the call sites already
//! produce; conversion into engine resources — fonts and images — happens at
//! lower time through [`SceneResources`], which keeps the handles alive and
//! deduplicates registrations across frames.

use crate::engine::{Brush, DrawContext};
use core::fmt;
use kurbo::{
    Affine, BezPath, Circle, Line, Point, Rect, RoundedRect, RoundedRectRadii, Shape, Vec2,
};
use peniko::{BlendMode, Fill, FontData, ImageBrush};
use rustc_hash::FxHashMap;
use std::sync::Arc;
use waterui_graphics::{GlyphRun, Scene2D};

use cherenkov::{
    BlendSpace, ColorStop, Content, Draw, EvenOdd, Extend, Glyph, GlyphStyle, Group,
    ImageColorSpace, ImageData, ImageId, ImagePattern, Interpolation, LinearGradient, Paint,
    RadialGradient, Recorder, Sampling, Shadow, ShapeData, SweepGradient, WorkingColor,
};

/// The opaque recording object drawing code builds. Internally an ordered op
/// list lowered into a Cherenkov [`Content`] at flush; nothing about the ops
/// reaches the drawing-facing API.
#[derive(Clone, Default)]
pub struct Recording {
    ops: Vec<Op>,
    /// Push scopes still open — the tracked-stack invariant the flush asserts.
    open_layers: u32,
}

#[derive(Clone)]
enum Op {
    Fill {
        rule: Fill,
        transform: Affine,
        brush: peniko::Brush,
        brush_transform: Option<Affine>,
        shape: ShapeData,
    },
    Stroke {
        stroke: kurbo::Stroke,
        transform: Affine,
        brush: peniko::Brush,
        brush_transform: Option<Affine>,
        shape: ShapeData,
    },
    Image {
        image: ImageBrush,
        transform: Affine,
    },
    Glyphs {
        font: FontData,
        font_size: f32,
        coords: Arc<[i16]>,
        transform: Affine,
        brush: peniko::Brush,
        brush_alpha: f32,
        style: peniko::Style,
        glyphs: Arc<[waterui_graphics::Glyph]>,
    },
    BlurredRoundedRect {
        transform: Affine,
        rect: Rect,
        color: peniko::Color,
        radius: f64,
        sigma: f64,
    },
    PushClip {
        rule: Fill,
        transform: Affine,
        clip: ShapeData,
    },
    PushGroup {
        rule: Fill,
        blend: BlendMode,
        opacity: f32,
        transform: Affine,
        clip: ShapeData,
    },
    Picture {
        transform: Affine,
        picture: cherenkov::Picture,
    },
    PopLayer,
}

/// Engine-facing caches shared by every lowered scene: registered fonts and
/// images, keyed by the identity of the peniko resource they came from.
///
/// Registration itself goes through the one `Engine` the host owns; the
/// handles live here for as long as a frame can show the recording that names
/// them.
pub(crate) struct SceneResources {
    engine: std::rc::Rc<crate::engine::GpuEngine>,
    fonts: FxHashMap<FontKey, cherenkov::Font>,
    images: FxHashMap<ImageKey, cherenkov::Image<cherenkov::Rgba8>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FontKey {
    blob: u64,
    index: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ImageKey {
    blob: u64,
    width: u32,
    height: u32,
    format: u8,
    alpha: u8,
}

impl fmt::Debug for SceneResources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SceneResources")
            .field("fonts", &self.fonts.len())
            .field("images", &self.images.len())
            .finish()
    }
}

impl SceneResources {
    /// Creates caches served by `engine`.
    pub(crate) fn new(engine: std::rc::Rc<crate::engine::GpuEngine>) -> Self {
        Self {
            engine,
            fonts: FxHashMap::default(),
            images: FxHashMap::default(),
        }
    }

    /// The engine these resources belong to.
    pub(crate) fn engine(&self) -> &crate::engine::GpuEngine {
        &self.engine
    }

    /// The registered font id for `font`, uploading the face on first use.
    pub(crate) fn font(&mut self, font: &FontData) -> cherenkov::FontId {
        let key = FontKey {
            blob: font.data.id(),
            index: font.index,
        };
        if let Some(registered) = self.fonts.get(&key) {
            return registered.id();
        }
        let source = cherenkov::FontSource::bytes(Arc::<[u8]>::from(font.data.data()))
            .with_index(font.index);
        let registered = self
            .engine
            .font(source)
            .expect("hydrolysis renderer: engine rejected a font source it must accept");
        let id = registered.id();
        self.fonts.insert(key, registered);
        id
    }

    /// The registered image id for `image`, uploading the pixels on first use.
    pub(crate) fn image(&mut self, image: &peniko::ImageData) -> ImageId {
        let key = ImageKey {
            blob: image.data.id(),
            width: image.width,
            height: image.height,
            format: image.format as u8,
            alpha: image.alpha_type as u8,
        };
        if let Some(registered) = self.images.get(&key) {
            return registered.id();
        }
        assert_eq!(
            image.format,
            peniko::ImageFormat::Rgba8,
            "hydrolysis renderer: unsupported image format {:?}; only Rgba8 uploads reach the engine",
            image.format
        );
        assert!(
            image.width > 0 && image.height > 0,
            "hydrolysis renderer: image with a zero dimension"
        );
        let bytes = image.data.data();
        assert_eq!(
            bytes.len(),
            image.width as usize * image.height as usize * 4,
            "hydrolysis renderer: Rgba8 image {}x{} carries {} bytes",
            image.width,
            image.height,
            bytes.len()
        );
        let data = ImageData::<cherenkov::Rgba8>::new(
            image.width,
            image.height,
            Arc::<[u8]>::from(bytes),
        )
        .expect("hydrolysis renderer: well-formed Rgba8 image rejected")
        .color_space(ImageColorSpace::Srgb);
        let data = if image.alpha_type == peniko::ImageAlphaType::AlphaPremultiplied {
            data.premultiplied()
        } else {
            data
        };
        let registered = self
            .engine
            .image(data)
            .expect("hydrolysis renderer: engine rejected an image it must accept");
        let id = registered.id();
        self.images.insert(key, registered);
        id
    }

    /// The engine paint for `brush`, registering any image it names.
    pub(crate) fn paint(&mut self, brush: &peniko::Brush) -> Paint {
        match brush {
            peniko::Brush::Solid(color) => Paint::Solid(working_color(*color)),
            peniko::Brush::Gradient(gradient) => gradient_paint(gradient),
            peniko::Brush::Image(brush) => {
                let pattern = ImagePattern {
                    image: self.image(&brush.image),
                    transform: Affine::IDENTITY,
                    extend_x: extend(brush.sampler.x_extend),
                    extend_y: extend(brush.sampler.y_extend),
                    sampling: sampling(brush.sampler.quality),
                };
                Paint::Image(pattern)
            }
        }
    }

    /// `brush` mapped through its own coordinate transform.
    pub(crate) fn transformed_paint(
        &mut self,
        brush: &peniko::Brush,
        brush_transform: Option<Affine>,
    ) -> Paint {
        transform_paint(self.paint(brush), brush_transform)
    }
}

impl Recording {
    /// Creates an empty recording.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Clears all recorded commands.
    pub(crate) fn reset(&mut self) {
        self.ops.clear();
        self.open_layers = 0;
    }

    /// Whether the recording encodes any visible content.
    ///
    /// Layer scopes alone emit clip geometry in the lowered content, matching
    /// what the old encoder counted as non-empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Fills `shape` under `transform` with `brush`.
    pub(crate) fn fill<S: Shape + 'static>(
        &mut self,
        rule: Fill,
        transform: Affine,
        brush: &peniko::Brush,
        brush_transform: Option<Affine>,
        shape: &S,
    ) {
        self.ops.push(Op::Fill {
            rule,
            transform,
            brush: brush.clone(),
            brush_transform,
            shape: shape_data(rule, shape),
        });
    }

    /// Strokes `shape` under `transform` with `brush`.
    pub(crate) fn stroke<S: Shape + 'static>(
        &mut self,
        stroke: &kurbo::Stroke,
        transform: Affine,
        brush: &peniko::Brush,
        brush_transform: Option<Affine>,
        shape: &S,
    ) {
        self.ops.push(Op::Stroke {
            stroke: stroke.clone(),
            transform,
            brush: brush.clone(),
            brush_transform,
            shape: shape_data(Fill::NonZero, shape),
        });
    }

    /// Draws an image under `transform`.
    pub(crate) fn image(&mut self, image: &ImageBrush, transform: Affine) {
        self.ops.push(Op::Image {
            image: image.clone(),
            transform,
        });
    }

    /// Draws a run of shaped glyphs.
    pub(crate) fn glyphs(&mut self, run: GlyphRun<'_>) {
        if run.glyphs.is_empty() {
            return;
        }
        self.ops.push(Op::Glyphs {
            font: run.font.clone(),
            font_size: run.font_size,
            coords: Arc::from(run.normalized_coords),
            transform: run.transform,
            brush: run.brush.clone(),
            brush_alpha: run.brush_alpha,
            style: run.style.to_owned(),
            glyphs: Arc::from(run.glyphs),
        });
    }

    /// Draws a blurred rounded rect shadow under `transform`.
    pub(crate) fn blurred_rounded_rect(
        &mut self,
        transform: Affine,
        rect: Rect,
        color: peniko::Color,
        corner_radius: f64,
        sigma: f64,
    ) {
        self.ops.push(Op::BlurredRoundedRect {
            transform,
            rect,
            color,
            radius: corner_radius,
            sigma,
        });
    }

    /// Draws a recorded picture under `transform` — the content of a child
    /// view recorded against the same engine.
    pub(crate) fn draw_picture(&mut self, transform: Affine, picture: cherenkov::Picture) {
        self.ops.push(Op::Picture { transform, picture });
    }

    /// Pushes a clip-only scope.
    pub(crate) fn push_clip<S: Shape + 'static>(&mut self, rule: Fill, transform: Affine, shape: &S) {
        self.open_layers += 1;
        self.ops.push(Op::PushClip {
            rule,
            transform,
            clip: shape_data(rule, shape),
        });
    }

    /// Pushes a compositing scope with blend mode, opacity and clip.
    pub(crate) fn push_group<S: Shape + 'static>(
        &mut self,
        rule: Fill,
        blend: BlendMode,
        opacity: f32,
        clip_transform: Affine,
        clip: &S,
    ) {
        self.open_layers += 1;
        self.ops.push(Op::PushGroup {
            rule,
            blend,
            opacity,
            transform: clip_transform,
            clip: shape_data(rule, clip),
        });
    }

    /// Opens a compositing scope, runs `f` inside it, then closes it — the
    /// lexical pairing recording call sites use when their content is not a
    /// whole renderer traversal (`&mut Recording`, not `&mut Renderer`).
    pub(crate) fn with_group<S: Shape + 'static>(
        &mut self,
        rule: Fill,
        blend: BlendMode,
        opacity: f32,
        clip_transform: Affine,
        clip: &S,
        f: impl FnOnce(&mut Self),
    ) {
        self.push_group(rule, blend, opacity, clip_transform, clip);
        f(self);
        self.pop_scope();
    }

    /// Closes the current scope.
    pub(crate) fn pop_scope(&mut self) {
        assert!(
            self.open_layers > 0,
            "hydrolysis recording: layer pop without a matching push"
        );
        self.open_layers -= 1;
        self.ops.push(Op::PopLayer);
    }

    /// Appends `other` under `placement`.
    pub(crate) fn append(&mut self, other: &Recording, placement: Affine) {
        self.ops.extend(other.ops.iter().cloned().map(|op| match op {
            Op::Fill {
                rule,
                transform,
                brush,
                brush_transform,
                shape,
            } => Op::Fill {
                rule,
                transform: placement * transform,
                brush,
                brush_transform,
                shape,
            },
            Op::Stroke {
                stroke,
                transform,
                brush,
                brush_transform,
                shape,
            } => Op::Stroke {
                stroke,
                transform: placement * transform,
                brush,
                brush_transform,
                shape,
            },
            Op::Image { image, transform } => Op::Image {
                image,
                transform: placement * transform,
            },
            Op::Glyphs {
                font,
                font_size,
                coords,
                transform,
                brush,
                brush_alpha,
                style,
                glyphs,
            } => Op::Glyphs {
                font,
                font_size,
                coords,
                transform: placement * transform,
                brush,
                brush_alpha,
                style,
                glyphs,
            },
            Op::BlurredRoundedRect {
                transform,
                rect,
                color,
                radius,
                sigma,
            } => Op::BlurredRoundedRect {
                transform: placement * transform,
                rect,
                color,
                radius,
                sigma,
            },
            Op::PushClip {
                rule,
                transform,
                clip,
            } => Op::PushClip {
                rule,
                transform: placement * transform,
                clip,
            },
            Op::PushGroup {
                rule,
                blend,
                opacity,
                transform,
                clip,
            } => Op::PushGroup {
                rule,
                blend,
                opacity,
                transform: placement * transform,
                clip,
            },
            Op::Picture { transform, picture } => Op::Picture {
                transform: placement * transform,
                picture,
            },
            Op::PopLayer => Op::PopLayer,
        }));
    }

    /// Push scopes still open, for the tracked-stack invariant the flush
    /// asserts.
    pub(crate) fn open_clip_count(&self) -> u32 {
        self.open_layers
    }

    /// Lowers the recording into engine [`Content`] through `resources`.
    ///
    /// # Panics
    /// Panics when a layer push has no matching pop — the flush keeps the
    /// invariant on every code path it accepts.
    pub(crate) fn to_content(&self, resources: &mut SceneResources) -> Content {
        Content::record(|recorder| {
            let mut index = 0;
            lower(&self.ops, &mut index, recorder, resources);
            assert_eq!(
                index,
                self.ops.len(),
                "hydrolysis recording: layer pop without a matching push"
            );
        })
    }
}

fn lower(ops: &[Op], index: &mut usize, recorder: &mut Recorder, resources: &mut SceneResources) {
    while *index < ops.len() {
        let op = &ops[*index];
        *index += 1;
        match op {
            Op::Fill {
                transform,
                brush,
                brush_transform,
                shape,
                ..
            } => {
                let paint = resources.transformed_paint(brush, *brush_transform);
                recorder.transform(*transform, |r| r.fill(shape.clone(), paint));
            }
            Op::Stroke {
                stroke,
                transform,
                brush,
                brush_transform,
                shape,
            } => {
                let paint = resources.transformed_paint(brush, *brush_transform);
                recorder.transform(*transform, |r| {
                    r.stroke(shape.clone(), stroke.clone(), paint);
                });
            }
            Op::Image { image, transform } => {
                let id = resources.image(&image.image);
                let dst = Rect::new(
                    0.0,
                    0.0,
                    f64::from(image.image.width),
                    f64::from(image.image.height),
                );
                let sampling = sampling(image.sampler.quality);
                let alpha = image.sampler.alpha;
                recorder.transform(*transform, |r| {
                    if alpha >= 1.0 {
                        r.image(id, dst, sampling);
                    } else {
                        let group = Group {
                            opacity: alpha,
                            ..Group::default()
                        };
                        r.group(group, |r| r.image(id, dst, sampling));
                    }
                });
            }
            Op::Glyphs {
                font,
                font_size,
                coords,
                transform,
                brush,
                brush_alpha,
                style,
                glyphs,
            } => {
                let font = resources.font(font);
                let mut paint = resources.paint(brush);
                if *brush_alpha < 1.0 {
                    paint = alpha_scaled_paint(paint, *brush_alpha);
                }
                let style = match style {
                    peniko::Style::Fill(_) => GlyphStyle::Fill,
                    peniko::Style::Stroke(stroke) => GlyphStyle::Stroke(stroke.clone()),
                };
                let run = cherenkov::GlyphRun {
                    font,
                    size: *font_size,
                    coords: coords.to_vec(),
                    glyphs: glyphs
                        .iter()
                        .map(|glyph| Glyph {
                            id: glyph.id,
                            x: glyph.x,
                            y: glyph.y,
                            transform: None,
                        })
                        .collect(),
                    style,
                };
                recorder.transform(*transform, |r| r.glyphs(run, paint));
            }
            Op::BlurredRoundedRect {
                transform,
                rect,
                color,
                radius,
                sigma,
            } => {
                let shape = ShapeData::of(&rect.to_rounded_rect(*radius));
                let shadow = Shadow {
                    sigma: *sigma,
                    offset: Vec2::ZERO,
                    spread: 0.0,
                    color: working_color(*color),
                };
                recorder.transform(*transform, |r| r.shadow(shape, shadow));
            }
            Op::PushClip { transform, clip, .. } => recorder.transform(*transform, |r| {
                r.clip(clip.clone(), |r| lower(ops, index, r, resources));
            }),
            Op::PushGroup {
                transform,
                clip,
                blend,
                opacity,
                ..
            } => {
                let blend = blend_mode(*blend);
                let alpha = *opacity;
                recorder.transform(*transform, |r| {
                    r.clip(clip.clone(), |r| {
                        if alpha >= 1.0 && blend == cherenkov::BlendMode::Normal {
                            lower(ops, index, r, resources);
                        } else {
                            let group = Group {
                                opacity: alpha,
                                blend,
                                blend_space: BlendSpace::Linear,
                                filter: None,
                            };
                            r.group(group, |r| lower(ops, index, r, resources));
                        }
                    });
                });
            }
            Op::Picture { transform, picture } => recorder.picture(picture, *transform),
            Op::PopLayer => return,
        }
    }
}

/// An owned shape carrying `rule`'s winding.
fn shape_data<S: Shape + 'static>(rule: Fill, shape: &S) -> ShapeData {
    match rule {
        Fill::NonZero => ShapeData::of(shape),
        Fill::EvenOdd => {
            ShapeData::of(&EvenOdd(shape.to_path(cherenkov::PATH_TOLERANCE)))
        }
    }
}

/// The working colour of a peniko sRGB colour.
pub(crate) fn working_color(color: peniko::Color) -> WorkingColor {
    WorkingColor::new(color.convert::<cherenkov::LinearDisplayP3>().components)
}

/// The engine paint of a peniko gradient.
fn gradient_paint(gradient: &peniko::Gradient) -> Paint {
    let stops: Vec<ColorStop> = gradient
        .stops
        .iter()
        .map(|stop| ColorStop {
            offset: stop.offset,
            color: dynamic_working_color(stop.color),
        })
        .collect();
    let extend = extend(gradient.extend);
    let interpolation = interpolation(gradient.interpolation_cs);
    match gradient.kind {
        peniko::GradientKind::Linear(position) => Paint::Linear(LinearGradient {
            start: position.start,
            end: position.end,
            stops,
            extend,
            interpolation,
        }),
        peniko::GradientKind::Radial(position) => Paint::Radial(RadialGradient {
            start_center: position.start_center,
            start_radius: f64::from(position.start_radius),
            end_center: position.end_center,
            end_radius: f64::from(position.end_radius),
            stops,
            extend,
            interpolation,
        }),
        peniko::GradientKind::Sweep(position) => Paint::Sweep(SweepGradient {
            center: position.center,
            start_angle: f64::from(position.start_angle),
            end_angle: f64::from(position.end_angle),
            stops,
            extend,
            interpolation,
        }),
    }
}

/// The working colour of a run-time colour space colour.
fn dynamic_working_color(color: peniko::color::DynamicColor) -> WorkingColor {
    let linear = color.convert(peniko::color::ColorSpaceTag::LinearSrgb);
    let alpha = linear.to_alpha_color::<cherenkov::LinearSrgb>();
    WorkingColor::new(alpha.convert::<cherenkov::LinearDisplayP3>().components)
}

/// The gradient interpolation space the engine supports; any other space is
/// a programmer error on the waterui side, not something to approximate.
fn interpolation(cs: peniko::color::ColorSpaceTag) -> Interpolation {
    match cs {
        peniko::color::ColorSpaceTag::Srgb => Interpolation::SrgbEncoded,
        peniko::color::ColorSpaceTag::LinearSrgb => Interpolation::Working,
        other => panic!(
            "hydrolysis renderer: gradient interpolation space {other:?} has no engine mapping"
        ),
    }
}

fn extend(extend: peniko::Extend) -> Extend {
    match extend {
        peniko::Extend::Pad => Extend::Pad,
        peniko::Extend::Repeat => Extend::Repeat,
        peniko::Extend::Reflect => Extend::Reflect,
    }
}

fn sampling(quality: peniko::ImageQuality) -> Sampling {
    match quality {
        peniko::ImageQuality::Low => Sampling::Nearest,
        peniko::ImageQuality::Medium | peniko::ImageQuality::High => Sampling::Linear,
    }
}

/// The engine blend mode of a peniko `mix`/`compose` pair. The engine carries
/// one combined mode; peniko's separable pair collapses where Porter-Duff and
/// CSS compositing overlap, and the remaining combinations have no engine
/// equivalent.
fn blend_mode(mode: BlendMode) -> cherenkov::BlendMode {
    use peniko::{Compose, Mix};
    match (mode.mix, mode.compose) {
        (Mix::Normal, Compose::SrcOver) => cherenkov::BlendMode::Normal,
        (Mix::Multiply, Compose::SrcOver) => cherenkov::BlendMode::Multiply,
        (Mix::Screen, Compose::SrcOver) => cherenkov::BlendMode::Screen,
        (Mix::Overlay, Compose::SrcOver) => cherenkov::BlendMode::Overlay,
        (Mix::Darken, Compose::SrcOver) => cherenkov::BlendMode::Darken,
        (Mix::Lighten, Compose::SrcOver) => cherenkov::BlendMode::Lighten,
        (Mix::ColorDodge, Compose::SrcOver) => cherenkov::BlendMode::ColorDodge,
        (Mix::ColorBurn, Compose::SrcOver) => cherenkov::BlendMode::ColorBurn,
        (Mix::HardLight, Compose::SrcOver) => cherenkov::BlendMode::HardLight,
        (Mix::SoftLight, Compose::SrcOver) => cherenkov::BlendMode::SoftLight,
        (Mix::Difference, Compose::SrcOver) => cherenkov::BlendMode::Difference,
        (Mix::Exclusion, Compose::SrcOver) => cherenkov::BlendMode::Exclusion,
        (Mix::Hue, Compose::SrcOver) => cherenkov::BlendMode::Hue,
        (Mix::Saturation, Compose::SrcOver) => cherenkov::BlendMode::Saturation,
        (Mix::Color, Compose::SrcOver) => cherenkov::BlendMode::Color,
        (Mix::Luminosity, Compose::SrcOver) => cherenkov::BlendMode::Luminosity,
        (Mix::Normal, Compose::Clear) => cherenkov::BlendMode::Clear,
        (Mix::Normal, Compose::Copy) => cherenkov::BlendMode::Src,
        (Mix::Normal, Compose::Dest) => cherenkov::BlendMode::Dst,
        (Mix::Normal, Compose::DestOver) => cherenkov::BlendMode::DestOver,
        (Mix::Normal, Compose::SrcIn) => cherenkov::BlendMode::SrcIn,
        (Mix::Normal, Compose::DestIn) => cherenkov::BlendMode::DestIn,
        (Mix::Normal, Compose::SrcOut) => cherenkov::BlendMode::SrcOut,
        (Mix::Normal, Compose::DestOut) => cherenkov::BlendMode::DestOut,
        (Mix::Normal, Compose::SrcAtop) => cherenkov::BlendMode::SrcAtop,
        (Mix::Normal, Compose::DestAtop) => cherenkov::BlendMode::DestAtop,
        (Mix::Normal, Compose::Xor) => cherenkov::BlendMode::Xor,
        (Mix::Normal, Compose::PlusLighter) => cherenkov::BlendMode::PlusLighter,
        (mix, compose) => panic!(
            "hydrolysis renderer: blend mode ({mix:?}, {compose:?}) has no engine equivalent"
        ),
    }
}

/// Maps a paint's own coordinates through `transform`. Solid colours carry no
/// space; image patterns carry their transform themselves; gradient geometry
/// is moved point by point, so a non-uniform scale on a radial gradient keeps
/// its centre and scales its radii by the transform's mean scale.
pub(crate) fn transform_paint(paint: Paint, transform: Option<Affine>) -> Paint {
    let Some(transform) = transform else {
        return paint;
    };
    if transform == Affine::IDENTITY {
        return paint;
    }
    let scale = {
        let [a, b, c, d, _, _] = transform.as_coeffs();
        ((a * a + b * b).sqrt() + (c * c + d * d).sqrt()) / 2.0
    };
    match paint {
        Paint::Linear(mut gradient) => {
            gradient.start = transform * gradient.start;
            gradient.end = transform * gradient.end;
            Paint::Linear(gradient)
        }
        Paint::Radial(mut gradient) => {
            gradient.start_center = transform * gradient.start_center;
            gradient.end_center = transform * gradient.end_center;
            gradient.start_radius *= scale;
            gradient.end_radius *= scale;
            Paint::Radial(gradient)
        }
        Paint::Sweep(mut gradient) => {
            gradient.center = transform * gradient.center;
            Paint::Sweep(gradient)
        }
        Paint::Mesh(mesh) => Paint::Mesh(cherenkov::MeshGradient::new(
            mesh.columns(),
            mesh.rows(),
            mesh.points().iter().map(|p| transform * *p).collect(),
            mesh.colors().to_vec(),
        )),
        Paint::Image(mut pattern) => {
            pattern.transform = transform * pattern.transform;
            Paint::Image(pattern)
        }
        other @ (Paint::Solid(_) | Paint::Shader(_)) => other,
        Paint::Transformed(mut paint) => {
            paint.transform = transform * paint.transform;
            Paint::Transformed(paint)
        }
    }
}

/// Scales a paint's alpha where a legacy alpha channel rides on the brush.
fn alpha_scaled_paint(paint: Paint, alpha: f32) -> Paint {
    match paint {
        Paint::Solid(color) => {
            Paint::Solid(color.with_alpha(color.components[3] * alpha))
        }
        _ => paint,
    }
}

/// The existing `Scene2D` contract, preserved unchanged on `Recording` so
/// current WaterUI and hydrolysis-m3 call sites keep compiling.
impl Scene2D for Recording {
    fn fill(
        &mut self,
        fill: Fill,
        transform: Affine,
        brush: &peniko::Brush,
        brush_transform: Option<Affine>,
        shape: &BezPath,
    ) {
        self.fill(fill, transform, brush, brush_transform, shape);
    }

    fn stroke(
        &mut self,
        stroke: &kurbo::Stroke,
        transform: Affine,
        brush: &peniko::Brush,
        brush_transform: Option<Affine>,
        shape: &BezPath,
    ) {
        self.stroke(stroke, transform, brush, brush_transform, shape);
    }

    fn push_layer(
        &mut self,
        fill: Fill,
        blend: BlendMode,
        alpha: f32,
        transform: Affine,
        clip: &BezPath,
    ) {
        self.push_group(fill, blend, alpha, transform, clip);
    }

    fn push_clip_layer(&mut self, fill: Fill, transform: Affine, clip: &BezPath) {
        self.push_clip(fill, transform, clip);
    }

    fn pop_layer(&mut self) {
        assert!(
            self.open_layers > 0,
            "hydrolysis recording: layer pop without a matching push"
        );
        self.open_layers -= 1;
        self.ops.push(Op::PopLayer);
    }

    fn draw_image(&mut self, image: &ImageBrush, transform: Affine) {
        self.image(image, transform);
    }

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_>) {
        if run.glyphs.is_empty() {
            return;
        }
        self.ops.push(Op::Glyphs {
            font: run.font.clone(),
            font_size: run.font_size,
            coords: Arc::from(run.normalized_coords),
            transform: run.transform,
            brush: run.brush.clone(),
            brush_alpha: run.brush_alpha,
            style: run.style.to_owned(),
            glyphs: Arc::from(run.glyphs),
        });
    }

    fn reset(&mut self) {
        Recording::reset(self);
    }
}

/// The existing `DrawContext` adapter: the WaterUI theme-drawing interface,
/// recorded into [`Recording`]. Renamed at cutover; the API is unchanged.
pub struct VelloDrawContext<'a> {
    scene: &'a mut Recording,
    transform_stack: Vec<Affine>,
}

impl<'a> VelloDrawContext<'a> {
    pub(crate) fn with_root_transform(scene: &'a mut Recording, transform: Affine) -> Self {
        Self {
            scene,
            transform_stack: vec![Affine::IDENTITY, transform],
        }
    }

    fn transform(&self) -> Affine {
        *self
            .transform_stack
            .last()
            .expect("hydrolysis draw context transform stack is empty")
    }

    fn fill_shape(&mut self, shape: &(impl Shape + 'static), brush: &Brush) {
        let brush = match brush {
            Brush::Solid(color) => peniko::Brush::Solid(*color),
            Brush::Gradient(gradient) => peniko::Brush::Gradient(gradient.clone()),
        };
        self.scene
            .fill(Fill::NonZero, self.transform(), &brush, None, shape);
    }

    fn stroke_shape(&mut self, shape: &(impl Shape + 'static), brush: &Brush, width: f64) {
        let stroke = kurbo::Stroke::new(width);
        let brush = match brush {
            Brush::Solid(color) => peniko::Brush::Solid(*color),
            Brush::Gradient(gradient) => peniko::Brush::Gradient(gradient.clone()),
        };
        self.scene
            .stroke(&stroke, self.transform(), &brush, None, shape);
    }
}

impl DrawContext for VelloDrawContext<'_> {
    fn fill_rect(&mut self, rect: Rect, brush: &Brush) {
        self.fill_shape(&rect, brush);
    }

    fn fill_rounded_rect(&mut self, rect: Rect, radii: RoundedRectRadii, brush: &Brush) {
        let rounded = RoundedRect::from_rect(rect, radii);
        self.fill_shape(&rounded, brush);
    }

    fn stroke_rect(&mut self, rect: Rect, brush: &Brush, width: f64) {
        self.stroke_shape(&rect, brush, width);
    }

    fn stroke_rounded_rect(
        &mut self,
        rect: Rect,
        radii: RoundedRectRadii,
        brush: &Brush,
        width: f64,
    ) {
        let rounded = RoundedRect::from_rect(rect, radii);
        self.stroke_shape(&rounded, brush, width);
    }

    fn stroke_line(&mut self, from: Point, to: Point, brush: &Brush, width: f64) {
        let line = Line::new(from, to);
        self.stroke_shape(&line, brush, width);
    }

    fn stroke_circle(&mut self, center: Point, radius: f64, brush: &Brush, width: f64) {
        let circle = Circle::new(center, radius);
        self.stroke_shape(&circle, brush, width);
    }

    fn fill_circle(&mut self, center: Point, radius: f64, brush: &Brush) {
        let circle = Circle::new(center, radius);
        self.fill_shape(&circle, brush);
    }

    fn fill_path(&mut self, path: &BezPath, brush: &Brush) {
        self.fill_shape(path, brush);
    }

    fn stroke_path(&mut self, path: &BezPath, brush: &Brush, width: f64) {
        self.stroke_shape(path, brush, width);
    }

    fn draw_shadow(
        &mut self,
        rect: Rect,
        radii: RoundedRectRadii,
        offset: Vec2,
        blur: f64,
        color: peniko::Color,
    ) {
        let radius = radii
            .as_single_radius()
            .expect("blurred shadows require uniform corner radii");
        self.scene.blurred_rounded_rect(
            self.transform(),
            rect + offset,
            color,
            radius,
            blur.max(0.0),
        );
    }

    fn push_layer(&mut self, alpha: f32, clip: Option<&Rect>) {
        let clip = clip
            .copied()
            .unwrap_or(Rect::new(-1.0e9, -1.0e9, 1.0e9, 1.0e9));
        self.scene.push_group(
            Fill::NonZero,
            BlendMode::default(),
            alpha,
            self.transform(),
            &clip,
        );
    }

    fn push_rounded_layer(&mut self, alpha: f32, clip: Rect, radii: RoundedRectRadii) {
        let clip = RoundedRect::from_rect(clip, radii);
        self.scene.push_group(
            Fill::NonZero,
            BlendMode::default(),
            alpha,
            self.transform(),
            &clip,
        );
    }

    fn pop_layer(&mut self) {
        self.scene.pop_scope();
    }

    fn push_transform(&mut self, affine: Affine) {
        let current = self.transform();
        self.transform_stack.push(current * affine);
    }

    fn pop_transform(&mut self) {
        assert!(
            self.transform_stack.len() > 1,
            "hydrolysis draw context transform stack underflow"
        );
        self.transform_stack.pop();
    }
}