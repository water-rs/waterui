//! Contour chart component.

use nami::{Binding, Signal};
use waterui_core::{Environment, View};

use crate::charts::canvas::{contour_geometry, draw_contour, interactive_signal_canvas};
use crate::composition::ChartComposition;
use crate::data::ContourData;
use crate::interaction::{GridDatum, HitResult, SelectionBindings};
use crate::params::PositiveF32;

/// Contour chart for isoline visualization.
pub struct ContourChart<S: Signal<Output = ContourData>> {
    data: S,
    line_width: f32,
    selection: SelectionBindings<GridDatum>,
    composition: ChartComposition<GridDatum>,
}

crate::charts::impl_chart_debug!(ContourChart, S, ContourData);

impl<S: Signal<Output = ContourData>> ContourChart<S> {
    /// Creates a contour chart from reactive scalar-field data.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            line_width: 2.0,
            selection: SelectionBindings::new(),
            composition: ChartComposition::default(),
        }
    }

    crate::composition::chart_composition_methods!(GridDatum);

    /// Sets the contour line width.
    ///
    /// Accepts any value convertible into [`PositiveF32`]. Passing a raw
    /// `f32` panics on `NaN`, infinity, or non-positive values.
    #[must_use]
    pub fn line_width(mut self, width: impl Into<PositiveF32>) -> Self {
        self.line_width = width.into().get();
        self
    }

    /// Tracks the currently focused contour cell in an external binding.
    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<GridDatum>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    /// Tracks the currently selected contour cell in an external binding.
    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<GridDatum>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = ContourData> + Clone + 'static> View for ContourChart<S> {
    fn body(self, env: &Environment) -> impl View {
        let line_width = self.line_width;
        interactive_signal_canvas(
            env,
            self.data,
            contour_geometry,
            move |ctx, data, _geometry| {
                draw_contour(ctx, data, line_width);
            },
            self.selection,
            self.composition,
        )
    }
}

/// Convenience constructor for [`ContourChart`]. Equivalent to [`ContourChart::new`].
#[must_use]
pub fn contour_chart<S: Signal<Output = ContourData>>(data: S) -> ContourChart<S> {
    ContourChart::new(data)
}
