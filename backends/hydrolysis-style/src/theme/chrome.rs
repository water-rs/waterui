//! The engine-provided chrome primitives of the theme contract: the
//! interaction machinery every interactive theme otherwise hand-writes.
//!
//! A theme describes a widget's interactive container as data — a
//! [`Surface`] with its [`CornerShape`], [`Fill`], [`Border`] and
//! [`Elevation`], the [`StateLayer`] opacities it states as tokens, the
//! [`PressFeedback`] a press shows, the [`FocusIndicator`] keyboard focus
//! shows and the [`HitRegion`] it takes touches in — bundled as a
//! [`Chrome`], and Hydrolysis realizes it. The realization records the
//! chrome once. Every value that follows the widget's inputs is a
//! [`Driven`] the renderer keeps as a live operand — the silhouette and its
//! press morph and press scale, the fill or backdrop effect, the border, the
//! elevation, the state layer, each ripple wave, the wash and the focus
//! ring — retargeted under the chrome's [`Motion`], so an interaction change
//! records nothing, walks no Hydrolysis tree and runs no layout pass.
//!
//! A theme hands out chrome in its looks
//! ([`ButtonLook`](super::ButtonLook)); a composed control installs one as a
//! plugin, and every button and tappable view under it takes that chrome.

use core::time::Duration;
use std::rc::Rc;

use cherenkov_record::{
    CaptureClass, ContentShader, ContinuousRect, MaterialEffect, MaterialShader, Paint, Shadow,
    ShapeData, WorkingColor,
};
use kurbo::{Affine, Circle, Point, Rect, RoundedRect, RoundedRectRadii, Size, Vec2};
use waterui_core::animation::Animation;
use waterui_core::plugin::Plugin;
use waterui_core::reactive::signal::IntoSignal;
use waterui_graphics::color::Color;
use waterui_interaction::interaction::{InteractionState, StateValue};

use super::look::{CustomPart, DrivenSet};
use super::{Custom, Driven, HitRegion, Motion};

/// The radii of one silhouette of a [`CornerShape`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed: a corner is a radius per corner, a fraction of the shorter side, or fully rounded"
)]
pub enum Corners {
    /// A radius per corner, in logical units, clamped to half the shorter
    /// side.
    Radii(RoundedRectRadii),
    /// Every radius `fraction` of the shorter side, so the corners follow
    /// the surface's size; `0.5` is [`Self::Full`].
    Fraction(f64),
    /// Fully rounded: every radius is half the shorter side — a pill, or a
    /// circle on a square surface. The radius follows the surface's size.
    Full,
}

impl Corners {
    /// The radii these corners take on `rect`.
    #[must_use]
    pub fn radii_on(self, rect: Rect) -> RoundedRectRadii {
        let limit = rect.width().min(rect.height()) / 2.0;
        match self {
            Self::Full => RoundedRectRadii::from_single_radius(limit),
            Self::Fraction(fraction) => {
                RoundedRectRadii::from_single_radius((limit * 2.0 * fraction).clamp(0.0, limit))
            }
            Self::Radii(radii) => RoundedRectRadii::new(
                radii.top_left.min(limit),
                radii.top_right.min(limit),
                radii.bottom_right.min(limit),
                radii.bottom_left.min(limit),
            ),
        }
    }
}

impl From<RoundedRectRadii> for Corners {
    fn from(radii: RoundedRectRadii) -> Self {
        Self::Radii(radii)
    }
}

impl From<f32> for Corners {
    fn from(radius: f32) -> Self {
        Self::Radii(RoundedRectRadii::from_single_radius(f64::from(radius)))
    }
}

/// Per-state radii are corners driven by the interaction state.
impl<V: 'static> From<StateValue<RoundedRectRadii>> for Driven<Corners, V> {
    fn from(value: StateValue<RoundedRectRadii>) -> Self {
        Self::new(move |inputs| Corners::Radii(*value.resolve(inputs.state)))
    }
}

/// A surface's silhouette: its corners as a [`Driven`] value — at rest, and
/// morphing while pressed, hovered or in any other state — and whether they
/// are circular or continuous.
///
/// The morph is a live operand: it follows the chrome's motion, the press
/// transition for a press — in both directions, so a release plays it
/// backwards. The state layer, the ripple clip, the wash and the focus ring
/// follow the morph.
#[derive(Debug, Clone)]
pub struct CornerShape {
    corners: Driven<Corners>,
    /// `Some` for continuous corners, carrying the curvature transition's
    /// extent as a fraction of each radius ([`ContinuousRect::smoothing`]).
    smoothing: Option<f64>,
    /// What the silhouette is drawn as: rounded corners on the rect, or the
    /// ellipse the rect inscribes.
    outline: Outline,
}

/// What a [`CornerShape`]'s silhouette is drawn as.
#[derive(Debug, Clone)]
enum Outline {
    /// A rectangle with the shape's corners.
    Rect,
    /// A custom path, its unit-space commands resolved against the rect.
    Path(std::rc::Rc<[waterui_shape::PathCommand]>),
    /// The circle inscribed in the rect, centred in it.
    Circle,
    /// The ellipse filling the rect.
    Ellipse,
}

impl CornerShape {
    /// Circular `corners`.
    #[must_use]
    pub fn new(corners: impl Into<Driven<Corners>>) -> Self {
        Self {
            corners: corners.into(),
            smoothing: None,
            outline: Outline::Rect,
        }
    }

    /// The circle inscribed in the surface and centred in it, whatever its
    /// aspect.
    #[doc(hidden)]
    #[must_use]
    pub fn circle() -> Self {
        Self {
            outline: Outline::Circle,
            ..Self::full()
        }
    }

    /// A custom path, resolved against the surface.
    #[doc(hidden)]
    #[must_use]
    pub fn path(commands: &[waterui_shape::PathCommand]) -> Self {
        Self {
            outline: Outline::Path(commands.into()),
            ..Self::full()
        }
    }

    /// The ellipse filling the surface.
    #[doc(hidden)]
    #[must_use]
    pub fn ellipse() -> Self {
        Self {
            outline: Outline::Ellipse,
            ..Self::full()
        }
    }

