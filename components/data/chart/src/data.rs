//! Core data types for chart components.

extern crate alloc;

use alloc::vec::Vec;
use core::num::NonZeroU32;
use nami::impl_constant;
use num_traits::ToPrimitive as _;
use waterui_graphics::color::Srgb;

/// Error returned when chart data violates structural constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartDataError {
    /// A dimension that must be greater than zero was zero.
    ZeroDimension,
    /// Input matrix rows have different lengths.
    RaggedMatrix,
    /// Number of provided values does not match rows * cols.
    ValueCountMismatch {
        /// Expected number of values.
        expected: usize,
        /// Actual number of values.
        actual: usize,
    },
    /// Radar charts require at least 3 axes.
    AxisCountTooSmall {
        /// Provided axis count.
        axis_count: u32,
    },
    /// A collection dimension is too large for the chart data format.
    DimensionTooLarge {
        /// Dimension name.
        dimension: &'static str,
        /// Provided value.
        value: usize,
    },
}

/// A single 2D data point.
#[derive(Debug, Clone, Copy, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct DataPoint {
    /// X coordinate value.
    pub x: f32,
    /// Y coordinate value.
    pub y: f32,
}

impl DataPoint {
    /// Creates a new data point.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// OHLCV candle for financial charts.
#[derive(Debug, Clone, Copy, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Candle {
    /// Unix timestamp (seconds).
    pub timestamp: f32,
    /// Opening price.
    pub open: f32,
    /// Highest price.
    pub high: f32,
    /// Lowest price.
    pub low: f32,
    /// Closing price.
    pub close: f32,
    /// Trading volume.
    pub volume: f32,
    /// Padding for 32-byte alignment.
    pub pad: [f32; 2],
}

impl Candle {
    /// Creates a new candle.
    #[must_use]
    pub const fn new(
        timestamp: f32,
        open: f32,
        high: f32,
        low: f32,
        close: f32,
        volume: f32,
    ) -> Self {
        Self {
            timestamp,
            open,
            high,
            low,
            close,
            volume,
            pad: [0.0; 2],
        }
    }

    /// Returns true if the candle closed higher than it opened (bullish).
    #[must_use]
    pub fn is_bullish(&self) -> bool {
        self.close >= self.open
    }
}

/// A single order book level for depth charts.
#[derive(Debug, Clone, Copy, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct DepthLevel {
    /// Price level.
    pub price: f32,
    /// Cumulative volume at this price level.
    pub cumulative_volume: f32,
}

impl DepthLevel {
    /// Creates a new depth level.
    #[must_use]
    pub const fn new(price: f32, cumulative_volume: f32) -> Self {
        Self {
            price,
            cumulative_volume,
        }
    }
}

/// Order book depth data for depth charts.
///
/// Contains bid (buy) and ask (sell) levels sorted by price.
/// Bids are sorted descending (highest first), asks ascending (lowest first).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DepthData {
    /// Bid levels (buy orders), sorted by price descending.
    pub bids: Vec<DepthLevel>,
    /// Ask levels (sell orders), sorted by price ascending.
    pub asks: Vec<DepthLevel>,
}

impl DepthData {
    /// Creates new depth data.
    #[must_use]
    pub const fn new(bids: Vec<DepthLevel>, asks: Vec<DepthLevel>) -> Self {
        Self { bids, asks }
    }

    /// Returns the mid price (average of best bid and best ask).
    #[must_use]
    pub fn mid_price(&self) -> Option<f32> {
        let best_bid = self.bids.first().map(|l| l.price)?;
        let best_ask = self.asks.first().map(|l| l.price)?;
        Some(f32::midpoint(best_bid, best_ask))
    }

    /// Computes data bounds for axis scaling.
    #[must_use]
    pub fn bounds(&self) -> DataBounds {
        if self.bids.is_empty() && self.asks.is_empty() {
            return DataBounds::default();
        }

        let mut min_price = f32::MAX;
        let mut max_price = f32::MIN;
        let mut max_volume = 0.0_f32;

        for level in &self.bids {
            min_price = min_price.min(level.price);
            max_price = max_price.max(level.price);
            max_volume = max_volume.max(level.cumulative_volume);
        }

        for level in &self.asks {
            min_price = min_price.min(level.price);
            max_price = max_price.max(level.price);
            max_volume = max_volume.max(level.cumulative_volume);
        }

        DataBounds::new(min_price, max_price, 0.0, max_volume)
    }
}

/// Heatmap cell for matrix data.
#[derive(Debug, Clone, Copy, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct HeatmapCell {
    /// Row index.
    pub row: u32,
    /// Column index.
    pub col: u32,
    /// Cell value.
    pub value: f32,
    /// Padding for 16-byte alignment.
    pub pad: f32,
}

impl HeatmapCell {
    /// Creates a new heatmap cell.
    #[must_use]
    pub const fn new(row: u32, col: u32, value: f32) -> Self {
        Self {
            row,
            col,
            value,
            pad: 0.0,
        }
    }
}

/// Heatmap data for matrix visualization.
///
/// Stores a 2D grid of values that are mapped to colors via a color scale.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeatmapData {
    /// Number of rows.
    pub rows: u32,
    /// Number of columns.
    pub cols: u32,
    /// Cell values (row-major order).
    pub values: Vec<f32>,
    /// Minimum value for color mapping.
    pub min_value: f32,
    /// Maximum value for color mapping.
    pub max_value: f32,
}

