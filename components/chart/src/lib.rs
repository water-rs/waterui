//! Chart components for WaterUI with GPU-accelerated rendering.
//!
//! This crate provides high-performance chart visualizations using GPU shaders,
//! targeting 120fps rendering for all chart types at any data scale.
//!
//! # Chart Types
//!
//! - **Basic**: Bar, Line, Pie, Scatter
//! - **Financial**: Candlestick (K-line), Depth chart
//! - **Scientific**: Heatmap, Contour
//! - **Geographic**: Choropleth
//!
//! # Example
//!
//! ```ignore
//! use waterui::prelude::*;
//! use waterui_chart::{BarChart, DataPoint};
//!
//! let data = binding(vec![
//!     DataPoint { x: 0.0, y: 100.0 },
//!     DataPoint { x: 1.0, y: 150.0 },
//!     DataPoint { x: 2.0, y: 80.0 },
//! ]);
//!
//! BarChart::new(&data)
//!     .color(Color::blue())
//!     .entry_animation(Animation::spring(200.0, 15.0))
//! ```

#![allow(clippy::multiple_crate_versions)]

extern crate alloc;

pub mod animation;
pub mod axes;
pub mod axis;
pub mod charts;
pub mod data;
pub mod interaction;
pub mod legend;
pub mod renderer;
pub mod tooltip;

// Re-export core types
pub use animation::{ChartAnimation, ChartAnimator, EasingType};
pub use data::{
    AreaData,
    // Area chart types
    AreaSeries,
    // Dedicated chart data types (with impl_constant!)
    BarData,
    BubbleData,
    // Bubble types
    BubblePoint,
    // Candlestick/financial types
    Candle,
    CandlestickData,
    ChoroplethData,
    ColorScale,
    // Contour types
    ContourData,
    DataBounds,
    // Basic types
    DataPoint,
    DepthData,
    // Depth chart types
    DepthLevel,
    // Gauge types
    GaugeData,
    GaugeRegion,
    // Choropleth/geographic types
    GeoPolygon,
    // Heatmap types
    HeatmapCell,
    HeatmapData,
    LineData,
    PieData,
    // Radar types
    RadarData,
    RadarSeries,
    ScatterData,
    SeriesStyle,
};
pub use interaction::{ChartViewport, HitResult, SelectionState, ZoomPanState};
pub use renderer::{
    AreaRenderer, BarChartRenderer, BubbleRenderer, CandlestickRenderer, ChartRenderer,
    ChoroplethRenderer, ContourRenderer, DepthRenderer, GaugeRenderer, HeatmapRenderer,
    LineChartRenderer, PieChartRenderer, RadarRenderer, ScatterChartRenderer,
};

// Re-export chart views
pub use charts::area::AreaChart;
pub use charts::bar::BarChart;
pub use charts::bubble::BubbleChart;
pub use charts::candlestick::CandlestickChart;
pub use charts::choropleth::ChoroplethChart;
pub use charts::contour::ContourChart;
pub use charts::depth::DepthChart;
pub use charts::gauge::GaugeChart;
pub use charts::heatmap::HeatmapChart;
pub use charts::line::LineChart;
pub use charts::pie::PieChart;
pub use charts::radar::RadarChart;
pub use charts::scatter::ScatterChart;

// Re-export reactive wrapper
pub use charts::SignalRenderer;

// Re-export axis types
pub use axes::{ChartAxes, ChartAxesReactive, ChartExt};
pub use axis::{AxisConfig, Tick, TickFormat};

// Re-export legend types
pub use legend::{Legend, LegendItem, LegendOrientation, LegendPosition};

// Re-export tooltip types
pub use tooltip::{Tooltip, TooltipContent, TooltipValue};