    /// One circular radius on every corner.
    #[must_use]
    pub fn radius(radius: f32) -> Self {
        Self::new(Corners::from(radius))
    }

    /// Fully rounded corners: a pill, or a circle on a square surface.
    #[must_use]
    pub fn full() -> Self {
        Self::new(Corners::Full)
    }

    /// Morphs to `corners` while the widget is physically pressed.
    #[must_use]
    pub fn pressed(mut self, corners: Corners) -> Self {
        let rest = self.corners.clone();
        let motion = rest.get_motion().cloned();
        let mut driven = Driven::new(move |inputs| {
            if inputs.state.contains(InteractionState::PRESSED) {
                corners
            } else {
                rest.target(inputs)
            }
        });
        if let Some(motion) = motion {
            driven = driven.motion(motion);
        }
        self.corners = driven;
        self
    }

    /// Continuous corners with the platform's default smoothing
    /// ([`ContinuousRect::DEFAULT_SMOOTHING`]).
    #[must_use]
    pub const fn continuous(self) -> Self {
        self.smoothing(ContinuousRect::DEFAULT_SMOOTHING)
    }

    /// Continuous corners whose curvature transition extends `smoothing` of
    /// each radius into the straight edges; `0` is a circular corner.
    #[must_use]
    pub const fn smoothing(mut self, smoothing: f64) -> Self {
        self.smoothing = Some(smoothing);
        self
    }

    /// The corners, as a function of the widget's inputs.
    #[must_use]
    pub const fn corners(&self) -> &Driven<Corners> {
        &self.corners
    }

    /// The silhouette of `corners` on `rect`.
    #[must_use]
    pub fn shape(&self, rect: Rect, corners: Corners) -> ShapeData {
        self.outset(rect, corners, 0.0)
    }

    /// The silhouette of `corners` on `rect` grown outward by `distance`
    /// (shrunk inward when negative), its radii grown with it so the edge
    /// stays parallel — the shape a focus ring around the surface, or a
    /// border inside it, follows.
    #[must_use]
    pub fn outset(&self, rect: Rect, corners: Corners, distance: f32) -> ShapeData {
        let distance = f64::from(distance);
        match &self.outline {
            Outline::Rect => {}
            // A path has no offset curve: a grown or shrunk silhouette is
            // the path resolved against the grown or shrunk rect.
            Outline::Path(commands) => {
                let path = path_commands_to_path(commands, rect.inflate(distance, distance));
                return ShapeData::Path {
                    elements: path.elements().into(),
                    rule: cherenkov_record::FillRule::NonZero,
                };
            }
            Outline::Circle => {
                let radius = (rect.width().min(rect.height()) / 2.0 + distance).max(0.0);
                return ShapeData::Circle(Circle::new(rect.center(), radius));
            }
            Outline::Ellipse => {
                let radii = (
                    (rect.width() / 2.0 + distance).max(0.0),
                    (rect.height() / 2.0 + distance).max(0.0),
                );
                return ShapeData::Ellipse(kurbo::Ellipse::new(rect.center(), radii, 0.0));
            }
        }
        let radii = corners.radii_on(rect);
        let grow = |radius: f64| (radius + distance).max(0.0);
        let radii = RoundedRectRadii::new(
            grow(radii.top_left),
            grow(radii.top_right),
            grow(radii.bottom_right),
            grow(radii.bottom_left),
        );
        let rect = rect.inflate(distance, distance);
        self.smoothing.map_or_else(
            || ShapeData::RoundedRect(RoundedRect::from_rect(rect, radii)),
            |smoothing| {
                ShapeData::Continuous(ContinuousRect::new(rect, radii).with_smoothing(smoothing))
            },
        )
    }
}

/// A surface's elevation: the key and ambient shadow pair it casts.
///
/// A theme with an elevation scale keeps its own level table and hands out
/// the pair for each level, as a [`Driven`] through [`Surface::elevation`],
/// so a surface that raises on hover animates between the two pairs as
/// live operands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Elevation {
    /// The directional shadow of the key light.
    pub key: Shadow,
    /// The soft, unoffset shadow of the ambient light.
    pub ambient: Shadow,
}

impl Elevation {
    /// No elevation: both shadows transparent.
    pub const NONE: Self = Self {
        key: Shadow {
            sigma: 0.0,
            offset: Vec2::ZERO,
            spread: 0.0,
            color: WorkingColor::TRANSPARENT,
        },
        ambient: Shadow {
            sigma: 0.0,
            offset: Vec2::ZERO,
            spread: 0.0,
            color: WorkingColor::TRANSPARENT,
        },
    };

    /// The pair `key` and `ambient`.
    #[must_use]
    pub const fn new(key: Shadow, ambient: Shadow) -> Self {
        Self { key, ambient }
    }

    /// Whether neither shadow paints.
    #[must_use]
    pub fn is_none(&self) -> bool {
        self.key.color.components[3] <= 0.0 && self.ambient.color.components[3] <= 0.0
    }

    /// How far the shadows reach beyond the caster's edge.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "logical-pixel shadow reach fits in f32"
    )]
    pub fn extent(&self) -> f32 {
        let reach =
            |shadow: &Shadow| shadow.sigma.mul_add(3.0, shadow.spread) + shadow.offset.hypot();
        reach(&self.key).max(reach(&self.ambient)) as f32
    }
}

/// A stroke drawn just inside a surface's edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Border {
    /// Stroke width in logical units; the stroke lies wholly inside the
    /// surface's silhouette.
    pub width: f32,
    /// The stroke's paint.
    pub paint: Paint,
}

impl Border {
    /// A border of `width` painted with `paint`.
    #[must_use]
    pub fn new(width: f32, paint: impl Into<Paint>) -> Self {
        Self {
            width,
            paint: paint.into(),
        }
    }
}

/// A backdrop material filling a surface.
///
/// It names a backdrop shader the theme registered
/// ([`WidgetTheme::register_backdrop_shaders`](super::WidgetTheme::register_backdrop_shaders)),
/// the capture class its group samples with, and its per-member effect as a
/// [`Driven`] value.
///
/// The material clips to the surface's silhouette. Both the silhouette and
/// the effect are live operands, so a glass surface's shape and effect
/// follow the press and the other inputs with no re-recording.
#[derive(Debug, Clone)]
pub struct Backdrop {
    shader: MaterialShader,
    capture: CaptureClass,
    effect: Driven<MaterialEffect>,
}

