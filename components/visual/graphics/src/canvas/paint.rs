//! What a canvas paints with and how strokes and layers are styled.
//!
//! Colours are in the working space, linear Display P3 with extended range
//! ([`WorkingColor`]). A theme colour ([`Color`]) is accepted wherever a
//! paint operand is, and stays bound to the environment it resolves in.

use alloc::vec::Vec;

use nami::impl_constant;
use waterui_core::layout::{Affine2, Point, Vec2};

use crate::canvas::names::Names;
use crate::canvas::resource::ImageSource;
use crate::color::{Color, WorkingColor};
use crate::draw::{self, kurbo};

/// A colour at a position along a gradient.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorStop {
    /// Where the stop sits, from 0 (the gradient's start) to 1 (its end).
    pub offset: f32,
    /// The colour at the stop.
    pub color: WorkingColor,
}

/// How a gradient or image pattern continues outside its defined range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Extend {
    /// The edge colour continues.
    #[default]
    Pad,
    /// The range repeats.
    Repeat,
    /// The range repeats, mirrored every other time.
    Reflect,
    /// Nothing is painted outside the range.
    Transparent,
}

impl Extend {
    const fn lower(self) -> draw::Extend {
        match self {
            Self::Pad => draw::Extend::Pad,
            Self::Repeat => draw::Extend::Repeat,
            Self::Reflect => draw::Extend::Reflect,
            Self::Transparent => draw::Extend::None,
        }
    }
}

/// The space a gradient interpolates its stops in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Interpolation {
    /// Linear light: the stops blend as light does. The default.
    #[default]
    Linear,
    /// The encoded (sRGB transfer) values, as CSS gradients blend.
    Encoded,
}

impl Interpolation {
    const fn lower(self) -> draw::Interpolation {
        match self {
            Self::Linear => draw::Interpolation::Working,
            Self::Encoded => draw::Interpolation::SrgbEncoded,
        }
    }
}

fn kpoint(point: Point) -> kurbo::Point {
    kurbo::Point::new(f64::from(point.x), f64::from(point.y))
}

pub fn affine(transform: Affine2) -> kurbo::Affine {
    kurbo::Affine::new([
        f64::from(transform.a),
        f64::from(transform.b),
        f64::from(transform.c),
        f64::from(transform.d),
        f64::from(transform.e),
        f64::from(transform.f),
    ])
}

fn stops(stops: &[ColorStop]) -> Vec<draw::ColorStop> {
    stops
        .iter()
        .map(|stop| draw::ColorStop {
            offset: stop.offset,
            color: stop.color,
        })
        .collect()
}

macro_rules! gradient_builders {
    ($ty:ident) => {
        impl $ty {
            /// Appends a colour stop at `offset` (0 to 1).
            #[must_use]
            pub fn stop(mut self, offset: f32, color: impl Into<WorkingColor>) -> Self {
                self.stops.push(ColorStop {
                    offset,
                    color: color.into(),
                });
                self
            }

            /// Sets how the gradient continues outside its range.
            #[must_use]
            pub const fn extend(mut self, extend: Extend) -> Self {
                self.extend = extend;
                self
            }

            /// Sets the space the stops interpolate in.
            #[must_use]
            pub const fn interpolation(mut self, interpolation: Interpolation) -> Self {
                self.interpolation = interpolation;
                self
            }
        }
    };
}

/// A gradient along the line from `start` to `end`.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearGradient {
    /// Where offset 0 lies.
    pub start: Point,
    /// Where offset 1 lies.
    pub end: Point,
    /// The colour stops, in offset order.
    pub stops: Vec<ColorStop>,
    /// How the gradient continues beyond its ends.
    pub extend: Extend,
    /// The space the stops interpolate in.
    pub interpolation: Interpolation,
}

impl LinearGradient {
    /// A gradient from `start` to `end` with no stops yet.
    #[must_use]
    pub const fn new(start: Point, end: Point) -> Self {
        Self {
            start,
            end,
            stops: Vec::new(),
            extend: Extend::Pad,
            interpolation: Interpolation::Linear,
        }
    }

