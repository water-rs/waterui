extern crate alloc;

use alloc::vec::Vec;
use core::f32::consts::{FRAC_PI_2, TAU};
use core::time::Duration;
use std::time::Instant;

use nami::{Binding, Signal, SignalExt as _};
use num_traits::ToPrimitive as _;
use waterui_canvas::{Canvas, DrawingContext, Path};
use waterui_core::{
    AnyView, Environment, Metadata, View,
    accessibility::{AccessibilityHidden, AccessibilityRole},
    event::{Event, HoverEvent, OnEvent},
    gesture::{
        DragEvent, DragGesture, GestureObserver, GesturePhase, GesturePoint, TapEvent, TapGesture,
    },
    layout::{Point, Rect, Size},
};
use waterui_graphics::color::Srgb;

use crate::{
    animation::{AnimationConfig, ChartAnimator},
    composition::ChartComposition,
    data::{
        AreaData, BubblePoint, Candle, ChoroplethData, ColorScale, ContourData, DataBounds,
        DataPoint, DepthData, GaugeData, HeatmapData, RadarData,
    },
    interaction::{
        AreaDatum, CartesianSelectionBindings, CartesianViewportBindings, ChartViewport,
        DepthDatum, DepthSide, GridDatum, HitResult, RadarDatum, RegionDatum, SelectionBindings,
        SliceDatum,
    },
};

const PLOT_PADDING_RATIO: f32 = 0.1;
const CHART_TRANSITION: AnimationConfig = AnimationConfig::ease_in_out(Duration::from_millis(240));
const PIE_DEFAULT_COLORS: [u32; 8] = [
    0x003B_82F6,
    0x0022_C55E,
    0x00EF_4444,
    0x00F5_9E0B,
    0x008B_5CF6,
    0x00EC_4899,
    0x0006_B6D4,
    0x00F9_7316,
];
const VIRIDIS_STOPS: [(f32, Srgb); 5] = [
    (0.0, Srgb::from_hex("#440154")),
    (0.25, Srgb::from_hex("#3B528B")),
    (0.5, Srgb::from_hex("#21918C")),
    (0.75, Srgb::from_hex("#5EC962")),
    (1.0, Srgb::from_hex("#FDE725")),
];

fn usize_to_f32(value: usize) -> f32 {
    value
        .to_f32()
        .expect("chart collection length should fit f32")
}

fn u32_to_f32(value: u32) -> f32 {
    value.to_f32().expect("chart dimension should fit f32")
}

fn u32_to_usize(value: u32) -> usize {
    usize::try_from(value).expect("chart dimension should fit usize")
}

struct ChartCanvasAccessibilityBoundary {
    content: AnyView,
}

impl ChartCanvasAccessibilityBoundary {
    fn new(content: impl View + 'static) -> Self {
        Self {
            content: AnyView::new(content),
        }
    }
}

impl View for ChartCanvasAccessibilityBoundary {
    fn body(self, env: &Environment) -> impl View {
        let mut canvas_env = env.clone();
        if canvas_env.get::<AccessibilityRole>().is_some() {
            canvas_env.insert(AccessibilityHidden::new(true));
        }
        Metadata::new(self.content, canvas_env)
    }
}

#[derive(Debug, Clone)]
struct ChartTransitionState<D> {
    epoch: Instant,
    animator: ChartAnimator,
    last_data: Option<D>,
}

impl<D> ChartTransitionState<D> {
    fn new() -> Self {
        Self {
            epoch: Instant::now(),
            animator: ChartAnimator::new(),
            last_data: None,
        }
    }

    fn progress_for(&mut self, data: &D) -> f32
    where
        D: Clone + PartialEq,
    {
        let now = self.epoch.elapsed();
        match &self.last_data {
            None => self.last_data = Some(data.clone()),
            Some(previous) if previous != data => {
                self.animator.start_transition(
                    now,
                    CHART_TRANSITION.duration,
                    CHART_TRANSITION.easing,
                );
                self.last_data = Some(data.clone());
            }
            Some(_) => {}
        }
        self.animator.update(now).progress
    }

    const fn is_animating(&self) -> bool {
        self.animator.is_animating()
    }
}

#[derive(Debug, Clone)]
enum HitShape {
    Circle {
        center: Point,
        radius: f32,
    },
    Rect {
        rect: Rect,
    },
    Sector {
        center: Point,
        inner_radius: f32,
        outer_radius: f32,
        start_angle: f32,
        end_angle: f32,
    },
    Polygon {
        vertices: Vec<Point>,
    },
}

#[derive(Debug, Clone)]
struct HitTarget<T> {
    result: HitResult<T>,
    shape: HitShape,
}

impl<T> HitTarget<T> {
    fn contains(&self, point: Point) -> bool {
        match &self.shape {
            HitShape::Circle { center, radius } => {
                distance_squared(*center, point) <= radius * radius
            }
            HitShape::Rect { rect } => rect_contains(*rect, point),
            HitShape::Sector {
                center,
                inner_radius,
                outer_radius,
                start_angle,
                end_angle,
            } => sector_contains(
                *center,
                point,
                *inner_radius,
                *outer_radius,
                *start_angle,
                *end_angle,
            ),
            HitShape::Polygon { vertices } => polygon_contains(vertices, point),
        }
    }

    fn score(&self, point: Point) -> Option<f32> {
        self.contains(point)
            .then_some(distance_squared(self.result.anchor.as_point(), point))
    }
}

pub(crate) trait HitGeometry<T: Clone>: Clone + 'static {
    fn hit_test(&self, point: Point) -> Option<HitResult<T>>;
    fn viewport(&self) -> ChartViewport;
}

#[derive(Debug, Clone)]
pub(crate) struct HitTargets<T> {
    viewport: ChartViewport,
    targets: Vec<HitTarget<T>>,
}

impl<T> HitTargets<T> {
    const fn new(viewport: ChartViewport) -> Self {
        Self {
            viewport,
            targets: Vec::new(),
        }
    }

    fn push_circle(&mut self, result: HitResult<T>, center: Point, radius: f32) {
        self.targets.push(HitTarget {
            result,
            shape: HitShape::Circle {
                center,
                radius: radius.max(1.0),
            },
        });
    }

    fn push_rect(&mut self, result: HitResult<T>, rect: Rect) {
        self.targets.push(HitTarget {
            result,
            shape: HitShape::Rect { rect },
        });
    }

    fn push_sector(
        &mut self,
        result: HitResult<T>,
        center: Point,
        inner_radius: f32,
        outer_radius: f32,
        start_angle: f32,
        end_angle: f32,
    ) {
        self.targets.push(HitTarget {
            result,
            shape: HitShape::Sector {
                center,
                inner_radius,
                outer_radius,
                start_angle,
                end_angle,
            },
        });
    }

    fn push_polygon(&mut self, result: HitResult<T>, vertices: Vec<Point>) {
        self.targets.push(HitTarget {
            result,
            shape: HitShape::Polygon { vertices },
        });
    }
}

impl<T: Clone + 'static> HitGeometry<T> for HitTargets<T> {
    fn hit_test(&self, point: Point) -> Option<HitResult<T>> {
        let mut best: Option<(f32, &HitTarget<T>)> = None;
        for target in &self.targets {
            let Some(score) = target.score(point) else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|(best_score, _)| score < *best_score)
            {
                best = Some((score, target));
            }
        }
        best.map(|(_, target)| target.result.clone())
    }

    fn viewport(&self) -> ChartViewport {
        self.viewport
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CartesianGeometry<T> {
    pub bounds: DataBounds,
    viewport: ChartViewport,
    targets: HitTargets<T>,
}

impl<T> CartesianGeometry<T> {
    const fn new(bounds: DataBounds, viewport: ChartViewport, targets: HitTargets<T>) -> Self {
        Self {
            bounds,
            viewport,
            targets,
        }
    }
}

impl<T: Clone + 'static> HitGeometry<T> for CartesianGeometry<T> {
    fn hit_test(&self, point: Point) -> Option<HitResult<T>> {
        self.targets.hit_test(point)
    }

    fn viewport(&self) -> ChartViewport {
        self.viewport
    }
}

const fn gesture_point_to_point(point: GesturePoint) -> Point {
    Point::new(point.x, point.y)
}

fn distance_squared(a: Point, b: Point) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx.mul_add(dx, dy * dy)
}

fn rect_contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.min_x()
        && point.x <= rect.max_x()
        && point.y >= rect.min_y()
        && point.y <= rect.max_y()
}

fn normalize_angle(angle: f32) -> f32 {
    angle.rem_euclid(TAU)
}

fn angle_in_sweep(angle: f32, start: f32, end: f32) -> bool {
    let angle = normalize_angle(angle);
    let start = normalize_angle(start);
    let end = normalize_angle(end);
    if start <= end {
        angle >= start && angle <= end
    } else {
        angle >= start || angle <= end
    }
}