impl HeatmapData {
    fn validate_grid(
        rows: u32,
        cols: u32,
        value_len: usize,
    ) -> Result<(NonZeroU32, NonZeroU32), ChartDataError> {
        let rows = NonZeroU32::new(rows).ok_or(ChartDataError::ZeroDimension)?;
        let cols = NonZeroU32::new(cols).ok_or(ChartDataError::ZeroDimension)?;
        let expected = rows.get() as usize * cols.get() as usize;
        if expected != value_len {
            return Err(ChartDataError::ValueCountMismatch {
                expected,
                actual: value_len,
            });
        }
        Ok((rows, cols))
    }

    /// Creates a new heatmap from a 2D matrix.
    ///
    /// # Panics
    ///
    /// Panics when the matrix is empty, ragged, or too large for the chart data format.
    #[must_use]
    pub fn from_matrix(matrix: &[&[f32]]) -> Self {
        Self::try_from_matrix(matrix).expect("Invalid heatmap matrix")
    }

    /// Creates a new heatmap from a 2D matrix with validation.
    ///
    /// # Errors
    ///
    /// Returns [`ChartDataError`] when the matrix is empty, ragged, or too large for the chart data format.
    pub fn try_from_matrix(matrix: &[&[f32]]) -> Result<Self, ChartDataError> {
        if matrix.is_empty() || matrix[0].is_empty() {
            return Err(ChartDataError::ZeroDimension);
        }

        let rows = u32::try_from(matrix.len()).map_err(|_| ChartDataError::DimensionTooLarge {
            dimension: "rows",
            value: matrix.len(),
        })?;
        let cols =
            u32::try_from(matrix[0].len()).map_err(|_| ChartDataError::DimensionTooLarge {
                dimension: "cols",
                value: matrix[0].len(),
            })?;
        let cols_len = usize::try_from(cols).map_err(|_| ChartDataError::DimensionTooLarge {
            dimension: "cols",
            value: matrix[0].len(),
        })?;
        for row in matrix {
            if row.len() != cols_len {
                return Err(ChartDataError::RaggedMatrix);
            }
        }

        let rows_len = usize::try_from(rows).map_err(|_| ChartDataError::DimensionTooLarge {
            dimension: "rows",
            value: matrix.len(),
        })?;
        let mut values = Vec::with_capacity(rows_len * cols_len);
        let mut min_value = f32::MAX;
        let mut max_value = f32::MIN;

        for row in matrix {
            for &value in *row {
                values.push(value);
                min_value = min_value.min(value);
                max_value = max_value.max(value);
            }
        }

        let (rows, cols) = Self::validate_grid(rows, cols, values.len())?;
        Ok(Self {
            rows: rows.get(),
            cols: cols.get(),
            values,
            min_value,
            max_value,
        })
    }

    /// Creates a new heatmap from row-major values.
    ///
    /// # Panics
    ///
    /// Panics when dimensions are zero or do not match the number of values.
    #[must_use]
    pub fn new(rows: u32, cols: u32, values: Vec<f32>) -> Self {
        Self::try_new(rows, cols, values).expect("Invalid heatmap dimensions or value count")
    }

    /// Creates a new heatmap from row-major values with validation.
    ///
    /// # Errors
    ///
    /// Returns [`ChartDataError`] when dimensions are zero or do not match the number of values.
    pub fn try_new(rows: u32, cols: u32, values: Vec<f32>) -> Result<Self, ChartDataError> {
        let (rows, cols) = Self::validate_grid(rows, cols, values.len())?;
        let (min_value, max_value) = values.iter().fold((f32::MAX, f32::MIN), |(min, max), &v| {
            (min.min(v), max.max(v))
        });
        Ok(Self {
            rows: rows.get(),
            cols: cols.get(),
            values,
            min_value,
            max_value,
        })
    }

    /// Returns the value at (row, col).
    #[must_use]
    pub fn get(&self, row: u32, col: u32) -> Option<f32> {
        if row < self.rows && col < self.cols {
            Some(self.values[(row * self.cols + col) as usize])
        } else {
            None
        }
    }

    /// Returns the total number of cells.
    #[must_use]
    pub const fn cell_count(&self) -> usize {
        (self.rows * self.cols) as usize
    }
}

/// Contour data for isoline visualization.
///
/// Stores a 2D grid of scalar values plus contour levels (thresholds)
/// where isolines should be drawn using the marching squares algorithm.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContourData {
    /// Number of rows in the grid.
    pub rows: u32,
    /// Number of columns in the grid.
    pub cols: u32,
    /// Scalar field values (row-major order).
    pub values: Vec<f32>,
    /// Contour levels (threshold values for isolines).
    pub levels: Vec<f32>,
    /// Minimum value in the field.
    pub min_value: f32,
    /// Maximum value in the field.
    pub max_value: f32,
}

impl ContourData {
    fn validate_grid(
        rows: u32,
        cols: u32,
        value_len: usize,
    ) -> Result<(NonZeroU32, NonZeroU32), ChartDataError> {
        let rows = NonZeroU32::new(rows).ok_or(ChartDataError::ZeroDimension)?;
        let cols = NonZeroU32::new(cols).ok_or(ChartDataError::ZeroDimension)?;
        let expected = rows.get() as usize * cols.get() as usize;
        if expected != value_len {
            return Err(ChartDataError::ValueCountMismatch {
                expected,
                actual: value_len,
            });
        }
        Ok((rows, cols))
    }

