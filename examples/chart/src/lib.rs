//! Chart example demonstrating all chart types.

use waterui::app::App;
use waterui::color::Srgb;
use waterui::prelude::*;
use waterui::reactive::{Binding, binding};
use waterui_chart::{AreaChart, AreaData, AreaSeries, AxisConfig, BarChart, BubbleChart, BubblePoint, Candle, CandlestickChart, ChartExt, ContourChart, ContourData, DataBounds, DataPoint, DepthChart, DepthData, DepthLevel, GaugeChart, GaugeData, GaugeRegion, HeatmapChart, HeatmapData, LineChart, PieChart, RadarChart, RadarData, RadarSeries, ScatterChart};

/// Main View - demonstrates different chart types
#[hot_reload]
fn main() -> impl View {
    let mode = binding(0_i32);

    zstack((
        // Background
        Color::srgb_hex("#1a1a2e"),

        // Content
        vstack((
            text("Charts Demo").size(24.0).bold().foreground(Color::from(Srgb::WHITE)),

            // Mode selector buttons
            // Chart type buttons
            hstack((
                Button::new(text("Bar")).action_with(&mode, |m: Binding<i32>, _env: Environment| m.set(0)),
                Button::new(text("Line")).action_with(&mode, |m: Binding<i32>, _env: Environment| m.set(1)),
                Button::new(text("Pie")).action_with(&mode, |m: Binding<i32>, _env: Environment| m.set(2)),
                Button::new(text("Scatter")).action_with(&mode, |m: Binding<i32>, _env: Environment| m.set(3)),
                Button::new(text("Candle")).action_with(&mode, |m: Binding<i32>, _env: Environment| m.set(4)),
                Button::new(text("Depth")).action_with(&mode, |m: Binding<i32>, _env: Environment| m.set(5)),
                Button::new(text("Heatmap")).action_with(&mode, |m: Binding<i32>, _env: Environment| m.set(6)),
                Button::new(text("Contour")).action_with(&mode, |m: Binding<i32>, _env: Environment| m.set(10)),
                Button::new(text("Radar")).action_with(&mode, |m: Binding<i32>, _env: Environment| m.set(11)),
                Button::new(text("Bubble")).action_with(&mode, |m: Binding<i32>, _env: Environment| m.set(12)),
                Button::new(text("Area")).action_with(&mode, |m: Binding<i32>, _env: Environment| m.set(13)),
                Button::new(text("Gauge")).action_with(&mode, |m: Binding<i32>, _env: Environment| m.set(14)),
            )).spacing(10.0),
            // GPU stress test buttons (data loads that choke Swift Charts)
            hstack((
                Button::new(text("10K Scatter")).action_with(&mode, |m: Binding<i32>, _env: Environment| m.set(7)),
                Button::new(text("1K Line")).action_with(&mode, |m: Binding<i32>, _env: Environment| m.set(8)),
                Button::new(text("100x100 Heat")).action_with(&mode, |m: Binding<i32>, _env: Environment| m.set(9)),
            )).spacing(10.0),

            spacer(),

            // Chart display
            watch(mode.clone(), |m| {
                chart_view(m)
            }),

            spacer(),
        ))
        .padding_with(EdgeInsets::all(20.0)),
    ))
}

