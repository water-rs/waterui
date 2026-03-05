extern crate alloc;

use alloc::vec::Vec;
use core::f32::consts::{FRAC_PI_2, PI, TAU};

use nami::Signal;
use waterui_canvas::{Canvas, DrawingContext, Path};
use waterui_core::View;
use waterui_core::dynamic::Dynamic;
use waterui_core::layout::{Point, Rect, Size};
use waterui_graphics::color::Srgb;

use crate::data::{
    AreaData, BubblePoint, Candle, ChoroplethData, ColorScale, ContourData, DataBounds, DataPoint,
    DepthData, GaugeData, HeatmapData, RadarData,
};

const PLOT_PADDING_RATIO: f32 = 0.1;
const PIE_DEFAULT_COLORS: [u32; 8] = [
    0x3B82F6, 0x22C55E, 0xEF4444, 0xF59E0B, 0x8B5CF6, 0xEC4899, 0x06B6D4, 0xF97316,
];
const VIRIDIS_STOPS: [(f32, Srgb); 5] = [
    (0.0, Srgb::from_hex("#440154")),
    (0.25, Srgb::from_hex("#3B528B")),
    (0.5, Srgb::from_hex("#21918C")),
    (0.75, Srgb::from_hex("#5EC962")),
    (1.0, Srgb::from_hex("#FDE725")),
];

pub(crate) fn reactive_canvas<S, D, F>(signal: S, make_canvas: F) -> impl View
where
    S: Signal<Output = D> + 'static,
    D: 'static,
    F: Fn(D) -> Canvas + 'static,
{
    Dynamic::watch(signal, move |data| make_canvas(data))
}

#[inline]
fn srgb_lerp(a: Srgb, b: Srgb, t: f32) -> Srgb {
    let t = t.clamp(0.0, 1.0);
    Srgb::new(
        a.red + (b.red - a.red) * t,
        a.green + (b.green - a.green) * t,
        a.blue + (b.blue - a.blue) * t,
    )
}

#[inline]
fn from_rgba(color: [f32; 4]) -> Srgb {
    Srgb::new(color[0], color[1], color[2])
}

