//! Bubble chart component.

use nami::{Binding, Signal};
use waterui_core::{Environment, View};
use waterui_graphics::color::Srgb;

use crate::charts::canvas::{
    bubble_bounds, bubble_geometry, draw_bubble, interactive_signal_canvas,
};
use crate::composition::ChartComposition;
use crate::data::BubblePoint;
use crate::interaction::{HitResult, SelectionBindings};
use crate::params::{ChartParamError, PositiveF32, UnitInterval};

/// Bubble chart for 3D data visualization.
pub struct BubbleChart<S: Signal<Output = Vec<BubblePoint>>> {
    data: S,
    color: Srgb,
    min_radius: f32,
    max_radius: f32,
    opacity: f32,
    selection: SelectionBindings<BubblePoint>,
    composition: ChartComposition<BubblePoint>,
}

impl<S: Signal<Output = Vec<BubblePoint>>> BubbleChart<S> {
    #[must_use]
    pub fn new(data: S) -> Self {
        Self {
            data,
            color: Srgb::new(0.23, 0.51, 0.96),
            min_radius: 5.0,
            max_radius: 30.0,
            opacity: 0.7,
            selection: SelectionBindings::default(),
            composition: ChartComposition::default(),
        }
    }

    crate::composition::chart_composition_methods!(BubblePoint);

    #[must_use]
    pub fn color(mut self, color: Srgb) -> Self {
        self.color = color;
        self
    }

    #[must_use]
    pub fn min_radius(self, radius: f32) -> Self {
        self.try_min_radius(radius)
            .expect("BubbleChart::min_radius(radius) requires finite radius > 0")
    }

    #[must_use]
    pub fn with_min_radius(mut self, radius: PositiveF32) -> Self {
        self.min_radius = radius.get();
        if self.max_radius < self.min_radius {
            self.max_radius = self.min_radius;
        }
        self
    }

    pub fn try_min_radius(self, radius: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_min_radius(PositiveF32::try_new(radius)?))
    }

    #[must_use]
    pub fn max_radius(self, radius: f32) -> Self {
        self.try_max_radius(radius)
            .expect("BubbleChart::max_radius(radius) requires finite radius > 0")
    }

    #[must_use]
    pub fn with_max_radius(mut self, radius: PositiveF32) -> Self {
        self.max_radius = radius.get();
        if self.max_radius < self.min_radius {
            self.min_radius = self.max_radius;
        }
        self
    }

    pub fn try_max_radius(self, radius: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_max_radius(PositiveF32::try_new(radius)?))
    }

    #[must_use]
    pub fn opacity(self, opacity: f32) -> Self {
        self.try_opacity(opacity)
            .expect("BubbleChart::opacity(opacity) requires finite 0.0 <= opacity <= 1.0")
    }

    #[must_use]
    pub fn with_opacity(mut self, opacity: UnitInterval) -> Self {
        self.opacity = opacity.get();
        self
    }

    pub fn try_opacity(self, opacity: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_opacity(UnitInterval::try_new(opacity)?))
    }

    #[must_use]
    pub fn focused(mut self, focused: &Binding<Option<HitResult<BubblePoint>>>) -> Self {
        self.selection = self.selection.with_focused(focused);
        self
    }

    #[must_use]
    pub fn selected(mut self, selected: &Binding<Option<HitResult<BubblePoint>>>) -> Self {
        self.selection = self.selection.with_selected(selected);
        self
    }
}

impl<S: Signal<Output = Vec<BubblePoint>> + Clone + 'static> View for BubbleChart<S> {
    fn body(self, _env: &Environment) -> impl View {
        let color = self.color;
        let min_radius = self.min_radius;
        let max_radius = self.max_radius;
        let opacity = self.opacity;
        interactive_signal_canvas(
            _env,
            self.data,
            move |ctx, data| {
                let bounds = bubble_bounds(data);
                bubble_geometry(ctx, data, bounds, min_radius, max_radius)
            },
            move |ctx, data, geometry| {
                draw_bubble(
                    ctx,
                    data,
                    geometry.bounds,
                    color,
                    min_radius,
                    max_radius,
                    opacity,
                );
            },
            self.selection,
            self.composition,
        )
    }
}