fn sector_contains(
    center: Point,
    point: Point,
    inner_radius: f32,
    outer_radius: f32,
    start_angle: f32,
    end_angle: f32,
) -> bool {
    let dx = point.x - center.x;
    let dy = point.y - center.y;
    let distance = dx.hypot(dy);
    if distance < inner_radius || distance > outer_radius {
        return false;
    }
    let angle = dy.atan2(dx);
    angle_in_sweep(angle, start_angle, end_angle)
}

fn polygon_contains(vertices: &[Point], point: Point) -> bool {
    if vertices.len() < 3 {
        return false;
    }

    let mut inside = false;
    let mut previous = *vertices
        .last()
        .expect("polygon_contains requires non-empty vertices");
    for &current in vertices {
        let crosses = (current.y > point.y) != (previous.y > point.y)
            && point.x
                < (previous.x - current.x) * (point.y - current.y)
                    / ((previous.y - current.y).abs().max(f32::EPSILON))
                    + current.x;
        if crosses {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

fn cartesian_x_from_point<T>(
    geometry: &CartesianGeometry<T>,
    point: Point,
    clamp: bool,
) -> Option<f32> {
    let viewport = geometry.viewport;
    if viewport.width <= 0.0 {
        return None;
    }
    let x = if clamp {
        point.x.clamp(viewport.x, viewport.x + viewport.width)
    } else if viewport.contains(point) {
        point.x
    } else {
        return None;
    };
    let normalized_x = ((x - viewport.x) / viewport.width).clamp(0.0, 1.0);
    Some(
        geometry
            .bounds
            .width()
            .mul_add(normalized_x, geometry.bounds.min_x),
    )
}

fn cartesian_y_from_point<T>(
    geometry: &CartesianGeometry<T>,
    point: Point,
    clamp: bool,
) -> Option<f32> {
    let viewport = geometry.viewport;
    if viewport.height <= 0.0 {
        return None;
    }
    let y = if clamp {
        point.y.clamp(viewport.y, viewport.y + viewport.height)
    } else if viewport.contains(point) {
        point.y
    } else {
        return None;
    };
    let normalized_y = ((y - viewport.y) / viewport.height).clamp(0.0, 1.0);
    Some(
        geometry
            .bounds
            .height()
            .mul_add(-normalized_y, geometry.bounds.max_y),
    )
}

#[inline]
fn normalized_scalar_range(start: f32, end: f32) -> core::ops::RangeInclusive<f32> {
    if start <= end {
        start..=end
    } else {
        end..=start
    }
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "This internal adapter owns generic callbacks and bindings so they can move into Canvas' static event closures without extra type erasure"
)]
pub(crate) fn interactive_cartesian_signal_canvas<S, D, T, B, G, F>(
    _env: &Environment,
    signal: S,
    bounds_of: B,
    build_geometry: G,
    mut draw_chart: F,
    mut selection: SelectionBindings<T>,
    cartesian_selection: CartesianSelectionBindings,
    cartesian_viewport: CartesianViewportBindings,
    composition: ChartComposition<T>,
) -> impl waterui_core::View
where
    S: Signal<Output = D> + 'static,
    S::Guard: 'static,
    D: Clone + PartialEq + 'static,
    T: Clone + PartialEq + 'static,
    B: Fn(&D) -> DataBounds + 'static,
    G: Fn(&DrawingContext<'_>, &D, DataBounds) -> CartesianGeometry<T> + 'static,
    F: FnMut(&mut DrawingContext<'_>, &D, &CartesianGeometry<T>) + 'static,
{
    cartesian_selection.validate();
    cartesian_viewport.validate();
    if !composition.is_empty() {
        selection = selection.activate_proxy();
    }
    if selection.is_active() {
        selection = selection.persist_internal();
    }
    let transition = Binding::container(ChartTransitionState::<D>::new());
    let geometry = Binding::container(None::<CartesianGeometry<T>>);
    let base_bounds = Binding::container(DataBounds::default());
    let chart_frame = Binding::container(ChartViewport::default());
    let plot_area_frame = Binding::container(ChartViewport::default());
    let x_range_start = Binding::container(None::<f32>);
    let y_range_start = Binding::container(None::<f32>);
    let viewport_state = cartesian_viewport.state_signal();
    let canvas = {
        let transition = transition;
        let geometry = geometry.clone();
        let base_bounds = base_bounds.clone();
        let chart_frame = chart_frame.clone();
        let plot_area_frame = plot_area_frame.clone();
        let cartesian_viewport = cartesian_viewport.clone();
        Canvas::with_signal(
            signal.zip(&viewport_state),
            move |ctx, (data, viewport_state)| {
                let current_base_bounds = normalize_bounds(bounds_of(&data));
                base_bounds.set(current_base_bounds);
                let visible_bounds =
                    cartesian_viewport.resolve_bounds(current_base_bounds, viewport_state);
                let chart_geometry = build_geometry(ctx, &data, visible_bounds);
                geometry.set(Some(chart_geometry.clone()));
                let current_chart_frame = ChartViewport::new(0.0, 0.0, ctx.width, ctx.height);
                if chart_frame.get() != current_chart_frame {
                    chart_frame.set(current_chart_frame);
                }
                let current_plot_area = chart_geometry.viewport();
                if plot_area_frame.get() != current_plot_area {
                    plot_area_frame.set(current_plot_area);
                }
                let (progress, animating) = {
                    transition.with_mut(|transition| {
                        let progress = transition.progress_for(&data);
                        let animating = transition.is_animating();
                        (progress, animating)
                    })
                };
                if animating {
                    ctx.request_next_frame();
                }
                ctx.save();
                ctx.set_global_alpha(progress);
                draw_chart(ctx, &data, &chart_geometry);
                ctx.restore();
            },
        )
    };
    let canvas = ChartCanvasAccessibilityBoundary::new(canvas);
    let canvas = Metadata::new(
        canvas,
        GestureObserver::new(TapGesture::new(), {
            let geometry = geometry.clone();
            let selection = selection.clone();
            let cartesian_selection = cartesian_selection.clone();
            move |env: Environment| {
                if !selection.is_active() && !cartesian_selection.is_active() {
                    return;
                }
                let tap = env
                    .get::<TapEvent>()
                    .expect("interactive_cartesian_signal_canvas: TapEvent missing from gesture environment");
                let point = gesture_point_to_point(tap.location);
                let Some(geometry) = geometry.get() else {
                    return;
                };
                let hit = geometry.hit_test(point);
                selection.set_focus(hit.clone());
                selection.set_selected(hit);
                if cartesian_selection.tracks_x_value() {
                    cartesian_selection
                        .set_x_value(cartesian_x_from_point(&geometry, point, false));
                }
                if cartesian_selection.tracks_y_value() {
                    cartesian_selection
                        .set_y_value(cartesian_y_from_point(&geometry, point, false));
                }
            }
        }),
    );
    let canvas = Metadata::new(
        canvas,
        GestureObserver::new(DragGesture::new(0.0), {
            let geometry = geometry.clone();
            let base_bounds = base_bounds;
            let selection = selection.clone();
            let cartesian_selection = cartesian_selection;
            let cartesian_viewport = cartesian_viewport;
            let x_range_start = x_range_start;
            let y_range_start = y_range_start;
            move |env: Environment| {
                let handles_selection_drag = selection.is_active()
                    || cartesian_selection.is_active()
                    || (!cartesian_viewport.allows_horizontal_drag()
                        && !cartesian_viewport.allows_vertical_drag());
                if !handles_selection_drag
                    && !cartesian_viewport.allows_horizontal_drag()
                    && !cartesian_viewport.allows_vertical_drag()
                {
                    return;
                }

                let drag_event = env.get::<DragEvent>().expect(
                    "interactive_cartesian_signal_canvas: DragEvent missing from gesture environment",
                );
                let Some(geometry) = geometry.get() else {
                    return;
                };
                let point = gesture_point_to_point(drag_event.location);

                if handles_selection_drag {
                    match drag_event.phase {
                        GesturePhase::Started => {
                            let hit = geometry.hit_test(point);
                            selection.set_focus(hit);
                            let start_point = Point::new(
                                point.x - drag_event.translation.x,
                                point.y - drag_event.translation.y,
                            );
                            if cartesian_selection.tracks_x_range() {
                                if let Some(start) =
                                    cartesian_x_from_point(&geometry, start_point, true)
                                {
                                    x_range_start.set(Some(start));
                                    if let Some(end) =
                                        cartesian_x_from_point(&geometry, point, true)
                                    {
                                        cartesian_selection
                                            .set_x_range(Some(normalized_scalar_range(start, end)));
                                    }
                                }
                            } else if cartesian_selection.tracks_x_value() {
                                cartesian_selection
                                    .set_x_value(cartesian_x_from_point(&geometry, point, true));
                            }
                            if cartesian_selection.tracks_y_range() {
                                if let Some(start) =
                                    cartesian_y_from_point(&geometry, start_point, true)
                                {
                                    y_range_start.set(Some(start));
                                    if let Some(end) =
                                        cartesian_y_from_point(&geometry, point, true)
                                    {
                                        cartesian_selection
                                            .set_y_range(Some(normalized_scalar_range(start, end)));
                                    }
                                }
                            } else if cartesian_selection.tracks_y_value() {
                                cartesian_selection
                                    .set_y_value(cartesian_y_from_point(&geometry, point, true));
                            }
                        }
                        GesturePhase::Updated => {
                            let hit = geometry.hit_test(point);
                            selection.set_focus(hit);
                            if cartesian_selection.tracks_x_range() {
                                if let (Some(start), Some(end)) = (
                                    x_range_start.get(),
                                    cartesian_x_from_point(&geometry, point, true),
                                ) {
                                    cartesian_selection
                                        .set_x_range(Some(normalized_scalar_range(start, end)));
                                }
                            } else if cartesian_selection.tracks_x_value() {
                                cartesian_selection
                                    .set_x_value(cartesian_x_from_point(&geometry, point, true));
                            }
                            if cartesian_selection.tracks_y_range() {
                                if let (Some(start), Some(end)) = (
                                    y_range_start.get(),
                                    cartesian_y_from_point(&geometry, point, true),
                                ) {
                                    cartesian_selection
                                        .set_y_range(Some(normalized_scalar_range(start, end)));
                                }
                            } else if cartesian_selection.tracks_y_value() {
                                cartesian_selection
                                    .set_y_value(cartesian_y_from_point(&geometry, point, true));
                            }
                        }
                        GesturePhase::Ended => {
                            let hit = geometry.hit_test(point);
                            selection.set_selected(hit);
                            selection.clear_focus();
                            if cartesian_selection.tracks_x_range() {
                                if let (Some(start), Some(end)) = (
                                    x_range_start.get(),
                                    cartesian_x_from_point(&geometry, point, true),
                                ) {
                                    cartesian_selection
                                        .set_x_range(Some(normalized_scalar_range(start, end)));
                                }
                                x_range_start.set(None);
                            } else if cartesian_selection.tracks_x_value() {
                                cartesian_selection
                                    .set_x_value(cartesian_x_from_point(&geometry, point, true));
                            }
                            if cartesian_selection.tracks_y_range() {
                                if let (Some(start), Some(end)) = (
                                    y_range_start.get(),
                                    cartesian_y_from_point(&geometry, point, true),
                                ) {
                                    cartesian_selection
                                        .set_y_range(Some(normalized_scalar_range(start, end)));
                                }
                                y_range_start.set(None);
                            } else if cartesian_selection.tracks_y_value() {
                                cartesian_selection
                                    .set_y_value(cartesian_y_from_point(&geometry, point, true));
                            }
                        }
                        GesturePhase::Cancelled => {
                            selection.clear_focus();
                            if cartesian_selection.tracks_x_range() {
                                x_range_start.set(None);
                            }
                            if cartesian_selection.tracks_y_range() {
                                y_range_start.set(None);
                            }
                        }
                    }
                    return;
                }

                let base_bounds = base_bounds.get();
                let plot_viewport = geometry.viewport;
                match drag_event.phase {
                    GesturePhase::Started => {
                        selection.clear_focus();
                        if cartesian_viewport.allows_horizontal_drag() {
                            x_range_start.set(Some(geometry.bounds.min_x));
                        }
                        if cartesian_viewport.allows_vertical_drag() {
                            y_range_start.set(Some(geometry.bounds.min_y));
                        }
                    }
                    GesturePhase::Updated | GesturePhase::Ended => {
                        if cartesian_viewport.allows_horizontal_drag() {
                            if let Some(start) = x_range_start.get() {
                                let visible_width = geometry.bounds.width();
                                let max_start =
                                    (base_bounds.max_x - visible_width).max(base_bounds.min_x);
                                let candidate = (drag_event.translation.x / plot_viewport.width)
                                    .mul_add(-visible_width, start);
                                let clamped = if max_start <= base_bounds.min_x {
                                    base_bounds.min_x
                                } else {
                                    candidate.clamp(base_bounds.min_x, max_start)
                                };
                                cartesian_viewport.set_x_position(clamped);
                            }
                            if matches!(drag_event.phase, GesturePhase::Ended) {
                                x_range_start.set(None);
                            }
                        }
                        if cartesian_viewport.allows_vertical_drag() {
                            if let Some(start) = y_range_start.get() {
                                let visible_height = geometry.bounds.height();
                                let max_start =
                                    (base_bounds.max_y - visible_height).max(base_bounds.min_y);
                                let candidate = (drag_event.translation.y / plot_viewport.height)
                                    .mul_add(-visible_height, start);
                                let clamped = if max_start <= base_bounds.min_y {
                                    base_bounds.min_y
                                } else {
                                    candidate.clamp(base_bounds.min_y, max_start)
                                };
                                cartesian_viewport.set_y_position(clamped);
                            }
                            if matches!(drag_event.phase, GesturePhase::Ended) {
                                y_range_start.set(None);
                            }
                        }
                    }
                    GesturePhase::Cancelled => {
                        x_range_start.set(None);
                        y_range_start.set(None);
                    }
                }
            }
        }),
    );
    let canvas = Metadata::new(
        canvas,
        OnEvent::new(Event::HoverMove, {
            let geometry = geometry;
            let selection = selection.clone();
            move |env: Environment| {
                if !selection.is_active() {
                    return;
                }
                let hover = env.get::<HoverEvent>().expect(
                    "interactive_cartesian_signal_canvas: HoverEvent missing from event environment",
                );
                let hit = geometry
                    .get()
                    .and_then(|geometry| geometry.hit_test(hover.location));
                selection.set_focus(hit);
            }
        }),
    );
    let canvas = Metadata::new(
        canvas,
        OnEvent::new(Event::HoverExit, {
            let selection = selection.clone();
            move || selection.clear_focus()
        }),
    );
    composition.apply(
        canvas,
        chart_frame.computed(),
        plot_area_frame.computed(),
        &selection,
    )
}

#[expect(
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "This internal adapter keeps Canvas drawing and event wiring together while moving callback ownership into static closures"
)]
pub(crate) fn interactive_signal_canvas<S, D, G, T, B, F>(
    _env: &Environment,
    signal: S,
    build_geometry: B,
    mut draw_chart: F,
    mut selection: SelectionBindings<T>,
    composition: ChartComposition<T>,
) -> impl waterui_core::View
where
    S: Signal<Output = D> + 'static,
    S::Guard: 'static,
    D: Clone + PartialEq + 'static,
    G: HitGeometry<T>,
    T: Clone + PartialEq + 'static,
    B: Fn(&DrawingContext<'_>, &D) -> G + 'static,
    F: FnMut(&mut DrawingContext<'_>, &D, &G) + 'static,
{
    if !composition.is_empty() {
        selection = selection.activate_proxy();
    }
    if selection.is_active() {
        selection = selection.persist_internal();
    }

    let transition = Binding::container(ChartTransitionState::<D>::new());
    let geometry = Binding::container(None::<G>);
    let chart_frame = Binding::container(ChartViewport::default());
    let plot_area_frame = Binding::container(ChartViewport::default());
    let canvas = {
        let transition = transition;
        let geometry = geometry.clone();
        let chart_frame = chart_frame.clone();
        let plot_area_frame = plot_area_frame.clone();
        Canvas::with_signal(signal, move |ctx, data| {
            let chart_geometry = build_geometry(ctx, &data);
            geometry.set(Some(chart_geometry.clone()));
            let current_chart_frame = ChartViewport::new(0.0, 0.0, ctx.width, ctx.height);
            if chart_frame.get() != current_chart_frame {
                chart_frame.set(current_chart_frame);
            }
            let current_plot_area = chart_geometry.viewport();
            if plot_area_frame.get() != current_plot_area {
                plot_area_frame.set(current_plot_area);
            }
            let (progress, animating) = {
                transition.with_mut(|transition| {
                    let progress = transition.progress_for(&data);
                    let animating = transition.is_animating();
                    (progress, animating)
                })
            };
            if animating {
                ctx.request_next_frame();
            }
            ctx.save();
            ctx.set_global_alpha(progress);
            draw_chart(ctx, &data, &chart_geometry);
            ctx.restore();
        })
    };
    let canvas = ChartCanvasAccessibilityBoundary::new(canvas);
    let canvas = Metadata::new(
        canvas,
        GestureObserver::new(TapGesture::new(), {
            let geometry = geometry.clone();
            let selection = selection.clone();
            move |env: Environment| {
                if !selection.is_active() {
                    return;
                }
                let tap = env
                    .get::<TapEvent>()
                    .expect("interactive_signal_canvas: TapEvent missing from gesture environment");
                let hit = geometry
                    .get()
                    .and_then(|geometry| geometry.hit_test(gesture_point_to_point(tap.location)));
                selection.set_focus(hit.clone());
                selection.set_selected(hit);
            }
        }),
    );
    let canvas = Metadata::new(
        canvas,
        GestureObserver::new(DragGesture::new(0.0), {
            let geometry = geometry.clone();
            let selection = selection.clone();
            move |env: Environment| {
                if !selection.is_active() {
                    return;
                }
                let drag_event = env.get::<DragEvent>().expect(
                    "interactive_signal_canvas: DragEvent missing from gesture environment",
                );
                match drag_event.phase {
                    GesturePhase::Started | GesturePhase::Updated => {
                        let hit = geometry.get().and_then(|geometry| {
                            geometry.hit_test(gesture_point_to_point(drag_event.location))
                        });
                        selection.set_focus(hit);
                    }
                    GesturePhase::Ended => {
                        let hit = geometry.get().and_then(|geometry| {
                            geometry.hit_test(gesture_point_to_point(drag_event.location))
                        });
                        selection.set_selected(hit);
                        selection.clear_focus();
                    }
                    GesturePhase::Cancelled => selection.clear_focus(),
                }
            }
        }),
    );
    let canvas = Metadata::new(
        canvas,
        OnEvent::new(Event::HoverMove, {
            let geometry = geometry;
            let selection = selection.clone();
            move |env: Environment| {
                if !selection.is_active() {
                    return;
                }
                let hover = env
                    .get::<HoverEvent>()
                    .expect("interactive_signal_canvas: HoverEvent missing from event environment");
                let hit = geometry
                    .get()
                    .and_then(|geometry| geometry.hit_test(hover.location));
                selection.set_focus(hit);
            }
        }),
    );
    let canvas = Metadata::new(
        canvas,
        OnEvent::new(Event::HoverExit, {
            let selection = selection.clone();
            move || selection.clear_focus()
        }),
    );
    composition.apply(
        canvas,
        chart_frame.computed(),
        plot_area_frame.computed(),
        &selection,
    )
}

#[inline]
fn srgb_lerp(a: Srgb, b: Srgb, t: f32) -> Srgb {
    let t = t.clamp(0.0, 1.0);
    Srgb::new(
        (b.red - a.red).mul_add(t, a.red),
        (b.green - a.green).mul_add(t, a.green),
        (b.blue - a.blue).mul_add(t, a.blue),
    )
}

#[inline]
const fn from_rgba(color: [f32; 4]) -> Srgb {
    Srgb::new(color[0], color[1], color[2])
}

#[inline]
const fn alpha(color: [f32; 4]) -> f32 {
    color[3].clamp(0.0, 1.0)
}

#[inline]
fn normalize_bounds(mut bounds: DataBounds) -> DataBounds {
    assert!(
        !(!bounds.min_x.is_finite()
            || !bounds.max_x.is_finite()
            || !bounds.min_y.is_finite()
            || !bounds.max_y.is_finite()),
        "chart data contains non-finite bounds"
    );
    if bounds.max_x <= bounds.min_x {
        bounds.max_x = bounds.min_x + 1.0;
    }
    if bounds.max_y <= bounds.min_y {
        bounds.max_y = bounds.min_y + 1.0;
    }
    bounds
}

#[inline]
pub(crate) fn point_bounds(data: &[DataPoint]) -> DataBounds {
    normalize_bounds(DataBounds::from_points(data).with_padding(0.1))
}

pub(crate) fn bar_bounds(data: &[DataPoint]) -> DataBounds {
    let source_bounds = DataBounds::from_points(data);
    normalize_bounds(DataBounds::new(
        source_bounds.min_x,
        source_bounds.max_x,
        source_bounds.min_y.min(0.0),
        source_bounds.max_y.max(0.0),
    ))
}

pub(crate) fn bubble_bounds(data: &[BubblePoint]) -> DataBounds {
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;

    for point in data {
        min_x = min_x.min(point.x);
        max_x = max_x.max(point.x);
        min_y = min_y.min(point.y);
        max_y = max_y.max(point.y);
    }

    normalize_bounds(DataBounds::new(min_x, max_x, min_y, max_y).with_padding(0.1))
}

pub(crate) fn candlestick_bounds(data: &[Candle]) -> DataBounds {
    normalize_bounds(DataBounds::from_candles(data).with_padding(0.05))
}

pub(crate) fn depth_bounds(data: &DepthData) -> DataBounds {
    normalize_bounds(data.bounds().with_padding(0.04))
}

pub(crate) fn area_bounds(data: &AreaData) -> DataBounds {
    normalize_bounds(data.bounds().with_padding(0.05))
}

fn plot_rect(ctx: &DrawingContext<'_>, padding_ratio: f32) -> Rect {
    let width = ctx.width;
    let height = ctx.height;
    let inset_x = width * padding_ratio;
    let inset_y = height * padding_ratio;
    Rect::new(
        Point::new(inset_x, inset_y),
        Size::new(
            inset_x.mul_add(-2.0, width).max(1.0),
            inset_y.mul_add(-2.0, height).max(1.0),
        ),
    )
}

#[inline]
const fn chart_viewport_from_rect(rect: Rect) -> ChartViewport {
    ChartViewport::new(rect.min_x(), rect.min_y(), rect.width(), rect.height())
}

#[inline]
fn map_xy(plot: Rect, bounds: DataBounds, x: f32, y: f32) -> Point {
    let nx = (x - bounds.min_x) / (bounds.max_x - bounds.min_x);
    let ny = (y - bounds.min_y) / (bounds.max_y - bounds.min_y);
    Point::new(
        nx.mul_add(plot.width(), plot.min_x()),
        ny.mul_add(-plot.height(), plot.max_y()),
    )
}

pub(crate) fn point_geometry(
    ctx: &DrawingContext<'_>,
    data: &[DataPoint],
    bounds: DataBounds,
    radius: f32,
) -> CartesianGeometry<DataPoint> {
    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);
    let mut targets = HitTargets::new(chart_viewport_from_rect(plot));
    for (index, point) in data.iter().enumerate() {
        let anchor = map_xy(plot, bounds, point.x, point.y);
        targets.push_circle(HitResult::new(0, index, *point, anchor), anchor, radius);
    }
    CartesianGeometry::new(bounds, chart_viewport_from_rect(plot), targets)
}

pub(crate) fn bar_geometry(
    ctx: &DrawingContext<'_>,
    data: &[DataPoint],
    bounds: DataBounds,
) -> CartesianGeometry<DataPoint> {
    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);
    let bar_width = if data.is_empty() {
        1.0
    } else {
        (plot.width() / usize_to_f32(data.len()) * 0.7).max(1.0)
    };
    let baseline_y = map_xy(plot, bounds, bounds.min_x, 0.0).y;
    let mut targets = HitTargets::new(chart_viewport_from_rect(plot));
    for (index, point) in data.iter().enumerate() {
        let center = map_xy(plot, bounds, point.x, point.y);
        let top = center.y.min(baseline_y);
        let bottom = center.y.max(baseline_y);
        let rect = Rect::new(
            Point::new(center.x - bar_width * 0.5, top),
            Size::new(bar_width, (bottom - top).max(1.0)),
        );
        let anchor = Point::new(center.x, top);
        targets.push_rect(HitResult::new(0, index, *point, anchor), rect);
    }
    CartesianGeometry::new(bounds, chart_viewport_from_rect(plot), targets)
}

pub(crate) fn bubble_geometry(
    ctx: &DrawingContext<'_>,
    data: &[BubblePoint],
    bounds: DataBounds,
    min_radius: f32,
    max_radius: f32,
) -> CartesianGeometry<BubblePoint> {
    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);
    let mut min_size = f32::MAX;
    let mut max_size = f32::MIN;
    for point in data {
        min_size = min_size.min(point.size);
        max_size = max_size.max(point.size);
    }
    let size_span = (max_size - min_size).max(1.0);
    let mut targets = HitTargets::new(chart_viewport_from_rect(plot));
    for (index, point) in data.iter().enumerate() {
        let anchor = map_xy(plot, bounds, point.x, point.y);
        let t = (point.size - min_size) / size_span;
        let radius = (max_radius - min_radius).mul_add(t, min_radius);
        targets.push_circle(
            HitResult::new(0, index, *point, anchor),
            anchor,
            radius.max(6.0),
        );
    }
    CartesianGeometry::new(bounds, chart_viewport_from_rect(plot), targets)
}

pub(crate) fn candlestick_geometry(
    ctx: &DrawingContext<'_>,
    data: &[Candle],
    bounds: DataBounds,
) -> CartesianGeometry<Candle> {
    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);
    let candle_width = if data.is_empty() {
        1.0
    } else {
        (plot.width() / usize_to_f32(data.len()) * 0.65).max(1.0)
    };
    let mut targets = HitTargets::new(chart_viewport_from_rect(plot));
    for (index, candle) in data.iter().enumerate() {
        let x = map_xy(plot, bounds, candle.timestamp, candle.close).x;
        let high = map_xy(plot, bounds, candle.timestamp, candle.high).y;
        let low = map_xy(plot, bounds, candle.timestamp, candle.low).y;
        let open_y = map_xy(plot, bounds, candle.timestamp, candle.open).y;
        let close_y = map_xy(plot, bounds, candle.timestamp, candle.close).y;
        let top = open_y.min(close_y);
        let bottom = open_y.max(close_y);
        let rect = Rect::new(
            Point::new(x - candle_width * 0.5, top.min(high)),
            Size::new(
                candle_width.max(4.0),
                (bottom.max(low) - top.min(high)).max(4.0),
            ),
        );
        let anchor = Point::new(x, (top + bottom) * 0.5);
        targets.push_rect(HitResult::new(0, index, *candle, anchor), rect);
    }
    CartesianGeometry::new(bounds, chart_viewport_from_rect(plot), targets)
}

