//! The engine-provided chrome primitives of the theme contract: the
//! interaction machinery every interactive theme otherwise hand-writes.
//!
//! A theme describes a widget's chrome as data — a [`Surface`] with its
//! [`CornerShape`], [`Elevation`] and [`Border`], the [`StateLayer`]
//! opacities it states as tokens, the [`PressFeedback`] a press shows and
//! the [`FocusIndicator`] keyboard focus shows — and Hydrolysis realizes it.
//! The realization records each primitive once. Hover, press and focus
//! change only live operands of that recording — the state-layer opacity,
//! each ripple wave's geometry and opacity, the focus indicator's opacity,
//! the pressed corner morph and every [`StateValue`] the surface resolves —
//! and the engine animates them with the theme's [`InteractionMotion`], so
//! an interaction change records nothing, walks no Hydrolysis tree and
//! runs no layout pass.
//!
//! [`Chrome`] bundles the primitives for one widget part; a theme hands it
//! out through hooks such as
//! [`WidgetTheme::button_chrome`](super::WidgetTheme::button_chrome).

use cherenkov_record::{ContinuousRect, Paint, Shadow, ShapeData, WorkingColor};
use kurbo::{Circle, Point, Rect, RoundedRect, RoundedRectRadii, Size};
use waterui_core::interaction::{InteractionState, StateValue};

use super::InteractionMotion;

/// The radii of one silhouette of a [`CornerShape`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed: a corner is either a radius per corner or fully rounded"
)]
pub enum Corners {
    /// A radius per corner, in logical units, clamped to half the shorter
    /// side.
    Radii(RoundedRectRadii),
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

impl From<f64> for Corners {
    fn from(radius: f64) -> Self {
        Self::Radii(RoundedRectRadii::from_single_radius(radius))
    }
}

/// A surface's silhouette: its corners at rest, the corners it morphs to
/// while physically pressed, and whether the corners are circular or
/// continuous.
///
/// The press morph is a live operand: it moves towards the pressed corners
/// while the widget is pressed and back on release, with
/// [`InteractionMotion::press_shape`] in both directions — unlike a ripple
/// wave it plays backwards on release. The state layer, the ripple clip and
/// the focus ring follow the morph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CornerShape {
    rest: Corners,
    pressed: Option<Corners>,
    /// `Some` for continuous corners, carrying the curvature transition's
    /// extent as a fraction of each radius ([`ContinuousRect::smoothing`]).
    smoothing: Option<f64>,
}

impl CornerShape {
    /// Circular `corners` at rest, with no press morph.
    #[must_use]
    pub const fn new(corners: Corners) -> Self {
        Self {
            rest: corners,
            pressed: None,
            smoothing: None,
        }
    }

    /// One circular radius on every corner.
    #[must_use]
    pub const fn radius(radius: f64) -> Self {
        Self::new(Corners::Radii(RoundedRectRadii::from_single_radius(radius)))
    }

    /// Fully rounded corners: a pill, or a circle on a square surface.
    #[must_use]
    pub const fn full() -> Self {
        Self::new(Corners::Full)
    }

    /// Morphs to `corners` while the widget is physically pressed.
    #[must_use]
    pub const fn pressed(mut self, corners: Corners) -> Self {
        self.pressed = Some(corners);
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

    /// The corners at rest.
    #[must_use]
    pub const fn rest(&self) -> Corners {
        self.rest
    }

    /// The corners while pressed: the press morph's target, or the resting
    /// corners when the shape does not morph.
    #[must_use]
    pub fn pressed_corners(&self) -> Corners {
        self.pressed.unwrap_or(self.rest)
    }

    /// The silhouette on `rect`, pressed or at rest.
    #[must_use]
    pub fn shape(&self, rect: Rect, pressed: bool) -> ShapeData {
        self.outset(rect, 0.0, pressed)
    }

    /// The silhouette on `rect` grown outward by `distance` (shrunk inward
    /// when negative), its radii grown with it so the edge stays parallel —
    /// the shape a focus ring around the surface, or a border inside it,
    /// follows.
    #[must_use]
    pub fn outset(&self, rect: Rect, distance: f64, pressed: bool) -> ShapeData {
        let corners = if pressed {
            self.pressed_corners()
        } else {
            self.rest
        };
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
/// the pair for each level; per interaction state through
/// [`Surface::elevation`], so a surface that raises on hover animates
/// between the two pairs as live operands.
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
            offset: kurbo::Vec2::ZERO,
            spread: 0.0,
            color: WorkingColor::TRANSPARENT,
        },
        ambient: Shadow {
            sigma: 0.0,
            offset: kurbo::Vec2::ZERO,
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
}

/// A stroke drawn just inside a surface's edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Border {
    /// Stroke width in logical units; the stroke lies wholly inside the
    /// surface's silhouette.
    pub width: f64,
    /// The stroke's paint.
    pub paint: Paint,
}

impl Border {
    /// A border of `width` painted with `paint`.
    #[must_use]
    pub fn new(width: f64, paint: impl Into<Paint>) -> Self {
        Self {
            width,
            paint: paint.into(),
        }
    }
}

/// A widget's container: its silhouette, fill, border and elevation.
///
/// Fill, border and elevation are [`StateValue`]s resolved against the
/// widget's interaction state; a change of the resolved value under hover,
/// press or focus is a live operand animated with the transition's
/// [`InteractionMotion`] animation.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Surface {
    /// The silhouette every chrome layer follows.
    pub shape: CornerShape,
    /// The container fill per interaction state.
    pub fill: StateValue<Paint>,
    /// The border per interaction state; `None` draws none.
    pub border: Option<StateValue<Border>>,
    /// The elevation per interaction state.
    pub elevation: StateValue<Elevation>,
}