    /// Creates contour data from a grid with auto-computed levels.
    ///
    /// Generates evenly-spaced contour levels between min and max values.
    ///
    /// # Panics
    ///
    /// Panics when dimensions are zero or do not match the number of values.
    #[must_use]
    pub fn new(rows: u32, cols: u32, values: Vec<f32>, num_levels: usize) -> Self {
        Self::try_new(rows, cols, values, num_levels)
            .expect("Invalid contour dimensions or value count")
    }

    /// Creates contour data from a grid with validation.
    ///
    /// # Errors
    ///
    /// Returns [`ChartDataError`] when dimensions are zero or do not match the number of values.
    pub fn try_new(
        rows: u32,
        cols: u32,
        values: Vec<f32>,
        num_levels: usize,
    ) -> Result<Self, ChartDataError> {
        let (rows, cols) = Self::validate_grid(rows, cols, values.len())?;
        let (min_value, max_value) = values.iter().fold((f32::MAX, f32::MIN), |(min, max), &v| {
            (min.min(v), max.max(v))
        });

        // Generate evenly-spaced levels
        let range = max_value - min_value;
        let levels = if range > 0.0 && num_levels > 0 {
            let denominator =
                (num_levels + 1)
                    .to_f32()
                    .ok_or(ChartDataError::DimensionTooLarge {
                        dimension: "levels",
                        value: num_levels + 1,
                    })?;
            (1..=num_levels)
                .map(|i| {
                    let numerator = i.to_f32().ok_or(ChartDataError::DimensionTooLarge {
                        dimension: "levels",
                        value: i,
                    })?;
                    Ok(range.mul_add(numerator / denominator, min_value))
                })
                .collect::<Result<Vec<_>, ChartDataError>>()?
        } else {
            Vec::new()
        };

        Ok(Self {
            rows: rows.get(),
            cols: cols.get(),
            values,
            levels,
            min_value,
            max_value,
        })
    }

    /// Creates contour data with explicit contour levels.
    ///
    /// # Panics
    ///
    /// Panics when dimensions are zero or do not match the number of values.
    #[must_use]
    pub fn with_levels(rows: u32, cols: u32, values: Vec<f32>, levels: Vec<f32>) -> Self {
        Self::try_with_levels(rows, cols, values, levels)
            .expect("Invalid contour dimensions or value count")
    }

    /// Creates contour data with explicit contour levels and validation.
    ///
    /// # Errors
    ///
    /// Returns [`ChartDataError`] when dimensions are zero or do not match the number of values.
    pub fn try_with_levels(
        rows: u32,
        cols: u32,
        values: Vec<f32>,
        levels: Vec<f32>,
    ) -> Result<Self, ChartDataError> {
        let (rows, cols) = Self::validate_grid(rows, cols, values.len())?;
        let (min_value, max_value) = values.iter().fold((f32::MAX, f32::MIN), |(min, max), &v| {
            (min.min(v), max.max(v))
        });
        Ok(Self {
            rows: rows.get(),
            cols: cols.get(),
            values,
            levels,
            min_value,
            max_value,
        })
    }

    /// Returns the value at (row, col).
    #[must_use]
    pub fn get(&self, row: u32, col: u32) -> Option<f32> {
        if row < self.rows && col < self.cols {
            Some(self.values[(row * self.cols + col) as usize])
        } else {
            None
        }
    }

    /// Returns the total number of cells.
    #[must_use]
    pub const fn cell_count(&self) -> usize {
        (self.rows * self.cols) as usize
    }

    /// Returns the number of contour levels.
    #[must_use]
    pub const fn level_count(&self) -> usize {
        self.levels.len()
    }
}

/// A bubble data point with position and size.
#[derive(Debug, Clone, Copy, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct BubblePoint {
    /// X coordinate.
    pub x: f32,
    /// Y coordinate.
    pub y: f32,
    /// Bubble size (radius will be scaled proportionally).
    pub size: f32,
    /// Color override (RGBA, or use default if alpha is 0).
    pub color: [f32; 4],
}

impl BubblePoint {
    /// Creates a new bubble point.
    #[must_use]
    pub const fn new(x: f32, y: f32, size: f32) -> Self {
        Self {
            x,
            y,
            size,
            color: [0.0, 0.0, 0.0, 0.0], // Use default color
        }
    }

    /// Creates a bubble point with a custom color.
    #[must_use]
    pub const fn with_color(
        x_position: f32,
        y_position: f32,
        size: f32,
        red: f32,
        green: f32,
        blue: f32,
        alpha: f32,
    ) -> Self {
        Self {
            x: x_position,
            y: y_position,
            size,
            color: [red, green, blue, alpha],
        }
    }
}

/// Parse a hex byte from a slice at the given offset.
fn parse_hex_byte(bytes: &[u8], offset: usize) -> u8 {
    let hex_digit = |b: u8| -> u8 {
        match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => 0,
        }
    };
    (hex_digit(bytes[offset]) << 4) | hex_digit(bytes[offset + 1])
}

/// A single data series for radar charts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RadarSeries {
    /// Series name for legend.
    pub name: alloc::string::String,
    /// Values for each axis (must match axis count).
    pub values: Vec<f32>,
    /// Series color (RGBA).
    pub color: [f32; 4],
}

impl RadarSeries {
    /// Creates a new radar series.
    #[must_use]
    pub fn new(name: impl Into<alloc::string::String>, values: Vec<f32>) -> Self {
        Self {
            name: name.into(),
            values,
            color: [0.23, 0.51, 0.96, 1.0], // Default blue
        }
    }

    /// Sets the series color.
    #[must_use]
    pub const fn color(mut self, r: f32, g: f32, b: f32, a: f32) -> Self {
        self.color = [r, g, b, a];
        self
    }

