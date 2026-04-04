//! Chart example demonstrating all chart types.

use waterui::app::App;
use waterui::color::Srgb;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::{binding, Binding};
use waterui::view_builder;
use waterui_chart::{
    AreaChart, AreaData, AreaSeries, AxisConfig, BarChart, BubbleChart, BubblePoint, Candle,
    CandlestickChart, ChartExt, ContourChart, ContourData, DataBounds, DataPoint, DepthChart,
    DepthData, DepthLevel, GaugeChart, GaugeData, GaugeRegion, HeatmapChart, HeatmapData,
    LineChart, PieChart, RadarChart, RadarData, RadarSeries, ScatterChart,
};

/// Chart types available in this demo.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
enum ChartMode {
    // Basic charts
    #[default]
    Bar,
    Line,
    Pie,
    Scatter,
    // Financial charts
    Candlestick,
    Depth,
    // Grid-based charts
    Heatmap,
    Contour,
    // Multi-series charts
    Radar,
    Bubble,
    Area,
    // Single-value charts
    Gauge,
    // Stress tests (GPU performance demos)
    StressScatter10K,
    StressLine1K,
    StressHeatmap10K,
}

impl ChartMode {
    const BASIC: &[Self] = &[
        Self::Bar,
        Self::Line,
        Self::Pie,
        Self::Scatter,
        Self::Candlestick,
        Self::Depth,
        Self::Heatmap,
        Self::Contour,
        Self::Radar,
        Self::Bubble,
        Self::Area,
        Self::Gauge,
    ];

    const STRESS: &[Self] = &[
        Self::StressScatter10K,
        Self::StressLine1K,
        Self::StressHeatmap10K,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Bar => "Bar",
            Self::Line => "Line",
            Self::Pie => "Pie",
            Self::Scatter => "Scatter",
            Self::Candlestick => "Candle",
            Self::Depth => "Depth",
            Self::Heatmap => "Heatmap",
            Self::Contour => "Contour",
            Self::Radar => "Radar",
            Self::Bubble => "Bubble",
            Self::Area => "Area",
            Self::Gauge => "Gauge",
            Self::StressScatter10K => "10K Scatter",
            Self::StressLine1K => "1K Line",
            Self::StressHeatmap10K => "100x100 Heat",
        }
    }

    #[view_builder]
    fn render(self) -> impl View {
        match self {
            Self::Bar => bar_chart_preview(),
            Self::Line => line_chart_preview(),
            Self::Pie => pie_chart_preview(),
            Self::Scatter => scatter_chart_preview(),
            Self::Candlestick => candlestick_chart_preview(),
            Self::Depth => depth_chart_preview(),
            Self::Heatmap => heatmap_chart_preview(),
            Self::Contour => contour_chart_preview(),
            Self::Radar => radar_chart_preview(),
            Self::Bubble => bubble_chart_preview(),
            Self::Area => area_chart_preview(),
            Self::Gauge => gauge_chart_preview(),
            Self::StressScatter10K => scatter_stress_preview(),
            Self::StressLine1K => line_stress_preview(),
            Self::StressHeatmap10K => heatmap_stress_preview(),
        }
    }
}

fn mode_buttons(modes: &[ChartMode], mode: &Binding<ChartMode>) -> HStack<(Vec<AnyView>,)> {
    modes
        .iter()
        .map(|&target| {
            button(target.label())
                .action(move |State(m): State<Binding<ChartMode>>| m.set(target))
                .state(mode)
        })
        .collect()
}

/// Main View - demonstrates different chart types
fn main() -> impl View {
    let mode = binding(ChartMode::default());

    zstack((
        Color::srgb_hex("#1a1a2e"),
        vstack((
            text("Charts Demo").title().bold().foreground(Srgb::WHITE),
            mode_buttons(ChartMode::BASIC, &mode),
            // GPU stress test buttons (data loads that choke Swift Charts)
            mode_buttons(ChartMode::STRESS, &mode),
            spacer(),
            watch(mode.clone(), ChartMode::render),
            spacer(),
        ))
        .padding_with(EdgeInsets::all(20.0)),
    ))
}

