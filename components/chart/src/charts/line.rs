//! Line chart component.

extern crate alloc;

use alloc::vec::Vec;

use nami::{Binding, Signal};
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{draw_line, interactive_cartesian_signal_canvas, point_bounds, point_geometry};
use crate::composition::ChartComposition;
use crate::data::DataPoint;
use crate::interaction::{CartesianSelectionBindings, CartesianViewportBindings, HitResult, SelectionBindings};
use crate::params::{ChartParamError, PositiveF32, UnitInterval};

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

impl<S: Signal<Output = Vec<DataPoint>>> LineChart<S> {
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

    #[must_use]
    pub fn color(mut self, color: Srgb) -> Self {
        self.color = color;
        self
    }

    #[must_use]
    pub fn line_width(self, width: f32) -> Self {
        self.try_line_width(width)
            .expect("LineChart::line_width(width) requires finite width > 0")
    }

    #[must_use]
    pub fn with_line_width(mut self, width: PositiveF32) -> Self {
        self.line_width = width.get();
        self
    }

    pub fn try_line_width(self, width: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_line_width(PositiveF32::try_new(width)?))
    }

    #[must_use]
    pub fn fill(self, opacity: f32) -> Self {
        self.try_fill(opacity)
            .expect("LineChart::fill(opacity) requires finite 0.0 <= opacity <= 1.0")
    }

    #[must_use]
    pub fn with_fill_opacity(mut self, opacity: UnitInterval) -> Self {
        self.show_fill = true;
        self.fill_opacity = opacity.get();
        self
    }

    pub fn try_fill(self, opacity: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_fill_opacity(UnitInterval::try_new(opacity)?))
    }

    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<DataPoint>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<DataPoint>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = Vec<DataPoint>> + Clone + 'static> View for LineChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let color = self.color;
        let line_width = self.line_width;
        let show_fill = self.show_fill;
        let fill_opacity = self.fill_opacity;
        interactive_cartesian_signal_canvas(
            _env,
            self.data,
            |data: &Vec<DataPoint>| point_bounds(data),
            move |ctx, data, bounds| {
                point_geometry(ctx, data, bounds, (line_width * 2.5).max(8.0))
            },
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
