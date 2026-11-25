//! Gradient views: a Cherenkov [`Paint`] filling the view's bounds.
//!
//! A gradient is authored in unit space — `[0, 1]` on both axes, `(0, 0)` the
//! top-left of the view — and the backend maps it onto the bounds it lays the
//! view out at. A radial radius is a circle, never an ellipse: on a `w` × `h`
//! box a radius `r` is a circle of `r * min(w, h)` points, so a radius of `0.5`
//! from the centre reaches the nearer edge and stops short of the farther one
//! on a non-square box.

extern crate alloc;

use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use crate::draw::kurbo::{Affine, Point};
use crate::draw::{
    ColorStop, LinearGradient, MeshColorInterpolation, MeshGradient as DrawMeshGradient, Paint,
    RadialGradient, SweepGradient, WorkingColor,
};
use nami::watcher::{BoxWatcherGuard, Context, WatcherGuard};
use nami::{Computed, Signal, SignalExt};
use waterui_color::Color;
use waterui_core::layout::{StretchAxis, UnitPoint};
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Angle, Environment, View, contract_violation, flatten_signal};

/// The family of a [`Gradient`], for backends that map it onto a platform type.
#[repr(u32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum GradientType {
    /// Colours run along a line.
    #[default]
    Linear = 0,
    /// Colours run outward from a centre.
    Radial = 1,
    /// Colours run around a centre.
    Angular = 2,
}

nami::impl_constant!(GradientType);

/// Ordered `(offset, color)` stops for a [`Gradient`].
///
/// Built from explicit `(offset, Color)` stops (`Vec<(f32, Color)>`) or evenly
/// spaced colour arrays (`[Color; N]`, or [`GradientStops::evenly_spaced`] for
/// dynamic collections). Any [`Signal`] yielding one of those types satisfies
/// [`IntoComputed<GradientStops>`].
#[derive(Debug, Clone, Default)]
pub struct GradientStops(Vec<(f32, Color)>);

impl GradientStops {
    /// Stops at explicit unit-space offsets in `[0, 1]`.
    #[must_use]
    pub const fn new(stops: Vec<(f32, Color)>) -> Self {
        Self(stops)
    }

    /// Evenly spaced stops across `[0, 1]`.
    #[must_use]
    pub fn evenly_spaced(colors: impl IntoIterator<Item = Color>) -> Self {
        let colors: Vec<Color> = colors.into_iter().collect();
        let len = colors.len();
        let stops = match len {
            0 => Vec::new(),
            1 => colors.into_iter().map(|color| (0.0, color)).collect(),
            _ => {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "gradient stop counts are far below 2^24, where every usize index is an exact f32"
                )]
                let denom = (len - 1) as f32;
                colors
                    .into_iter()
                    .enumerate()
                    .map(|(index, color)| {
                        #[expect(
                            clippy::cast_precision_loss,
                            reason = "gradient stop counts are far below 2^24, where every usize index is an exact f32"
                        )]
                        let offset = if index + 1 == len {
                            1.0
                        } else {
                            index as f32 / denom
                        };
                        (offset, color)
                    })
                    .collect()
            }
        };
        Self(stops)
    }
}

nami::impl_constant!(GradientStops);

impl From<Vec<(f32, Color)>> for GradientStops {
    fn from(stops: Vec<(f32, Color)>) -> Self {
        Self(stops)
    }
}

impl<const N: usize> From<[Color; N]> for GradientStops {
    fn from(colors: [Color; N]) -> Self {
        Self::evenly_spaced(colors)
    }
}

#[derive(Clone)]
struct AllSignals<T> {
    signals: Rc<[Computed<T>]>,
}

struct AllSignalsGuard {
    _guards: Vec<BoxWatcherGuard>,
}

impl WatcherGuard for AllSignalsGuard {}