#[inline]
fn alpha(color: [f32; 4]) -> f32 {
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
fn plot_rect(ctx: &DrawingContext<'_>, padding_ratio: f32) -> Rect {
    let width = ctx.width;
    let height = ctx.height;
    let inset_x = width * padding_ratio;
    let inset_y = height * padding_ratio;
    Rect::new(
        Point::new(inset_x, inset_y),
        Size::new(
            (width - inset_x * 2.0).max(1.0),
            (height - inset_y * 2.0).max(1.0),
        ),
    )
}

#[inline]
fn map_xy(plot: Rect, bounds: DataBounds, x: f32, y: f32) -> Point {
    let nx = (x - bounds.min_x) / (bounds.max_x - bounds.min_x);
    let ny = (y - bounds.min_y) / (bounds.max_y - bounds.min_y);
    Point::new(
        plot.min_x() + nx * plot.width(),
        plot.max_y() - ny * plot.height(),
    )
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
    color: Srgb,
    line_width: f32,
    show_fill: bool,
    fill_opacity: f32,
) {
    if data.len() < 2 {
        return;
    }

    let bounds = normalize_bounds(DataBounds::from_points(data).with_padding(0.1));
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

pub(crate) fn draw_bar(ctx: &mut DrawingContext<'_>, data: &[DataPoint], color: Srgb) {
    if data.is_empty() {
        return;
    }

    let source_bounds = DataBounds::from_points(data);
    let bounds = normalize_bounds(DataBounds::new(
        source_bounds.min_x,
        source_bounds.max_x,
        source_bounds.min_y.min(0.0),
        source_bounds.max_y.max(0.0),
    ));
    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);

    let bar_width = (plot.width() / data.len() as f32) * 0.7;
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
    color: Srgb,
    radius: f32,
) {
    if data.is_empty() {
        return;
    }

    let bounds = normalize_bounds(DataBounds::from_points(data).with_padding(0.1));
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
    default_color: Srgb,
    min_radius: f32,
    max_radius: f32,
    opacity: f32,
) {
    if data.is_empty() {
        return;
    }

    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    let mut min_size = f32::MAX;
    let mut max_size = f32::MIN;

    for point in data {
        min_x = min_x.min(point.x);
        max_x = max_x.max(point.x);
        min_y = min_y.min(point.y);
        max_y = max_y.max(point.y);
        min_size = min_size.min(point.size);
        max_size = max_size.max(point.size);
    }

    let bounds = normalize_bounds(DataBounds::new(min_x, max_x, min_y, max_y).with_padding(0.1));
    let size_span = (max_size - min_size).max(1.0);
    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);

    for point in data {
        let center = map_xy(plot, bounds, point.x, point.y);
        let t = (point.size - min_size) / size_span;
        let radius = min_radius + (max_radius - min_radius) * t;

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
    bullish: Srgb,
    bearish: Srgb,
) {
    if data.is_empty() {
        return;
    }

    let bounds = normalize_bounds(DataBounds::from_candles(data).with_padding(0.05));
    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);
    let candle_width = (plot.width() / data.len() as f32 * 0.65).max(1.0);

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

pub(crate) fn draw_depth(ctx: &mut DrawingContext<'_>, data: &DepthData, bid: Srgb, ask: Srgb) {
    if data.bids.is_empty() && data.asks.is_empty() {
        return;
    }

    let bounds = normalize_bounds(data.bounds().with_padding(0.04));
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
    let cell_w = plot.width() / data.cols as f32;
    let cell_h = plot.height() / data.rows as f32;

    for row in 0..data.rows {
        for col in 0..data.cols {
            let idx = (row * data.cols + col) as usize;
            let value = data.values[idx];
            let color = viridis(value, data.min_value, data.max_value);
            let rect = Rect::new(
                Point::new(
                    plot.min_x() + col as f32 * cell_w,
                    plot.min_y() + row as f32 * cell_h,
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

    Point::new(p1.x + (p2.x - p1.x) * t, p1.y + (p2.y - p1.y) * t)
}

pub(crate) fn draw_contour(ctx: &mut DrawingContext<'_>, data: &ContourData, line_width: f32) {
    if data.rows < 2 || data.cols < 2 || data.values.is_empty() || data.levels.is_empty() {
        return;
    }

    let plot = plot_rect(ctx, PLOT_PADDING_RATIO);
    let step_x = plot.width() / (data.cols - 1) as f32;
    let step_y = plot.height() / (data.rows - 1) as f32;

    for (level_index, &level) in data.levels.iter().enumerate() {
        let color = viridis(level, data.min_value, data.max_value);
        let mut path = Path::new();
        let mut has_segment = false;

        for row in 0..(data.rows - 1) {
            for col in 0..(data.cols - 1) {
                let i00 = (row * data.cols + col) as usize;
                let i10 = i00 + 1;
                let i01 = ((row + 1) * data.cols + col) as usize;
                let i11 = i01 + 1;

                let v00 = data.values[i00];
                let v10 = data.values[i10];
                let v01 = data.values[i01];
                let v11 = data.values[i11];

                let p00 = Point::new(
                    plot.min_x() + col as f32 * step_x,
                    plot.min_y() + row as f32 * step_y,
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
    let value_end = start_angle + (end_angle - start_angle) * normalized;

    if data.regions.is_empty() {
        let mut value_arc = Path::new();
        value_arc.arc(center, ring_r, start_angle, value_end, false);
        ctx.set_line_width(stroke_w);
        ctx.set_stroke_style(value_color);
        ctx.stroke_path(&value_arc);
    } else {
        let mut last_threshold = data.min_value;
        for region in &data.regions {
            let start_t = ((last_threshold - data.min_value) / (data.max_value - data.min_value))
                .clamp(0.0, 1.0);
            let end_t = ((region.threshold - data.min_value) / (data.max_value - data.min_value))
                .clamp(0.0, 1.0);
            let seg_start = start_angle + (end_angle - start_angle) * start_t;
            let seg_end = start_angle + (end_angle - start_angle) * end_t;
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
            center.x + value_end.cos() * needle_len,
            center.y + value_end.sin() * needle_len,
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
    let axis_count = data.axis_count as usize;
    let max_value = data.max_value.max(1.0);

    ctx.set_line_width(1.0);
    ctx.set_stroke_style(Srgb::new(0.5, 0.5, 0.5));

    for ring in 1..=ring_count.max(1) {
        let t = ring as f32 / ring_count.max(1) as f32;
        let r = radius * t;
        let mut path = Path::new();
        for axis in 0..axis_count {
            let angle = -FRAC_PI_2 + axis as f32 * TAU / axis_count as f32;
            let p = Point::new(center.x + angle.cos() * r, center.y + angle.sin() * r);
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
        let angle = -FRAC_PI_2 + axis as f32 * TAU / axis_count as f32;
        let end = Point::new(
            center.x + angle.cos() * radius,
            center.y + angle.sin() * radius,
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
            let angle = -FRAC_PI_2 + axis as f32 * TAU / axis_count as f32;
            let p = Point::new(
                center.x + angle.cos() * radius * ratio,
                center.y + angle.sin() * radius * ratio,
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

        let color = if let Some(custom) = colors.get(index) {
            *custom
        } else {
            Srgb::from_u32(PIE_DEFAULT_COLORS[index % PIE_DEFAULT_COLORS.len()])
        };

        let mut path = Path::new();
        if inner_r > 0.0 {
            let outer_start = Point::new(
                center.x + angle.cos() * outer_r,
                center.y + angle.sin() * outer_r,
            );
            let inner_end = Point::new(
                center.x + end.cos() * inner_r,
                center.y + end.sin() * inner_r,
            );
            path.move_to(outer_start);
            path.arc(center, outer_r, angle, end, false);
            path.line_to(inner_end);
            path.arc(center, inner_r, end, angle, true);
            path.close();
        } else {
            path.move_to(center);
            path.line_to(Point::new(
                center.x + angle.cos() * outer_r,
                center.y + angle.sin() * outer_r,
            ));
            path.arc(center, outer_r, angle, end, false);
            path.close();
        }

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
    let offset_x = plot.min_x() + (plot.width() - content_w) * 0.5;
    let offset_y = plot.min_y() + (plot.height() - content_h) * 0.5;

    for polygon in &data.polygons {
        if polygon.vertices.len() < 3 {
            continue;
        }

        let mut path = Path::new();
        for (index, vertex) in polygon.vertices.iter().enumerate() {
            let x = offset_x + (vertex[0] - min_x) * scale;
            let y = offset_y + (max_y - vertex[1]) * scale;
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

pub(crate) fn draw_area(ctx: &mut DrawingContext<'_>, data: &AreaData) {
    if data.x_values.is_empty() || data.series.is_empty() {
        return;
    }

    let bounds = normalize_bounds(data.bounds().with_padding(0.05));
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

            for index in 1..point_count.min(series.values.len()) {
                let top = cumulative[index] + series.values[index];
                path.line_to(map_xy(plot, bounds, data.x_values[index], top));
            }

            for index in (0..point_count.min(series.values.len())).rev() {
                path.line_to(map_xy(
                    plot,
                    bounds,
                    data.x_values[index],
                    cumulative[index],
                ));
                cumulative[index] += series.values[index];
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
        for index in 1..point_count.min(series.values.len()) {
            let y = if data.stacked {
                cumulative[index]
            } else {
                series.values[index]
            };
            top_line.line_to(map_xy(plot, bounds, data.x_values[index], y));
        }
        ctx.set_line_width(1.5);
        ctx.set_stroke_style(color);
        ctx.stroke_path(&top_line);
    }
}