    fn lower(&self) -> draw::LinearGradient {
        draw::LinearGradient {
            start: kpoint(self.start),
            end: kpoint(self.end),
            stops: stops(&self.stops),
            extend: self.extend.lower(),
            interpolation: self.interpolation.lower(),
        }
    }
}

gradient_builders!(LinearGradient);

/// A gradient between two circles: offset 0 on the start circle, offset 1 on
/// the end circle. A start circle of radius 0 inside the end circle is a
/// focal gradient.
#[derive(Clone, Debug, PartialEq)]
pub struct RadialGradient {
    /// The start circle's centre.
    pub start_center: Point,
    /// The start circle's radius.
    pub start_radius: f32,
    /// The end circle's centre.
    pub end_center: Point,
    /// The end circle's radius.
    pub end_radius: f32,
    /// The colour stops, in offset order.
    pub stops: Vec<ColorStop>,
    /// How the gradient continues beyond its circles.
    pub extend: Extend,
    /// The space the stops interpolate in.
    pub interpolation: Interpolation,
}

impl RadialGradient {
    /// A gradient from `center` outward to `radius`.
    #[must_use]
    pub const fn new(center: Point, radius: f32) -> Self {
        Self::two_point(center, 0.0, center, radius)
    }

    /// A gradient from the start circle to the end circle.
    #[must_use]
    pub const fn two_point(
        start_center: Point,
        start_radius: f32,
        end_center: Point,
        end_radius: f32,
    ) -> Self {
        Self {
            start_center,
            start_radius,
            end_center,
            end_radius,
            stops: Vec::new(),
            extend: Extend::Pad,
            interpolation: Interpolation::Linear,
        }
    }

    fn lower(&self) -> draw::RadialGradient {
        draw::RadialGradient {
            start_center: kpoint(self.start_center),
            start_radius: f64::from(self.start_radius),
            end_center: kpoint(self.end_center),
            end_radius: f64::from(self.end_radius),
            stops: stops(&self.stops),
            extend: self.extend.lower(),
            interpolation: self.interpolation.lower(),
        }
    }
}

gradient_builders!(RadialGradient);

/// A gradient sweeping around `center` from `start_angle` to `end_angle`
/// (radians, clockwise from the positive x axis in canvas space).
#[derive(Clone, Debug, PartialEq)]
pub struct ConicGradient {
    /// The centre the gradient sweeps around.
    pub center: Point,
    /// The angle of offset 0.
    pub start_angle: f32,
    /// The angle of offset 1.
    pub end_angle: f32,
    /// The colour stops, in offset order.
    pub stops: Vec<ColorStop>,
    /// How the gradient continues beyond its angles.
    pub extend: Extend,
    /// The space the stops interpolate in.
    pub interpolation: Interpolation,
}

impl ConicGradient {
    /// A gradient around `center` from `start_angle` to `end_angle`.
    #[must_use]
    pub const fn new(center: Point, start_angle: f32, end_angle: f32) -> Self {
        Self {
            center,
            start_angle,
            end_angle,
            stops: Vec::new(),
            extend: Extend::Pad,
            interpolation: Interpolation::Linear,
        }
    }

    fn lower(&self) -> draw::SweepGradient {
        draw::SweepGradient {
            center: kpoint(self.center),
            start_angle: f64::from(self.start_angle),
            end_angle: f64::from(self.end_angle),
            stops: stops(&self.stops),
            extend: self.extend.lower(),
            interpolation: self.interpolation.lower(),
        }
    }
}

gradient_builders!(ConicGradient);

/// A grid of bilinear colour patches: `columns × rows` patches over a
/// `(columns + 1) × (rows + 1)` grid of points, each point with its colour.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshGradient {
    columns: u32,
    rows: u32,
    points: Vec<Point>,
    colors: Vec<WorkingColor>,
}