impl Backdrop {
    /// The material `shader` sampled through `capture`, with `effect`.
    #[must_use]
    pub fn new(
        shader: MaterialShader,
        capture: CaptureClass,
        effect: impl Into<Driven<MaterialEffect>>,
    ) -> Self {
        Self {
            shader,
            capture,
            effect: effect.into(),
        }
    }

    /// The backdrop shader.
    #[must_use]
    pub const fn shader(&self) -> MaterialShader {
        self.shader
    }

    /// The capture class.
    #[must_use]
    pub const fn capture(&self) -> CaptureClass {
        self.capture
    }

    /// The per-member effect.
    #[must_use]
    pub const fn effect(&self) -> &Driven<MaterialEffect> {
        &self.effect
    }
}

/// A content-effect shader shading the widget content painted on a surface.
///
/// It names a content shader the theme registered
/// ([`WidgetTheme::register_backdrop_shaders`](super::WidgetTheme::register_backdrop_shaders))
/// and its per-member uniform parameters as a [`Driven`] value.
///
/// Hydrolysis isolates the widget's content into a child layer clipped to the
/// surface's live silhouette and runs the shader over that isolated content
/// before interaction overlays (state layer, ripples, wash, focus indicator)
/// paint above it. When no content effect is set, no isolation layer is
/// created.
#[derive(Debug, Clone)]
pub struct ContentEffect {
    shader: ContentShader,
    effect: Driven<MaterialEffect>,
}

impl ContentEffect {
    /// The content `shader` driven with `effect`.
    #[must_use]
    pub fn new(shader: ContentShader, effect: impl Into<Driven<MaterialEffect>>) -> Self {
        Self {
            shader,
            effect: effect.into(),
        }
    }

    /// The content shader.
    #[must_use]
    pub const fn shader(&self) -> ContentShader {
        self.shader
    }

    /// The per-member effect uniforms.
    #[must_use]
    pub const fn effect(&self) -> &Driven<MaterialEffect> {
        &self.effect
    }
}

impl<V: 'static> From<MaterialEffect> for Driven<MaterialEffect, V> {
    fn from(effect: MaterialEffect) -> Self {
        Self::constant(effect)
    }
}

/// What fills a surface: a paint, or a backdrop material.
#[derive(Debug, Clone)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each fill variant by variant; a new one must fail their build instead of falling into a wildcard arm"
)]
pub enum Fill {
    /// A paint, as a [`Driven`] value.
    Paint(Driven<Paint>),
    /// A backdrop material.
    Backdrop(Backdrop),
}

impl From<Driven<Paint>> for Fill {
    fn from(paint: Driven<Paint>) -> Self {
        Self::Paint(paint)
    }
}

impl From<Paint> for Fill {
    fn from(paint: Paint) -> Self {
        Self::Paint(Driven::constant(paint))
    }
}

impl From<WorkingColor> for Fill {
    fn from(color: WorkingColor) -> Self {
        Self::Paint(Driven::from(color))
    }
}

impl From<StateValue<Paint>> for Fill {
    fn from(paint: StateValue<Paint>) -> Self {
        Self::Paint(Driven::from(paint))
    }
}

impl From<Backdrop> for Fill {
    fn from(backdrop: Backdrop) -> Self {
        Self::Backdrop(backdrop)
    }
}

/// A widget's container: its silhouette, fill, border, elevation and optional
/// content effect.
#[derive(Debug, Clone)]
pub struct Surface {
    shape: CornerShape,
    fill: Fill,
    border: Option<Driven<Border>>,
    elevation: Driven<Elevation>,
    content_effect: Option<ContentEffect>,
}

impl Surface {
    /// A flat, borderless surface of `shape` filled with `fill`.
    #[must_use]
    pub fn new(shape: CornerShape, fill: impl Into<Fill>) -> Self {
        Self {
            shape,
            fill: fill.into(),
            border: None,
            elevation: Driven::constant(Elevation::NONE),
            content_effect: None,
        }
    }

    /// A surface of `shape` with no fill, border or elevation: a composed
    /// control whose content draws its own container.
    #[must_use]
    pub fn clear(shape: CornerShape) -> Self {
        Self::new(shape, WorkingColor::TRANSPARENT)
    }

    /// Strokes `border` inside the surface's edge.
    #[must_use]
    pub fn border(mut self, border: impl Into<Driven<Border>>) -> Self {
        self.border = Some(border.into());
        self
    }

    /// Casts `elevation`'s shadows.
    #[must_use]
    pub fn elevation(mut self, elevation: impl Into<Driven<Elevation>>) -> Self {
        self.elevation = elevation.into();
        self
    }

    /// Applies `effect` to the isolated widget content painted on this surface.
    #[must_use]
    pub fn content_effect(mut self, effect: ContentEffect) -> Self {
        self.content_effect = Some(effect);
        self
    }

    /// The silhouette every chrome layer follows.
    #[must_use]
    pub const fn shape(&self) -> &CornerShape {
        &self.shape
    }

    /// The fill.
    #[must_use]
    pub const fn fill(&self) -> &Fill {
        &self.fill
    }

    /// The border; `None` draws none.
    #[must_use]
    pub const fn get_border(&self) -> Option<&Driven<Border>> {
        self.border.as_ref()
    }

    /// The elevation.
    #[must_use]
    pub const fn get_elevation(&self) -> &Driven<Elevation> {
        &self.elevation
    }

    /// The optional content effect shading the widget's own content; `None`
    /// paints the content directly without an isolation layer.
    #[must_use]
    pub const fn get_content_effect(&self) -> Option<&ContentEffect> {
        self.content_effect.as_ref()
    }
}