impl<T: Clone + 'static> Signal for AllSignals<T> {
    type Output = Vec<T>;
    type Guard = AllSignalsGuard;

    fn snapshot(&self) -> Self::Output {
        self.signals.iter().map(Signal::snapshot).collect()
    }

    fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {
        let watcher = Rc::new(watcher);
        let guards = self
            .signals
            .iter()
            .enumerate()
            .map(|(index, signal)| {
                let watcher = Rc::clone(&watcher);
                let signals = Rc::clone(&self.signals);
                signal.watch(move |ctx| {
                    let metadata = ctx.metadata().clone();
                    let next_value = ctx.into_value();
                    let snapshot = signals
                        .iter()
                        .enumerate()
                        .map(|(i, s)| {
                            if i == index {
                                next_value.clone()
                            } else {
                                s.snapshot()
                            }
                        })
                        .collect();
                    watcher(Context::new(snapshot, metadata));
                })
            })
            .collect();
        AllSignalsGuard { _guards: guards }
    }
}

/// Combines a list of signals into one signal producing their resolved values.
pub(super) fn all_signals<T: Clone + 'static>(signals: Vec<Computed<T>>) -> Computed<Vec<T>> {
    Computed::new(AllSignals {
        signals: signals.into(),
    })
}

/// Resolves a reactive list of [`Color`] values in `env`.
pub(super) fn resolve_colors(
    colors: &Computed<Vec<Color>>,
    env: &Environment,
) -> Computed<Vec<WorkingColor>> {
    let env = env.clone();
    flatten_signal(colors.map(move |colors: Vec<Color>| {
        let signals = colors
            .into_iter()
            .map(|color| color.resolve(&env))
            .collect();
        all_signals(signals)
    }))
}

fn resolve_stops(
    stops: &Computed<GradientStops>,
    env: &Environment,
) -> Computed<Vec<(f32, WorkingColor)>> {
    let env = env.clone();
    flatten_signal(stops.map(move |GradientStops(entries)| {
        let (offsets, color_signals): (Vec<f32>, Vec<Computed<WorkingColor>>) = entries
            .into_iter()
            .map(|(offset, color)| (offset, color.resolve(&env)))
            .unzip();
        all_signals(color_signals)
            .map(move |colors| offsets.iter().copied().zip(colors).collect::<Vec<_>>())
            .computed()
    }))
}

fn stops(stops: Vec<(f32, WorkingColor)>) -> Vec<ColorStop> {
    let given = stops.len();
    if given == 0 {
        contract_violation!(
            "a gradient must contain at least one stop; a single transparent stop is used"
        );
    }
    let mut stops = stops
        .into_iter()
        .filter_map(|(offset, color)| {
            if !color
                .components
                .iter()
                .all(|component| component.is_finite())
            {
                contract_violation!(
                    "gradient stop colour must be finite, got {color:?}; the stop is dropped"
                );
                return None;
            }
            Some(ColorStop {
                offset: stop_offset(offset),
                color,
            })
        })
        .collect::<Vec<_>>();
    if stops.is_empty() {
        stops.push(ColorStop {
            offset: 0.0,
            color: WorkingColor::TRANSPARENT,
        });
    }
    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    stops
}

/// `offset` when it lies within `[0, 1]`; otherwise the nearest end, `0` for
/// `NaN`.
fn stop_offset(offset: f32) -> f32 {
    if (0.0..=1.0).contains(&offset) {
        return offset;
    }
    contract_violation!(
        "gradient stop position must be within [0, 1], got {offset}; the nearest end of the range is used"
    );
    if offset.is_nan() {
        0.0
    } else {
        offset.clamp(0.0, 1.0)
    }
}

pub(super) fn point(unit: UnitPoint, name: &str) -> Point {
    let finite = |value: f32, axis: &str| {
        if value.is_finite() {
            f64::from(value)
        } else {
            contract_violation!("{name}.{axis} must be finite; 0 is used");
            0.0
        }
    };
    Point::new(finite(unit.x, "x"), finite(unit.y, "y"))
}

/// The mesh a gradient with an invalid grid draws: one transparent patch.
fn empty_mesh() -> DrawMeshGradient {
    DrawMeshGradient::new(
        1,
        1,
        vec![
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(0.0, 1.0),
            Point::new(1.0, 1.0),
        ],
        vec![WorkingColor::TRANSPARENT; 4],
    )
    .expect("a one-patch grid with four points and four colours is a valid mesh")
}

