//! Chart axes view wrapper with labels and grid lines.
//!
//! Wraps a chart with native text labels positioned using absolute layout.
//! Uses the same coordinate system as the chart shader for accurate positioning.

extern crate alloc;

use alloc::vec::Vec;

use nami::Signal;
use waterui_core::dynamic::watch;
use waterui_core::{AnyView, Environment, View};
use waterui_graphics::color::Color;
use waterui_layout::frame::Frame;
use waterui_layout::padding::{EdgeInsets, Padding};
use waterui_layout::{PositionExt, StretchAxis, UnitPoint, absolute};
use waterui_text::text;

use crate::axis::{AxisConfig, Tick};
use crate::data::DataBounds;

/// Axis padding configuration (in points).
#[derive(Clone)]
struct AxisPadding {
    left: f32,
    bottom: f32,
    top: f32,
    right: f32,
}

impl Default for AxisPadding {
    fn default() -> Self {
        Self {
            left: 50.0,
            bottom: 30.0,
            top: 10.0,
            right: 10.0,
        }
    }
}

/// A chart wrapped with axis labels.
///
/// Use the builder pattern to configure axes.
///
/// # Example
///
/// ```ignore
/// use waterui_chart::{BarChart, ChartExt, AxisConfig, TickFormat};
///
/// BarChart::new(&data)
///     .axes(bounds)
///     .y_axis(AxisConfig::new().tick_count(5).show_grid())
///     .x_axis(AxisConfig::new().format(TickFormat::SI))
/// ```
pub struct ChartAxes<C> {
    chart: C,
    x_axis: Option<AxisConfig>,
    y_axis: Option<AxisConfig>,
    bounds: DataBounds,
    padding: AxisPadding,
}

impl<C> ChartAxes<C> {
    /// Creates a new chart axes wrapper.
    pub(crate) fn new(chart: C, bounds: DataBounds) -> Self {
        Self {
            chart,
            x_axis: Some(AxisConfig::default()),
            y_axis: Some(AxisConfig::default()),
            bounds,
            padding: AxisPadding::default(),
        }
    }

    /// Configures the Y-axis.
    #[must_use]
    pub fn y_axis(mut self, config: AxisConfig) -> Self {
        self.y_axis = Some(config);
        self
    }

    /// Disables the Y-axis.
    #[must_use]
    pub fn no_y_axis(mut self) -> Self {
        self.y_axis = None;
        self
    }

    /// Configures the X-axis.
    #[must_use]
    pub fn x_axis(mut self, config: AxisConfig) -> Self {
        self.x_axis = Some(config);
        self
    }

    /// Disables the X-axis.
    #[must_use]
    pub fn no_x_axis(mut self) -> Self {
        self.x_axis = None;
        self
    }

    /// Sets padding for axis labels.
    #[must_use]
    pub fn padding(mut self, left: f32, bottom: f32) -> Self {
        self.padding.left = left;
        self.padding.bottom = bottom;
        self
    }

    /// Sets all four padding values.
    #[must_use]
    pub fn padding_all(mut self, left: f32, bottom: f32, top: f32, right: f32) -> Self {
        self.padding.left = left;
        self.padding.bottom = bottom;
        self.padding.top = top;
        self.padding.right = right;
        self
    }
}