/// The tint a widget lays over its surface to show its interaction state:
/// a colour and the opacity a theme states for each state as a token.
///
/// The layer shows the opacity of the dominant state: dragged, then pressed
/// when the press feedback tints ([`PressFeedback::tint`]), then focused,
/// then hovered — focus outranks hover. A ripple shows the press as waves
/// above the layer instead, at [`Self::pressed`].
#[derive(Debug, Clone)]
pub struct StateLayer {
    /// The layer's colour; each opacity multiplies its alpha. A colour that
    /// changes — a theme token, a selection-dependent colour — updates the
    /// layer in place.
    pub color: Color,
    /// Opacity while hovered.
    pub hover: f32,
    /// Opacity while showing keyboard focus.
    pub focus: f32,
    /// Opacity while pressed: the tint's, or each ripple wave's.
    pub pressed: f32,
    /// Opacity while dragged.
    pub dragged: f32,
}

impl StateLayer {
    /// A layer of `color` with no opacity in any state.
    #[must_use]
    pub fn new(color: impl IntoSignal<Color> + 'static) -> Self {
        Self {
            color: Color::from_signal(color),
            hover: 0.0,
            focus: 0.0,
            pressed: 0.0,
            dragged: 0.0,
        }
    }

    /// The layer's opacity tokens.
    #[must_use]
    pub const fn opacities(mut self, hover: f32, focus: f32, pressed: f32, dragged: f32) -> Self {
        self.hover = hover;
        self.focus = focus;
        self.pressed = pressed;
        self.dragged = dragged;
        self
    }

    /// The layer's opacity in `state` under `press`: the dominant state's
    /// token, `0` at rest or while disabled.
    #[must_use]
    pub const fn opacity(&self, state: InteractionState, press: &PressFeedback) -> f32 {
        if state.contains(InteractionState::DISABLED) {
            0.0
        } else if state.contains(InteractionState::DRAGGED) {
            self.dragged
        } else if state.contains(InteractionState::PRESSED) && press.tint {
            self.pressed
        } else if state.contains(InteractionState::FOCUSED) {
            self.focus
        } else if state.contains(InteractionState::HOVERED) {
            self.hover
        } else {
            0.0
        }
    }
}

/// Ripple waves: one per press, growing from the press point, with the
/// geometry and timing a theme supplies.
///
/// A wave starts as a circle of `initial_scale` of the full diameter
/// centred on the press point (the frame's centre for a keyboard or
/// synthetic press), and grows to the full diameter — the frame's diagonal,
/// floored at `min_diameter` — under `grow`, while its centre drifts
/// `drift` of the way from the press point to the frame's centre. Its
/// opacity rises to [`StateLayer::pressed`] under `fade_in` and, once
/// released and `minimum_duration` has passed since the press, falls to
/// `0` under `fade_out`. Rapid re-presses overlap: older waves keep fading
/// while the newest grows, up to 4 concurrent waves
/// at once. A wave stays recorded for the widget's life, so its fade runs
/// until the animation settles — a spring that undershoots is never cut
/// off at its first zero crossing. Waves are clipped to the silhouette.
#[derive(Debug, Clone, PartialEq)]
pub struct Ripple {
    /// The smallest full diameter, so small targets still show a ripple.
    pub min_diameter: f32,
    /// The wave's starting diameter as a fraction of the full diameter.
    pub initial_scale: f64,
    /// How far the wave's centre drifts from the press point towards the
    /// frame's centre as it grows: `0` stays on the press point, `1` ends
    /// centred.
    pub drift: f64,
    /// The wave's growth from its starting to its full circle.
    pub grow: Animation,
    /// The wave's opacity rising on the press.
    pub fade_in: Animation,
    /// The wave's opacity falling after the release.
    pub fade_out: Animation,
    /// How long after its press a wave holds before it fades out.
    pub minimum_duration: Duration,
}

impl Ripple {
    /// The full diameter of a wave on `frame`.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "logical-pixel ripple diameter fits in f32"
    )]
    pub fn diameter(&self, frame: Rect) -> f32 {
        frame
            .width()
            .hypot(frame.height())
            .max(f64::from(self.min_diameter)) as f32
    }

    /// A wave's starting and final circles on `frame` for a press at
    /// `origin`, in the frame's space; `None` presses at the centre.
    #[must_use]
    pub fn wave(&self, frame: Rect, origin: Option<Point>) -> (Circle, Circle) {
        let center = frame.center();
        let origin = origin.unwrap_or(center);
        let radius = f64::from(self.diameter(frame)) / 2.0;
        let end = origin.lerp(center, self.drift);
        (
            Circle::new(origin, radius * self.initial_scale),
            Circle::new(end, radius),
        )
    }
}

/// A highlight centred on the press point: a radial falloff of `color`
/// over `radius`, clipped to the silhouette, fading in and out with the
/// press transition.
#[derive(Debug, Clone)]
pub struct Wash {
    /// The highlight's colour at the press point; it falls to transparent
    /// at `radius`.
    pub color: Color,
    /// How far the highlight reaches from the press point.
    pub radius: f32,
}

/// Elastic drag deformation for a pressed surface or control: a bounded
/// rubber-band lag along the drag offset paired with an area-preserving
/// stretch tensor (`lambda_par * lambda_perp == 1.0`).
///
/// All parameters default to zero deformation (identity transform), so a
/// theme opts into the exact lag, displacement/velocity coupling, and stretch
/// bounds its motion language calls for.
#[derive(Debug, Clone, PartialEq)]
pub struct DragDeformation {
    max_lag: f32,
    lag_scale: f32,
    displacement_gain: f64,
    displacement_ref: f32,
    velocity_gain: f64,
    velocity_ref: f32,
    min_stretch: f64,
    max_stretch: f64,
    active_motion: Option<Animation>,
}

impl Default for DragDeformation {
    fn default() -> Self {
        Self::new()
    }
}