pub(crate) fn depth_geometry(
    ctx: &DrawingContext<'_>,
    data: &DepthData,
    bounds: DataBounds,
) -> CartesianGeometry<DepthDatum> {
    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);
    let mut targets = HitTargets::new(chart_viewport_from_rect(plot));
    for (index, level) in data.bids.iter().enumerate() {
        let anchor = map_xy(plot, bounds, level.price, level.cumulative_volume);
        let value = DepthDatum::new(DepthSide::Bid, level.price, level.cumulative_volume);
        targets.push_circle(HitResult::new(0, index, value, anchor), anchor, 10.0);
    }
    for (index, level) in data.asks.iter().enumerate() {
        let anchor = map_xy(plot, bounds, level.price, level.cumulative_volume);
        let value = DepthDatum::new(DepthSide::Ask, level.price, level.cumulative_volume);
        targets.push_circle(HitResult::new(1, index, value, anchor), anchor, 10.0);
    }
    CartesianGeometry::new(bounds, chart_viewport_from_rect(plot), targets)
}

pub(crate) fn area_geometry(
    ctx: &DrawingContext<'_>,
    data: &AreaData,
    bounds: DataBounds,
) -> CartesianGeometry<AreaDatum> {
    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);
    let point_count = data.x_values.len();
    let mut cumulative = vec![0.0f32; point_count];
    let mut targets = HitTargets::new(chart_viewport_from_rect(plot));
    for (series_index, series) in data.series.iter().enumerate() {
        if series.values.is_empty() {
            continue;
        }
        for (index, ((cumulative_y, series_y), x)) in cumulative
            .iter_mut()
            .zip(&series.values)
            .zip(&data.x_values)
            .enumerate()
        {
            let y = if data.stacked {
                *cumulative_y + *series_y
            } else {
                *series_y
            };
            let anchor = map_xy(plot, bounds, *x, y);
            let value = AreaDatum::new(series_index, *x, y);
            targets.push_circle(
                HitResult::new(series_index, index, value, anchor),
                anchor,
                10.0,
            );
            if data.stacked {
                *cumulative_y = y;
            }
        }
    }
    CartesianGeometry::new(bounds, chart_viewport_from_rect(plot), targets)
}