impl MeshGradient {
    /// A mesh of `columns × rows` patches. `points` and `colors` list the
    /// grid's vertices row by row.
    ///
    /// # Panics
    ///
    /// When the grid is empty, or either list does not hold exactly one
    /// entry per vertex.
    #[must_use]
    pub fn new(columns: u32, rows: u32, points: Vec<Point>, colors: Vec<WorkingColor>) -> Self {
        assert!(
            columns > 0 && rows > 0,
            "a mesh gradient has at least one patch, got {columns}x{rows}"
        );
        let vertices = (columns as usize + 1) * (rows as usize + 1);
        assert!(
            points.len() == vertices && colors.len() == vertices,
            "a {columns}x{rows} mesh needs {vertices} points and colours, got {} and {}",
            points.len(),
            colors.len()
        );
        Self {
            columns,
            rows,
            points,
            colors,
        }
    }

    /// Patches per row.
    #[must_use]
    pub const fn columns(&self) -> u32 {
        self.columns
    }

    /// Patches per column.
    #[must_use]
    pub const fn rows(&self) -> u32 {
        self.rows
    }

    /// The grid's vertices, row by row.
    #[must_use]
    pub fn points(&self) -> &[Point] {
        &self.points
    }

    /// The colour at each vertex, row by row.
    #[must_use]
    pub fn colors(&self) -> &[WorkingColor] {
        &self.colors
    }

    fn lower(&self) -> draw::MeshGradient {
        draw::MeshGradient::new(
            self.columns,
            self.rows,
            self.points.iter().copied().map(kpoint).collect(),
            self.colors.clone(),
        )
    }
}

/// How an image is sampled when it is drawn at another size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Sampling {
    /// The nearest texel: hard pixel edges.
    Nearest,
    /// Bilinear filtering. The default.
    #[default]
    Linear,
}

impl Sampling {
    pub(crate) const fn lower(self) -> draw::Sampling {
        match self {
            Self::Nearest => draw::Sampling::Nearest,
            Self::Linear => draw::Sampling::Linear,
        }
    }
}

/// An image tiled across the painted area.
#[derive(Clone, Debug, PartialEq)]
pub struct ImagePattern {
    /// The image.
    pub image: ImageSource,
    /// Maps the image's pixel space into canvas space.
    pub transform: Affine2,
    /// How the pattern continues horizontally beyond the image.
    pub extend_x: Extend,
    /// How the pattern continues vertically beyond the image.
    pub extend_y: Extend,
    /// How the image is sampled.
    pub sampling: Sampling,
}

impl ImagePattern {
    /// `image` at one pixel per point from the canvas origin, repeated on
    /// both axes.
    #[must_use]
    pub const fn new(image: ImageSource) -> Self {
        Self {
            image,
            transform: Affine2::IDENTITY,
            extend_x: Extend::Repeat,
            extend_y: Extend::Repeat,
            sampling: Sampling::Linear,
        }
    }

    /// Sets the image-to-canvas transform.
    #[must_use]
    pub const fn transform(mut self, transform: Affine2) -> Self {
        self.transform = transform;
        self
    }

    /// Sets how the pattern continues on each axis.
    #[must_use]
    pub const fn extend(mut self, x: Extend, y: Extend) -> Self {
        self.extend_x = x;
        self.extend_y = y;
        self
    }

    /// Sets how the image is sampled.
    #[must_use]
    pub const fn sampling(mut self, sampling: Sampling) -> Self {
        self.sampling = sampling;
        self
    }
}

/// What fills an area or a stroke.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Paint {
    /// One colour.
    Color(WorkingColor),
    /// A [`LinearGradient`].
    Linear(LinearGradient),
    /// A [`RadialGradient`].
    Radial(RadialGradient),
    /// A [`ConicGradient`].
    Conic(ConicGradient),
    /// A [`MeshGradient`].
    Mesh(MeshGradient),
    /// An [`ImagePattern`].
    Image(ImagePattern),
}