impl Surface {
    /// The silhouette every chrome layer follows.
    #[must_use]
    pub const fn shape(&self) -> CornerShape {
        self.shape
    }

    /// The container fill per interaction state.
    #[must_use]
    pub const fn fill(&self) -> &StateValue<Paint> {
        &self.fill
    }

    /// The border per interaction state; `None` draws none.
    #[must_use]
    pub const fn get_border(&self) -> Option<&StateValue<Border>> {
        self.border.as_ref()
    }

    /// The elevation per interaction state.
    #[must_use]
    pub const fn get_elevation(&self) -> &StateValue<Elevation> {
        &self.elevation
    }
}

impl Surface {
    /// A flat, borderless surface of `shape` filled with `fill`.
    #[must_use]
    pub fn new(shape: CornerShape, fill: impl Into<StateValue<Paint>>) -> Self {
        Self {
            shape,
            fill: fill.into(),
            border: None,
            elevation: StateValue::new(Elevation::NONE),
        }
    }

    /// Strokes `border` inside the surface's edge.
    #[must_use]
    pub fn border(mut self, border: impl Into<StateValue<Border>>) -> Self {
        self.border = Some(border.into());
        self
    }

    /// Casts `elevation`'s shadows.
    #[must_use]
    pub fn elevation(mut self, elevation: impl Into<StateValue<Elevation>>) -> Self {
        self.elevation = elevation.into();
        self
    }
}

/// The tint a widget lays over its surface to show its interaction state:
/// a colour and the opacity a theme states for each state as a token.
///
/// The layer shows the opacity of the dominant state: dragged, then pressed
/// when the press feedback is [`PressFeedback::Tint`], then focused, then
/// hovered — focus outranks hover. A [`PressFeedback::Ripple`] shows the
/// press as waves above the layer instead, at [`Self::pressed`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StateLayer {
    /// The layer's colour; each opacity multiplies its alpha.
    pub color: WorkingColor,
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
    pub const fn new(color: WorkingColor) -> Self {
        Self {
            color,
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
        } else if state.contains(InteractionState::PRESSED) && matches!(press, PressFeedback::Tint)
        {
            self.pressed
        } else if state.contains(InteractionState::FOCUSED) {
            self.focus
        } else if state.contains(InteractionState::HOVERED) {
            self.hover
        } else {
            0.0
        }
    }

    /// The layer's paint at `opacity`.
    #[must_use]
    pub fn paint(&self, opacity: f32) -> Paint {
        Paint::from(self.color.with_alpha(self.color.components[3] * opacity))
    }
}

/// What a press shows.
#[derive(Debug, Clone, Copy, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each press feedback variant by variant; a new one must fail their build instead of falling into a wildcard arm"
)]
pub enum PressFeedback {
    /// A wave per press, growing from the press point. Rapid re-presses
    /// overlap: older waves keep fading while the newest grows, up to
    /// [`MAX_PRESS_WAVES`](super::MAX_PRESS_WAVES) at once.
    Ripple(Ripple),
    /// The state layer at [`StateLayer::pressed`] while pressed.
    Tint,
}

/// Ripple geometry, from the parameters a theme supplies.
///
/// A wave starts as a circle of `initial_scale` of the full diameter
/// centred on the press point (the frame's centre for a keyboard or
/// synthetic press), and grows to the full diameter — the frame's diagonal,
/// floored at `min_diameter` — while its centre drifts `drift` of the way
/// from the press point to the frame's centre, under
/// [`InteractionMotion::press_grow`]. Its opacity rises to
/// [`StateLayer::pressed`] under [`InteractionMotion::press_fade_in`] and,
/// once released and the [`InteractionMotion::minimum_press_duration`]
/// has passed, falls to `0` under [`InteractionMotion::press_fade_out`].
/// A wave stays recorded for the widget's life, so its fade runs until the
/// animation settles — a spring that undershoots is never cut off at its
/// first zero crossing. Waves are clipped to the surface's silhouette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ripple {
    /// The smallest full diameter, so small targets still show a ripple.
    pub min_diameter: f64,
    /// The wave's starting diameter as a fraction of the full diameter.
    pub initial_scale: f64,
    /// How far the wave's centre drifts from the press point towards the
    /// frame's centre as it grows: `0` stays on the press point, `1` ends
    /// centred.
    pub drift: f64,
}