pub(crate) fn pie_geometry(
    ctx: &DrawingContext<'_>,
    data: &[DataPoint],
    inner_radius: f32,
) -> HitTargets<SliceDatum> {
    let total: f32 = data.iter().map(|point| point.y.max(0.0)).sum();
    let plot = plot_rect(ctx, 0.06);
    let center = plot.center();
    let outer_r = plot.width().min(plot.height()) * 0.45;
    let inner_r = outer_r * inner_radius;
    let mid_r = (inner_r + outer_r) * 0.5;
    let mut angle = -FRAC_PI_2;
    let mut targets = HitTargets::new(chart_viewport_from_rect(plot));
    if total <= 0.0 {
        return targets;
    }
    for (index, point) in data.iter().enumerate() {
        let value = point.y.max(0.0);
        if value <= 0.0 {
            continue;
        }
        let sweep = TAU * (value / total);
        let end = angle + sweep;
        let mid = angle + sweep * 0.5;
        let anchor = Point::new(
            mid.cos().mul_add(mid_r, center.x),
            mid.sin().mul_add(mid_r, center.y),
        );
        let datum = SliceDatum::new(index, point.y, angle, end);
        targets.push_sector(
            HitResult::new(0, index, datum, anchor),
            center,
            inner_r,
            outer_r,
            angle,
            end,
        );
        angle = end;
    }
    targets
}

