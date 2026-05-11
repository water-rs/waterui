//! Line chart component.

extern crate alloc;

use alloc::vec::Vec;

use nami::{Binding, Signal};
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{
    draw_line, interactive_cartesian_signal_canvas, point_bounds, point_geometry,
};
use crate::composition::ChartComposition;
use crate::data::DataPoint;
use crate::interaction::{
    CartesianSelectionBindings, CartesianViewportBindings, HitResult, SelectionBindings,
};
use crate::params::{PositiveF32, UnitInterval};

/// Line chart visualization.
///
/// Renders data as connected lines with optional area fill.
/// Supports smooth animations and semantic focus/selection.
pub struct LineChart<S: Signal<Output = Vec<DataPoint>>> {
    data: S,
    color: Srgb,
    line_width: f32,
    show_fill: bool,
    fill_opacity: f32,
    selection: SelectionBindings<DataPoint>,
    cartesian_selection: CartesianSelectionBindings,
    cartesian_viewport: CartesianViewportBindings,
    composition: ChartComposition<DataPoint>,
}

crate::charts::impl_chart_debug!(LineChart, S, Vec<DataPoint>);

impl<S: Signal<Output = Vec<DataPoint>>> LineChart<S> {
    /// Creates a line chart from reactive point data.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            color: Srgb::from_hex("#22C55E"),
            line_width: 2.0,
            show_fill: false,
            fill_opacity: 0.3,
            selection: SelectionBindings::default(),
            cartesian_selection: CartesianSelectionBindings::default(),
            cartesian_viewport: CartesianViewportBindings::default(),
            composition: ChartComposition::default(),
        }
    }

    crate::interaction::chart_x_selection_methods!();

    crate::composition::chart_composition_methods!(DataPoint);

    /// Sets the line color.
    #[must_use]
    pub const fn color(mut self, color: Srgb) -> Self {
        self.color = color;
        self
    }

    /// Sets the line width.
    ///
    /// Accepts any value convertible into [`PositiveF32`]. Passing a raw
    /// `f32` panics on `NaN`, infinity, or non-positive values; pass a
    /// pre-validated [`PositiveF32`] to bypass the panic-on-invalid path.
    #[must_use]
    pub fn line_width(mut self, width: impl Into<PositiveF32>) -> Self {
        self.line_width = width.into().get();
        self
    }

    /// Enables area fill under the line with the given opacity.
    ///
    /// Accepts any value convertible into [`UnitInterval`]. Passing a raw
    /// `f32` panics on `NaN`, infinity, or values outside `[0.0, 1.0]`.
    #[must_use]
    pub fn fill(mut self, opacity: impl Into<UnitInterval>) -> Self {
        self.show_fill = true;
        self.fill_opacity = opacity.into().get();
        self
    }

    /// Tracks the currently focused point in an external binding.
    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<DataPoint>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    /// Tracks the currently selected point in an external binding.
    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<DataPoint>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = Vec<DataPoint>> + Clone + 'static> View for LineChart<S> {
    fn body(self, env: &Environment) -> impl View {
        let color = self.color;
        let line_width = self.line_width;
        let show_fill = self.show_fill;
        let fill_opacity = self.fill_opacity;
        interactive_cartesian_signal_canvas(
            env,
            self.data,
            |data: &Vec<DataPoint>| point_bounds(data),
            move |ctx, data, bounds| point_geometry(ctx, data, bounds, (line_width * 2.5).max(8.0)),
            move |ctx, data, geometry| {
                draw_line(
                    ctx,
                    data,
                    geometry.bounds,
                    color,
                    line_width,
                    show_fill,
                    fill_opacity,
                );
            },
            self.selection,
            self.cartesian_selection,
            self.cartesian_viewport,
            self.composition,
        )
    }
}

/// Convenience constructor for [`LineChart`]. Equivalent to [`LineChart::new`].
#[must_use]
pub fn line_chart<S: Signal<Output = Vec<DataPoint>>>(data: S) -> LineChart<S> {
    LineChart::new(data)
}