#[preview]
fn bar_chart_preview() -> impl View {
    let bar_points = vec![
        DataPoint::new(0.0, 65.0),
        DataPoint::new(1.0, 85.0),
        DataPoint::new(2.0, 45.0),
        DataPoint::new(3.0, 95.0),
        DataPoint::new(4.0, 75.0),
        DataPoint::new(5.0, 55.0),
    ];
    let bounds = DataBounds::from_points(&bar_points);
    let bar_data = binding(bar_points);
    BarChart::new(bar_data)
        .color(Srgb::from_hex("#3B82F6"))
        .axes(bounds)
        .y_axis(AxisConfig::new().tick_count(5).show_grid().label("Sales"))
        .x_axis(AxisConfig::new().tick_count(6).label("Month"))
        .size(350.0, 280.0)
}

#[preview]
fn line_chart_preview() -> impl View {
    let line_points = vec![
        DataPoint::new(0.0, 10.0),
        DataPoint::new(1.0, 45.0),
        DataPoint::new(2.0, 30.0),
        DataPoint::new(3.0, 70.0),
        DataPoint::new(4.0, 55.0),
        DataPoint::new(5.0, 80.0),
        DataPoint::new(6.0, 60.0),
    ];
    let bounds = DataBounds::from_points(&line_points);
    let line_data = binding(line_points);
    LineChart::new(line_data)
        .color(Srgb::from_hex("#22C55E"))
        .line_width(3.0)
        .axes(bounds)
        .y_axis(AxisConfig::new().tick_count(5))
        .size(350.0, 280.0)
}

#[preview]
fn pie_chart_preview() -> impl View {
    let pie_points = vec![
        DataPoint::new(0.0, 35.0),
        DataPoint::new(1.0, 25.0),
        DataPoint::new(2.0, 20.0),
        DataPoint::new(3.0, 15.0),
        DataPoint::new(4.0, 5.0),
    ];
    let pie_data = binding(pie_points);
    PieChart::new(pie_data).donut(0.5).size(300.0, 300.0)
}

#[preview]
fn scatter_chart_preview() -> impl View {
    let scatter_points = vec![
        DataPoint::new(1.0, 2.5),
        DataPoint::new(2.0, 4.0),
        DataPoint::new(2.5, 3.0),
        DataPoint::new(3.0, 5.5),
        DataPoint::new(4.0, 4.5),
        DataPoint::new(5.0, 6.0),
        DataPoint::new(5.5, 5.0),
        DataPoint::new(6.0, 7.0),
        DataPoint::new(7.0, 6.5),
        DataPoint::new(8.0, 8.0),
    ];
    let bounds = DataBounds::from_points(&scatter_points);
    let scatter_data = binding(scatter_points);
    ScatterChart::new(scatter_data)
        .color(Srgb::from_hex("#EF4444"))
        .radius(6.0)
        .axes(bounds)
        .size(350.0, 280.0)
}

#[preview]
fn candlestick_chart_preview() -> impl View {
    let candle_data = vec![
        Candle::new(0.0, 100.0, 110.0, 95.0, 105.0, 1000.0), // Bullish
        Candle::new(1.0, 105.0, 115.0, 100.0, 98.0, 1200.0), // Bearish
        Candle::new(2.0, 98.0, 108.0, 92.0, 106.0, 800.0),   // Bullish
        Candle::new(3.0, 106.0, 120.0, 105.0, 118.0, 1500.0), // Bullish
        Candle::new(4.0, 118.0, 125.0, 110.0, 112.0, 1100.0), // Bearish
        Candle::new(5.0, 112.0, 118.0, 108.0, 116.0, 900.0), // Bullish
    ];
    let candles = binding(candle_data);
    CandlestickChart::new(candles)
        .bullish_color(Srgb::from_hex("#22C55E"))
        .bearish_color(Srgb::from_hex("#EF4444"))
        .size(350.0, 280.0)
}

#[preview]
fn depth_chart_preview() -> impl View {
    let depth_data = DepthData::new(
        vec![
            DepthLevel::new(100.0, 500.0),
            DepthLevel::new(99.5, 1200.0),
            DepthLevel::new(99.0, 2000.0),
            DepthLevel::new(98.5, 3500.0),
            DepthLevel::new(98.0, 5000.0),
        ],
        vec![
            DepthLevel::new(100.5, 400.0),
            DepthLevel::new(101.0, 1000.0),
            DepthLevel::new(101.5, 1800.0),
            DepthLevel::new(102.0, 3000.0),
            DepthLevel::new(102.5, 4500.0),
        ],
    );
    let depth = binding(depth_data);
    DepthChart::new(depth)
        .bid_color(Srgb::from_hex("#22C55E"))
        .ask_color(Srgb::from_hex("#EF4444"))
        .size(350.0, 280.0)
}

