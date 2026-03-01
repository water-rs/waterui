use waterui_chart::data::{
    AreaData, AreaSeries, BubblePoint, Candle, ChoroplethData, ContourData, DataPoint, DepthData,
    DepthLevel, GaugeData, GaugeRegion, GeoPolygon, HeatmapData, RadarData, RadarSeries,
};
use waterui_chart::{
    AreaChart, BarChart, BubbleChart, CandlestickChart, ChoroplethChart, ContourChart, DepthChart,
    GaugeChart, HeatmapChart, LineChart, PieChart, RadarChart, ScatterChart,
};
use waterui_core::Environment;
use waterui_testing::TestHost;

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

fn basic_points() -> Vec<DataPoint> {
    (0..32)
        .map(|i| {
            let x = i as f32;
            let y = 48.0 + (x * 0.35).sin() * 20.0 + (x * 0.11).cos() * 8.0;
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

fn contour_data(rows: u32, cols: u32, levels: usize) -> ContourData {
    let mut values = Vec::with_capacity((rows * cols) as usize);
    for r in 0..rows {
        for c in 0..cols {
            let x = c as f32 / cols as f32;
            let y = r as f32 / rows as f32;
            values.push((x * 8.0).sin() + (y * 6.0).cos() + (x - 0.5) * (y - 0.5) * 3.0);
        }
    }
    ContourData::try_new(rows, cols, values, levels).expect("contour seed should be valid")
}

fn radar_data() -> RadarData {
    RadarData::new(5)
        .labels(vec!["Speed", "Power", "Range", "Defense", "Magic"])
        .series(RadarSeries::new("A", vec![80.0, 90.0, 70.0, 60.0, 85.0]).color_hex("#3B82F6"))
        .series(RadarSeries::new("B", vec![65.0, 75.0, 92.0, 78.0, 68.0]).color_hex("#EF4444"))
        .max_value(100.0)
}

fn area_data() -> AreaData {
    AreaData::new(vec![0.0, 1.0, 2.0, 3.0, 4.0])
        .series(AreaSeries::new("S1", vec![10.0, 20.0, 15.0, 25.0, 30.0]).color(0.23, 0.51, 0.96, 0.7))
        .series(AreaSeries::new("S2", vec![6.0, 9.0, 8.0, 12.0, 16.0]).color(0.94, 0.27, 0.27, 0.65))
}

fn choropleth_data() -> ChoroplethData {
    ChoroplethData::new(vec![
        GeoPolygon::new(0, 0.2, vec![[0.0, 0.0], [1.2, 0.0], [1.1, 1.0], [0.0, 1.0]]),
        GeoPolygon::new(1, 0.8, vec![[1.2, 0.0], [2.2, 0.0], [2.4, 1.1], [1.1, 1.0]]),
        GeoPolygon::new(2, 0.5, vec![[0.3, 1.0], [1.4, 1.1], [1.0, 2.1], [0.0, 1.8]]),
    ])
}

fn render_case(name: &str, host: &TestHost, view: impl waterui_core::View) -> VisualStats {
    let output = host.render(view);
    analyze_output(name, &output.rgba8, output.width, output.height)
}

#[test]
fn chart_canvas_visual_smoke() {
    let host = TestHost::new(Environment::new(), 960, 600);

    let bar_stats = render_case("bar", &host, BarChart::new(nami::binding(basic_points())));
    let line_stats = render_case(
        "line",
        &host,
        LineChart::new(nami::binding(basic_points())).fill(0.25),
    );
    let pie_stats = render_case(
        "pie_donut",
        &host,
        PieChart::new(nami::binding(vec![
            DataPoint::new(0.0, 30.0),
            DataPoint::new(1.0, 45.0),
            DataPoint::new(2.0, 25.0),
        ]))
        .donut(0.45),
    );
    let scatter_stats = render_case(
        "scatter",
        &host,
        ScatterChart::new(nami::binding(basic_points())).radius(5.0),
    );
    let bubble_stats = render_case(
        "bubble",
        &host,
        BubbleChart::new(nami::binding(bubble_points()))
            .min_radius(4.0)
            .max_radius(24.0),
    );
    let candle_stats = render_case(
        "candlestick",
        &host,
        CandlestickChart::new(nami::binding(candle_points())),
    );
    let depth_stats = render_case("depth", &host, DepthChart::new(nami::binding(depth_data())));
    let heatmap_stats = render_case(
        "heatmap",
        &host,
        HeatmapChart::new(nami::binding(heatmap_data(24, 36))),
    );
    let contour_stats = render_case(
        "contour",
        &host,
        ContourChart::new(nami::binding(contour_data(28, 28, 7))).line_width(1.8),
    );
    let gauge_stats = render_case(
        "gauge",
        &host,
        GaugeChart::new(nami::binding(
            GaugeData::new(72.0, 0.0, 100.0)
                .region(GaugeRegion::hex(30.0, "#22C55E"))
                .region(GaugeRegion::hex(70.0, "#EAB308"))
                .region(GaugeRegion::hex(100.0, "#EF4444")),
        )),
    );
    let radar_stats = render_case(
        "radar",
        &host,
        RadarChart::new(nami::binding(radar_data()))
            .ring_count(6)
            .fill_opacity(0.32),
    );
    let area_stats = render_case("area", &host, AreaChart::new(nami::binding(area_data())));
    let choropleth_stats = render_case(
        "choropleth",
        &host,
        ChoroplethChart::new(nami::binding(choropleth_data())).stroke_width(1.5),
    );

    let stats = [
        bar_stats,
        line_stats,
        pie_stats,
        scatter_stats,
        bubble_stats,
        candle_stats,
        depth_stats,
        heatmap_stats,
        contour_stats,
        gauge_stats,
        radar_stats,
        area_stats,
        choropleth_stats,
    ];

    let total_opaque: usize = stats.iter().map(|s| s.opaque_pixels).sum();
    assert!(total_opaque > 10_000, "combined opaque footprint too small");
    assert!(stats.iter().all(|s| s.non_uniform));
}
