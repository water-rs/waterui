use core::future::Future;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use waterui_chart::animation::{ChartAnimation, EasingType};
use waterui_chart::data::{
    AreaData, AreaSeries, BubblePoint, Candle, ChoroplethData, ColorScale, ContourData, DataPoint,
    DepthData, DepthLevel, GaugeData, GaugeRegion, GeoPolygon, HeatmapData, RadarData, RadarSeries,
};
use waterui_chart::renderer::{
    AreaRenderer, BarChartRenderer, BubbleRenderer, CandlestickRenderer, ChartRenderer,
    ChoroplethRenderer, ContourRenderer, DepthRenderer, GaugeRenderer, HeatmapRenderer,
    LineChartRenderer, PieChartRenderer, RadarRenderer, ScatterChartRenderer,
};
use waterui_graphics::color::Srgb;
use waterui_graphics::{
    GpuContext, GpuFrame, GpuRenderer, GpuSurface, OffscreenRenderConfig, OffscreenSize,
};

#[derive(Debug, Clone, Copy)]
struct VisualStats {
    opaque_pixels: usize,
    non_uniform: bool,
}

fn alpha_bbox_dimensions(
    rgba: &[u8],
    width: u32,
    height: u32,
    alpha_threshold: u8,
) -> Option<(u32, u32)> {
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0_u32;
    let mut max_y = 0_u32;
    let mut found = false;

    for (idx, px) in rgba.chunks_exact(4).enumerate() {
        if px[3] <= alpha_threshold {
            continue;
        }
        let x = (idx as u32) % width;
        let y = (idx as u32) / width;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
        found = true;
    }

    if !found {
        return None;
    }

    Some((max_x - min_x + 1, max_y - min_y + 1))
}

#[derive(Debug)]
struct SeededChart<R>
where
    R: ChartRenderer,
    R::Data: Clone,
{
    renderer: R,
    initial_data: R::Data,
    transition_data: Option<R::Data>,
    animation: ChartAnimation,
}

impl<R> SeededChart<R>
where
    R: ChartRenderer,
    R::Data: Clone,
{
    fn static_frame(renderer: R, data: R::Data) -> Self {
        Self {
            renderer,
            initial_data: data,
            transition_data: None,
            animation: ChartAnimation::new(),
        }
    }

    fn transition_frame(renderer: R, initial_data: R::Data, transition_data: R::Data) -> Self {
        Self {
            renderer,
            initial_data,
            transition_data: Some(transition_data),
            animation: ChartAnimation {
                time: 0.15,
                progress: 0.45,
                easing: EasingType::EaseInOut as u32,
                entry_active: 0,
            },
        }
    }
}

impl<R> GpuRenderer for SeededChart<R>
where
    R: ChartRenderer,
    R::Data: Clone,
{
    fn setup(&mut self, ctx: &GpuContext) -> impl Future<Output = ()> {
        self.renderer
            .update_data(&self.initial_data, ctx.device, ctx.queue);
        let transition_data = self.transition_data.clone();
        let animation = self.animation;
        let renderer = &mut self.renderer;
        async move {
            renderer.setup(ctx).await;
            if let Some(next) = transition_data {
                renderer.update_data(&next, ctx.device, ctx.queue);
            }
            renderer.set_animation(&animation);
        }
    }

    fn render(&mut self, frame: &GpuFrame) {
        self.renderer.set_animation(&self.animation);
        self.renderer.render(frame);
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.renderer.resize(width, height);
    }
}