impl DragDeformation {
    /// Identity drag deformation: zero lag and unit stretch.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_lag: 0.0,
            lag_scale: 1.0,
            displacement_gain: 0.0,
            displacement_ref: 1.0,
            velocity_gain: 0.0,
            velocity_ref: 1.0,
            min_stretch: 1.0,
            max_stretch: 1.0,
            active_motion: None,
        }
    }

    /// Configures bounded rubber-band lag along the drag offset:
    /// `max_lag * tanh(|drag| / lag_scale)`.
    #[must_use]
    pub const fn lag(mut self, max_lag: f32, lag_scale: f32) -> Self {
        self.max_lag = max_lag;
        self.lag_scale = lag_scale;
        self
    }

    /// Configures displacement-driven stretch along the drag offset:
    /// `gain * tanh(|drag| / reference)`.
    #[must_use]
    pub const fn displacement_stretch(mut self, gain: f64, reference: f32) -> Self {
        self.displacement_gain = gain;
        self.displacement_ref = reference;
        self
    }

    /// Configures velocity-driven stretch along the smoothed pointer velocity:
    /// `gain * tanh(|velocity| / reference)`.
    #[must_use]
    pub const fn velocity_stretch(mut self, gain: f64, reference: f32) -> Self {
        self.velocity_gain = gain;
        self.velocity_ref = reference;
        self
    }

    /// Clamps the primary stretch eigenvalue `lambda_par` to
    /// `[min_stretch, max_stretch]`; the perpendicular eigenvalue is always
    /// `1.0 / lambda_par`.
    #[must_use]
    pub const fn stretch_bounds(mut self, min_stretch: f64, max_stretch: f64) -> Self {
        self.min_stretch = min_stretch;
        self.max_stretch = max_stretch;
        self
    }

    /// Optional animation used while tracking pointer moves during an active
    /// drag (`None` snaps directly to the deformed state with zero latency).
    #[must_use]
    pub const fn active_motion(mut self, animation: Animation) -> Self {
        self.active_motion = Some(animation);
        self
    }

    /// Maximum rubber-band lag in logical points.
    #[must_use]
    pub const fn get_max_lag(&self) -> f32 {
        self.max_lag
    }

    /// Characteristic drag distance for rubber-band lag, in logical points.
    #[must_use]
    pub const fn get_lag_scale(&self) -> f32 {
        self.lag_scale
    }

    /// Displacement stretch gain.
    #[must_use]
    pub const fn get_displacement_gain(&self) -> f64 {
        self.displacement_gain
    }

    /// Characteristic displacement for stretch, in logical points.
    #[must_use]
    pub const fn get_displacement_ref(&self) -> f32 {
        self.displacement_ref
    }

    /// Velocity stretch gain.
    #[must_use]
    pub const fn get_velocity_gain(&self) -> f64 {
        self.velocity_gain
    }

    /// Characteristic velocity for stretch, in logical points per second.
    #[must_use]
    pub const fn get_velocity_ref(&self) -> f32 {
        self.velocity_ref
    }

    /// Lower bound on the primary stretch eigenvalue.
    #[must_use]
    pub const fn get_min_stretch(&self) -> f64 {
        self.min_stretch
    }

    /// Upper bound on the primary stretch eigenvalue.
    #[must_use]
    pub const fn get_max_stretch(&self) -> f64 {
        self.max_stretch
    }

    /// Optional animation used while tracking active drag moves.
    #[must_use]
    pub const fn get_active_motion(&self) -> Option<&Animation> {
        self.active_motion.as_ref()
    }

    /// Rubber-band translation offset for `drag`.
    #[must_use]
    pub fn lag_offset(&self, drag: Vec2) -> Vec2 {
        let dist = drag.hypot();
        let max_lag = f64::from(self.max_lag);
        let lag_scale = f64::from(self.lag_scale);
        if max_lag <= 0.0 || lag_scale <= 0.0 || dist <= 1e-9 || !dist.is_finite() {
            return Vec2::ZERO;
        }
        let mag = max_lag * (dist / lag_scale).tanh();
        drag * (mag / dist)
    }

    fn drive_vector(&self, drag: Vec2, velocity: Vec2) -> Vec2 {
        let mut drive = Vec2::ZERO;
        let dist = drag.hypot();
        let displacement_ref = f64::from(self.displacement_ref);
        if self.displacement_gain > 0.0 && displacement_ref > 0.0 && dist > 1e-9 && dist.is_finite()
        {
            let mag = self.displacement_gain * (dist / displacement_ref).tanh();
            drive += drag * (mag / dist);
        }
        let speed = velocity.hypot();
        let velocity_ref = f64::from(self.velocity_ref);
        if self.velocity_gain > 0.0 && velocity_ref > 0.0 && speed > 1e-9 && speed.is_finite() {
            let mag = self.velocity_gain * (speed / velocity_ref).tanh();
            drive += velocity * (mag / speed);
        }
        drive
    }

    /// Area-preserving stretch eigenvalues `(lambda_par, lambda_perp)` with
    /// `lambda_par * lambda_perp == 1.0`.
    #[must_use]
    pub fn stretch(&self, drag: Vec2, velocity: Vec2) -> (f64, f64) {
        let excess = self.drive_vector(drag, velocity).hypot();
        if excess <= 1e-9 {
            return (1.0, 1.0);
        }
        let lo = self.min_stretch.max(1e-3);
        let hi = self.max_stretch.max(lo);
        let lambda_par = (1.0 + excess).clamp(lo, hi);
        (lambda_par, 1.0 / lambda_par)
    }

    /// Axis-aligned area-preserving scales `(sx, sy)` (`sx * sy == 1.0`) for
    /// deforming an axis-aligned silhouette rectangle.
    #[must_use]
    pub fn axis_scales(&self, drag: Vec2, velocity: Vec2) -> (f64, f64) {
        let drive = self.drive_vector(drag, velocity);
        let mag = drive.hypot();
        if mag <= 1e-9 {
            return (1.0, 1.0);
        }
        let (lambda_par, _) = self.stretch(drag, velocity);
        let ux = drive.x / mag;
        let uy = drive.y / mag;
        let cos2 = ux.mul_add(ux, -(uy * uy));
        let sx = lambda_par.powf(cos2);
        (sx, 1.0 / sx)
    }

    /// Deformed axis-aligned silhouette rectangle for `frame` under `(drag, velocity)`.
    #[must_use]
    pub fn deformed_rect(&self, frame: Rect, drag: Vec2, velocity: Vec2) -> Rect {
        let lag = self.lag_offset(drag);
        let (sx, sy) = self.axis_scales(drag, velocity);
        Rect::from_center_size(
            frame.center() + lag,
            Size::new(frame.width() * sx, frame.height() * sy),
        )
    }

    /// Combined rubber-band lag and 2D area-preserving stretch tensor about
    /// `center` (`det == 1.0`).
    #[must_use]
    pub fn transform(&self, center: Point, drag: Vec2, velocity: Vec2) -> Affine {
        let lag = self.lag_offset(drag);
        let drive = self.drive_vector(drag, velocity);
        let mag = drive.hypot();
        let (diag_x, shear, diag_y) = if mag <= 1e-9 {
            (1.0, 0.0, 1.0)
        } else {
            let (lambda_par, lambda_perp) = self.stretch(drag, velocity);
            let ux = drive.x / mag;
            let uy = drive.y / mag;
            let ux2 = ux * ux;
            let uy2 = uy * uy;
            (
                lambda_par.mul_add(ux2, lambda_perp * uy2),
                (lambda_par - lambda_perp) * ux * uy,
                lambda_par.mul_add(uy2, lambda_perp * ux2),
            )
        };
        let pivot = center.to_vec2();
        Affine::translate(pivot + lag)
            * Affine::new([diag_x, shear, shear, diag_y, 0.0, 0.0])
            * Affine::translate(-pivot)
    }

    /// Worst-case visual expansion beyond `size` on any side, in logical points.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "logical-pixel drag deformation extent fits in f32"
    )]
    pub fn extent(&self, size: Size) -> f32 {
        let lag = f64::from(self.max_lag.max(0.0));
        let stretch_excess = (self.max_stretch - 1.0).max(0.0);
        let half_max = 0.5 * size.width.max(size.height).max(0.0);
        stretch_excess.mul_add(half_max, lag) as f32
    }
}

