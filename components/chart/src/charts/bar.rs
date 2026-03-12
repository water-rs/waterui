//! Bar chart component.

extern crate alloc;

use alloc::vec::Vec;

use nami::{Binding, Signal};
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{bar_bounds, bar_geometry, draw_bar, interactive_signal_canvas};
use crate::data::DataPoint;
use crate::interaction::{HitResult, SelectionBindings};

/// Bar chart visualization.
pub struct BarChart<S: Signal<Output = Vec<DataPoint>>> {
    data: S,
    color: Srgb,
    selection: SelectionBindings<DataPoint>,
}

impl<S: Signal<Output = Vec<DataPoint>>> BarChart<S> {
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            color: Srgb::from_hex("#3B82F6"),
            selection: SelectionBindings::default(),
        }
    }

    #[must_use]
    pub fn color(mut self, color: Srgb) -> Self {
        self.color = color;
        self
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

impl<S: Signal<Output = Vec<DataPoint>> + Clone + 'static> View for BarChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let color = self.color;
        interactive_signal_canvas(
            self.data,
            move |ctx, data| {
                let bounds = bar_bounds(data);
                bar_geometry(ctx, data, bounds)
            },
            move |ctx, data, geometry| {
                draw_bar(ctx, data, geometry.bounds, color);
            },
            self.selection,
        )
    }
}