fn analyze_output(name: &str, rgba: &[u8], width: u32, height: u32) -> VisualStats {
    let expected_len = (width as usize) * (height as usize) * 4;
    assert_eq!(
        rgba.len(),
        expected_len,
        "{name}: unexpected rgba size, expected {expected_len}, got {}",
        rgba.len()
    );

    let opaque_pixels = rgba.chunks_exact(4).filter(|px| px[3] > 0).count();
    assert!(
        opaque_pixels > 128,
        "{name}: image appears empty or fully transparent (opaque_pixels={opaque_pixels})"
    );

    let first = [rgba[0], rgba[1], rgba[2], rgba[3]];
    let non_uniform = rgba
        .chunks_exact(4)
        .any(|px| [px[0], px[1], px[2], px[3]] != first);
    assert!(non_uniform, "{name}: image is fully uniform");

    // Radial charts should stay close to isotropic geometry on non-square surfaces.
    if matches!(name, "gauge" | "radar" | "pie_donut") {
        let (bbox_w, bbox_h) = alpha_bbox_dimensions(rgba, width, height, 8)
            .expect("radial chart must have visible alpha footprint");
        assert!(
            bbox_w > 32 && bbox_h > 32,
            "{name}: radial chart alpha bbox too small: {bbox_w}x{bbox_h}"
        );
        let ratio = bbox_w as f32 / bbox_h as f32;
        assert!(
            (0.75..=1.35).contains(&ratio),
            "{name}: likely aspect distortion, alpha bbox ratio={ratio:.3} (bbox={bbox_w}x{bbox_h})"
        );
    }

    VisualStats {
        opaque_pixels,
        non_uniform,
    }
}

fn render_case<R>(
    name: &str,
    seeded: SeededChart<R>,
    size: OffscreenSize,
    out_dir: &Path,
) -> Result<VisualStats, String>
where
    R: ChartRenderer + 'static,
    R::Data: Clone,
{
    let output = GpuSurface::new(seeded)
        .render_offscreen(OffscreenRenderConfig::new(size))
        .map_err(|e| format!("{name}: offscreen render failed: {e}"))?;

    let png_path = out_dir.join(format!("{name}.png"));
    output
        .save_png(&png_path)
        .map_err(|e| format!("{name}: failed to save png {:?}: {e}", png_path))?;
    Ok(analyze_output(
        name,
        &output.rgba8,
        output.width,
        output.height,
    ))
}

fn basic_points() -> Vec<DataPoint> {
    (0..32)
        .map(|i| {
            let x = i as f32;
            let y = 48.0 + (x * 0.35).sin() * 20.0 + (x * 0.11).cos() * 8.0;
            DataPoint::new(x, y)
        })
        .collect()
}

fn shifted_points() -> Vec<DataPoint> {
    (0..32)
        .map(|i| {
            let x = i as f32;
            let y = 50.0 + (x * 0.43).sin() * 22.0 + (x * 0.07).cos() * 6.0;
            DataPoint::new(x, y)
        })
        .collect()
}

fn bubble_points() -> Vec<BubblePoint> {
    (0..48)
        .map(|i| {
            let x = i as f32 * 0.5;
            let y = 40.0 + (x * 0.27).sin() * 25.0;
            let size = 4.0 + (i % 9) as f32 * 2.5;
            BubblePoint::new(x, y, size)
        })
        .collect()
}

fn candle_points() -> Vec<Candle> {
    let mut out = Vec::with_capacity(60);
    let mut price = 120.0_f32;
    for i in 0..60 {
        let t = i as f32 * 60.0;
        let drift = (i as f32 * 0.17).sin() * 3.5;
        let open = price;
        let close = open + drift;
        let high = open.max(close) + 1.2 + (i % 5) as f32 * 0.3;
        let low = open.min(close) - 1.0 - (i % 4) as f32 * 0.2;
        let volume = 30_000.0 + (i as f32 * 180.0);
        out.push(Candle::new(t, open, high, low, close, volume));
        price = close;
    }
    out
}

fn depth_data() -> DepthData {
    let mut bids = Vec::new();
    let mut asks = Vec::new();
    let mut bid_cum = 0.0_f32;
    let mut ask_cum = 0.0_f32;
    for i in 0..40 {
        let p_bid = 99.8 - i as f32 * 0.05;
        bid_cum += 4.0 + (i % 6) as f32 * 0.9;
        bids.push(DepthLevel::new(p_bid, bid_cum));

        let p_ask = 100.2 + i as f32 * 0.05;
        ask_cum += 4.3 + (i % 5) as f32 * 0.8;
        asks.push(DepthLevel::new(p_ask, ask_cum));
    }
    DepthData::new(bids, asks)
}