impl Ripple {
    /// The full diameter of a wave on `frame`.
    #[must_use]
    pub fn diameter(&self, frame: Rect) -> f64 {
        frame.width().hypot(frame.height()).max(self.min_diameter)
    }

    /// A wave's starting and final circles on `frame` for a press at
    /// `origin`, in the frame's space; `None` presses at the centre.
    #[must_use]
    pub fn wave(&self, frame: Rect, origin: Option<Point>) -> (Circle, Circle) {
        let center = frame.center();
        let origin = origin.unwrap_or(center);
        let radius = self.diameter(frame) / 2.0;
        let end = origin.lerp(center, self.drift);
        (
            Circle::new(origin, radius * self.initial_scale),
            Circle::new(end, radius),
        )
    }
}

/// What keyboard focus shows, beyond the state layer's
/// [`StateLayer::focus`] opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each focus indicator variant by variant; a new one must fail their build instead of falling into a wildcard arm"
)]
pub enum FocusIndicator {
    /// A ring following the surface's silhouette — its press morph
    /// included — fading in and out with
    /// [`InteractionMotion::focus_enter`] and
    /// [`InteractionMotion::focus_exit`].
    Ring {
        /// Stroke width.
        width: f64,
        /// Gap between the surface's edge and the ring's inner edge.
        offset: f64,
        /// Ring colour.
        color: WorkingColor,
    },
    /// The state layer's focus opacity alone.
    Tint,
}

/// One widget part's chrome, as engine-provided primitives.
///
/// The surface draws beneath the widget's content; the state layer, the
/// press waves and the focus indicator draw above it. All of it lies in the
/// chrome's frame: the widget's bounds, or a fixed size centred in them
/// ([`Self::centered`]) — an icon button's container inside its touch
/// target.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Chrome {
    /// The container.
    pub surface: Surface,
    /// The interaction tint.
    pub state_layer: StateLayer,
    /// What a press shows.
    pub press: PressFeedback,
    /// What keyboard focus shows.
    pub focus: FocusIndicator,
    /// A fixed frame size centred in the widget's bounds; `None` fills
    /// them.
    pub size: Option<Size>,
    /// The motion this widget's interaction feedback runs with; `None`
    /// uses the theme's [`WidgetTheme::interaction_motion`](super::WidgetTheme::interaction_motion).
    pub motion: Option<InteractionMotion>,
}

impl Chrome {
    /// The container.
    #[must_use]
    pub const fn surface(&self) -> &Surface {
        &self.surface
    }

    /// The interaction tint.
    #[must_use]
    pub const fn state_layer(&self) -> StateLayer {
        self.state_layer
    }

    /// What a press shows.
    #[must_use]
    pub const fn press(&self) -> PressFeedback {
        self.press
    }

    /// Sets the `press`.
    pub const fn set_press(&mut self, press: PressFeedback) {
        self.press = press;
    }

    /// What keyboard focus shows.
    #[must_use]
    pub const fn focus(&self) -> FocusIndicator {
        self.focus
    }

    /// A fixed frame size centred in the widget's bounds; `None` fills
    /// them.
    #[must_use]
    pub const fn size(&self) -> Option<Size> {
        self.size
    }

    /// The motion this widget's interaction feedback runs with; `None`
    /// uses the theme's [`WidgetTheme::interaction_motion`](super::WidgetTheme::interaction_motion).
    #[must_use]
    pub const fn get_motion(&self) -> Option<&InteractionMotion> {
        self.motion.as_ref()
    }
}

impl Chrome {
    /// Chrome filling the widget's bounds with the theme-wide motion.
    #[must_use]
    pub const fn new(
        surface: Surface,
        state_layer: StateLayer,
        press: PressFeedback,
        focus: FocusIndicator,
    ) -> Self {
        Self {
            surface,
            state_layer,
            press,
            focus,
            size: None,
            motion: None,
        }
    }

    /// Lays the chrome out at `size`, centred in the widget's bounds and
    /// capped to them.
    #[must_use]
    pub const fn centered(mut self, size: Size) -> Self {
        self.size = Some(size);
        self
    }

    /// Runs this widget's interaction feedback with `motion` instead of the
    /// theme-wide motion.
    #[must_use]
    pub const fn motion(mut self, motion: InteractionMotion) -> Self {
        self.motion = Some(motion);
        self
    }

    /// The chrome's frame inside the widget's `bounds`.
    #[must_use]
    pub fn frame(&self, bounds: Rect) -> Rect {
        self.size.map_or(bounds, |size| {
            let size = Size::new(
                size.width.min(bounds.width()),
                size.height.min(bounds.height()),
            );
            Rect::from_center_size(bounds.center(), size)
        })
    }
}