pub(crate) fn gauge_geometry(
    ctx: &DrawingContext<'_>,
    data: &GaugeData,
    start_angle: f32,
    end_angle: f32,
    inner_radius: f32,
    outer_radius: f32,
) -> HitTargets<SliceDatum> {
    let area = plot_rect(ctx, 0.02);
    let center = area.center();
    let min_dim = area.width().min(area.height());
    let outer_r = (min_dim * outer_radius).max(1.0);
    let inner_r = (min_dim * inner_radius).max(0.5);
    let normalized = data.normalized_value();
    let value_end = (end_angle - start_angle).mul_add(normalized, start_angle);
    let mid = (value_end - start_angle).mul_add(0.5, start_angle);
    let anchor = Point::new(
        mid.cos().mul_add((inner_r + outer_r) * 0.5, center.x),
        mid.sin().mul_add((inner_r + outer_r) * 0.5, center.y),
    );
    let mut targets = HitTargets::new(chart_viewport_from_rect(area));
    let datum = SliceDatum::new(0, data.value, start_angle, value_end);
    targets.push_sector(
        HitResult::new(0, 0, datum, anchor),
        center,
        inner_r,
        outer_r,
        start_angle,
        value_end,
    );
    targets
}

pub(crate) fn radar_geometry(ctx: &DrawingContext<'_>, data: &RadarData) -> HitTargets<RadarDatum> {
    let plot = plot_rect(ctx, 0.08);
    let center = plot.center();
    let radius = plot.width().min(plot.height()) * 0.45;
    let axis_count = u32_to_usize(data.axis_count);
    let max_value = data.max_value.max(1.0);
    let mut targets = HitTargets::new(chart_viewport_from_rect(plot));
    for (series_index, series) in data.series.iter().enumerate() {
        if series.values.len() < axis_count {
            continue;
        }
        for axis in 0..axis_count {
            let ratio = (series.values[axis] / max_value).clamp(0.0, 1.0);
            let angle = -FRAC_PI_2 + usize_to_f32(axis) * TAU / usize_to_f32(axis_count);
            let anchor = Point::new(
                (angle.cos() * radius).mul_add(ratio, center.x),
                (angle.sin() * radius).mul_add(ratio, center.y),
            );
            let label = data.labels.as_slice().get(axis).cloned();
            let datum = RadarDatum::new(axis, label, series.values[axis]);
            targets.push_circle(
                HitResult::new(series_index, axis, datum, anchor),
                anchor,
                10.0,
            );
        }
    }
    targets
}

pub(crate) fn heatmap_geometry(
    ctx: &DrawingContext<'_>,
    data: &HeatmapData,
) -> HitTargets<GridDatum> {
    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);
    let cell_w = plot.width() / u32_to_f32(data.cols.max(1));
    let cell_h = plot.height() / u32_to_f32(data.rows.max(1));
    let mut targets = HitTargets::new(chart_viewport_from_rect(plot));
    let rows = u32_to_usize(data.rows);
    let cols = u32_to_usize(data.cols);
    for row in 0..rows {
        for col in 0..cols {
            let idx = row * cols + col;
            let rect = Rect::new(
                Point::new(
                    usize_to_f32(col).mul_add(cell_w, plot.min_x()),
                    usize_to_f32(row).mul_add(cell_h, plot.min_y()),
                ),
                Size::new(cell_w.max(1.0), cell_h.max(1.0)),
            );
            let datum = GridDatum::new(row, col, data.values[idx]);
            targets.push_rect(HitResult::new(0, idx, datum, rect.center()), rect);
        }
    }
    targets
}

pub(crate) fn contour_geometry(
    ctx: &DrawingContext<'_>,
    data: &ContourData,
) -> HitTargets<GridDatum> {
    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);
    let cell_w = plot.width() / u32_to_f32((data.cols.saturating_sub(1)).max(1));
    let cell_h = plot.height() / u32_to_f32((data.rows.saturating_sub(1)).max(1));
    let mut targets = HitTargets::new(chart_viewport_from_rect(plot));
    if data.rows < 2 || data.cols < 2 {
        return targets;
    }
    let rows = u32_to_usize(data.rows);
    let cols = u32_to_usize(data.cols);
    for row in 0..(rows - 1) {
        for col in 0..(cols - 1) {
            let i00 = row * cols + col;
            let i10 = i00 + 1;
            let i01 = (row + 1) * cols + col;
            let i11 = i01 + 1;
            let average =
                (data.values[i00] + data.values[i10] + data.values[i01] + data.values[i11]) * 0.25;
            let rect = Rect::new(
                Point::new(
                    usize_to_f32(col).mul_add(cell_w, plot.min_x()),
                    usize_to_f32(row).mul_add(cell_h, plot.min_y()),
                ),
                Size::new(cell_w.max(1.0), cell_h.max(1.0)),
            );
            let datum = GridDatum::new(row, col, average);
            targets.push_rect(HitResult::new(0, i00, datum, rect.center()), rect);
        }
    }
    targets
}