    /// Sets the series color from a hex string (e.g., "#3B82F6").
    #[must_use]
    pub fn color_hex(mut self, hex: &str) -> Self {
        // Parse hex color directly
        let bytes = hex.as_bytes();
        let offset = usize::from(!bytes.is_empty() && bytes[0] == b'#');
        if bytes.len() - offset >= 6 {
            let r = parse_hex_byte(bytes, offset);
            let g = parse_hex_byte(bytes, offset + 2);
            let b = parse_hex_byte(bytes, offset + 4);
            self.color = [
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                1.0,
            ];
        }
        self
    }
}

/// Radar/Spider chart data.
///
/// Displays multivariate data on radial axes emanating from a center point.
/// Each axis represents a variable, and data series form polygons connecting
/// the values on each axis.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RadarData {
    /// Number of axes (dimensions).
    pub axis_count: u32,
    /// Optional axis labels.
    pub labels: Vec<alloc::string::String>,
    /// Data series to display.
    pub series: Vec<RadarSeries>,
    /// Maximum value for scaling (auto-computed if not set).
    pub max_value: f32,
}

impl RadarData {
    /// Creates radar data with the specified number of axes.
    ///
    /// # Panics
    ///
    /// Panics when `axis_count < 3`.
    #[must_use]
    pub fn new(axis_count: u32) -> Self {
        Self::try_new(axis_count).expect("Radar charts require axis_count >= 3")
    }

    /// Creates radar data with validation.
    ///
    /// # Errors
    ///
    /// Returns [`ChartDataError`] when `axis_count < 3`.
    pub const fn try_new(axis_count: u32) -> Result<Self, ChartDataError> {
        if axis_count < 3 {
            return Err(ChartDataError::AxisCountTooSmall { axis_count });
        }

        Ok(Self {
            axis_count,
            labels: Vec::new(),
            series: Vec::new(),
            max_value: 1.0,
        })
    }

    /// Creates radar data from a compile-time validated non-zero axis count.
    ///
    /// # Panics
    ///
    /// Panics when `axis_count < 3`.
    #[must_use]
    pub fn from_non_zero_axis_count(axis_count: NonZeroU32) -> Self {
        Self::try_new(axis_count.get()).expect("Radar charts require axis_count >= 3")
    }

    /// Sets axis labels.
    #[must_use]
    pub fn labels(mut self, labels: Vec<impl Into<alloc::string::String>>) -> Self {
        self.labels = labels.into_iter().map(Into::into).collect();
        self
    }

    /// Adds a data series.
    #[must_use]
    pub fn series(mut self, series: RadarSeries) -> Self {
        // Update max value
        for &v in &series.values {
            if v > self.max_value {
                self.max_value = v;
            }
        }
        self.series.push(series);
        self
    }

    /// Sets the maximum value for scaling.
    #[must_use]
    pub const fn max_value(mut self, max: f32) -> Self {
        self.max_value = max;
        self
    }

    /// Returns the number of series.
    #[must_use]
    pub const fn series_count(&self) -> usize {
        self.series.len()
    }

    /// Returns total vertex count for all series (for GPU instancing).
    #[must_use]
    pub const fn total_vertices(&self) -> usize {
        self.series.len() * self.axis_count as usize
    }
}

/// Geographic polygon for choropleth maps.
#[derive(Debug, Clone, PartialEq)]
pub struct GeoPolygon {
    /// Unique identifier.
    pub id: u32,
    /// Data value for coloring.
    pub value: f32,
    /// Polygon vertices as [x, y] pairs (lon, lat).
    pub vertices: Vec<[f32; 2]>,
    /// Triangle indices for tessellation.
    pub indices: Vec<u32>,
}

impl GeoPolygon {
    /// Creates a new polygon from vertices (auto-tessellates).
    ///
    /// # Panics
    ///
    /// Panics when the polygon has more vertices than fit in `u32` indices.
    #[must_use]
    pub fn new(id: u32, value: f32, vertices: Vec<[f32; 2]>) -> Self {
        // Simple fan triangulation for convex polygons
        // For complex polygons, use ear-clipping or other algorithms
        let indices = if vertices.len() >= 3 {
            let mut idx = Vec::with_capacity((vertices.len() - 2) * 3);
            for i in 1..(vertices.len() - 1) {
                idx.push(0);
                idx.push(u32::try_from(i).expect("polygon vertex index should fit u32"));
                idx.push(u32::try_from(i + 1).expect("polygon vertex index should fit u32"));
            }
            idx
        } else {
            Vec::new()
        };
        Self {
            id,
            value,
            vertices,
            indices,
        }
    }

    /// Creates a polygon with pre-computed triangle indices.
    #[must_use]
    pub const fn with_indices(
        id: u32,
        value: f32,
        vertices: Vec<[f32; 2]>,
        indices: Vec<u32>,
    ) -> Self {
        Self {
            id,
            value,
            vertices,
            indices,
        }
    }

    /// Returns the bounding box [`min_x`, `min_y`, `max_x`, `max_y`].
    #[must_use]
    pub fn bounds(&self) -> [f32; 4] {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for &[x, y] in &self.vertices {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        [min_x, min_y, max_x, max_y]
    }
}

/// Choropleth map data for geographic visualization.
///
/// Displays regions colored by data values using a color scale.
/// Supports multiple polygons with tessellated geometry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChoroplethData {
    /// Geographic polygons to render.
    pub polygons: Vec<GeoPolygon>,
    /// Minimum data value for color mapping.
    pub min_value: f32,
    /// Maximum data value for color mapping.
    pub max_value: f32,
    /// Color scale for mapping values to colors.
    pub color_scale: ColorScale,
}