#[preview]
fn heatmap_chart_preview() -> impl View {
    let rows = 8;
    let cols = 8;
    let mut values = Vec::with_capacity(rows * cols);
    for r in 0..rows {
        for c in 0..cols {
            let distance = ((r as f32 - c as f32).abs()) / (rows as f32);
            let value = 1.0 - distance;
            values.push(value);
        }
    }
    let heatmap_data = HeatmapData::new(rows as u32, cols as u32, values);
    let heatmap = binding(heatmap_data);
    HeatmapChart::new(heatmap).size(300.0, 300.0)
}

#[preview]
fn scatter_stress_preview() -> impl View {
    let point_count = 10_000;
    let mut points = Vec::with_capacity(point_count);
    for i in 0..point_count {
        let t = i as f32 / point_count as f32;
        let angle = t * 20.0 * core::f32::consts::PI;
        let radius = t * 100.0;
        let noise_x = ((i * 7919) % 1000) as f32 / 1000.0 * 10.0;
        let noise_y = ((i * 7907) % 1000) as f32 / 1000.0 * 10.0;
        let x = radius * angle.cos() + noise_x;
        let y = radius * angle.sin() + noise_y;
        points.push(DataPoint::new(x, y));
    }
    let bounds = DataBounds::from_points(&points);
    let scatter_data = binding(points);
    vstack((
        text("10,000 Points @ 120fps")
            .caption()
            .foreground(Srgb::WHITE),
        ScatterChart::new(scatter_data)
            .color(Srgb::from_hex("#8B5CF6"))
            .radius(2.0)
            .axes(bounds)
            .size(350.0, 280.0),
    ))
}

#[preview]
fn line_stress_preview() -> impl View {
    let point_count = 1_000;
    let mut points = Vec::with_capacity(point_count);
    for i in 0..point_count {
        let x = i as f32;
        let t = i as f32 / point_count as f32;
        let y = 50.0
            + 30.0 * (t * 10.0 * core::f32::consts::PI).sin()
            + 15.0 * (t * 23.0 * core::f32::consts::PI).sin()
            + 8.0 * (t * 47.0 * core::f32::consts::PI).sin();
        points.push(DataPoint::new(x, y));
    }
    let bounds = DataBounds::from_points(&points);
    let line_data = binding(points);
    vstack((
        text("1,000 Points @ 120fps")
            .caption()
            .foreground(Srgb::WHITE),
        LineChart::new(line_data)
            .color(Srgb::from_hex("#06B6D4"))
            .line_width(1.5)
            .axes(bounds)
            .size(350.0, 280.0),
    ))
}

#[preview]
fn heatmap_stress_preview() -> impl View {
    let rows = 100;
    let cols = 100;
    let mut values = Vec::with_capacity(rows * cols);
    for r in 0..rows {
        for c in 0..cols {
            let x0 = (c as f32 / cols as f32) * 3.5 - 2.5;
            let y0 = (r as f32 / rows as f32) * 2.0 - 1.0;
            let mut x = 0.0_f32;
            let mut y = 0.0_f32;
            let mut iter = 0;
            while x * x + y * y <= 4.0 && iter < 50 {
                let xtemp = x * x - y * y + x0;
                y = 2.0 * x * y + y0;
                x = xtemp;
                iter += 1;
            }
            values.push(iter as f32 / 50.0);
        }
    }
    let heatmap_data = HeatmapData::new(rows as u32, cols as u32, values);
    let heatmap = binding(heatmap_data);
    vstack((
        text("10,000 Cells @ 120fps")
            .caption()
            .foreground(Srgb::WHITE),
        HeatmapChart::new(heatmap).size(300.0, 300.0),
    ))
}