pub(crate) fn choropleth_geometry(
    ctx: &DrawingContext<'_>,
    data: &ChoroplethData,
) -> HitTargets<RegionDatum> {
    let plot = plot_rect(ctx, 0.04);
    let [min_x, min_y, max_x, max_y] = data.bounds();
    let width = (max_x - min_x).max(1.0);
    let height = (max_y - min_y).max(1.0);
    let scale = (plot.width() / width).min(plot.height() / height);
    let content_w = width * scale;
    let content_h = height * scale;
    let offset_x = (plot.width() - content_w).mul_add(0.5, plot.min_x());
    let offset_y = (plot.height() - content_h).mul_add(0.5, plot.min_y());
    let mut targets = HitTargets::new(chart_viewport_from_rect(plot));
    for (index, polygon) in data.polygons.iter().enumerate() {
        if polygon.vertices.len() < 3 {
            continue;
        }
        let vertices: Vec<Point> = polygon
            .vertices
            .iter()
            .map(|vertex| {
                let x = (vertex[0] - min_x).mul_add(scale, offset_x);
                let y = (max_y - vertex[1]).mul_add(scale, offset_y);
                Point::new(x, y)
            })
            .collect();
        let [polygon_min_x, polygon_min_y, polygon_max_x, polygon_max_y] = polygon.bounds();
        let anchor = Point::new(
            (polygon_min_x + polygon_max_x)
                .mul_add(0.5, -min_x)
                .mul_add(scale, offset_x),
            (polygon_min_y + polygon_max_y)
                .mul_add(-0.5, max_y)
                .mul_add(scale, offset_y),
        );
        let datum = RegionDatum::new(index, polygon.id, polygon.value);
        targets.push_polygon(HitResult::new(0, index, datum, anchor), vertices);
    }
    targets
}

#[inline]
fn color_from_scale(scale: &ColorScale, value: f32, min: f32, max: f32) -> Srgb {
    if scale.stops.is_empty() {
        return Srgb::new(0.5, 0.5, 0.5);
    }

    let t = if max > min {
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let mut previous = scale.stops[0];
    if t <= previous.0 {
        return previous.1;
    }

    for &(stop_t, stop_color) in &scale.stops[1..] {
        if t <= stop_t {
            let span = (stop_t - previous.0).max(f32::EPSILON);
            let local_t = (t - previous.0) / span;
            return srgb_lerp(previous.1, stop_color, local_t);
        }
        previous = (stop_t, stop_color);
    }

    previous.1
}

#[inline]
fn viridis(value: f32, min: f32, max: f32) -> Srgb {
    let t = if max > min {
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let mut previous = VIRIDIS_STOPS[0];
    if t <= previous.0 {
        return previous.1;
    }

    for &(stop_t, stop_color) in &VIRIDIS_STOPS[1..] {
        if t <= stop_t {
            let span = (stop_t - previous.0).max(f32::EPSILON);
            let local_t = (t - previous.0) / span;
            return srgb_lerp(previous.1, stop_color, local_t);
        }
        previous = (stop_t, stop_color);
    }

    previous.1
}
pub(crate) fn draw_line(
    ctx: &mut DrawingContext<'_>,
    data: &[DataPoint],
    bounds: DataBounds,
    color: Srgb,
    line_width: f32,
    show_fill: bool,
    fill_opacity: f32,
) {
    if data.len() < 2 {
        return;
    }

    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);

    let mut line = Path::new();
    let first = map_xy(plot, bounds, data[0].x, data[0].y);
    line.move_to(first);

    for point in &data[1..] {
        line.line_to(map_xy(plot, bounds, point.x, point.y));
    }

    if show_fill {
        let baseline = map_xy(plot, bounds, data[data.len() - 1].x, bounds.min_y);
        line.line_to(baseline);
        line.line_to(map_xy(plot, bounds, data[0].x, bounds.min_y));
        line.close();

        ctx.save();
        ctx.set_global_alpha(fill_opacity);
        ctx.set_fill_style(color);
        ctx.fill_path(&line);
        ctx.restore();

        let mut stroke_path = Path::new();
        stroke_path.move_to(first);
        for point in &data[1..] {
            stroke_path.line_to(map_xy(plot, bounds, point.x, point.y));
        }
        ctx.set_line_width(line_width);
        ctx.set_stroke_style(color);
        ctx.stroke_path(&stroke_path);
        return;
    }

    ctx.set_line_width(line_width);
    ctx.set_stroke_style(color);
    ctx.stroke_path(&line);
}

pub(crate) fn draw_bar(
    ctx: &mut DrawingContext<'_>,
    data: &[DataPoint],
    bounds: DataBounds,
    color: Srgb,
) {
    if data.is_empty() {
        return;
    }

    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);

    let bar_width = (plot.width() / usize_to_f32(data.len())) * 0.7;
    let baseline_y = map_xy(plot, bounds, bounds.min_x, 0.0).y;

    ctx.set_fill_style(color);

    for point in data {
        let center = map_xy(plot, bounds, point.x, point.y);
        let top = center.y.min(baseline_y);
        let bottom = center.y.max(baseline_y);
        let rect = Rect::new(
            Point::new(center.x - bar_width * 0.5, top),
            Size::new(bar_width.max(1.0), (bottom - top).max(1.0)),
        );
        ctx.fill_rect(rect);
    }
}

pub(crate) fn draw_scatter(
    ctx: &mut DrawingContext<'_>,
    data: &[DataPoint],
    bounds: DataBounds,
    color: Srgb,
    radius: f32,
) {
    if data.is_empty() {
        return;
    }

    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);

    ctx.set_fill_style(color);
    for point in data {
        let p = map_xy(plot, bounds, point.x, point.y);
        ctx.fill_circle(p, radius);
    }
}

pub(crate) fn draw_bubble(
    ctx: &mut DrawingContext<'_>,
    data: &[BubblePoint],
    bounds: DataBounds,
    default_color: Srgb,
    min_radius: f32,
    max_radius: f32,
    opacity: f32,
) {
    if data.is_empty() {
        return;
    }

    let mut min_size = f32::MAX;
    let mut max_size = f32::MIN;

    for point in data {
        min_size = min_size.min(point.size);
        max_size = max_size.max(point.size);
    }

    let size_span = (max_size - min_size).max(1.0);
    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);

    for point in data {
        let center = map_xy(plot, bounds, point.x, point.y);
        let t = (point.size - min_size) / size_span;
        let radius = (max_radius - min_radius).mul_add(t, min_radius);

        let (bubble_color, bubble_alpha) = if point.color[3] > 0.0 {
            (from_rgba(point.color), alpha(point.color))
        } else {
            (default_color, opacity)
        };

        ctx.save();
        ctx.set_global_alpha((bubble_alpha * opacity).clamp(0.0, 1.0));
        ctx.set_fill_style(bubble_color);
        ctx.fill_circle(center, radius);
        ctx.restore();
    }
}

pub(crate) fn draw_candlestick(
    ctx: &mut DrawingContext<'_>,
    data: &[Candle],
    bounds: DataBounds,
    bullish: Srgb,
    bearish: Srgb,
) {
    if data.is_empty() {
        return;
    }

    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);
    let candle_width = (plot.width() / usize_to_f32(data.len()) * 0.65).max(1.0);

    for candle in data {
        let color = if candle.close >= candle.open {
            bullish
        } else {
            bearish
        };

        let x = map_xy(plot, bounds, candle.timestamp, candle.close).x;
        let high = map_xy(plot, bounds, candle.timestamp, candle.high).y;
        let low = map_xy(plot, bounds, candle.timestamp, candle.low).y;
        let open_y = map_xy(plot, bounds, candle.timestamp, candle.open).y;
        let close_y = map_xy(plot, bounds, candle.timestamp, candle.close).y;

        ctx.set_line_width(1.0);
        ctx.set_stroke_style(color);
        ctx.stroke_line(Point::new(x, high), Point::new(x, low));

        let top = open_y.min(close_y);
        let bottom = open_y.max(close_y);
        let body = Rect::new(
            Point::new(x - candle_width * 0.5, top),
            Size::new(candle_width, (bottom - top).max(1.0)),
        );
        ctx.set_fill_style(color);
        ctx.fill_rect(body);
    }
}

