//! Contour chart component.

use nami::{Binding, Signal};
use waterui_core::{Environment, View};

use crate::charts::canvas::{contour_geometry, draw_contour, interactive_signal_canvas};
use crate::composition::ChartComposition;
use crate::data::ContourData;
use crate::interaction::{GridDatum, HitResult, SelectionBindings};
use crate::params::{ChartParamError, PositiveF32};

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

    /// Sets the contour line width and panics if the value is invalid.
    ///
    /// # Panics
    ///
    /// Panics when `width` is not finite or is not strictly positive.
    #[must_use]
    pub fn line_width(self, width: f32) -> Self {
        self.try_line_width(width)
            .expect("ContourChart::line_width(width) requires finite width > 0")
    }

    /// Sets the contour line width using an already-validated positive value.
    #[must_use]
    pub const fn with_line_width(mut self, width: PositiveF32) -> Self {
        self.line_width = width.get();
        self
    }

    /// Attempts to set the contour line width.
    ///
    /// # Errors
    ///
    /// Returns [`ChartParamError`] when `width` is not finite or is not strictly positive.
    pub fn try_line_width(self, width: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_line_width(PositiveF32::try_new(width)?))
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