/// What a press shows: any combination of a ripple, a tint, a scale, a wash
/// and an optional drag deformation, each driven by the chrome's press
/// transition.
#[derive(Debug, Clone, Default)]
pub struct PressFeedback {
    ripple: Option<Ripple>,
    tint: bool,
    scale: Option<f64>,
    max_growth: Option<f32>,
    wash: Option<Wash>,
    drag_deformation: Option<DragDeformation>,
}

impl PressFeedback {
    /// No press feedback.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ripple: None,
            tint: false,
            scale: None,
            max_growth: None,
            wash: None,
            drag_deformation: None,
        }
    }

    /// A wave per press ([`Ripple`]).
    #[must_use]
    pub const fn ripple(mut self, ripple: Ripple) -> Self {
        self.ripple = Some(ripple);
        self
    }

    /// The state layer at [`StateLayer::pressed`] while pressed.
    #[must_use]
    pub const fn tint(mut self) -> Self {
        self.tint = true;
        self
    }

    /// Grows the silhouette, its radii and the content that follows it
    /// ([`Label::follows_press_scale`](super::Label::follows_press_scale))
    /// by `1 + amount` while pressed, about the frame's centre. A press
    /// transition that overshoots passes through, so a spring release
    /// undershoots and rebounds.
    #[must_use]
    pub const fn scale(mut self, amount: f64) -> Self {
        self.scale = Some(amount);
        self
    }

    /// Caps the uniform press scale so neither half-axis grows (or shrinks)
    /// by more than `max_growth` logical points, preserving aspect ratio on
    /// wide or tall controls. Default is `None` (unconstrained uniform scale).
    #[must_use]
    pub const fn max_growth(mut self, max_growth: f32) -> Self {
        self.max_growth = Some(max_growth);
        self
    }

    /// A highlight centred on the press point ([`Wash`]).
    #[must_use]
    pub fn wash(mut self, wash: Wash) -> Self {
        self.wash = Some(wash);
        self
    }

    /// Elastic lag and area-preserving stretch applied while a press drags.
    #[must_use]
    pub const fn drag_deformation(mut self, deformation: DragDeformation) -> Self {
        self.drag_deformation = Some(deformation);
        self
    }

    /// The ripple, if the press shows one.
    #[must_use]
    pub const fn get_ripple(&self) -> Option<&Ripple> {
        self.ripple.as_ref()
    }

    /// Whether the press tints the state layer.
    #[must_use]
    pub const fn get_tint(&self) -> bool {
        self.tint
    }

    /// The press scale amount, if the press scales.
    #[must_use]
    pub const fn get_scale(&self) -> Option<f64> {
        self.scale
    }

    /// The half-axis growth cap in logical points, if one is set.
    #[must_use]
    pub const fn get_max_growth(&self) -> Option<f32> {
        self.max_growth
    }

    /// Effective uniform press scale factor on a control of `size`, honoring
    /// [`Self::max_growth`] without distorting aspect ratio.
    #[must_use]
    pub fn effective_scale(&self, size: Size) -> f64 {
        let Some(amount) = self.scale else {
            return 1.0;
        };
        let Some(cap) = self.max_growth else {
            return 1.0 + amount;
        };
        let half_max = 0.5 * size.width.max(size.height);
        if !half_max.is_finite() || half_max <= 0.0 {
            return 1.0 + amount;
        }
        let limit = f64::from(cap.max(0.0)) / half_max;
        1.0 + amount.clamp(-limit, limit)
    }

    /// The wash, if the press shows one.
    #[must_use]
    pub const fn get_wash(&self) -> Option<&Wash> {
        self.wash.as_ref()
    }

    /// The drag deformation, if the press deforms under drag.
    #[must_use]
    pub const fn get_drag_deformation(&self) -> Option<&DragDeformation> {
        self.drag_deformation.as_ref()
    }
}

/// What keyboard focus shows, beyond the state layer's
/// [`StateLayer::focus`] opacity.
#[derive(Debug, Clone)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each focus indicator variant by variant; a new one must fail their build instead of falling into a wildcard arm"
)]
pub enum FocusIndicator {
    /// A ring following the silhouette — its morph and press scale
    /// included — fading in and out with the focus transition.
    Ring {
        /// Stroke width.
        width: f32,
        /// Gap between the surface's edge and the ring's inner edge.
        offset: f32,
        /// Ring colour.
        color: Color,
    },
    /// The state layer's focus opacity alone.
    Tint,
}