/// The mesh paint over a `columns` × `rows` grid of unit-space control points
/// and their colours, row-major.
///
/// A grid with fewer than two vertices on a side, or a list that does not
/// hold exactly `columns * rows` entries, is a contract violation: the mesh
/// is one transparent patch.
pub(super) fn mesh_paint(
    columns: u32,
    rows: u32,
    points: Vec<UnitPoint>,
    colors: Vec<WorkingColor>,
    smooths_colors: bool,
) -> DrawMeshGradient {
    if columns < 2 || rows < 2 {
        contract_violation!(
            "mesh gradients need at least a 2x2 grid of vertices, got {columns}x{rows}; the mesh is one transparent patch"
        );
        return empty_mesh();
    }
    let vertices = columns as usize * rows as usize;
    if points.len() != vertices {
        contract_violation!(
            "mesh gradients require exactly columns*rows control points, got {} for {vertices}; the mesh is one transparent patch",
            points.len()
        );
        return empty_mesh();
    }
    if colors.len() != vertices {
        contract_violation!(
            "mesh gradients require exactly columns*rows colours, got {} for {vertices}; the mesh is one transparent patch",
            colors.len()
        );
        return empty_mesh();
    }
    let points = points
        .into_iter()
        .map(|position| point(position, "mesh gradient vertex"))
        .collect();
    let colors = colors
        .into_iter()
        .map(|color| {
            if color
                .components
                .iter()
                .all(|component| component.is_finite())
            {
                color
            } else {
                contract_violation!(
                    "mesh gradient vertex colour must be finite, got {color:?}; transparent is used"
                );
                WorkingColor::TRANSPARENT
            }
        })
        .collect();
    DrawMeshGradient::new(columns - 1, rows - 1, points, colors)
        .expect("the checks above guarantee a non-empty grid with one entry per vertex")
        .interpolation(if smooths_colors {
            MeshColorInterpolation::Smoothstep
        } else {
            MeshColorInterpolation::Linear
        })
}

#[derive(Debug, Clone)]
enum GradientGeometry {
    Linear {
        start: Point,
        end: Point,
    },
    Radial {
        center: Point,
        start_radius: f32,
        end_radius: f32,
    },
    Angular {
        center: Point,
        start_angle: Angle,
        end_angle: Angle,
    },
}

impl GradientGeometry {
    const fn gradient_type(&self) -> GradientType {
        match self {
            Self::Linear { .. } => GradientType::Linear,
            Self::Radial { .. } => GradientType::Radial,
            Self::Angular { .. } => GradientType::Angular,
        }
    }

    fn build_paint(&self, stops: Vec<ColorStop>) -> Paint {
        match *self {
            Self::Linear { start, end } => {
                let mut gradient = LinearGradient::new(start, end);
                gradient.stops = stops;
                Paint::Linear(gradient)
            }
            Self::Radial {
                center,
                start_radius,
                end_radius,
            } => {
                let mut gradient = RadialGradient::two_point(
                    center,
                    f64::from(start_radius),
                    center,
                    f64::from(end_radius),
                );
                gradient.stops = stops;
                Paint::Radial(gradient)
            }
            Self::Angular {
                center,
                start_angle,
                end_angle,
            } => {
                let mut gradient = SweepGradient::new(
                    center,
                    f64::from(start_angle.to_radians()),
                    f64::from(end_angle.to_radians()),
                );
                gradient.stops = stops;
                Paint::Sweep(gradient)
            }
        }
    }
}

/// A gradient view: a reactive [`Paint`] in unit space that fills the view's bounds.
///
/// Stop colours resolve in the view's [`Environment`] and rebind in place when
/// the colour scheme or stops signal changes.
///
/// # Layout Behavior
///
/// A gradient is a greedy view: it expands to fill all available space on
/// both axes. Constrain it with `.frame()` or use it as a background.
#[derive(Clone)]
pub struct Gradient {
    geometry: GradientGeometry,
    stops: Computed<GradientStops>,
}

impl fmt::Debug for Gradient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Gradient")
            .field("geometry", &self.geometry)
            .finish_non_exhaustive()
    }
}