pub(crate) fn draw_depth(
    ctx: &mut DrawingContext<'_>,
    data: &DepthData,
    bounds: DataBounds,
    bid: Srgb,
    ask: Srgb,
) {
    if data.bids.is_empty() && data.asks.is_empty() {
        return;
    }

    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);

    let draw_side = |ctx: &mut DrawingContext<'_>,
                     levels: &[_],
                     color: Srgb,
                     value_get: fn(&crate::data::DepthLevel) -> (f32, f32)| {
        if levels.is_empty() {
            return;
        }
        let mut path = Path::new();
        let (first_x, _) = value_get(&levels[0]);
        let base_start = map_xy(plot, bounds, first_x, 0.0);
        path.move_to(base_start);

        for level in levels {
            let (x, y) = value_get(level);
            path.line_to(map_xy(plot, bounds, x, y));
        }

        let (last_x, _) = value_get(&levels[levels.len() - 1]);
        path.line_to(map_xy(plot, bounds, last_x, 0.0));
        path.close();

        ctx.save();
        ctx.set_global_alpha(0.28);
        ctx.set_fill_style(color);
        ctx.fill_path(&path);
        ctx.restore();

        let mut outline = Path::new();
        let (x0, y0) = value_get(&levels[0]);
        outline.move_to(map_xy(plot, bounds, x0, y0));
        for level in &levels[1..] {
            let (x, y) = value_get(level);
            outline.line_to(map_xy(plot, bounds, x, y));
        }
        ctx.set_line_width(2.0);
        ctx.set_stroke_style(color);
        ctx.stroke_path(&outline);
    };

    draw_side(ctx, &data.bids, bid, |level| {
        (level.price, level.cumulative_volume)
    });
    draw_side(ctx, &data.asks, ask, |level| {
        (level.price, level.cumulative_volume)
    });
}

pub(crate) fn draw_heatmap(ctx: &mut DrawingContext<'_>, data: &HeatmapData) {
    if data.rows == 0 || data.cols == 0 || data.values.is_empty() {
        return;
    }

    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);
    let cell_w = plot.width() / u32_to_f32(data.cols);
    let cell_h = plot.height() / u32_to_f32(data.rows);

    for row in 0..data.rows {
        for col in 0..data.cols {
            let idx = u32_to_usize(row * data.cols + col);
            let value = data.values[idx];
            let color = viridis(value, data.min_value, data.max_value);
            let rect = Rect::new(
                Point::new(
                    u32_to_f32(col).mul_add(cell_w, plot.min_x()),
                    u32_to_f32(row).mul_add(cell_h, plot.min_y()),
                ),
                Size::new(cell_w.max(1.0), cell_h.max(1.0)),
            );
            ctx.set_fill_style(color);
            ctx.fill_rect(rect);
        }
    }
}

fn contour_interpolate(p1: Point, p2: Point, v1: f32, v2: f32, level: f32) -> Point {
    let denom = (v2 - v1).abs();
    let t = if denom <= f32::EPSILON {
        0.5
    } else {
        (level - v1) / (v2 - v1)
    }
    .clamp(0.0, 1.0);

    Point::new(
        (p2.x - p1.x).mul_add(t, p1.x),
        (p2.y - p1.y).mul_add(t, p1.y),
    )
}

pub(crate) fn draw_contour(ctx: &mut DrawingContext<'_>, data: &ContourData, line_width: f32) {
    if data.rows < 2 || data.cols < 2 || data.values.is_empty() || data.levels.is_empty() {
        return;
    }

    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);
    let step_x = plot.width() / u32_to_f32(data.cols - 1);
    let step_y = plot.height() / u32_to_f32(data.rows - 1);

    for (level_index, &level) in data.levels.iter().enumerate() {
        let color = viridis(level, data.min_value, data.max_value);
        let mut path = Path::new();
        let mut has_segment = false;

        for row in 0..(data.rows - 1) {
            for col in 0..(data.cols - 1) {
                let i00 = u32_to_usize(row * data.cols + col);
                let i10 = i00 + 1;
                let i01 = u32_to_usize((row + 1) * data.cols + col);
                let i11 = i01 + 1;

                let v00 = data.values[i00];
                let v10 = data.values[i10];
                let v01 = data.values[i01];
                let v11 = data.values[i11];

                let p00 = Point::new(
                    u32_to_f32(col).mul_add(step_x, plot.min_x()),
                    u32_to_f32(row).mul_add(step_y, plot.min_y()),
                );
                let p10 = Point::new(p00.x + step_x, p00.y);
                let p01 = Point::new(p00.x, p00.y + step_y);
                let p11 = Point::new(p00.x + step_x, p00.y + step_y);

                let mut points = [None, None, None, None];
                if (v00 > level) != (v10 > level) {
                    points[0] = Some(contour_interpolate(p00, p10, v00, v10, level));
                }
                if (v10 > level) != (v11 > level) {
                    points[1] = Some(contour_interpolate(p10, p11, v10, v11, level));
                }
                if (v01 > level) != (v11 > level) {
                    points[2] = Some(contour_interpolate(p01, p11, v01, v11, level));
                }
                if (v00 > level) != (v01 > level) {
                    points[3] = Some(contour_interpolate(p00, p01, v00, v01, level));
                }

                let edges: Vec<Point> = points.into_iter().flatten().collect();
                if edges.len() == 2 {
                    path.move_to(edges[0]);
                    path.line_to(edges[1]);
                    has_segment = true;
                } else if edges.len() == 4 {
                    path.move_to(edges[0]);
                    path.line_to(edges[1]);
                    path.move_to(edges[2]);
                    path.line_to(edges[3]);
                    has_segment = true;
                }
            }
        }

        if has_segment {
            ctx.set_stroke_style(color);
            ctx.set_line_width(line_width);
            ctx.stroke_path(&path);
        }

        if level_index == 0 {
            ctx.save();
            ctx.set_global_alpha(0.08);
            draw_heatmap(
                ctx,
                &HeatmapData {
                    rows: data.rows,
                    cols: data.cols,
                    values: data.values.clone(),
                    min_value: data.min_value,
                    max_value: data.max_value,
                },
            );
            ctx.restore();
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Gauge rendering consumes independent geometric and color parameters from the public builder state without allocating an intermediate style object"
)]
pub(crate) fn draw_gauge(
    ctx: &mut DrawingContext<'_>,
    data: &GaugeData,
    start_angle: f32,
    end_angle: f32,
    inner_radius: f32,
    outer_radius: f32,
    background_color: Srgb,
    value_color: Srgb,
    needle_color: Srgb,
) {
    let area = plot_rect(ctx, 0.02);
    let center = area.center();
    let min_dim = area.width().min(area.height());
    let outer_r = (min_dim * outer_radius).max(1.0);
    let inner_r = (min_dim * inner_radius).max(0.5);
    let stroke_w = (outer_r - inner_r).max(1.0);
    let ring_r = inner_r + stroke_w * 0.5;

    let mut background = Path::new();
    background.arc(center, ring_r, start_angle, end_angle, false);
    ctx.set_line_width(stroke_w);
    ctx.set_stroke_style(background_color);
    ctx.stroke_path(&background);

    let normalized = data.normalized_value();
    let value_end = (end_angle - start_angle).mul_add(normalized, start_angle);
    let value_span = data.max_value - data.min_value;

    if data.regions.is_empty() {
        let mut value_arc = Path::new();
        value_arc.arc(center, ring_r, start_angle, value_end, false);
        ctx.set_line_width(stroke_w);
        ctx.set_stroke_style(value_color);
        ctx.stroke_path(&value_arc);
    } else {
        assert!(
            value_span.is_finite() && value_span > 0.0,
            "draw_gauge requires finite max_value > min_value"
        );
        let mut last_threshold = data.min_value;
        for region in &data.regions {
            let start_t = ((last_threshold - data.min_value) / value_span).clamp(0.0, 1.0);
            let end_t = ((region.threshold - data.min_value) / value_span).clamp(0.0, 1.0);
            let seg_start = (end_angle - start_angle).mul_add(start_t, start_angle);
            let seg_end = (end_angle - start_angle).mul_add(end_t, start_angle);
            let mut segment = Path::new();
            segment.arc(center, ring_r, seg_start, seg_end, false);
            ctx.set_line_width(stroke_w);
            ctx.set_stroke_style(from_rgba(region.color));
            ctx.stroke_path(&segment);
            last_threshold = region.threshold;
        }
    }

    if data.show_needle {
        let needle_len = outer_r * 0.95;
        let tip = Point::new(
            value_end.cos().mul_add(needle_len, center.x),
            value_end.sin().mul_add(needle_len, center.y),
        );
        ctx.set_line_width((stroke_w * 0.12).max(2.0));
        ctx.set_stroke_style(needle_color);
        ctx.stroke_line(center, tip);

        ctx.set_fill_style(needle_color);
        ctx.fill_circle(center, (stroke_w * 0.16).max(3.0));
    }
}