impl ChoroplethData {
    /// Creates new choropleth data from polygons.
    #[must_use]
    pub fn new(polygons: Vec<GeoPolygon>) -> Self {
        let (min_value, max_value) = polygons.iter().fold((f32::MAX, f32::MIN), |(min, max), p| {
            (min.min(p.value), max.max(p.value))
        });
        Self {
            polygons,
            min_value,
            max_value,
            color_scale: ColorScale::viridis(),
        }
    }

    /// Sets the color scale.
    #[must_use]
    pub fn color_scale(mut self, scale: ColorScale) -> Self {
        self.color_scale = scale;
        self
    }

    /// Sets explicit value range.
    #[must_use]
    pub const fn range(mut self, min: f32, max: f32) -> Self {
        self.min_value = min;
        self.max_value = max;
        self
    }

    /// Returns total vertex count across all polygons.
    #[must_use]
    pub fn total_vertices(&self) -> usize {
        self.polygons.iter().map(|p| p.vertices.len()).sum()
    }

    /// Returns total triangle count across all polygons.
    #[must_use]
    pub fn total_triangles(&self) -> usize {
        self.polygons.iter().map(|p| p.indices.len() / 3).sum()
    }

    /// Computes geographic bounds [`min_lon`, `min_lat`, `max_lon`, `max_lat`].
    #[must_use]
    pub fn bounds(&self) -> [f32; 4] {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for polygon in &self.polygons {
            let [polygon_min_x, polygon_min_y, polygon_max_x, polygon_max_y] = polygon.bounds();
            min_x = min_x.min(polygon_min_x);
            min_y = min_y.min(polygon_min_y);
            max_x = max_x.max(polygon_max_x);
            max_y = max_y.max(polygon_max_y);
        }
        [min_x, min_y, max_x, max_y]
    }
}

/// Series styling configuration.
#[derive(Debug, Clone)]
pub struct SeriesStyle {
    /// Series color.
    pub color: Srgb,
    /// Line width for line/area charts.
    pub line_width: f32,
    /// Fill opacity (0.0 - 1.0).
    pub opacity: f32,
}

impl Default for SeriesStyle {
    fn default() -> Self {
        Self {
            color: Srgb::from_hex("#3B82F6"), // Blue
            line_width: 2.0,
            opacity: 1.0,
        }
    }
}

impl SeriesStyle {
    /// Creates a new series style with the given color.
    #[must_use]
    pub fn new(color: impl Into<Srgb>) -> Self {
        Self {
            color: color.into(),
            ..Default::default()
        }
    }

    /// Sets the line width.
    #[must_use]
    pub const fn line_width(mut self, width: f32) -> Self {
        self.line_width = width;
        self
    }

    /// Sets the opacity.
    #[must_use]
    pub const fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }
}

/// A data series with styling.
#[derive(Debug, Clone)]
pub struct Series<T> {
    /// Series name for legend.
    pub name: alloc::string::String,
    /// Data points.
    pub data: Vec<T>,
    /// Visual style.
    pub style: SeriesStyle,
}

impl<T> Series<T> {
    /// Creates a new series with the given name.
    #[must_use]
    pub fn new(name: impl Into<alloc::string::String>) -> Self {
        Self {
            name: name.into(),
            data: Vec::new(),
            style: SeriesStyle::default(),
        }
    }

    /// Sets the data points.
    #[must_use]
    pub fn data(mut self, data: Vec<T>) -> Self {
        self.data = data;
        self
    }

    /// Sets the series color.
    #[must_use]
    pub fn color(mut self, color: impl Into<Srgb>) -> Self {
        self.style.color = color.into();
        self
    }

    /// Sets the style.
    #[must_use]
    pub const fn style(mut self, style: SeriesStyle) -> Self {
        self.style = style;
        self
    }
}

/// Color scale for continuous data mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorScale {
    /// Color stops as (`value_ratio`, `srgb_color`) pairs.
    /// `value_ratio` is 0.0 to 1.0 representing min to max.
    pub stops: Vec<(f32, Srgb)>,
}

impl Default for ColorScale {
    fn default() -> Self {
        Self::viridis()
    }
}

impl ColorScale {
    /// Creates a sequential color scale from one color to another.
    #[must_use]
    pub fn sequential(from: Srgb, to: Srgb) -> Self {
        Self {
            stops: vec![(0.0, from), (1.0, to)],
        }
    }

    /// Creates a diverging color scale.
    #[must_use]
    pub fn diverging(low: Srgb, mid: Srgb, high: Srgb) -> Self {
        Self {
            stops: vec![(0.0, low), (0.5, mid), (1.0, high)],
        }
    }

    /// Creates a Viridis-like color scale (perceptually uniform).
    #[must_use]
    pub fn viridis() -> Self {
        Self {
            stops: vec![
                (0.0, Srgb::from_hex("#440154")),
                (0.25, Srgb::from_hex("#3B528B")),
                (0.5, Srgb::from_hex("#21918C")),
                (0.75, Srgb::from_hex("#5EC962")),
                (1.0, Srgb::from_hex("#FDE725")),
            ],
        }
    }

    /// Creates a heat color scale (blue to red).
    #[must_use]
    pub fn heat() -> Self {
        Self {
            stops: vec![
                (0.0, Srgb::from_hex("#0000FF")),
                (0.5, Srgb::from_hex("#FFFF00")),
                (1.0, Srgb::from_hex("#FF0000")),
            ],
        }
    }
}