fn chart_view(mode: i32) -> AnyView {
    // Sample data as raw vectors (for bounds calculation)
    let bar_points = vec![
        DataPoint::new(0.0, 65.0),
        DataPoint::new(1.0, 85.0),
        DataPoint::new(2.0, 45.0),
        DataPoint::new(3.0, 95.0),
        DataPoint::new(4.0, 75.0),
        DataPoint::new(5.0, 55.0),
    ];

    let line_points = vec![
        DataPoint::new(0.0, 10.0),
        DataPoint::new(1.0, 45.0),
        DataPoint::new(2.0, 30.0),
        DataPoint::new(3.0, 70.0),
        DataPoint::new(4.0, 55.0),
        DataPoint::new(5.0, 80.0),
        DataPoint::new(6.0, 60.0),
    ];

    let pie_points = vec![
        DataPoint::new(0.0, 35.0),
        DataPoint::new(1.0, 25.0),
        DataPoint::new(2.0, 20.0),
        DataPoint::new(3.0, 15.0),
        DataPoint::new(4.0, 5.0),
    ];

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

    match mode {
        0 => {
            // Bar chart with axes and axis labels
            let bounds = DataBounds::from_points(&bar_points);
            let bar_data = binding(bar_points);
            AnyView::new(
                BarChart::new(bar_data)
                    .color(Srgb::from_hex("#3B82F6"))
                    .axes(bounds)
                    .y_axis(AxisConfig::new().tick_count(5).show_grid().label("Sales"))
                    .x_axis(AxisConfig::new().tick_count(6).label("Month"))
                    .size(350.0, 280.0)
            )
        }
        1 => {
            // Line chart with axes
            let bounds = DataBounds::from_points(&line_points);
            let line_data = binding(line_points);
            AnyView::new(
                LineChart::new(line_data)
                    .color(Srgb::from_hex("#22C55E"))
                    .line_width(3.0)
                    .axes(bounds)
                    .y_axis(AxisConfig::new().tick_count(5))
                    .size(350.0, 280.0)
            )
        }
        2 => {
            // Pie chart doesn't need axes
            let pie_data = binding(pie_points);
            AnyView::new(
                PieChart::new(pie_data)
                    .donut(0.5)
                    .size(300.0, 300.0)
            )
        }
        3 => {
            // Scatter chart with axes
            let bounds = DataBounds::from_points(&scatter_points);
            let scatter_data = binding(scatter_points);
            AnyView::new(
                ScatterChart::new(scatter_data)
                    .color(Srgb::from_hex("#EF4444"))
                    .radius(6.0)
                    .axes(bounds)
                    .size(350.0, 280.0)
            )
        }
        4 => {
            // Candlestick chart - sample OHLCV data
            let candle_data = vec![
                Candle::new(0.0, 100.0, 110.0, 95.0, 105.0, 1000.0),  // Bullish
                Candle::new(1.0, 105.0, 115.0, 100.0, 98.0, 1200.0),  // Bearish
                Candle::new(2.0, 98.0, 108.0, 92.0, 106.0, 800.0),    // Bullish
                Candle::new(3.0, 106.0, 120.0, 105.0, 118.0, 1500.0), // Bullish
                Candle::new(4.0, 118.0, 125.0, 110.0, 112.0, 1100.0), // Bearish
                Candle::new(5.0, 112.0, 118.0, 108.0, 116.0, 900.0),  // Bullish
            ];
            let candles = binding(candle_data);
            AnyView::new(
                CandlestickChart::new(candles)
                    .bullish_color(Srgb::from_hex("#22C55E"))
                    .bearish_color(Srgb::from_hex("#EF4444"))
                    .size(350.0, 280.0)
            )
        }
        5 => {
            // Depth chart - order book visualization
            let depth_data = DepthData::new(
                // Bids (buy orders) - sorted by price descending
                vec![
                    DepthLevel::new(100.0, 500.0),    // Best bid
                    DepthLevel::new(99.5, 1200.0),   // Cumulative
                    DepthLevel::new(99.0, 2000.0),
                    DepthLevel::new(98.5, 3500.0),
                    DepthLevel::new(98.0, 5000.0),
                ],
                // Asks (sell orders) - sorted by price ascending
                vec![
                    DepthLevel::new(100.5, 400.0),   // Best ask
                    DepthLevel::new(101.0, 1000.0),
                    DepthLevel::new(101.5, 1800.0),
                    DepthLevel::new(102.0, 3000.0),
                    DepthLevel::new(102.5, 4500.0),
                ],
            );
            let depth = binding(depth_data);
            AnyView::new(
                DepthChart::new(depth)
                    .bid_color(Srgb::from_hex("#22C55E"))
                    .ask_color(Srgb::from_hex("#EF4444"))
                    .size(350.0, 280.0)
            )
        }
        6 => {
            // Heatmap chart - correlation matrix visualization
            // 8x8 sample data showing a pattern (e.g., correlation matrix)
            let rows = 8;
            let cols = 8;
            let mut values = Vec::with_capacity(rows * cols);
            for r in 0..rows {
                for c in 0..cols {
                    // Create a pattern: diagonal = 1.0, fading with distance
                    let distance = ((r as f32 - c as f32).abs()) / (rows as f32);
                    let value = 1.0 - distance;
                    values.push(value);
                }
            }
            let heatmap_data = HeatmapData::new(rows as u32, cols as u32, values);
            let heatmap = binding(heatmap_data);
            AnyView::new(
                HeatmapChart::new(heatmap)
                    .size(300.0, 300.0)
            )
        }
        7 => {
            // GPU STRESS TEST: 10,000 scatter points
            // Swift Charts would completely choke on this data volume
            let point_count = 10_000;
            let mut points = Vec::with_capacity(point_count);
            for i in 0..point_count {
                let t = i as f32 / point_count as f32;
                // Spiral galaxy pattern
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
            AnyView::new(
                vstack((
                    text("10,000 Points @ 120fps").size(14.0).foreground(Color::from(Srgb::WHITE)),
                    ScatterChart::new(scatter_data)
                        .color(Srgb::from_hex("#8B5CF6"))
                        .radius(2.0)
                        .axes(bounds)
                        .size(350.0, 280.0)
                ))
            )
        }
        8 => {
            // GPU STRESS TEST: 1,000 line points
            let point_count = 1_000;
            let mut points = Vec::with_capacity(point_count);
            for i in 0..point_count {
                let x = i as f32;
                // Complex waveform: multiple sine waves + noise
                let t = i as f32 / point_count as f32;
                let y = 50.0
                    + 30.0 * (t * 10.0 * core::f32::consts::PI).sin()
                    + 15.0 * (t * 23.0 * core::f32::consts::PI).sin()
                    + 8.0 * (t * 47.0 * core::f32::consts::PI).sin();
                points.push(DataPoint::new(x, y));
            }
            let bounds = DataBounds::from_points(&points);
            let line_data = binding(points);
            AnyView::new(
                vstack((
                    text("1,000 Points @ 120fps").size(14.0).foreground(Color::from(Srgb::WHITE)),
                    LineChart::new(line_data)
                        .color(Srgb::from_hex("#06B6D4"))
                        .line_width(1.5)
                        .axes(bounds)
                        .size(350.0, 280.0)
                ))
            )
        }
        9 => {
            // GPU STRESS TEST: 100x100 heatmap = 10,000 cells
            let rows = 100;
            let cols = 100;
            let mut values = Vec::with_capacity(rows * cols);
            for r in 0..rows {
                for c in 0..cols {
                    // Mandelbrot-like fractal pattern
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
            AnyView::new(
                vstack((
                    text("10,000 Cells @ 120fps").size(14.0).foreground(Color::from(Srgb::WHITE)),
                    HeatmapChart::new(heatmap)
                        .size(300.0, 300.0)
                ))
            )
        }
        10 => {
            // Contour chart - topographic elevation map
            let rows = 50;
            let cols = 50;
            let mut values = Vec::with_capacity(rows * cols);
            for r in 0..rows {
                for c in 0..cols {
                    // Create a terrain-like surface with peaks and valleys
                    let x = (c as f32 / cols as f32) * 4.0 - 2.0;
                    let y = (r as f32 / rows as f32) * 4.0 - 2.0;
                    // Multiple Gaussian peaks
                    let peak1 = (-((x - 0.5).powi(2) + (y - 0.5).powi(2)) / 0.5).exp();
                    let peak2 = (-((x + 0.8).powi(2) + (y - 0.3).powi(2)) / 0.3).exp() * 0.7;
                    let peak3 = (-((x - 0.2).powi(2) + (y + 0.9).powi(2)) / 0.4).exp() * 0.8;
                    // Add some ripples
                    let ripple = ((x * 3.0).sin() * (y * 3.0).cos()) * 0.1;
                    values.push(peak1 + peak2 + peak3 + ripple);
                }
            }
            let contour_data = ContourData::new(rows as u32, cols as u32, values, 8);
            let contour = binding(contour_data);
            AnyView::new(
                ContourChart::new(contour)
                    .line_width(2.0)
                    .size(350.0, 350.0)
            )
        }
        11 => {
            // Radar chart - character stats comparison
            let radar_data = RadarData::new(6)
                .labels(vec!["Speed", "Power", "Range", "Defense", "Magic", "Luck"])
                .series(RadarSeries::new("Warrior", vec![70.0, 95.0, 40.0, 90.0, 20.0, 50.0])
                    .color(0.23, 0.51, 0.96, 1.0)) // Blue
                .series(RadarSeries::new("Mage", vec![50.0, 30.0, 80.0, 40.0, 95.0, 60.0])
                    .color(0.58, 0.29, 0.89, 1.0)) // Purple
                .series(RadarSeries::new("Rogue", vec![95.0, 60.0, 50.0, 30.0, 40.0, 85.0])
                    .color(0.16, 0.82, 0.49, 1.0)) // Green
                .max_value(100.0);
            let radar = binding(radar_data);
            AnyView::new(
                RadarChart::new(radar)
                    .ring_count(5)
                    .line_width(2.0)
                    .fill_opacity(0.3)
                    .size(350.0, 350.0)
            )
        }
        12 => {
            // Bubble chart - countries by GDP and population
            let bubble_data = vec![
                BubblePoint::with_color(14.0, 1.4, 100.0, 0.94, 0.27, 0.27, 0.7),  // China
                BubblePoint::with_color(21.0, 0.33, 80.0, 0.23, 0.51, 0.96, 0.7),  // USA
                BubblePoint::with_color(3.0, 1.38, 50.0, 0.16, 0.82, 0.49, 0.7),   // India
                BubblePoint::with_color(4.0, 0.13, 40.0, 0.98, 0.76, 0.18, 0.7),   // Japan
                BubblePoint::with_color(4.0, 0.08, 35.0, 0.58, 0.29, 0.89, 0.7),   // Germany
                BubblePoint::with_color(2.8, 0.07, 30.0, 0.96, 0.49, 0.13, 0.7),   // UK
                BubblePoint::with_color(2.7, 0.07, 28.0, 0.13, 0.69, 0.76, 0.7),   // France
                BubblePoint::with_color(2.0, 0.21, 45.0, 0.45, 0.78, 0.24, 0.7),   // Brazil
            ];
            let bubbles = binding(bubble_data);
            AnyView::new(
                BubbleChart::new(bubbles)
                    .min_radius(10.0)
                    .max_radius(50.0)
                    .opacity(0.7)
                    .size(400.0, 350.0)
            )
        }
        13 => {
            // Area chart - stacked revenue by category over months
            let area_data = AreaData::new(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0])
                .series(AreaSeries::new("Products", vec![30.0, 45.0, 35.0, 50.0, 60.0, 55.0])
                    .color(0.23, 0.51, 0.96, 0.7)) // Blue
                .series(AreaSeries::new("Services", vec![20.0, 25.0, 30.0, 35.0, 40.0, 45.0])
                    .color(0.16, 0.82, 0.49, 0.7)) // Green
                .series(AreaSeries::new("Subscriptions", vec![10.0, 15.0, 20.0, 25.0, 30.0, 35.0])
                    .color(0.58, 0.29, 0.89, 0.7)) // Purple
                .stacked(true);
            let areas = binding(area_data);
            AnyView::new(
                AreaChart::new(areas)
                    .size(400.0, 350.0)
            )
        }
        14 => {
            // Gauge chart - speedometer with colored zones
            let gauge_data = GaugeData::new(72.0, 0.0, 100.0)
                .region(GaugeRegion::hex(30.0, "#22C55E"))  // Green: 0-30 (safe)
                .region(GaugeRegion::hex(70.0, "#EAB308"))  // Yellow: 30-70 (warning)
                .region(GaugeRegion::hex(100.0, "#EF4444")) // Red: 70-100 (danger)
                .show_needle(true);
            let gauge = binding(gauge_data);
            AnyView::new(
                GaugeChart::new(gauge)
                    .arc_degrees(-135.0, 135.0)
                    .radii(0.3, 0.45)
                    .background_color(Srgb::from_hex("#333333"))
                    .needle_color(Srgb::from_hex("#FFFFFF"))
                    .size(300.0, 300.0)
            )
        }
        _ => AnyView::new(()),
    }
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
