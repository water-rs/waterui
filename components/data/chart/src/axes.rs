//! Chart axes view wrapper with labels and grid lines.
//!
//! Wraps a chart with native text labels positioned using absolute layout.
//! Uses the same coordinate system as the chart shader for accurate positioning.

extern crate alloc;

use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};

use nami::signal::IntoComputed;
use nami::{Computed, SignalExt, collection::SignalCollection};
use waterui_core::accessibility::{AccessibilityLabel, AccessibilityRole};
use waterui_core::{AnyView, Environment, Metadata, View, id::Identifiable, views::ForEach};
use waterui_graphics::color::Color;
use waterui_layout::container::{FixedContainer, LazyContainer};
use waterui_layout::padding::{EdgeInsets, Padding};
use waterui_layout::{
    AbsoluteLayout, Layout, Point, PositionExt, ProposalSize, Rect, Size, StretchAxis, SubView,
    UnitPoint, absolute,
};
use waterui_text::{Text, text};

use crate::axis::{AxisConfig, Tick};
use crate::data::DataBounds;

struct AxisAccessibilityBoundary {
    content: AnyView,
}

impl AxisAccessibilityBoundary {
    fn new(content: impl View + 'static) -> Self {
        Self {
            content: AnyView::new(content),
        }
    }
}

impl View for AxisAccessibilityBoundary {
    fn body(self, env: &Environment) -> impl View {
        let mut content_env = env.clone();
        content_env.remove::<AccessibilityLabel>();
        content_env.remove::<AccessibilityRole>();
        Metadata::new(self.content, content_env)
    }
}

/// Axis padding configuration (in points).
#[derive(Debug, Clone)]
struct AxisPadding {
    left: f32,
    bottom: f32,
    top: f32,
    right: f32,
    plot: f32,
}

impl Default for AxisPadding {
    fn default() -> Self {
        Self {
            left: 50.0,
            bottom: 30.0,
            top: 10.0,
            right: 10.0,
            plot: 0.1,
        }
    }
}

/// A chart wrapped with axis labels.
///
/// Use the builder pattern to configure axes.
///
/// # Example
///
/// ```rust
/// # use waterui::prelude::*;
/// use waterui_chart::{AxisConfig, BarChart, ChartExt, DataBounds, DataPoint, TickFormat};
///
/// # fn revenue() -> impl View {
/// # let points = vec![DataPoint::new(0.0, 100.0), DataPoint::new(1.0, 150.0)];
/// # let bounds = DataBounds::from_points(&points);
/// # let data = binding(points);
/// BarChart::new(data)
///     .axes(bounds)
///     .y_axis(AxisConfig::new().tick_count(5).show_grid())
///     .x_axis(AxisConfig::new().format(TickFormat::SI))
/// # }
/// ```
pub struct ChartAxes<C> {
    chart: C,
    x_axis: Option<AxisConfig>,
    y_axis: Option<AxisConfig>,
    bounds: Computed<DataBounds>,
    padding: AxisPadding,
}

impl<C> core::fmt::Debug for ChartAxes<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChartAxes")
            .field("x_axis", &self.x_axis)
            .field("y_axis", &self.y_axis)
            .field("bounds", &self.bounds)
            .field("padding", &self.padding)
            .finish_non_exhaustive()
    }
}

