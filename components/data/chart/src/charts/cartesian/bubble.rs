//! Bubble chart component.

use nami::{Binding, Signal};
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{
    bubble_bounds, bubble_geometry, draw_bubble, interactive_cartesian_signal_canvas,
};
use crate::composition::ChartComposition;
use crate::data::BubblePoint;
use crate::interaction::{
    CartesianSelectionBindings, CartesianViewportBindings, HitResult, SelectionBindings,
};
use crate::params::{PositiveF32, UnitInterval};

/// Bubble chart for 3D data visualization.
pub struct BubbleChart<S: Signal<Output = Vec<BubblePoint>>> {
    data: S,
    color: Srgb,
    min_radius: f32,
    max_radius: f32,
    opacity: f32,
    selection: SelectionBindings<BubblePoint>,
    cartesian_selection: CartesianSelectionBindings,
    cartesian_viewport: CartesianViewportBindings,
    composition: ChartComposition<BubblePoint>,
}

crate::charts::impl_chart_debug!(BubbleChart, S, Vec<BubblePoint>);

impl<S: Signal<Output = Vec<BubblePoint>>> BubbleChart<S> {
    /// Creates a bubble chart from reactive bubble data.
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            color: Srgb::new(0.23, 0.51, 0.96),
            min_radius: 5.0,
            max_radius: 30.0,
            opacity: 0.7,
            selection: SelectionBindings::default(),
            cartesian_selection: CartesianSelectionBindings::default(),
            cartesian_viewport: CartesianViewportBindings::default(),
            composition: ChartComposition::default(),
        }
    }

    crate::interaction::chart_x_selection_methods!();

    crate::composition::chart_composition_methods!(BubblePoint);

    /// Sets the bubble fill color.
    #[must_use]
    pub const fn color(mut self, color: Srgb) -> Self {
        self.color = color;
        self
    }

    /// Sets the minimum bubble radius.
    ///
    /// Accepts any value convertible into [`PositiveF32`]. Passing a raw
    /// `f32` panics on `NaN`, infinity, or non-positive values. If the
    /// minimum exceeds the current maximum, the maximum is widened to match.
    #[must_use]
    pub fn min_radius(mut self, radius: impl Into<PositiveF32>) -> Self {
        self.min_radius = radius.into().get();
        if self.max_radius < self.min_radius {
            self.max_radius = self.min_radius;
        }
        self
    }

    /// Sets the maximum bubble radius.
    ///
    /// Accepts any value convertible into [`PositiveF32`]. Passing a raw
    /// `f32` panics on `NaN`, infinity, or non-positive values. If the
    /// maximum is less than the current minimum, the minimum is reduced to
    /// match.
    #[must_use]
    pub fn max_radius(mut self, radius: impl Into<PositiveF32>) -> Self {
        self.max_radius = radius.into().get();
        if self.max_radius < self.min_radius {
            self.min_radius = self.max_radius;
        }
        self
    }

    /// Sets bubble opacity.
    ///
    /// Accepts any value convertible into [`UnitInterval`]. Passing a raw
    /// `f32` panics on `NaN`, infinity, or values outside `[0.0, 1.0]`.
    #[must_use]
    pub fn opacity(mut self, opacity: impl Into<UnitInterval>) -> Self {
        self.opacity = opacity.into().get();
        self
    }

    /// Tracks the currently focused bubble in an external binding.
    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<BubblePoint>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    /// Tracks the currently selected bubble in an external binding.
    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<BubblePoint>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = Vec<BubblePoint>> + Clone + 'static> View for BubbleChart<S> {
    fn body(self, env: &Environment) -> impl View {
        let color = self.color;
        let min_radius = self.min_radius;
        let max_radius = self.max_radius;
        let opacity = self.opacity;
        interactive_cartesian_signal_canvas(
            env,
            self.data,
            |data: &Vec<BubblePoint>| bubble_bounds(data),
            move |ctx, data, bounds| bubble_geometry(ctx, data, bounds, min_radius, max_radius),
            move |ctx, data, geometry, transition_alpha| {
                draw_bubble(
                    ctx,
                    data,
                    geometry.bounds,
                    color,
                    min_radius,
                    max_radius,
                    opacity,
                    transition_alpha,
                );
            },
            self.selection,
            self.cartesian_selection,
            self.cartesian_viewport,
            self.composition,
        )
    }
}

/// Convenience constructor for [`BubbleChart`]. Equivalent to [`BubbleChart::new`].
#[must_use]
pub fn bubble_chart<S: Signal<Output = Vec<BubblePoint>>>(data: S) -> BubbleChart<S> {
    BubbleChart::new(data)
}
