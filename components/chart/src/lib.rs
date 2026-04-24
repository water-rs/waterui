//! Chart components for `WaterUI` rendered through `Canvas`/`Scene2D`.
//!
//! This crate provides chart visualizations on top of WaterUI's scene pipeline,
//! sharing one rendering path across native backends and hydrolysis.
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
//! use waterui::Binding;
//! use waterui::graphics::color::Srgb;
//! use waterui_chart::{BarChart, DataPoint};
//!
//! let data = Binding::container(vec![
//!     DataPoint::new(0.0, 100.0),
//!     DataPoint::new(1.0, 150.0),
//!     DataPoint::new(2.0, 80.0),
//! ]);
//!
//! BarChart::new(data)
//!     .color(Srgb::from_hex("#3B82F6"))
//! ```

#![allow(clippy::multiple_crate_versions)]
#![allow(missing_docs, missing_debug_implementations)]
#![allow(
    clippy::bool_to_int_with_if,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::double_must_use,
    clippy::float_cmp,
    clippy::if_same_then_else,
    clippy::imprecise_flops,
    clippy::manual_midpoint,
    clippy::manual_range_contains,
    clippy::missing_const_for_fn,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::needless_pass_by_value,
    clippy::needless_range_loop,
    clippy::option_if_let_else,
    clippy::pub_underscore_fields,
    clippy::redundant_clone,
    clippy::redundant_closure,
    clippy::redundant_closure_for_method_calls,
    clippy::redundant_pub_crate,
    clippy::struct_excessive_bools,
    clippy::suboptimal_flops,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::use_self,
    clippy::used_underscore_binding
)]

extern crate alloc;

pub mod animation;
pub mod axes;
pub mod axis;
pub mod charts;
mod composition;
pub mod data;
pub mod interaction;
pub mod legend;
mod local_state;
pub mod params;
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
pub use interaction::{
    AreaDatum, ChartAnchor, ChartScrollableAxes, ChartViewport, DepthDatum, DepthSide, GridDatum,
    HitResult, RadarDatum, RegionDatum, SliceDatum,
};
pub use params::{
    ArcAngles, ChartParamError, DonutInnerRadius, GaugeRadii, PositiveF32, UnitInterval,
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

// Re-export axis types
pub use axes::{ChartAxes, ChartAxesReactive, ChartExt};
pub use axis::{AxisConfig, Tick, TickFormat};
pub use composition::ChartProxy;

// Re-export legend types
pub use legend::{Legend, LegendItem, LegendOrientation, LegendPosition};

// Re-export tooltip types
pub use tooltip::{Tooltip, TooltipContent, TooltipValue};