impl<C> ChartAxes<C> {
    /// Creates a new chart axes wrapper.
    pub(crate) fn new(chart: C, bounds: impl IntoComputed<DataBounds>) -> Self {
        let bounds = bounds.into_computed();
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
    pub const fn padding(mut self, left: f32, bottom: f32) -> Self {
        self.padding.left = left;
        self.padding.bottom = bottom;
        self
    }

    /// Sets all four padding values.
    #[must_use]
    pub const fn padding_all(mut self, left: f32, bottom: f32, top: f32, right: f32) -> Self {
        self.padding.left = left;
        self.padding.bottom = bottom;
        self.padding.top = top;
        self.padding.right = right;
        self
    }

    /// Sets inner plot padding as a fraction of the plot area.
    #[must_use]
    pub const fn plot_padding(mut self, padding: f32) -> Self {
        self.padding.plot = padding;
        self
    }
}

impl<C: View + 'static> View for ChartAxes<C> {
    fn body(self, _env: &Environment) -> impl View {
        let padding = self.padding.clone();
        let x_axis = self.x_axis.clone();
        let y_axis = self.y_axis.clone();

        // Get axis title labels (clone before moving into closure)
        let y_label = y_axis.as_ref().and_then(|axis| axis.axis_label().cloned());
        let x_label = x_axis.as_ref().and_then(|axis| axis.axis_label().cloned());

        // Chart with padding to make room for axis labels
        let padded_chart = Padding::new(
            EdgeInsets::new(padding.top, padding.bottom, padding.left, padding.right),
            self.chart,
        );

        let y_show_grid = y_axis
            .as_ref()
            .is_some_and(super::axis::AxisConfig::has_grid);
        let x_show_grid = x_axis
            .as_ref()
            .is_some_and(super::axis::AxisConfig::has_grid);
        let y_ticks = self
            .bounds
            .map(move |bounds: DataBounds| {
                y_axis
                    .as_ref()
                    .map(|axis| axis.compute_ticks(bounds.min_y, bounds.max_y))
                    .unwrap_or_default()
                    .into_iter()
                    .map(ReactiveTick)
                    .collect()
            })
            .computed();
        let x_ticks = self
            .bounds
            .map(move |bounds: DataBounds| {
                x_axis
                    .as_ref()
                    .map(|axis| axis.compute_ticks(bounds.min_x, bounds.max_x))
                    .unwrap_or_default()
                    .into_iter()
                    .map(ReactiveTick)
                    .collect()
            })
            .computed();
        let reactive_axes = AxisAccessibilityBoundary::new(absolute((
            ReactiveAxis::new(y_ticks, true, y_show_grid, padding.clone()),
            ReactiveAxis::new(x_ticks, false, x_show_grid, padding),
        )));

        // Use absolute layout - it stretches to fill parent
        absolute((
            padded_chart,
            reactive_axes,
            AxisAccessibilityBoundary::new(axis_titles(y_label, x_label)),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

fn grid_color() -> Color {
    Color::srgb_f32(0.5, 0.5, 0.5).with_opacity(0.3)
}

#[derive(Clone)]
struct ReactiveTick(Tick);

impl Identifiable for ReactiveTick {
    type Id = (u32, u32, String);

    fn id(&self) -> Self::Id {
        (
            self.0.value().to_bits(),
            self.0.position().to_bits(),
            self.0.label().to_string(),
        )
    }
}

#[derive(Debug, Clone)]
struct ReactiveAxisLayout {
    vertical: bool,
    position: f32,
    padding: AxisPadding,
    label: bool,
}

impl ReactiveAxisLayout {
    fn chart_bounds(&self, bounds: Rect) -> (f32, f32, f32, f32) {
        let chart_left = bounds.x() + self.padding.left;
        let chart_right = bounds.x() + bounds.width() - self.padding.right;
        let chart_top = bounds.y() + self.padding.top;
        let chart_bottom = bounds.y() + bounds.height() - self.padding.bottom;
        let chart_width = (chart_right - chart_left).max(0.0);
        let chart_height = (chart_bottom - chart_top).max(0.0);
        let plot_padding = self.padding.plot.clamp(0.0, 0.45);
        let plot_pad_x = chart_width * plot_padding;
        let plot_pad_y = chart_height * plot_padding;
        (
            chart_left + plot_pad_x,
            chart_top + plot_pad_y,
            plot_pad_x.mul_add(-2.0, chart_width).max(0.0),
            plot_pad_y.mul_add(-2.0, chart_height).max(0.0),
        )
    }
}

impl Layout for ReactiveAxisLayout {
    fn size_that_fits(&self, proposal: ProposalSize, _children: &[&dyn SubView]) -> Size {
        Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        )
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        let [child] = children else {
            panic!("reactive axis item must contain exactly one child");
        };
        let (chart_left, chart_top, chart_width, chart_height) = self.chart_bounds(bounds);
        let position = self.position.clamp(0.0, 1.0);

        if self.label {
            let size = child.measure(ProposalSize::UNSPECIFIED).size;
            if self.vertical {
                let x = bounds.x() + self.padding.left - 5.0 - size.width;
                let y = size
                    .height
                    .mul_add(-0.5, (1.0 - position).mul_add(chart_height, chart_top));
                vec![Rect::new(Point::new(x, y), size)]
            } else {
                let x = size
                    .width
                    .mul_add(-0.5, position.mul_add(chart_width, chart_left));
                let y = bounds.y() + bounds.height() - self.padding.bottom + 5.0 - size.height;
                vec![Rect::new(Point::new(x, y), size)]
            }
        } else if self.vertical {
            let y = (1.0 - position).mul_add(chart_height, chart_top);
            vec![Rect::new(
                Point::new(chart_left, y - 0.5),
                Size::new(chart_width, 1.0),
            )]
        } else {
            let x = position.mul_add(chart_width, chart_left);
            vec![Rect::new(
                Point::new(x - 0.5, chart_top),
                Size::new(1.0, chart_height),
            )]
        }
    }

    fn stretch_axis(&self, _children: &[StretchAxis]) -> StretchAxis {
        StretchAxis::Both
    }
}

struct ReactiveAxisTick {
    tick: Tick,
    vertical: bool,
    show_grid: bool,
    padding: AxisPadding,
}

impl View for ReactiveAxisTick {
    fn body(self, _env: &Environment) -> impl View {
        let grid = self.show_grid.then(|| {
            FixedContainer::new(
                ReactiveAxisLayout {
                    vertical: self.vertical,
                    position: self.tick.position(),
                    padding: self.padding.clone(),
                    label: false,
                },
                (grid_color(),),
            )
        });
        let label = FixedContainer::new(
            ReactiveAxisLayout {
                vertical: self.vertical,
                position: self.tick.position(),
                padding: self.padding,
                label: true,
            },
            (text(self.tick.label().to_string()).size(11.0),),
        );

        absolute((grid, label))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

struct ReactiveAxis {
    ticks: Computed<Vec<ReactiveTick>>,
    vertical: bool,
    show_grid: bool,
    padding: AxisPadding,
}

impl ReactiveAxis {
    const fn new(
        ticks: Computed<Vec<ReactiveTick>>,
        vertical: bool,
        show_grid: bool,
        padding: AxisPadding,
    ) -> Self {
        Self {
            ticks,
            vertical,
            show_grid,
            padding,
        }
    }
}

impl View for ReactiveAxis {
    fn body(self, _env: &Environment) -> impl View {
        let vertical = self.vertical;
        let show_grid = self.show_grid;
        let padding = self.padding;
        LazyContainer::new(
            AbsoluteLayout,
            ForEach::new(
                SignalCollection::new(self.ticks),
                move |tick: ReactiveTick| ReactiveAxisTick {
                    tick: tick.0,
                    vertical,
                    show_grid,
                    padding: padding.clone(),
                },
            ),
        )
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

fn axis_titles(y_label: Option<Text>, x_label: Option<Text>) -> impl View {
    absolute((
        y_label.map(|label| {
            label.size(12.0).bold().position_in_offset(
                UnitPoint::TOP_LEADING,
                UnitPoint::TOP_LEADING,
                2.0,
                2.0,
            )
        }),
        x_label.map(|label| {
            label.size(12.0).bold().position_in_offset(
                UnitPoint::BOTTOM,
                UnitPoint::BOTTOM,
                0.0,
                -2.0,
            )
        }),
    ))
}

/// Extension trait for adding axes to charts.
pub trait ChartExt: View + Sized {
    /// Wraps the chart with axis labels.
    ///
    /// Both X and Y axes are enabled by default with auto-formatting.
    ///
    /// # Example
    ///
    /// Bounds accept a constant or any signal, so axis labels update
    /// automatically when reactive bounds change:
    ///
    /// ```rust
    /// # use waterui::prelude::*;
    /// # use waterui_chart::{BarChart, ChartExt, DataBounds, DataPoint};
    /// # fn revenue() -> impl View {
    /// let data: Binding<Vec<DataPoint>> =
    ///     binding(vec![DataPoint::new(0.0, 100.0), DataPoint::new(1.0, 150.0)]);
    /// let bounds = data.map(|d| DataBounds::from_points(&d));
    ///
    /// BarChart::new(data).axes(bounds)
    /// # }
    /// ```
    fn axes(self, bounds: impl IntoComputed<DataBounds>) -> ChartAxes<Self> {
        ChartAxes::new(self, bounds)
    }
}

impl<V: View + Sized> ChartExt for V {}