impl Paint {
    /// The image this paint samples, if any.
    pub(crate) const fn image(&self) -> Option<&ImageSource> {
        match self {
            Self::Image(pattern) => Some(&pattern.image),
            _ => None,
        }
    }

    /// The recorded form; `None` when the paint samples an image `names`
    /// has no id for.
    pub(crate) fn lower(&self, names: &Names) -> Option<draw::Paint> {
        Some(match self {
            Self::Color(color) => draw::Paint::Solid(*color),
            Self::Linear(gradient) => draw::Paint::Linear(gradient.lower()),
            Self::Radial(gradient) => draw::Paint::Radial(gradient.lower()),
            Self::Conic(gradient) => draw::Paint::Sweep(gradient.lower()),
            Self::Mesh(mesh) => draw::Paint::Mesh(mesh.lower()),
            Self::Image(pattern) => draw::Paint::Image(draw::ImagePattern {
                image: names.image(&pattern.image)?,
                transform: affine(pattern.transform),
                extend_x: pattern.extend_x.lower(),
                extend_y: pattern.extend_y.lower(),
                sampling: pattern.sampling.lower(),
            }),
        })
    }
}

macro_rules! paint_from {
    ($($ty:ident => $variant:ident),+ $(,)?) => {
        $(
            impl From<$ty> for Paint {
                fn from(value: $ty) -> Self {
                    Self::$variant(value)
                }
            }
        )+
    };
}

paint_from!(
    WorkingColor => Color,
    LinearGradient => Linear,
    RadialGradient => Radial,
    ConicGradient => Conic,
    MeshGradient => Mesh,
    ImagePattern => Image,
);

/// How a stroke ends an open subpath.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Cap {
    /// Flat, at the end point. The default.
    #[default]
    Butt,
    /// A half circle around the end point.
    Round,
    /// Flat, half the width beyond the end point.
    Square,
}

/// How a stroke turns a corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Join {
    /// A sharp corner, cut to a bevel past the miter limit. The default.
    #[default]
    Miter,
    /// A rounded corner.
    Round,
    /// A corner cut straight across.
    Bevel,
}

/// How a shape's outline is stroked.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeStyle {
    /// The stroke's width, in points.
    pub width: f32,
    /// How open subpaths end.
    pub cap: Cap,
    /// How corners turn.
    pub join: Join,
    /// The longest miter, as a multiple of the width, before a miter join
    /// becomes a bevel.
    pub miter_limit: f32,
    /// Alternating dash and gap lengths, in points; empty for a solid line.
    pub dash: Vec<f32>,
    /// How far into the dash pattern the stroke starts.
    pub dash_offset: f32,
}

impl StrokeStyle {
    /// A solid stroke `width` points wide with butt caps and miter joins.
    #[must_use]
    pub const fn new(width: f32) -> Self {
        Self {
            width,
            cap: Cap::Butt,
            join: Join::Miter,
            miter_limit: 4.0,
            dash: Vec::new(),
            dash_offset: 0.0,
        }
    }

    /// Sets the caps.
    #[must_use]
    pub const fn cap(mut self, cap: Cap) -> Self {
        self.cap = cap;
        self
    }

    /// Sets the joins.
    #[must_use]
    pub const fn join(mut self, join: Join) -> Self {
        self.join = join;
        self
    }

    /// Sets the miter limit.
    #[must_use]
    pub const fn miter_limit(mut self, limit: f32) -> Self {
        self.miter_limit = limit;
        self
    }

    /// Dashes the stroke: `pattern` alternates dash and gap lengths, and
    /// `offset` is how far into it the stroke starts.
    #[must_use]
    pub fn dash(mut self, pattern: impl Into<Vec<f32>>, offset: f32) -> Self {
        self.dash = pattern.into();
        self.dash_offset = offset;
        self
    }