/// A single data series for stacked area charts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AreaSeries {
    /// Series name for legend.
    pub name: alloc::string::String,
    /// Y values for each X position (must have same length across all series).
    pub values: Vec<f32>,
    /// Series color (RGBA).
    pub color: [f32; 4],
}

impl AreaSeries {
    /// Creates a new area series.
    #[must_use]
    pub fn new(name: impl Into<alloc::string::String>, values: Vec<f32>) -> Self {
        Self {
            name: name.into(),
            values,
            color: [0.23, 0.51, 0.96, 0.7], // Default blue with alpha
        }
    }

    /// Sets the series color.
    #[must_use]
    pub const fn color(mut self, r: f32, g: f32, b: f32, a: f32) -> Self {
        self.color = [r, g, b, a];
        self
    }

    /// Sets the series color from a hex string (e.g., "#3B82F6").
    #[must_use]
    pub fn color_hex(mut self, hex: &str) -> Self {
        let bytes = hex.as_bytes();
        let offset = usize::from(!bytes.is_empty() && bytes[0] == b'#');
        if bytes.len() - offset >= 6 {
            let r = parse_hex_byte(bytes, offset);
            let g = parse_hex_byte(bytes, offset + 2);
            let b = parse_hex_byte(bytes, offset + 4);
            self.color = [
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                0.7,
            ];
        }
        self
    }
}

/// Stacked area chart data.
///
/// Contains multiple data series that stack on top of each other.
/// All series must have the same number of data points.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AreaData {
    /// X values shared by all series.
    pub x_values: Vec<f32>,
    /// Data series to stack (rendered bottom to top).
    pub series: Vec<AreaSeries>,
    /// Whether areas should be stacked or overlaid.
    pub stacked: bool,
}

impl AreaData {
    /// Creates new area data with the given X values.
    #[must_use]
    pub const fn new(x_values: Vec<f32>) -> Self {
        Self {
            x_values,
            series: Vec::new(),
            stacked: true,
        }
    }

    /// Adds a data series.
    #[must_use]
    pub fn series(mut self, series: AreaSeries) -> Self {
        self.series.push(series);
        self
    }

    /// Sets whether areas should be stacked.
    #[must_use]
    pub const fn stacked(mut self, stacked: bool) -> Self {
        self.stacked = stacked;
        self
    }

    /// Returns the number of data points.
    #[must_use]
    pub const fn point_count(&self) -> usize {
        self.x_values.len()
    }

    /// Returns the number of series.
    #[must_use]
    pub const fn series_count(&self) -> usize {
        self.series.len()
    }

    /// Computes data bounds for axis scaling.
    #[must_use]
    pub fn bounds(&self) -> DataBounds {
        if self.x_values.is_empty() || self.series.is_empty() {
            return DataBounds::default();
        }

        let min_x = self.x_values.iter().fold(f32::MAX, |a, &b| a.min(b));
        let max_x = self.x_values.iter().fold(f32::MIN, |a, &b| a.max(b));

        // For stacked areas, max Y is sum of all series at each X
        let mut max_y = 0.0_f32;
        let mut min_y = 0.0_f32;

        for i in 0..self.x_values.len() {
            let mut cumulative = 0.0;
            for series in &self.series {
                if i < series.values.len() {
                    let v = series.values[i];
                    if self.stacked {
                        cumulative += v;
                    } else {
                        max_y = max_y.max(v);
                        min_y = min_y.min(v);
                    }
                }
            }
            if self.stacked {
                max_y = max_y.max(cumulative);
            }
        }

        DataBounds::new(min_x, max_x, min_y, max_y)
    }
}

/// Data bounds for axis scaling.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DataBounds {
    /// Minimum X value.
    pub min_x: f32,
    /// Maximum X value.
    pub max_x: f32,
    /// Minimum Y value.
    pub min_y: f32,
    /// Maximum Y value.
    pub max_y: f32,
}

impl DataBounds {
    /// Creates new bounds.
    #[must_use]
    pub const fn new(min_x: f32, max_x: f32, min_y: f32, max_y: f32) -> Self {
        Self {
            min_x,
            max_x,
            min_y,
            max_y,
        }
    }

    /// Computes bounds from data points.
    #[must_use]
    pub fn from_points(points: &[DataPoint]) -> Self {
        if points.is_empty() {
            return Self::default();
        }

        let mut bounds = Self {
            min_x: f32::MAX,
            max_x: f32::MIN,
            min_y: f32::MAX,
            max_y: f32::MIN,
        };

        for p in points {
            bounds.min_x = bounds.min_x.min(p.x);
            bounds.max_x = bounds.max_x.max(p.x);
            bounds.min_y = bounds.min_y.min(p.y);
            bounds.max_y = bounds.max_y.max(p.y);
        }

        bounds
    }

    /// Computes bounds from candles.
    #[must_use]
    pub fn from_candles(candles: &[Candle]) -> Self {
        if candles.is_empty() {
            return Self::default();
        }

        let mut bounds = Self {
            min_x: f32::MAX,
            max_x: f32::MIN,
            min_y: f32::MAX,
            max_y: f32::MIN,
        };

        for c in candles {
            bounds.min_x = bounds.min_x.min(c.timestamp);
            bounds.max_x = bounds.max_x.max(c.timestamp);
            bounds.min_y = bounds.min_y.min(c.low);
            bounds.max_y = bounds.max_y.max(c.high);
        }

        bounds
    }

    /// Returns the width (X range).
    #[must_use]
    pub fn width(&self) -> f32 {
        self.max_x - self.min_x
    }