impl Gradient {
    /// A gradient whose colours run along the line from `start` to `end`.
    #[must_use]
    pub fn linear(
        stops: impl IntoComputed<GradientStops>,
        start: UnitPoint,
        end: UnitPoint,
    ) -> Self {
        Self {
            geometry: GradientGeometry::Linear {
                start: point(start, "linear gradient start"),
                end: point(end, "linear gradient end"),
            },
            stops: stops.into_computed(),
        }
    }

    /// A gradient whose colours run outward from `center`, between two radii.
    ///
    /// `center` is in unit space. Both radii are fractions of the shorter side
    /// of the box the view is laid out at: on a `w` × `h` box each radius `r`
    /// is a circle of `r * min(w, h)` points around `center` mapped onto the
    /// box. A non-square box does not stretch the circle into an ellipse.
    ///
    /// A start radius that is negative or not finite is a contract violation
    /// and is `0`; an end radius that is not finite or not above `0` is one
    /// too, and is `0.5`, the circle that reaches the nearer edge from the
    /// centre.
    #[must_use]
    pub fn radial(
        stops: impl IntoComputed<GradientStops>,
        center: UnitPoint,
        start_radius: f32,
        end_radius: f32,
    ) -> Self {
        let start_radius = if start_radius.is_finite() && start_radius >= 0.0 {
            start_radius
        } else {
            contract_violation!(
                "radial gradient start radius must be finite and >= 0, got {start_radius}; 0 is used"
            );
            0.0
        };
        let end_radius = if end_radius.is_finite() && end_radius > 0.0 {
            end_radius
        } else {
            contract_violation!(
                "radial gradient end radius must be finite and > 0, got {end_radius}; 0.5 is used"
            );
            0.5
        };
        Self {
            geometry: GradientGeometry::Radial {
                center: point(center, "radial gradient center"),
                start_radius,
                end_radius,
            },
            stops: stops.into_computed(),
        }
    }

    /// A gradient whose colours run around `center`, from `start_angle` to
    /// `end_angle`.
    ///
    /// An angle that is not finite, or a sweep that is not positive or is
    /// more than a full turn, is a contract violation: the gradient runs a
    /// full turn from the start angle (`Angle::ZERO` when it is not finite).
    #[must_use]
    pub fn angular(
        stops: impl IntoComputed<GradientStops>,
        center: UnitPoint,
        start_angle: Angle,
        end_angle: Angle,
    ) -> Self {
        let (start_angle, end_angle) = if start_angle.is_finite() && end_angle.is_finite() {
            let sweep = end_angle - start_angle;
            if sweep <= Angle::ZERO {
                contract_violation!(
                    "angular gradient sweep must be positive, got {sweep}; a full turn is used"
                );
                (start_angle, start_angle + Angle::FULL)
            } else if sweep > Angle::FULL {
                contract_violation!(
                    "angular gradient sweep must be <= one full turn, got {sweep}; a full turn is used"
                );
                (start_angle, start_angle + Angle::FULL)
            } else {
                (start_angle, end_angle)
            }
        } else {
            contract_violation!(
                "angular gradient angles must be finite, got {start_angle} and {end_angle}; a full turn from 0 is used"
            );
            (Angle::ZERO, Angle::FULL)
        };
        Self {
            geometry: GradientGeometry::Angular {
                center: point(center, "angular gradient center"),
                start_angle,
                end_angle,
            },
            stops: stops.into_computed(),
        }
    }

    /// The family of this gradient.
    #[must_use]
    pub const fn gradient_type(&self) -> GradientType {
        self.geometry.gradient_type()
    }

    /// Resolves this gradient's stops in `env` into a live [`Paint`] in unit space.
    ///
    /// Both changes to the stops signal and changes to any stop's environment
    /// resolution (such as a colour-scheme switch) emit an updated paint.
    #[must_use]
    pub fn resolve(&self, env: &Environment) -> Computed<Paint> {
        let geometry = self.geometry.clone();
        resolve_stops(&self.stops, env)
            .map(move |resolved| geometry.build_paint(stops(resolved)))
            .computed()
    }

    /// The transform that maps unit space onto a `width` × `height` box.
    #[must_use]
    pub fn transform_to(width: f32, height: f32) -> Affine {
        Affine::scale_non_uniform(f64::from(width), f64::from(height))
    }
}