impl<C: View + 'static> View for ChartAxes<C> {
    fn body(self, _env: &Environment) -> impl View {
        let y_show_grid = self.y_axis.as_ref().is_some_and(|a| a.has_grid());
        let x_show_grid = self.x_axis.as_ref().is_some_and(|a| a.has_grid());

        let y_ticks: Vec<Tick> = self
            .y_axis
            .as_ref()
            .map(|a| a.compute_ticks(self.bounds.min_y, self.bounds.max_y))
            .unwrap_or_default();

        let x_ticks: Vec<Tick> = self
            .x_axis
            .as_ref()
            .map(|a| a.compute_ticks(self.bounds.min_x, self.bounds.max_x))
            .unwrap_or_default();

        // Get axis title labels (clone before consuming self)
        let y_label: Option<String> = self.y_axis.as_ref().and_then(|a| a.axis_label().map(Into::into));
        let x_label: Option<String> = self.x_axis.as_ref().and_then(|a| a.axis_label().map(Into::into));

        let padding = self.padding;

        // Chart with padding to make room for axis labels
        let padded_chart = Padding::new(
            EdgeInsets::new(padding.top, padding.bottom, padding.left, padding.right),
            self.chart,
        );

        // Build views dynamically
        let mut views: Vec<AnyView> = Vec::new();

        // Grid lines behind the chart (if enabled)
        views.push(AnyView::new(GridLines {
            y_ticks: if y_show_grid { y_ticks.clone() } else { Vec::new() },
            x_ticks: if x_show_grid { x_ticks.clone() } else { Vec::new() },
            padding_left: padding.left,
            padding_right: padding.right,
            padding_top: padding.top,
            padding_bottom: padding.bottom,
        }));

        // Padded chart
        views.push(AnyView::new(padded_chart));

        // Y-axis tick labels (positioned left of chart)
        views.push(AnyView::new(AxisLabels {
            ticks: y_ticks,
            is_vertical: true,
            padding_offset: padding.left - 5.0,
            padding_start: padding.top,
            padding_end: padding.bottom,
        }));

        // X-axis tick labels (positioned below chart)
        views.push(AnyView::new(AxisLabels {
            ticks: x_ticks,
            is_vertical: false,
            padding_offset: padding.bottom - 5.0,
            padding_start: padding.left,
            padding_end: padding.right,
        }));

        // Y-axis title label (positioned at top-left)
        if let Some(label) = y_label {
            views.push(AnyView::new(
                text(label)
                    .size(12.0)
                    .bold()
                    .position_in_offset(UnitPoint::TOP_LEADING, UnitPoint::TOP_LEADING, 2.0_f32, 2.0_f32),
            ));
        }

        // X-axis title label (positioned at bottom-center)
        if let Some(label) = x_label {
            views.push(AnyView::new(
                text(label)
                    .size(12.0)
                    .bold()
                    .position_in_offset(UnitPoint::BOTTOM, UnitPoint::BOTTOM, 0.0_f32, -2.0_f32),
            ));
        }

        absolute(views)
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// Grid lines rendered behind the chart.
struct GridLines {
    y_ticks: Vec<Tick>,
    x_ticks: Vec<Tick>,
    padding_left: f32,
    padding_right: f32,
    padding_top: f32,
    padding_bottom: f32,
}

impl View for GridLines {
    fn body(self, _env: &Environment) -> impl View {
        let mut lines: Vec<AnyView> = Vec::new();

        // Horizontal grid lines (from Y-axis ticks)
        for tick in &self.y_ticks {
            lines.push(AnyView::new(GridLine {
                is_horizontal: true,
                position: tick.position(),
                padding_start: self.padding_left,
                padding_end: self.padding_right,
                padding_cross_start: self.padding_top,
                padding_cross_end: self.padding_bottom,
            }));
        }

        // Vertical grid lines (from X-axis ticks)
        for tick in &self.x_ticks {
            lines.push(AnyView::new(GridLine {
                is_horizontal: false,
                position: tick.position(),
                padding_start: self.padding_top,
                padding_end: self.padding_bottom,
                padding_cross_start: self.padding_left,
                padding_cross_end: self.padding_right,
            }));
        }

        absolute(lines)
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// A single grid line.
struct GridLine {
    is_horizontal: bool,
    position: f32,
    padding_start: f32,
    padding_end: f32,
    padding_cross_start: f32,
    padding_cross_end: f32,
}

/// Grid line color - light gray with low opacity.
fn grid_color() -> Color {
    Color::srgb_f32(0.5, 0.5, 0.5).with_opacity(0.3)
}

impl View for GridLine {
    fn body(self, _env: &Environment) -> impl View {
        if self.is_horizontal {
            // Horizontal line: full width, 1pt tall, positioned by Y
            let y_pos = map_position_y(self.position, self.padding_cross_start, self.padding_cross_end);
            Frame::new(grid_color())
                .height(1.0)
                .position_in_offset(
                    UnitPoint::LEADING,
                    UnitPoint::new(0.0, y_pos),
                    self.padding_start,
                    0.0_f32,
                )
        } else {
            // Vertical line: 1pt wide, full height, positioned by X
            let x_pos = map_position_x(self.position, self.padding_cross_start, self.padding_cross_end);
            Frame::new(grid_color())
                .width(1.0)
                .position_in_offset(
                    UnitPoint::TOP,
                    UnitPoint::new(x_pos, 0.0),
                    0.0_f32,
                    self.padding_start,
                )
        }
    }
}

/// Internal view for rendering axis labels.
struct AxisLabels {
    ticks: Vec<Tick>,
    is_vertical: bool,
    padding_offset: f32,
    padding_start: f32,
    padding_end: f32,
}

impl View for AxisLabels {
    fn body(self, _env: &Environment) -> impl View {
        // Create positioned text labels for each tick
        // Using a tuple of up to 10 labels (practical limit for most charts)
        let labels: Vec<_> = self
            .ticks
            .into_iter()
            .map(|tick| {
                if self.is_vertical {
                    // Y-axis: labels positioned vertically along left edge
                    // Position is 0 at bottom, 1 at top
                    // We invert because screen Y goes down
                    AxisLabel {
                        label: tick.label().into(),
                        is_vertical: true,
                        position: tick.position(),
                        offset: self.padding_offset,
                        padding_start: self.padding_start,
                        padding_end: self.padding_end,
                    }
                } else {
                    // X-axis: labels positioned horizontally along bottom edge
                    AxisLabel {
                        label: tick.label().into(),
                        is_vertical: false,
                        position: tick.position(),
                        offset: self.padding_offset,
                        padding_start: self.padding_start,
                        padding_end: self.padding_end,
                    }
                }
            })
            .collect();

        AxisLabelCollection { labels }
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// Collection of axis labels that renders as multiple positioned children.
struct AxisLabelCollection {
    labels: Vec<AxisLabel>,
}

impl View for AxisLabelCollection {
    fn body(self, _env: &Environment) -> impl View {
        // Convert labels to AnyView for TupleViews compatibility
        let views: Vec<AnyView> = self
            .labels
            .into_iter()
            .map(|label| AnyView::new(label))
            .collect();

        absolute(views)
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// Single axis label with positioning.
struct AxisLabel {
    label: alloc::string::String,
    is_vertical: bool,
    position: f32,
    offset: f32,
    padding_start: f32,
    padding_end: f32,
}

impl View for AxisLabel {
    fn body(self, _env: &Environment) -> impl View {
        if self.is_vertical {
            // Y-axis label: anchor at trailing (right) edge, position vertically
            // The chart area spans from padding_start to (1 - padding_end_ratio)
            // We need to map tick position [0,1] to this range
            // Since we're in an absolute container that fills the parent,
            // we use position_in_offset to set fractional Y position

            text(self.label).size(11.0).position_in_offset(
                UnitPoint::TRAILING,
                // Position at the fractional Y, accounting for padding
                // We use a workaround: position relative to left edge with X offset
                UnitPoint::new(0.0, map_position_y(self.position, self.padding_start, self.padding_end)),
                self.offset,
                0.0_f32,
            )
        } else {
            // X-axis label: anchor at top, position horizontally
            text(self.label).size(11.0).position_in_offset(
                UnitPoint::TOP,
                UnitPoint::new(
                    map_position_x(self.position, self.padding_start, self.padding_end),
                    1.0,
                ),
                0.0_f32,
                -self.offset,
            )
        }
    }
}

/// Maps a normalized position [0,1] to account for padding.
///
/// For Y-axis: 0 = bottom, 1 = top (inverted for screen coordinates)
fn map_position_y(position: f32, padding_top: f32, padding_bottom: f32) -> f32 {
    // In a 100pt tall container with 10pt top and 30pt bottom padding:
    // - Chart area is 60pt (from y=10 to y=70)
    // - Position 0 (bottom of chart) = y=70 (70/100 = 0.7 in unit coords)
    // - Position 1 (top of chart) = y=10 (10/100 = 0.1 in unit coords)
    //
    // We need to output fractional coordinates [0, 1] in screen space.
    // Since we don't know the actual height, we use relative math.
    //
    // Let's use a simpler approach: assume standard container size
    // and accept that this is approximate. For better precision,
    // the user can adjust padding.
    //
    // chart_top_frac = padding_top / container_height
    // chart_bottom_frac = (container_height - padding_bottom) / container_height
    //
    // We don't know container_height, so we estimate based on typical chart size (300pt)
    const ESTIMATED_HEIGHT: f32 = 300.0;

    let top_frac = padding_top / ESTIMATED_HEIGHT;
    let bottom_frac = (ESTIMATED_HEIGHT - padding_bottom) / ESTIMATED_HEIGHT;
    let chart_range = bottom_frac - top_frac;

    // Invert position (0 = bottom = high Y, 1 = top = low Y)
    top_frac + (1.0 - position) * chart_range
}

/// Maps a normalized position [0,1] to account for padding.
///
/// For X-axis: 0 = left, 1 = right
fn map_position_x(position: f32, padding_left: f32, padding_right: f32) -> f32 {
    const ESTIMATED_WIDTH: f32 = 400.0;

    let left_frac = padding_left / ESTIMATED_WIDTH;
    let right_frac = (ESTIMATED_WIDTH - padding_right) / ESTIMATED_WIDTH;
    let chart_range = right_frac - left_frac;

    left_frac + position * chart_range
}

/// A chart wrapped with reactive axis labels.
///
/// Axis labels automatically update when bounds change.
///
/// # Example
///
/// ```ignore
/// use waterui_chart::{BarChart, ChartExt, AxisConfig};
///
/// let data = binding(vec![...]);
/// let bounds = data.map(|d| DataBounds::from_points(&d));
///
/// BarChart::new(&data)
///     .axes_reactive(bounds)
///     .y_axis(AxisConfig::new().tick_count(5).show_grid())
/// ```
pub struct ChartAxesReactive<C, B> {
    chart: C,
    bounds: B,
    x_axis: Option<AxisConfig>,
    y_axis: Option<AxisConfig>,
    padding: AxisPadding,
}

impl<C, B> ChartAxesReactive<C, B> {
    /// Creates a new reactive chart axes wrapper.
    pub(crate) fn new(chart: C, bounds: B) -> Self {
        Self {
            chart,
            bounds,
            x_axis: Some(AxisConfig::default()),
            y_axis: Some(AxisConfig::default()),
            padding: AxisPadding::default(),
        }
    }

    /// Configures the Y-axis.
    #[must_use]
    pub fn y_axis(mut self, config: AxisConfig) -> Self {
        self.y_axis = Some(config);
        self
    }

    /// Disables the Y-axis.
    #[must_use]
    pub fn no_y_axis(mut self) -> Self {
        self.y_axis = None;
        self
    }

    /// Configures the X-axis.
    #[must_use]
    pub fn x_axis(mut self, config: AxisConfig) -> Self {
        self.x_axis = Some(config);
        self
    }

    /// Disables the X-axis.
    #[must_use]
    pub fn no_x_axis(mut self) -> Self {
        self.x_axis = None;
        self
    }

    /// Sets padding for axis labels.
    #[must_use]
    pub fn padding(mut self, left: f32, bottom: f32) -> Self {
        self.padding.left = left;
        self.padding.bottom = bottom;
        self
    }

    /// Sets all four padding values.
    #[must_use]
    pub fn padding_all(mut self, left: f32, bottom: f32, top: f32, right: f32) -> Self {
        self.padding.left = left;
        self.padding.bottom = bottom;
        self.padding.top = top;
        self.padding.right = right;
        self
    }
}

impl<C, B> View for ChartAxesReactive<C, B>
where
    C: View + 'static,
    B: Signal<Output = DataBounds> + Clone + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let padding = self.padding.clone();
        let x_axis = self.x_axis.clone();
        let y_axis = self.y_axis.clone();

        // Get axis title labels (clone before moving into closure)
        let y_label: Option<String> = y_axis.as_ref().and_then(|a| a.axis_label().map(Into::into));
        let x_label: Option<String> = x_axis.as_ref().and_then(|a| a.axis_label().map(Into::into));

        // Chart with padding to make room for axis labels
        let padded_chart = Padding::new(
            EdgeInsets::new(padding.top, padding.bottom, padding.left, padding.right),
            self.chart,
        );

        // Create reactive axis overlay that updates when bounds change
        let bounds = self.bounds;
        let reactive_axes = watch(bounds, move |b: DataBounds| {
            let y_show_grid = y_axis.as_ref().is_some_and(|a| a.has_grid());
            let x_show_grid = x_axis.as_ref().is_some_and(|a| a.has_grid());

            let y_ticks: Vec<Tick> = y_axis
                .as_ref()
                .map(|a| a.compute_ticks(b.min_y, b.max_y))
                .unwrap_or_default();

            let x_ticks: Vec<Tick> = x_axis
                .as_ref()
                .map(|a| a.compute_ticks(b.min_x, b.max_x))
                .unwrap_or_default();

            let mut views: Vec<AnyView> = Vec::new();

            // Grid lines behind
            views.push(AnyView::new(GridLines {
                y_ticks: if y_show_grid { y_ticks.clone() } else { Vec::new() },
                x_ticks: if x_show_grid { x_ticks.clone() } else { Vec::new() },
                padding_left: padding.left,
                padding_right: padding.right,
                padding_top: padding.top,
                padding_bottom: padding.bottom,
            }));

            // Y-axis tick labels
            views.push(AnyView::new(AxisLabels {
                ticks: y_ticks,
                is_vertical: true,
                padding_offset: padding.left - 5.0,
                padding_start: padding.top,
                padding_end: padding.bottom,
            }));

            // X-axis tick labels
            views.push(AnyView::new(AxisLabels {
                ticks: x_ticks,
                is_vertical: false,
                padding_offset: padding.bottom - 5.0,
                padding_start: padding.left,
                padding_end: padding.right,
            }));

            // Y-axis title label
            if let Some(ref label) = y_label {
                views.push(AnyView::new(
                    text(label.clone())
                        .size(12.0)
                        .bold()
                        .position_in_offset(UnitPoint::TOP_LEADING, UnitPoint::TOP_LEADING, 2.0_f32, 2.0_f32),
                ));
            }

            // X-axis title label
            if let Some(ref label) = x_label {
                views.push(AnyView::new(
                    text(label.clone())
                        .size(12.0)
                        .bold()
                        .position_in_offset(UnitPoint::BOTTOM, UnitPoint::BOTTOM, 0.0_f32, -2.0_f32),
                ));
            }

            absolute(views)
        });

        // Use absolute layout - it stretches to fill parent
        absolute((padded_chart, reactive_axes))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// Extension trait for adding axes to charts.
pub trait ChartExt: View + Sized {
    /// Wraps the chart with axis labels.
    ///
    /// Both X and Y axes are enabled by default with auto-formatting.
    ///
    /// # Example
    ///
    /// ```ignore
    /// BarChart::new(&data)
    ///     .axes(DataBounds::from_points(&data.get()))
    /// ```
    fn axes(self, bounds: DataBounds) -> ChartAxes<Self> {
        ChartAxes::new(self, bounds)
    }

    /// Wraps the chart with reactive axis labels.
    ///
    /// Axis labels automatically update when the bounds signal changes.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let data = binding(vec![...]);
    /// let bounds = data.map(|d| DataBounds::from_points(&d));
    ///
    /// BarChart::new(&data)
    ///     .axes_reactive(bounds)
    /// ```
    fn axes_reactive<B>(self, bounds: B) -> ChartAxesReactive<Self, B>
    where
        B: Signal<Output = DataBounds>,
    {
        ChartAxesReactive::new(self, bounds)
    }
}

impl<V: View + Sized> ChartExt for V {}