    /// Returns the height (Y range).
    #[must_use]
    pub fn height(&self) -> f32 {
        self.max_y - self.min_y
    }

    /// Expands bounds to include padding.
    #[must_use]
    pub fn with_padding(mut self, padding_ratio: f32) -> Self {
        let x_pad = self.width() * padding_ratio;
        let y_pad = self.height() * padding_ratio;
        self.min_x -= x_pad;
        self.max_x += x_pad;
        self.min_y -= y_pad;
        self.max_y += y_pad;
        self
    }
}

/// Gauge region for colored threshold zones.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GaugeRegion {
    /// Threshold value (region applies up to this value).
    pub threshold: f32,
    /// Region color (RGBA).
    pub color: [f32; 4],
}

impl GaugeRegion {
    /// Creates a new gauge region.
    #[must_use]
    pub const fn new(threshold: f32, r: f32, g: f32, b: f32, a: f32) -> Self {
        Self {
            threshold,
            color: [r, g, b, a],
        }
    }

    /// Creates a region from a hex color.
    #[must_use]
    pub fn hex(threshold: f32, hex: &str) -> Self {
        let bytes = hex.as_bytes();
        let offset = usize::from(!bytes.is_empty() && bytes[0] == b'#');
        let (r, g, b) = if bytes.len() - offset >= 6 {
            (
                f32::from(parse_hex_byte(bytes, offset)) / 255.0,
                f32::from(parse_hex_byte(bytes, offset + 2)) / 255.0,
                f32::from(parse_hex_byte(bytes, offset + 4)) / 255.0,
            )
        } else {
            (0.5, 0.5, 0.5)
        };
        Self {
            threshold,
            color: [r, g, b, 1.0],
        }
    }
}

/// Gauge chart data for speedometer-style visualization.
///
/// Displays a single value within a range using a circular arc gauge.
/// Supports colored threshold regions and an optional needle indicator.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GaugeData {
    /// Current gauge value.
    pub value: f32,
    /// Minimum value.
    pub min_value: f32,
    /// Maximum value.
    pub max_value: f32,
    /// Colored threshold regions (sorted by threshold).
    pub regions: Vec<GaugeRegion>,
    /// Whether to show the needle indicator.
    pub show_needle: bool,
}

impl GaugeData {
    /// Creates a new gauge with value and range.
    ///
    /// # Panics
    ///
    /// Panics when any value is non-finite or `max <= min`.
    #[must_use]
    pub fn new(value: f32, min: f32, max: f32) -> Self {
        assert!(
            value.is_finite() && min.is_finite() && max.is_finite() && max > min,
            "GaugeData::new(value, min, max) requires finite max > min"
        );
        Self {
            value,
            min_value: min,
            max_value: max,
            regions: Vec::new(),
            show_needle: true,
        }
    }

    /// Adds a colored threshold region.
    #[must_use]
    pub fn region(mut self, region: GaugeRegion) -> Self {
        self.regions.push(region);
        // Sort by threshold
        self.regions.sort_by(|a, b| {
            a.threshold
                .partial_cmp(&b.threshold)
                .unwrap_or(core::cmp::Ordering::Equal)
        });
        self
    }

    /// Sets whether to show the needle.
    #[must_use]
    pub const fn show_needle(mut self, show: bool) -> Self {
        self.show_needle = show;
        self
    }

    /// Returns the normalized value (0.0 to 1.0).
    #[must_use]
    pub fn normalized_value(&self) -> f32 {
        if self.max_value <= self.min_value {
            return 0.0;
        }
        ((self.value - self.min_value) / (self.max_value - self.min_value)).clamp(0.0, 1.0)
    }
}

// ============================================================================
// Dedicated chart data types
// ============================================================================

/// Bar chart data.
///
/// Contains data points for bar chart visualization.
/// Each point represents a bar where x is the category and y is the value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BarData {
    /// Data points for the bars.
    pub points: Vec<DataPoint>,
}

impl BarData {
    /// Creates new bar data from points.
    #[must_use]
    pub const fn new(points: Vec<DataPoint>) -> Self {
        Self { points }
    }

    /// Computes data bounds for axis scaling.
    #[must_use]
    pub fn bounds(&self) -> DataBounds {
        DataBounds::from_points(&self.points)
    }
}

impl From<Vec<DataPoint>> for BarData {
    fn from(points: Vec<DataPoint>) -> Self {
        Self::new(points)
    }
}

/// Line chart data.
///
/// Contains data points for line chart visualization.
/// Points are connected in order to form a line.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LineData {
    /// Data points for the line.
    pub points: Vec<DataPoint>,
}

impl LineData {
    /// Creates new line data from points.
    #[must_use]
    pub const fn new(points: Vec<DataPoint>) -> Self {
        Self { points }
    }

    /// Computes data bounds for axis scaling.
    #[must_use]
    pub fn bounds(&self) -> DataBounds {
        DataBounds::from_points(&self.points)
    }
}

impl From<Vec<DataPoint>> for LineData {
    fn from(points: Vec<DataPoint>) -> Self {
        Self::new(points)
    }
}

/// Pie chart data.
///
/// Contains data points for pie chart visualization.
/// Each point represents a slice where y is the value (x is ignored).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PieData {
    /// Data points for the slices.
    pub points: Vec<DataPoint>,
}

impl PieData {
    /// Creates new pie data from points.
    #[must_use]
    pub const fn new(points: Vec<DataPoint>) -> Self {
        Self { points }
    }