fn heatmap_data(rows: u32, cols: u32) -> HeatmapData {
    let mut values = Vec::with_capacity((rows * cols) as usize);
    for r in 0..rows {
        for c in 0..cols {
            let x = c as f32 / cols as f32;
            let y = r as f32 / rows as f32;
            let v = (x * 12.0).sin() * 0.6 + (y * 10.0).cos() * 0.4 + (x - 0.5) * (y - 0.5) * 2.0;
            values.push(v);
        }
    }
    HeatmapData::try_new(rows, cols, values).expect("heatmap seed should be valid")
}

fn contour_data(rows: u32, cols: u32) -> ContourData {
    let mut values = Vec::with_capacity((rows * cols) as usize);
    for r in 0..rows {
        for c in 0..cols {
            let x = c as f32 / (cols - 1) as f32 * 2.0 - 1.0;
            let y = r as f32 / (rows - 1) as f32 * 2.0 - 1.0;
            let d = (x * x + y * y).sqrt();
            let v = (1.0 - d * 0.8) + (x * 3.0).sin() * 0.15 + (y * 4.0).cos() * 0.1;
            values.push(v);
        }
    }
    ContourData::try_new(rows, cols, values, 10).expect("contour seed should be valid")
}

fn radar_data() -> RadarData {
    RadarData::try_new(6)
        .expect("radar axis count should be valid")
        .labels(vec!["A", "B", "C", "D", "E", "F"])
        .series(
            RadarSeries::new("alpha", vec![72.0, 35.0, 88.0, 61.0, 54.0, 77.0])
                .color_hex("#3B82F6"),
        )
        .series(
            RadarSeries::new("beta", vec![44.0, 68.0, 52.0, 79.0, 70.0, 49.0]).color_hex("#EF4444"),
        )
        .max_value(100.0)
}

fn area_data() -> AreaData {
    let x_values: Vec<f32> = (0..24).map(|v| v as f32).collect();
    AreaData::new(x_values)
        .series(
            AreaSeries::new(
                "cpu",
                (0..24)
                    .map(|i| 35.0 + (i as f32 * 0.23).sin() * 18.0 + 8.0)
                    .collect(),
            )
            .color_hex("#3B82F6"),
        )
        .series(
            AreaSeries::new(
                "mem",
                (0..24)
                    .map(|i| 20.0 + (i as f32 * 0.31).cos() * 12.0 + 6.0)
                    .collect(),
            )
            .color_hex("#22C55E"),
        )
}

fn choropleth_data() -> ChoroplethData {
    let polygons = vec![
        GeoPolygon::new(
            0,
            0.2,
            vec![[-1.0, -0.2], [-0.4, -0.2], [-0.4, 0.5], [-1.0, 0.5]],
        ),
        GeoPolygon::new(
            1,
            0.4,
            vec![[-0.35, -0.4], [0.2, -0.4], [0.2, 0.1], [-0.35, 0.1]],
        ),
        GeoPolygon::new(
            2,
            0.8,
            vec![[0.25, -0.3], [0.9, -0.3], [0.9, 0.3], [0.25, 0.3]],
        ),
        GeoPolygon::new(
            3,
            0.6,
            vec![[-0.2, 0.15], [0.4, 0.15], [0.4, 0.8], [-0.2, 0.8]],
        ),
    ];
    ChoroplethData::new(polygons).color_scale(ColorScale::viridis())
}