#[preview]
fn contour_chart_preview() -> impl View {
    let rows = 50;
    let cols = 50;
    let mut values = Vec::with_capacity(rows * cols);
    for r in 0..rows {
        for c in 0..cols {
            let x = (c as f32 / cols as f32) * 4.0 - 2.0;
            let y = (r as f32 / rows as f32) * 4.0 - 2.0;
            let peak1 = (-((x - 0.5).powi(2) + (y - 0.5).powi(2)) / 0.5).exp();
            let peak2 = (-((x + 0.8).powi(2) + (y - 0.3).powi(2)) / 0.3).exp() * 0.7;
            let peak3 = (-((x - 0.2).powi(2) + (y + 0.9).powi(2)) / 0.4).exp() * 0.8;
            let ripple = ((x * 3.0).sin() * (y * 3.0).cos()) * 0.1;
            values.push(peak1 + peak2 + peak3 + ripple);
        }
    }
    let contour_data = ContourData::new(rows as u32, cols as u32, values, 8);
    let contour = binding(contour_data);
    ContourChart::new(contour)
        .line_width(2.0)
        .size(350.0, 350.0)
}

#[preview]
fn radar_chart_preview() -> impl View {
    let radar_data = RadarData::new(6)
        .labels(vec!["Speed", "Power", "Range", "Defense", "Magic", "Luck"])
        .series(
            RadarSeries::new("Warrior", vec![70.0, 95.0, 40.0, 90.0, 20.0, 50.0])
                .color(0.23, 0.51, 0.96, 1.0),
        )
        .series(
            RadarSeries::new("Mage", vec![50.0, 30.0, 80.0, 40.0, 95.0, 60.0])
                .color(0.58, 0.29, 0.89, 1.0),
        )
        .series(
            RadarSeries::new("Rogue", vec![95.0, 60.0, 50.0, 30.0, 40.0, 85.0])
                .color(0.16, 0.82, 0.49, 1.0),
        )
        .max_value(100.0);
    let radar = binding(radar_data);
    RadarChart::new(radar)
        .ring_count(5)
        .line_width(2.0)
        .fill_opacity(0.3)
        .size(350.0, 350.0)
}

#[preview]
fn bubble_chart_preview() -> impl View {
    let bubble_data = vec![
        BubblePoint::with_color(14.0, 1.4, 100.0, 0.94, 0.27, 0.27, 0.7),
        BubblePoint::with_color(21.0, 0.33, 80.0, 0.23, 0.51, 0.96, 0.7),
        BubblePoint::with_color(3.0, 1.38, 50.0, 0.16, 0.82, 0.49, 0.7),
        BubblePoint::with_color(4.0, 0.13, 40.0, 0.98, 0.76, 0.18, 0.7),
        BubblePoint::with_color(4.0, 0.08, 35.0, 0.58, 0.29, 0.89, 0.7),
        BubblePoint::with_color(2.8, 0.07, 30.0, 0.96, 0.49, 0.13, 0.7),
        BubblePoint::with_color(2.7, 0.07, 28.0, 0.13, 0.69, 0.76, 0.7),
        BubblePoint::with_color(2.0, 0.21, 45.0, 0.45, 0.78, 0.24, 0.7),
    ];
    let bubbles = binding(bubble_data);
    BubbleChart::new(bubbles)
        .min_radius(10.0)
        .max_radius(50.0)
        .opacity(0.7)
        .size(400.0, 350.0)
}

#[preview]
fn area_chart_preview() -> impl View {
    let area_data = AreaData::new(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0])
        .series(
            AreaSeries::new("Products", vec![30.0, 45.0, 35.0, 50.0, 60.0, 55.0])
                .color(0.23, 0.51, 0.96, 0.7),
        )
        .series(
            AreaSeries::new("Services", vec![20.0, 25.0, 30.0, 35.0, 40.0, 45.0])
                .color(0.16, 0.82, 0.49, 0.7),
        )
        .series(
            AreaSeries::new("Subscriptions", vec![10.0, 15.0, 20.0, 25.0, 30.0, 35.0])
                .color(0.58, 0.29, 0.89, 0.7),
        )
        .stacked(true);
    let areas = binding(area_data);
    AreaChart::new(areas).size(400.0, 350.0)
}

#[preview]
fn gauge_chart_preview() -> impl View {
    let gauge_data = GaugeData::new(72.0, 0.0, 100.0)
        .region(GaugeRegion::hex(30.0, "#22C55E"))
        .region(GaugeRegion::hex(70.0, "#EAB308"))
        .region(GaugeRegion::hex(100.0, "#EF4444"))
        .show_needle(true);
    let gauge = binding(gauge_data);
    GaugeChart::new(gauge)
        .arc_degrees(-135.0, 135.0)
        .radii(0.3, 0.45)
        .background_color(Srgb::from_hex("#333333"))
        .needle_color(Srgb::from_hex("#FFFFFF"))
        .size(300.0, 300.0)
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