    pub(crate) fn lower(&self) -> kurbo::Stroke {
        kurbo::Stroke::new(f64::from(self.width))
            .with_caps(match self.cap {
                Cap::Butt => kurbo::Cap::Butt,
                Cap::Round => kurbo::Cap::Round,
                Cap::Square => kurbo::Cap::Square,
            })
            .with_join(match self.join {
                Join::Miter => kurbo::Join::Miter,
                Join::Round => kurbo::Join::Round,
                Join::Bevel => kurbo::Join::Bevel,
            })
            .with_miter_limit(f64::from(self.miter_limit))
            .with_dashes(
                f64::from(self.dash_offset),
                self.dash.iter().copied().map(f64::from),
            )
    }
}

/// A blurred shadow cast by a shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    /// The blur's standard deviation, in points.
    pub blur: f32,
    /// How far the shadow is displaced from the shape.
    pub offset: Vec2,
    /// How far the shape grows (positive) or shrinks (negative) before it is
    /// blurred.
    pub spread: f32,
    /// The shadow's colour.
    pub color: WorkingColor,
}

impl Shadow {
    /// An undisplaced shadow of `color` blurred by `blur` points.
    #[must_use]
    pub const fn new(blur: f32, color: WorkingColor) -> Self {
        Self {
            blur,
            offset: Vec2 { dx: 0.0, dy: 0.0 },
            spread: 0.0,
            color,
        }
    }

    /// Sets the displacement.
    #[must_use]
    pub const fn offset(mut self, dx: f32, dy: f32) -> Self {
        self.offset = Vec2 { dx, dy };
        self
    }

    /// Sets the spread.
    #[must_use]
    pub const fn spread(mut self, spread: f32) -> Self {
        self.spread = spread;
        self
    }

    pub(crate) fn lower(&self) -> draw::Shadow {
        draw::Shadow::new(f64::from(self.blur), self.color)
            .offset((f64::from(self.offset.dx), f64::from(self.offset.dy)))
            .spread(f64::from(self.spread))
    }
}

/// How a layer's content composites onto what lies beneath it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BlendMode {
    /// Source over. The default.
    #[default]
    Normal,
    /// Multiply.
    Multiply,
    /// Screen.
    Screen,
    /// Overlay.
    Overlay,
    /// Darken.
    Darken,
    /// Lighten.
    Lighten,
    /// Colour dodge.
    ColorDodge,
    /// Colour burn.
    ColorBurn,
    /// Hard light.
    HardLight,
    /// Soft light.
    SoftLight,
    /// Difference.
    Difference,
    /// Exclusion.
    Exclusion,
    /// Hue.
    Hue,
    /// Saturation.
    Saturation,
    /// Colour.
    Color,
    /// Luminosity.
    Luminosity,
    /// Clears both source and destination.
    Clear,
    /// The source replaces the destination.
    Source,
    /// The destination is kept and the source discarded.
    Destination,
    /// The destination over the source.
    DestinationOver,
    /// The source where it overlaps the destination.
    SourceIn,
    /// The destination where it overlaps the source.
    DestinationIn,
    /// The source outside the destination.
    SourceOut,
    /// The destination outside the source.
    DestinationOut,
    /// The source atop the destination.
    SourceAtop,
    /// The destination atop the source.
    DestinationAtop,
    /// Source and destination where they do not overlap.
    Xor,
    /// Source and destination summed without clamping.
    PlusLighter,
}

impl BlendMode {
    const fn lower(self) -> draw::BlendMode {
        use draw::BlendMode as B;
        match self {
            Self::Normal => B::Normal,
            Self::Multiply => B::Multiply,
            Self::Screen => B::Screen,
            Self::Overlay => B::Overlay,
            Self::Darken => B::Darken,
            Self::Lighten => B::Lighten,
            Self::ColorDodge => B::ColorDodge,
            Self::ColorBurn => B::ColorBurn,
            Self::HardLight => B::HardLight,
            Self::SoftLight => B::SoftLight,
            Self::Difference => B::Difference,
            Self::Exclusion => B::Exclusion,
            Self::Hue => B::Hue,
            Self::Saturation => B::Saturation,
            Self::Color => B::Color,
            Self::Luminosity => B::Luminosity,
            Self::Clear => B::Clear,
            Self::Source => B::Src,
            Self::Destination => B::Dst,
            Self::DestinationOver => B::DestOver,
            Self::SourceIn => B::SrcIn,
            Self::DestinationIn => B::DestIn,
            Self::SourceOut => B::SrcOut,
            Self::DestinationOut => B::DestOut,
            Self::SourceAtop => B::SrcAtop,
            Self::DestinationAtop => B::DestAtop,
            Self::Xor => B::Xor,
            Self::PlusLighter => B::PlusLighter,
        }
    }
}