    /// Returns the total of all values.
    #[must_use]
    pub fn total(&self) -> f32 {
        self.points.iter().map(|p| p.y).sum()
    }
}

impl From<Vec<DataPoint>> for PieData {
    fn from(points: Vec<DataPoint>) -> Self {
        Self::new(points)
    }
}

/// Scatter chart data.
///
/// Contains data points for scatter plot visualization.
/// Each point is rendered as an independent marker.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScatterData {
    /// Data points for the scatter plot.
    pub points: Vec<DataPoint>,
}

impl ScatterData {
    /// Creates new scatter data from points.
    #[must_use]
    pub const fn new(points: Vec<DataPoint>) -> Self {
        Self { points }
    }

    /// Computes data bounds for axis scaling.
    #[must_use]
    pub fn bounds(&self) -> DataBounds {
        DataBounds::from_points(&self.points)
    }
}

impl From<Vec<DataPoint>> for ScatterData {
    fn from(points: Vec<DataPoint>) -> Self {
        Self::new(points)
    }
}

/// Candlestick chart data.
///
/// Contains OHLCV candles for financial chart visualization.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CandlestickData {
    /// OHLCV candles.
    pub candles: Vec<Candle>,
}

impl CandlestickData {
    /// Creates new candlestick data from candles.
    #[must_use]
    pub const fn new(candles: Vec<Candle>) -> Self {
        Self { candles }
    }

    /// Computes data bounds for axis scaling.
    #[must_use]
    pub fn bounds(&self) -> DataBounds {
        DataBounds::from_candles(&self.candles)
    }
}

impl From<Vec<Candle>> for CandlestickData {
    fn from(candles: Vec<Candle>) -> Self {
        Self::new(candles)
    }
}

/// Bubble chart data.
///
/// Contains bubble points for bubble chart visualization.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BubbleData {
    /// Bubble points.
    pub points: Vec<BubblePoint>,
}

impl BubbleData {
    /// Creates new bubble data from points.
    #[must_use]
    pub const fn new(points: Vec<BubblePoint>) -> Self {
        Self { points }
    }

    /// Computes data bounds for axis scaling.
    #[must_use]
    pub fn bounds(&self) -> DataBounds {
        if self.points.is_empty() {
            return DataBounds::default();
        }

        let mut bounds = DataBounds {
            min_x: f32::MAX,
            max_x: f32::MIN,
            min_y: f32::MAX,
            max_y: f32::MIN,
        };

        for p in &self.points {
            bounds.min_x = bounds.min_x.min(p.x);
            bounds.max_x = bounds.max_x.max(p.x);
            bounds.min_y = bounds.min_y.min(p.y);
            bounds.max_y = bounds.max_y.max(p.y);
        }

        bounds
    }
}

impl From<Vec<BubblePoint>> for BubbleData {
    fn from(points: Vec<BubblePoint>) -> Self {
        Self::new(points)
    }
}

// ============================================================================
// impl_constant! for all data types
// ============================================================================
// These allow data types to be passed directly to chart components without
// wrapping in a Binding/Computed signal.

impl_constant!(DataPoint);
impl_constant!(Candle);
impl_constant!(DepthLevel);
impl_constant!(DepthData);
impl_constant!(HeatmapCell);
impl_constant!(HeatmapData);
impl_constant!(ContourData);
impl_constant!(BubblePoint);
impl_constant!(RadarSeries);
impl_constant!(RadarData);
impl_constant!(GeoPolygon);
impl_constant!(ChoroplethData);
impl_constant!(SeriesStyle);
impl_constant!(ColorScale);
impl_constant!(AreaSeries);
impl_constant!(AreaData);
impl_constant!(DataBounds);
impl_constant!(GaugeRegion);
impl_constant!(GaugeData);

// New dedicated chart data types
impl_constant!(BarData);
impl_constant!(LineData);
impl_constant!(PieData);
impl_constant!(ScatterData);
impl_constant!(CandlestickData);
impl_constant!(BubbleData);

#[cfg(test)]
mod tests {
    use super::{ChartDataError, ContourData, HeatmapData, RadarData};

    #[test]
    fn heatmap_try_new_rejects_zero_dimension() {
        let err = HeatmapData::try_new(0, 2, vec![]).expect_err("zero rows must fail");
        assert_eq!(err, ChartDataError::ZeroDimension);
    }

    #[test]
    fn heatmap_try_new_rejects_value_count_mismatch() {
        let err = HeatmapData::try_new(2, 2, vec![1.0, 2.0]).expect_err("mismatched len must fail");
        assert_eq!(
            err,
            ChartDataError::ValueCountMismatch {
                expected: 4,
                actual: 2,
            }
        );
    }

    #[test]
    fn heatmap_try_from_matrix_rejects_ragged_input() {
        let rows = [&[1.0, 2.0][..], &[3.0][..]];
        let err = HeatmapData::try_from_matrix(&rows).expect_err("ragged matrix must fail");
        assert_eq!(err, ChartDataError::RaggedMatrix);
    }

    #[test]
    fn contour_try_new_rejects_value_count_mismatch() {
        let err =
            ContourData::try_new(3, 3, vec![0.0; 4], 5).expect_err("mismatched len must fail");
        assert_eq!(
            err,
            ChartDataError::ValueCountMismatch {
                expected: 9,
                actual: 4,
            }
        );
    }

    #[test]
    fn radar_try_new_rejects_axis_count_too_small() {
        let err = RadarData::try_new(2).expect_err("axis count < 3 must fail");
        assert_eq!(err, ChartDataError::AxisCountTooSmall { axis_count: 2 });
    }
}
