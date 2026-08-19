//! Chart components for `WaterUI` rendered through `Canvas`/`Scene2D`.
//!
//! This crate provides chart visualizations on top of `WaterUI`'s scene pipeline,
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

extern crate alloc;

pub mod animation;
pub mod axes;
pub mod axis;
pub mod charts;
mod composition;
pub mod data;
pub mod interaction;
pub mod legend;
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

// Re-export chart views and their ergonomic free-function constructors
pub use charts::area::{AreaChart, area_chart};
pub use charts::bar::{BarChart, bar_chart};
pub use charts::bubble::{BubbleChart, bubble_chart};
pub use charts::candlestick::{CandlestickChart, candlestick_chart};
pub use charts::choropleth::{ChoroplethChart, choropleth_chart};
pub use charts::contour::{ContourChart, contour_chart};
pub use charts::depth::{DepthChart, depth_chart};
pub use charts::gauge::{GaugeChart, gauge_chart};
pub use charts::heatmap::{HeatmapChart, heatmap_chart};
pub use charts::line::{LineChart, line_chart};
pub use charts::pie::{PieChart, pie_chart};
pub use charts::radar::{RadarChart, radar_chart};
pub use charts::scatter::{ScatterChart, scatter_chart};

// Re-export axis types
pub use axes::{ChartAxes, ChartExt};
pub use axis::{AxisConfig, Tick, TickFormat};
pub use composition::ChartProxy;

// Re-export legend types
pub use legend::{Legend, LegendItem, LegendOrientation, LegendPosition};

// Re-export tooltip types
pub use tooltip::{Tooltip, TooltipContent, TooltipValue};