pub(crate) fn draw_radar(
    ctx: &mut DrawingContext<'_>,
    data: &RadarData,
    ring_count: u32,
    line_width: f32,
    fill_opacity: f32,
) {
    if data.axis_count < 3 || data.series.is_empty() {
        return;
    }

    let plot = plot_rect(ctx, 0.08);
    let center = plot.center();
    let radius = plot.width().min(plot.height()) * 0.45;
    let axis_count = u32_to_usize(data.axis_count);
    let max_value = data.max_value.max(1.0);

    ctx.set_line_width(1.0);
    ctx.set_stroke_style(Srgb::new(0.5, 0.5, 0.5));

    for ring in 1..=ring_count.max(1) {
        let t = u32_to_f32(ring) / u32_to_f32(ring_count.max(1));
        let r = radius * t;
        let mut path = Path::new();
        for axis in 0..axis_count {
            let angle = -FRAC_PI_2 + usize_to_f32(axis) * TAU / usize_to_f32(axis_count);
            let p = Point::new(
                angle.cos().mul_add(r, center.x),
                angle.sin().mul_add(r, center.y),
            );
            if axis == 0 {
                path.move_to(p);
            } else {
                path.line_to(p);
            }
        }
        path.close();
        ctx.stroke_path(&path);
    }

    for axis in 0..axis_count {
        let angle = -FRAC_PI_2 + usize_to_f32(axis) * TAU / usize_to_f32(axis_count);
        let end = Point::new(
            angle.cos().mul_add(radius, center.x),
            angle.sin().mul_add(radius, center.y),
        );
        ctx.stroke_line(center, end);
    }

    for series in &data.series {
        if series.values.len() < axis_count {
            continue;
        }

        let mut poly = Path::new();
        for axis in 0..axis_count {
            let ratio = (series.values[axis] / max_value).clamp(0.0, 1.0);
            let angle = -FRAC_PI_2 + usize_to_f32(axis) * TAU / usize_to_f32(axis_count);
            let p = Point::new(
                (angle.cos() * radius).mul_add(ratio, center.x),
                (angle.sin() * radius).mul_add(ratio, center.y),
            );
            if axis == 0 {
                poly.move_to(p);
            } else {
                poly.line_to(p);
            }
        }
        poly.close();

        let color = from_rgba(series.color);
        let series_alpha = alpha(series.color);

        ctx.save();
        ctx.set_global_alpha((fill_opacity * series_alpha).clamp(0.0, 1.0));
        ctx.set_fill_style(color);
        ctx.fill_path(&poly);
        ctx.restore();

        ctx.set_line_width(line_width);
        ctx.set_stroke_style(color);
        ctx.stroke_path(&poly);
    }
}

pub(crate) fn draw_pie(
    ctx: &mut DrawingContext<'_>,
    data: &[DataPoint],
    colors: &[Srgb],
    inner_radius: f32,
) {
    if data.is_empty() {
        return;
    }

    let total: f32 = data.iter().map(|point| point.y.max(0.0)).sum();
    if total <= 0.0 {
        return;
    }

    let plot = plot_rect(ctx, 0.06);
    let center = plot.center();
    let outer_r = plot.width().min(plot.height()) * 0.45;
    let inner_r = outer_r * inner_radius;

    let mut angle = -FRAC_PI_2;
    for (index, point) in data.iter().enumerate() {
        let value = point.y.max(0.0);
        if value <= 0.0 {
            continue;
        }
        let sweep = TAU * (value / total);
        let end = angle + sweep;

        let color = colors.get(index).map_or_else(
            || Srgb::from_u32(PIE_DEFAULT_COLORS[index % PIE_DEFAULT_COLORS.len()]),
            |custom| *custom,
        );

        let mut path = Path::new();
        if inner_r > 0.0 {
            let outer_start = Point::new(
                angle.cos().mul_add(outer_r, center.x),
                angle.sin().mul_add(outer_r, center.y),
            );
            let inner_end = Point::new(
                end.cos().mul_add(inner_r, center.x),
                end.sin().mul_add(inner_r, center.y),
            );
            path.move_to(outer_start);
            path.arc(center, outer_r, angle, end, false);
            path.line_to(inner_end);
            path.arc(center, inner_r, end, angle, true);
            path.close();
        } else {
            path.move_to(center);
            path.line_to(Point::new(
                angle.cos().mul_add(outer_r, center.x),
                angle.sin().mul_add(outer_r, center.y),
            ));
            path.arc(center, outer_r, angle, end, false);
        }
        path.close();

        ctx.set_fill_style(color);
        ctx.fill_path(&path);

        angle = end;
    }
}

pub(crate) fn draw_choropleth(
    ctx: &mut DrawingContext<'_>,
    data: &ChoroplethData,
    stroke_width: f32,
    stroke_color: Srgb,
    show_stroke: bool,
) {
    if data.polygons.is_empty() {
        return;
    }

    let plot = plot_rect(ctx, 0.04);
    let [min_x, min_y, max_x, max_y] = data.bounds();
    let width = (max_x - min_x).max(1.0);
    let height = (max_y - min_y).max(1.0);

    let scale = (plot.width() / width).min(plot.height() / height);
    let content_w = width * scale;
    let content_h = height * scale;
    let offset_x = (plot.width() - content_w).mul_add(0.5, plot.min_x());
    let offset_y = (plot.height() - content_h).mul_add(0.5, plot.min_y());

    for polygon in &data.polygons {
        if polygon.vertices.len() < 3 {
            continue;
        }

        let mut path = Path::new();
        for (index, vertex) in polygon.vertices.iter().enumerate() {
            let x = (vertex[0] - min_x).mul_add(scale, offset_x);
            let y = (max_y - vertex[1]).mul_add(scale, offset_y);
            let point = Point::new(x, y);
            if index == 0 {
                path.move_to(point);
            } else {
                path.line_to(point);
            }
        }
        path.close();

        let fill_color = color_from_scale(
            &data.color_scale,
            polygon.value,
            data.min_value,
            data.max_value,
        );
        ctx.set_fill_style(fill_color);
        ctx.fill_path(&path);

        if show_stroke {
            ctx.set_line_width(stroke_width);
            ctx.set_stroke_style(stroke_color);
            ctx.stroke_path(&path);
        }
    }
}

pub(crate) fn draw_area(ctx: &mut DrawingContext<'_>, data: &AreaData, bounds: DataBounds) {
    if data.x_values.is_empty() || data.series.is_empty() {
        return;
    }

    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);
    let point_count = data.x_values.len();
    let mut cumulative = vec![0.0f32; point_count];

    for series in &data.series {
        if series.values.is_empty() {
            continue;
        }

        let color = from_rgba(series.color);
        let opacity = alpha(series.color);

        let mut path = Path::new();

        if data.stacked {
            let first_top = cumulative[0] + series.values[0];
            path.move_to(map_xy(plot, bounds, data.x_values[0], cumulative[0]));
            path.line_to(map_xy(plot, bounds, data.x_values[0], first_top));

            for ((cumulative_y, series_y), x) in cumulative
                .iter()
                .zip(&series.values)
                .zip(&data.x_values)
                .skip(1)
            {
                let top = *cumulative_y + *series_y;
                path.line_to(map_xy(plot, bounds, *x, top));
            }

            for ((cumulative_y, series_y), x) in cumulative
                .iter_mut()
                .zip(&series.values)
                .zip(&data.x_values)
                .rev()
            {
                path.line_to(map_xy(plot, bounds, *x, *cumulative_y));
                *cumulative_y += *series_y;
            }
        } else {
            path.move_to(map_xy(plot, bounds, data.x_values[0], 0.0));
            path.line_to(map_xy(plot, bounds, data.x_values[0], series.values[0]));

            for index in 1..point_count.min(series.values.len()) {
                path.line_to(map_xy(
                    plot,
                    bounds,
                    data.x_values[index],
                    series.values[index],
                ));
            }

            for index in (0..point_count.min(series.values.len())).rev() {
                path.line_to(map_xy(plot, bounds, data.x_values[index], 0.0));
            }
        }

        path.close();

        ctx.save();
        ctx.set_global_alpha(opacity);
        ctx.set_fill_style(color);
        ctx.fill_path(&path);
        ctx.restore();

        let mut top_line = Path::new();
        top_line.move_to(map_xy(
            plot,
            bounds,
            data.x_values[0],
            if data.stacked {
                cumulative[0]
            } else {
                series.values[0]
            },
        ));
        for ((cumulative_y, series_y), x) in cumulative
            .iter()
            .zip(&series.values)
            .zip(&data.x_values)
            .skip(1)
        {
            let y = if data.stacked {
                *cumulative_y
            } else {
                *series_y
            };
            top_line.line_to(map_xy(plot, bounds, *x, y));
        }
        ctx.set_line_width(1.5);
        ctx.set_stroke_style(color);
        ctx.stroke_path(&top_line);
    }
}