#[test]
fn offscreen_visual_regression_suite() {
    waterui_graphics::shared_context::init_shared_context()
        .expect("offscreen visual suite requires a working GPU adapter");

    let out_dir = std::env::var("WATERUI_CHART_VISUAL_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/waterui_chart_visual_offscreen"));
    fs::create_dir_all(&out_dir).expect("failed to create output dir");

    let size = OffscreenSize::try_from_pixels(1280, 720).expect("valid offscreen size");
    let mut report = String::new();
    writeln!(&mut report, "name,opaque_pixels,non_uniform").expect("write report header");

    let bar_a = basic_points();
    let bar_b = shifted_points();
    let mut bar_renderer = BarChartRenderer::new();
    bar_renderer.set_color(Srgb::from_hex("#0EA5E9"));
    let stats = render_case(
        "bar_transition",
        SeededChart::transition_frame(bar_renderer, bar_a, bar_b),
        size,
        &out_dir,
    )
    .expect("bar render failed");
    writeln!(
        &mut report,
        "bar_transition,{},{},",
        stats.opaque_pixels, stats.non_uniform
    )
    .expect("write report row");

    let line_a = basic_points();
    let line_b = shifted_points();
    let mut line_renderer = LineChartRenderer::new();
    line_renderer.set_color(Srgb::from_hex("#22C55E"));
    line_renderer.set_line_width(2.5);
    line_renderer.set_fill(true, 0.3);
    let stats = render_case(
        "line_transition",
        SeededChart::transition_frame(line_renderer, line_a, line_b),
        size,
        &out_dir,
    )
    .expect("line render failed");
    writeln!(
        &mut report,
        "line_transition,{},{},",
        stats.opaque_pixels, stats.non_uniform
    )
    .expect("write report row");

    let pie_data = vec![
        DataPoint::new(0.0, 28.0),
        DataPoint::new(1.0, 19.0),
        DataPoint::new(2.0, 36.0),
        DataPoint::new(3.0, 17.0),
    ];
    let mut pie_renderer = PieChartRenderer::new();
    pie_renderer.set_inner_radius(0.42);
    pie_renderer.set_colors(vec![
        Srgb::from_hex("#3B82F6"),
        Srgb::from_hex("#F97316"),
        Srgb::from_hex("#22C55E"),
        Srgb::from_hex("#A855F7"),
    ]);
    let stats = render_case(
        "pie_donut",
        SeededChart::static_frame(pie_renderer, pie_data),
        size,
        &out_dir,
    )
    .expect("pie render failed");
    writeln!(
        &mut report,
        "pie_donut,{},{}",
        stats.opaque_pixels, stats.non_uniform
    )
    .expect("write report row");

    let mut scatter_renderer = ScatterChartRenderer::new();
    scatter_renderer.set_color(Srgb::from_hex("#E11D48"));
    scatter_renderer.set_radius(5.0);
    let stats = render_case(
        "scatter",
        SeededChart::static_frame(scatter_renderer, basic_points()),
        size,
        &out_dir,
    )
    .expect("scatter render failed");
    writeln!(
        &mut report,
        "scatter,{},{}",
        stats.opaque_pixels, stats.non_uniform
    )
    .expect("write report row");

    let bubble_renderer = BubbleRenderer::new()
        .color(Srgb::from_hex("#0EA5E9"))
        .min_radius(3.0)
        .max_radius(18.0)
        .opacity(0.75);
    let stats = render_case(
        "bubble",
        SeededChart::static_frame(bubble_renderer, bubble_points()),
        size,
        &out_dir,
    )
    .expect("bubble render failed");
    writeln!(
        &mut report,
        "bubble,{},{}",
        stats.opaque_pixels, stats.non_uniform
    )
    .expect("write report row");

    let mut candlestick_renderer = CandlestickRenderer::new();
    candlestick_renderer.set_bullish_color(Srgb::from_hex("#10B981"));
    candlestick_renderer.set_bearish_color(Srgb::from_hex("#EF4444"));
    let stats = render_case(
        "candlestick",
        SeededChart::static_frame(candlestick_renderer, candle_points()),
        size,
        &out_dir,
    )
    .expect("candlestick render failed");
    writeln!(
        &mut report,
        "candlestick,{},{}",
        stats.opaque_pixels, stats.non_uniform
    )
    .expect("write report row");

    let mut depth_renderer = DepthRenderer::new();
    depth_renderer.set_bid_color(Srgb::from_hex("#22C55E"), Srgb::from_hex("#16A34A"));
    depth_renderer.set_ask_color(Srgb::from_hex("#EF4444"), Srgb::from_hex("#DC2626"));
    let stats = render_case(
        "depth",
        SeededChart::static_frame(depth_renderer, depth_data()),
        size,
        &out_dir,
    )
    .expect("depth render failed");
    writeln!(
        &mut report,
        "depth,{},{}",
        stats.opaque_pixels, stats.non_uniform
    )
    .expect("write report row");

    let stats = render_case(
        "heatmap",
        SeededChart::static_frame(HeatmapRenderer::new(), heatmap_data(32, 48)),
        size,
        &out_dir,
    )
    .expect("heatmap render failed");
    writeln!(
        &mut report,
        "heatmap,{},{}",
        stats.opaque_pixels, stats.non_uniform
    )
    .expect("write report row");

    let contour_renderer = ContourRenderer::new().line_width(2.0);
    let stats = render_case(
        "contour",
        SeededChart::static_frame(contour_renderer, contour_data(40, 60)),
        size,
        &out_dir,
    )
    .expect("contour render failed");
    writeln!(
        &mut report,
        "contour,{},{}",
        stats.opaque_pixels, stats.non_uniform
    )
    .expect("write report row");

    let gauge_renderer = GaugeRenderer::new()
        .arc_angles(-core::f32::consts::PI * 0.75, core::f32::consts::PI * 0.75)
        .radii(0.30, 0.45)
        .background_color(Srgb::from_hex("#1F2937"))
        .value_color(Srgb::from_hex("#3B82F6"))
        .needle_color(Srgb::from_hex("#F9FAFB"));
    let gauge_data = GaugeData::new(72.0, 0.0, 100.0)
        .region(GaugeRegion::hex(30.0, "#22C55E"))
        .region(GaugeRegion::hex(70.0, "#F59E0B"))
        .region(GaugeRegion::hex(100.0, "#EF4444"))
        .show_needle(true);
    let stats = render_case(
        "gauge",
        SeededChart::static_frame(gauge_renderer, gauge_data),
        size,
        &out_dir,
    )
    .expect("gauge render failed");
    writeln!(
        &mut report,
        "gauge,{},{}",
        stats.opaque_pixels, stats.non_uniform
    )
    .expect("write report row");

    let stats = render_case(
        "radar",
        SeededChart::static_frame(RadarRenderer::new(), radar_data()),
        size,
        &out_dir,
    )
    .expect("radar render failed");
    writeln!(
        &mut report,
        "radar,{},{}",
        stats.opaque_pixels, stats.non_uniform
    )
    .expect("write report row");

    let stats = render_case(
        "area",
        SeededChart::static_frame(AreaRenderer::new(), area_data()),
        size,
        &out_dir,
    )
    .expect("area render failed");
    writeln!(
        &mut report,
        "area,{},{}",
        stats.opaque_pixels, stats.non_uniform
    )
    .expect("write report row");

    let choropleth_renderer = ChoroplethRenderer::new()
        .stroke_width(1.5)
        .stroke_color(Srgb::from_hex("#111827"));
    let stats = render_case(
        "choropleth",
        SeededChart::static_frame(choropleth_renderer, choropleth_data()),
        size,
        &out_dir,
    )
    .expect("choropleth render failed");
    writeln!(
        &mut report,
        "choropleth,{},{}",
        stats.opaque_pixels, stats.non_uniform
    )
    .expect("write report row");

    fs::write(out_dir.join("report.csv"), report).expect("failed to write visual report");
}