impl waterui_core::NativeView for Gradient {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

impl View for Gradient {
    fn body(self, _env: &Environment) -> impl View {
        waterui_core::Native::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::RefCell;
    use nami::Binding;
    use waterui_color::{ColorScheme, Working};

    fn working(color: WorkingColor) -> Color {
        Color::new(Working(color))
    }

    fn resolve_paint(gradient: &Gradient) -> Paint {
        gradient.resolve(&Environment::new()).snapshot()
    }

    fn linear_stops(gradient: &Gradient) -> Vec<ColorStop> {
        let Paint::Linear(linear) = resolve_paint(gradient) else {
            panic!("a linear gradient is a linear paint");
        };
        linear.stops
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "sorting preserves these exact input offsets"
    )]
    fn stops_are_sorted_by_offset() {
        let gradient = Gradient::linear(
            vec![
                (1.0, working(WorkingColor::WHITE)),
                (0.0, working(WorkingColor::BLACK)),
            ],
            UnitPoint::TOP_LEADING,
            UnitPoint::TOP_TRAILING,
        );
        let stops = linear_stops(&gradient);
        assert_eq!(stops[0].offset, 0.0);
        assert_eq!(stops[1].offset, 1.0);
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "evenly spaced endpoints and midpoint are exact powers of two"
    )]
    fn evenly_spaced_color_array_generates_unit_stops() {
        let gradient = Gradient::linear(
            [
                working(WorkingColor::BLACK),
                working(WorkingColor::WHITE),
                working(WorkingColor::BLACK),
            ],
            UnitPoint::TOP,
            UnitPoint::BOTTOM,
        );
        let stops = linear_stops(&gradient);
        assert_eq!(stops.len(), 3);
        assert_eq!(stops[0].offset, 0.0);
        assert_eq!(stops[1].offset, 0.5);
        assert_eq!(stops[2].offset, 1.0);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "within [0, 1]"))]
    fn an_offset_past_one_is_clamped() {
        let gradient = Gradient::linear(
            vec![
                (1.5, working(WorkingColor::WHITE)),
                (f32::NAN, working(WorkingColor::BLACK)),
            ],
            UnitPoint::TOP_LEADING,
            UnitPoint::TOP_TRAILING,
        );
        let offsets: Vec<f32> = linear_stops(&gradient)
            .iter()
            .map(|stop| stop.offset)
            .collect();
        assert_eq!(offsets, [0.0, 1.0]);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "at least one stop"))]
    fn a_gradient_without_stops_is_one_transparent_stop() {
        let gradient = Gradient::linear(
            Vec::<(f32, Color)>::new(),
            UnitPoint::TOP_LEADING,
            UnitPoint::TOP_TRAILING,
        );
        assert_eq!(
            linear_stops(&gradient),
            [ColorStop {
                offset: 0.0,
                color: WorkingColor::TRANSPARENT
            }]
        );
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "colour must be finite"))]
    fn a_stop_with_a_non_finite_colour_is_dropped() {
        let broken = WorkingColor::new([f32::NAN, 0.0, 0.0, 1.0]);
        let gradient = Gradient::linear(
            vec![(0.0, working(broken)), (1.0, working(WorkingColor::WHITE))],
            UnitPoint::TOP_LEADING,
            UnitPoint::TOP_TRAILING,
        );
        assert_eq!(
            linear_stops(&gradient),
            [ColorStop {
                offset: 1.0,
                color: WorkingColor::WHITE
            }]
        );
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "must be finite"))]
    fn a_non_finite_point_is_the_origin() {
        let gradient = Gradient::linear(
            vec![(0.0, working(WorkingColor::BLACK))],
            UnitPoint::new(f32::INFINITY, 0.5),
            UnitPoint::TOP_TRAILING,
        );
        let Paint::Linear(linear) = resolve_paint(&gradient) else {
            panic!("a linear gradient is a linear paint");
        };
        assert_eq!(linear.start, Point::new(0.0, 0.5));
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "sweep must be positive"))]
    fn a_backwards_angular_sweep_is_a_full_turn() {
        let gradient = Gradient::angular(
            vec![(0.0, working(WorkingColor::BLACK))],
            UnitPoint::CENTER,
            Angle::radians(1.0),
            Angle::radians(0.5),
        );
        let Paint::Sweep(sweep) = resolve_paint(&gradient) else {
            panic!("an angular gradient is a sweep paint");
        };
        assert!(
            (sweep.end_angle - sweep.start_angle - f64::from(Angle::FULL.to_radians())).abs()
                < 1e-6
        );
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "end radius"))]
    fn an_empty_radial_end_radius_is_half_the_shorter_side() {
        let gradient = Gradient::radial(
            vec![(0.0, working(WorkingColor::BLACK))],
            UnitPoint::CENTER,
            0.0,
            0.0,
        );
        let Paint::Radial(radial) = resolve_paint(&gradient) else {
            panic!("a radial gradient is a radial paint");
        };
        assert!((radial.end_radius - 0.5).abs() < 1e-9);
    }

    #[test]
    fn theme_token_gradient_recolours_on_scheme_change() {
        use waterui::theme::color::{Accent, Surface};

        let scheme = Binding::container(ColorScheme::Light);
        let mut env = Environment::new();
        waterui::theme::install_color_scheme(&mut env, scheme.computed());
        waterui::theme::install_color_signal::<Accent>(
            &mut env,
            scheme
                .map(|mode| match mode {
                    ColorScheme::Light => WorkingColor::new([0.1, 0.2, 0.9, 1.0]),
                    ColorScheme::Dark => WorkingColor::new([0.4, 0.6, 1.0, 1.0]),
                })
                .computed(),
        );
        waterui::theme::install_color_signal::<Surface>(
            &mut env,
            scheme
                .map(|mode| match mode {
                    ColorScheme::Light => WorkingColor::WHITE,
                    ColorScheme::Dark => WorkingColor::BLACK,
                })
                .computed(),
        );

        let gradient = Gradient::linear(
            [Accent.into(), Surface.into()],
            UnitPoint::TOP,
            UnitPoint::BOTTOM,
        );
        let paint = gradient.resolve(&env);
        let updates = Rc::new(RefCell::new(Vec::new()));
        let captured = Rc::clone(&updates);
        let _guard = paint.watch(move |ctx| {
            captured.borrow_mut().push(ctx.into_value());
        });

        let Paint::Linear(initial) = paint.snapshot() else {
            panic!("expected linear paint");
        };
        assert_eq!(
            initial.stops[0].color,
            WorkingColor::new([0.1, 0.2, 0.9, 1.0])
        );
        assert_eq!(initial.stops[1].color, WorkingColor::WHITE);

        scheme.set(ColorScheme::Dark);

        assert!(
            !updates.borrow().is_empty(),
            "scheme change must notify watchers in place"
        );
        let Paint::Linear(dark) = paint.snapshot() else {
            panic!("expected linear paint");
        };
        assert_eq!(dark.stops[0].color, WorkingColor::new([0.4, 0.6, 1.0, 1.0]));
        assert_eq!(dark.stops[1].color, WorkingColor::BLACK);
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "the test checks the exact animated stop offset"
    )]
    fn animated_stop_position_updates_in_place() {
        let progress = Binding::container(0.25_f32);
        let rose = Color::srgb_hex("#FF4D80");
        let sky = Color::srgb_hex("#38BDF8");
        let stops_signal = progress.map(move |p| vec![(0.0, rose.clone()), (p, sky.clone())]);

        let gradient = Gradient::linear(stops_signal, UnitPoint::LEADING, UnitPoint::TRAILING);
        let paint = gradient.resolve(&Environment::new());
        let updates = Rc::new(RefCell::new(Vec::new()));
        let captured = Rc::clone(&updates);
        let _guard = paint.watch(move |ctx| {
            captured.borrow_mut().push(ctx.into_value());
        });

        let Paint::Linear(initial) = paint.snapshot() else {
            panic!("expected linear paint");
        };
        assert_eq!(initial.stops[1].offset, 0.25);

        progress.set(0.75);

        assert_eq!(updates.borrow().len(), 1);
        let Paint::Linear(next) = paint.snapshot() else {
            panic!("expected linear paint");
        };
        assert_eq!(next.stops[1].offset, 0.75);
    }
}