#[derive(Clone)]
struct ChromeParts {
    surface: Surface,
    state_layer: StateLayer,
    press: PressFeedback,
    focus: FocusIndicator,
    motion: Motion,
    hit: HitRegion,
    /// A fixed frame size and its alignment in the widget's bounds; `None`
    /// fills them.
    size: Option<(Size, Vec2)>,
    customs: Vec<Rc<dyn CustomPart<()>>>,
    keyboard_focusable: bool,
}

/// One interactive part's chrome, as engine-provided primitives: its
/// surface, state layer, press feedback, focus indicator, motion and hit
/// region.
///
/// The surface and the custom parts draw beneath the widget's content; the
/// state layer, the press feedback and the focus indicator draw above it.
/// All of it lies in the chrome's frame: the widget's bounds, or a fixed
/// size placed in them ([`Self::centered`], [`Self::sized`]) — an icon
/// button's container inside its touch target.
///
/// Installed as a plugin, a chrome is the presentation of every button and
/// tappable view in the subtree: a composed control — a chip, a navigation
/// destination, a list row — keeps its semantics and content while the
/// chrome supplies its interaction feedback. A clone is the same chrome.
#[derive(Clone)]
pub struct Chrome(Rc<ChromeParts>);

impl core::fmt::Debug for Chrome {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Chrome")
            .field("surface", &self.0.surface)
            .field("state_layer", &self.0.state_layer)
            .field("press", &self.0.press)
            .field("focus", &self.0.focus)
            .field("motion", &self.0.motion)
            .field("hit", &self.0.hit)
            .field("size", &self.0.size)
            .field("customs", &self.0.customs.len())
            .field("keyboard_focusable", &self.0.keyboard_focusable)
            .finish()
    }
}

impl Plugin for Chrome {}

impl Chrome {
    /// Chrome filling the widget's bounds, running with `motion`.
    #[must_use]
    pub fn new(
        surface: Surface,
        state_layer: StateLayer,
        press: PressFeedback,
        focus: FocusIndicator,
        motion: Motion,
    ) -> Self {
        Self(Rc::new(ChromeParts {
            surface,
            state_layer,
            press,
            focus,
            motion,
            hit: HitRegion::NONE,
            size: None,
            customs: Vec::new(),
            keyboard_focusable: true,
        }))
    }

    fn parts(&mut self) -> &mut ChromeParts {
        Rc::make_mut(&mut self.0)
    }

    /// Lays the chrome out at `size`, centred in the widget's bounds and
    /// capped to them.
    #[must_use]
    pub fn centered(self, size: Size) -> Self {
        self.sized(size, Vec2::new(0.5, 0.5))
    }

    /// Lays the chrome out at `size`, capped to the widget's bounds, placed
    /// at `alignment` of the room left in them on each axis — `0` at the
    /// leading or top edge, `1` at the trailing or bottom one.
    #[must_use]
    pub fn sized(mut self, size: Size, alignment: Vec2) -> Self {
        self.parts().size = Some((size, alignment));
        self
    }

    /// Takes touches in `hit`.
    #[must_use]
    pub fn hit(mut self, hit: HitRegion) -> Self {
        self.parts().hit = hit;
        self
    }

    /// Records `part` over the surface, beneath the widget's content.
    #[must_use]
    pub fn custom<D: DrivenSet<()> + 'static>(mut self, part: Custom<(), D>) -> Self
    where
        D::Live: 'static,
    {
        self.parts().customs.push(Rc::new(part));
        self
    }

    /// Takes pointer presses only, outside keyboard focus traversal — a
    /// modal scrim.
    #[must_use]
    pub fn pointer_only(mut self) -> Self {
        self.parts().keyboard_focusable = false;
        self
    }

    /// Sets the press feedback.
    pub fn set_press(&mut self, press: PressFeedback) {
        self.parts().press = press;
    }

    /// The container.
    #[must_use]
    pub fn surface(&self) -> &Surface {
        &self.0.surface
    }

    /// The interaction tint.
    #[must_use]
    pub fn state_layer(&self) -> &StateLayer {
        &self.0.state_layer
    }

    /// What a press shows.
    #[must_use]
    pub fn press(&self) -> &PressFeedback {
        &self.0.press
    }

    /// What keyboard focus shows.
    #[must_use]
    pub fn focus(&self) -> &FocusIndicator {
        &self.0.focus
    }

    /// The motion the chrome's values run with.
    #[must_use]
    pub fn motion(&self) -> &Motion {
        &self.0.motion
    }

    /// Where the chrome takes touches.
    #[must_use]
    pub fn get_hit(&self) -> &HitRegion {
        &self.0.hit
    }

    /// Whether the chrome joins keyboard focus traversal.
    #[must_use]
    pub fn keyboard_focusable(&self) -> bool {
        self.0.keyboard_focusable
    }

    /// The custom parts.
    #[doc(hidden)]
    #[must_use]
    pub fn customs(&self) -> &[Rc<dyn CustomPart<()>>] {
        &self.0.customs
    }

    /// Whether `other` is this chrome.
    #[doc(hidden)]
    #[must_use]
    pub fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// The chrome's frame inside the widget's `bounds`.
    #[must_use]
    pub fn frame(&self, bounds: Rect) -> Rect {
        self.0.size.map_or(bounds, |(size, alignment)| {
            let size = Size::new(
                size.width.min(bounds.width()),
                size.height.min(bounds.height()),
            );
            let x0 = (bounds.width() - size.width).mul_add(alignment.x.clamp(0.0, 1.0), bounds.x0);
            let y0 =
                (bounds.height() - size.height).mul_add(alignment.y.clamp(0.0, 1.0), bounds.y0);
            Rect::from_origin_size((x0, y0), size)
        })
    }

    /// The touch target of a widget laid out at `bounds`.
    #[must_use]
    pub fn target(&self, bounds: Rect) -> Rect {
        self.0.hit.target(bounds, self.frame(bounds))
    }
}