/// The space a layer blends in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BlendSpace {
    /// Linear light. The default.
    #[default]
    Linear,
    /// The encoded (sRGB transfer) values, as the web blends.
    Encoded,
}

/// An isolated group: its content composites as one image, with an opacity
/// and a blend mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    /// The group's opacity, from 0 to 1.
    pub opacity: f32,
    /// How the group blends onto what lies beneath it.
    pub blend: BlendMode,
    /// The space it blends in.
    pub blend_space: BlendSpace,
}

impl Layer {
    /// A normally blended layer at `opacity`.
    #[must_use]
    pub const fn opacity(opacity: f32) -> Self {
        Self {
            opacity,
            blend: BlendMode::Normal,
            blend_space: BlendSpace::Linear,
        }
    }

    /// Sets the blend mode.
    #[must_use]
    pub const fn blend(mut self, blend: BlendMode) -> Self {
        self.blend = blend;
        self
    }

    /// Sets the blend space.
    #[must_use]
    pub const fn blend_space(mut self, space: BlendSpace) -> Self {
        self.blend_space = space;
        self
    }

    pub(crate) const fn lower(self) -> draw::Group {
        draw::Group {
            opacity: self.opacity,
            blend: self.blend.lower(),
            blend_space: match self.blend_space {
                BlendSpace::Linear => draw::BlendSpace::Linear,
                BlendSpace::Encoded => draw::BlendSpace::SrgbEncoded,
            },
            filter: None,
        }
    }
}

/// A value a paint operand accepts: a [`Paint`] or anything that converts to
/// one, or a theme [`Color`], which resolves in the canvas's environment and
/// stays bound to it.
pub trait IntoPaint: sealed::Resolve + Clone + 'static {}

impl<T: sealed::Resolve + Clone + 'static> IntoPaint for T {}

pub mod sealed {
    use nami::{Computed, SignalExt as _};
    use waterui_core::{Environment, flatten_signal};

    use super::{
        Color, ConicGradient, ImagePattern, LinearGradient, MeshGradient, Paint, RadialGradient,
        WorkingColor,
    };

    /// Turns a signal of paint-like values into a signal of paints.
    pub trait Resolve: Sized + Clone + 'static {
        /// The paint signal `source` drives, in `env`.
        fn resolve(source: Computed<Self>, env: &Environment) -> Computed<Paint>;
    }

    macro_rules! resolve_into {
        ($($ty:ty),+) => {
            $(
                impl Resolve for $ty {
                    fn resolve(source: Computed<Self>, _env: &Environment) -> Computed<Paint> {
                        source.map(Paint::from).computed()
                    }
                }
            )+
        };
    }

    resolve_into!(
        Paint,
        WorkingColor,
        LinearGradient,
        RadialGradient,
        ConicGradient,
        MeshGradient,
        ImagePattern
    );

    impl Resolve for Color {
        fn resolve(source: Computed<Self>, env: &Environment) -> Computed<Paint> {
            let env = env.clone();
            let resolved: Computed<WorkingColor> =
                flatten_signal(source.map(move |color: Self| color.resolve(&env)));
            resolved.map(Paint::Color).computed()
        }
    }
}

impl_constant!(
    Paint,
    LinearGradient,
    RadialGradient,
    ConicGradient,
    MeshGradient,
    ImagePattern,
    StrokeStyle,
    Shadow,
    Layer,
    BlendMode,
    BlendSpace,
    Sampling
);