/// Resolves unit-space [`waterui_shape::PathCommand`]s against `bounds`.
#[doc(hidden)]
#[must_use]
pub fn path_commands_to_path(
    commands: &[waterui_shape::PathCommand],
    bounds: Rect,
) -> kurbo::BezPath {
    use waterui_core::contract_violation;
    use waterui_shape::PathCommand;

    let width = bounds.width();
    let height = bounds.height();
    let mut path = kurbo::BezPath::new();
    let mut has_current = false;

    for command in commands {
        match command {
            PathCommand::MoveTo { x, y } => {
                path.move_to(Point::new(f64::from(*x) * width, f64::from(*y) * height));
                has_current = true;
            }
            PathCommand::LineTo { x, y } => {
                if !has_current {
                    contract_violation!(
                        "PathCommand::LineTo requires an active current point; the command is ignored"
                    );
                    continue;
                }
                path.line_to(Point::new(f64::from(*x) * width, f64::from(*y) * height));
            }
            PathCommand::QuadTo { cx, cy, x, y } => {
                if !has_current {
                    contract_violation!(
                        "PathCommand::QuadTo requires an active current point; the command is ignored"
                    );
                    continue;
                }
                path.quad_to(
                    Point::new(f64::from(*cx) * width, f64::from(*cy) * height),
                    Point::new(f64::from(*x) * width, f64::from(*y) * height),
                );
            }
            PathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => {
                if !has_current {
                    contract_violation!(
                        "PathCommand::CubicTo requires an active current point; the command is ignored"
                    );
                    continue;
                }
                path.curve_to(
                    Point::new(f64::from(*c1x) * width, f64::from(*c1y) * height),
                    Point::new(f64::from(*c2x) * width, f64::from(*c2y) * height),
                    Point::new(f64::from(*x) * width, f64::from(*y) * height),
                );
            }
            PathCommand::Arc {
                cx,
                cy,
                rx,
                ry,
                start,
                sweep,
            } => {
                let center_x = f64::from(*cx) * width;
                let center_y = f64::from(*cy) * height;
                let radius_x = f64::from(*rx) * width;
                let radius_y = f64::from(*ry) * height;
                let start = f64::from(start.to_radians());
                let step = f64::from(sweep.to_radians()) / 32.0;

                let start_point = Point::new(
                    radius_x.mul_add(start.cos(), center_x),
                    radius_y.mul_add(start.sin(), center_y),
                );
                if has_current {
                    path.line_to(start_point);
                } else {
                    path.move_to(start_point);
                    has_current = true;
                }

                let mut angle = start;
                for _ in 0..32 {
                    angle += step;
                    path.line_to(Point::new(
                        radius_x.mul_add(angle.cos(), center_x),
                        radius_y.mul_add(angle.sin(), center_y),
                    ));
                }
            }
            PathCommand::Close => {
                path.close_path();
                has_current = false;
            }
        }
    }

    path
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;
    use kurbo::{Affine, Point, Rect, Size, Vec2};

    use super::{DragDeformation, PressFeedback};

    #[test]
    fn press_feedback_max_growth_caps_wide_frames_without_distorting_aspect() {
        let uncapped = PressFeedback::new().scale(0.25);
        assert_relative_eq!(uncapped.effective_scale(Size::new(300.0, 44.0)), 1.25);

        let capped = PressFeedback::new().scale(0.25).max_growth(10.0);
        // Compact 44x44: half_max = 22, 0.25 * 22 = 5.5 <= 10.0 -> uncapped 1.25
        assert_relative_eq!(capped.effective_scale(Size::new(44.0, 44.0)), 1.25);
        // Wide 300x44: half_max = 150, 10.0 / 150.0 = 1/15 -> 1 + 1/15
        let wide_scale = capped.effective_scale(Size::new(300.0, 44.0));
        assert_relative_eq!(wide_scale, 1.0 + 10.0 / 150.0, epsilon = 1e-12);
        // Half-width delta equals exactly 10.0 pt
        assert_relative_eq!(150.0 * (wide_scale - 1.0), 10.0, epsilon = 1e-12);
    }

    #[test]
    fn drag_deformation_preserves_area_and_bounds_lag() {
        let deform = DragDeformation::new()
            .lag(6.0, 40.0)
            .displacement_stretch(0.10, 60.0)
            .velocity_stretch(0.12, 800.0)
            .stretch_bounds(0.85, 1.25);

        // Rest state is identity.
        assert_eq!(deform.lag_offset(Vec2::ZERO), Vec2::ZERO);
        assert_eq!(deform.stretch(Vec2::ZERO, Vec2::ZERO), (1.0, 1.0));
        assert_eq!(
            deform.transform(Point::new(50.0, 20.0), Vec2::ZERO, Vec2::ZERO),
            Affine::IDENTITY
        );

        // Large drag asymptotically approaches max_lag without exceeding it.
        let far_lag = deform.lag_offset(Vec2::new(1000.0, 0.0));
        assert!(far_lag.x > 5.99 && far_lag.x <= 6.0);
        assert_relative_eq!(far_lag.y, 0.0, epsilon = 1e-12);

        // Area preservation for arbitrary angle drag + velocity.
        for (drag, vel) in [
            (Vec2::new(40.0, 0.0), Vec2::new(600.0, 0.0)),
            (Vec2::new(0.0, -50.0), Vec2::new(0.0, -400.0)),
            (Vec2::new(30.0, 40.0), Vec2::new(500.0, -250.0)),
        ] {
            let (par, perp) = deform.stretch(drag, vel);
            assert!(par > 1.0 && par <= 1.25);
            assert_relative_eq!(par * perp, 1.0, epsilon = 1e-12);

            let (sx, sy) = deform.axis_scales(drag, vel);
            assert_relative_eq!(sx * sy, 1.0, epsilon = 1e-12);

            let affine = deform.transform(Point::new(50.0, 20.0), drag, vel);
            assert_relative_eq!(affine.determinant(), 1.0, epsilon = 1e-12);

            let frame = Rect::from_origin_size((10.0, 10.0), (80.0, 40.0));
            let deformed = deform.deformed_rect(frame, drag, vel);
            assert_relative_eq!(deformed.area(), frame.area(), epsilon = 1e-9);
        }
    }
}
