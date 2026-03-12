extern crate alloc;

use alloc::rc::Rc;

use nami::{Computed, SignalExt};
use waterui_core::{AnyView, View};
use waterui_layout::background::background;
use waterui_layout::overlay::Overlay;

use crate::interaction::{ChartAnchor, ChartViewport, HitResult, SelectionBindings};

type ChartLayerBuilder<T> = Rc<dyn Fn(ChartProxy<T>) -> AnyView>;

/// Reactive chart composition proxy passed to chart overlay/background builders.
#[derive(Clone)]
pub struct ChartProxy<T: Clone + PartialEq + 'static> {
    chart_frame: Computed<ChartViewport>,
    plot_area_frame: Computed<ChartViewport>,
    focused: Computed<Option<HitResult<T>>>,
    selected: Computed<Option<HitResult<T>>>,
}

impl<T: Clone + PartialEq + 'static> ChartProxy<T> {
    pub(crate) const fn new(
        chart_frame: Computed<ChartViewport>,
        plot_area_frame: Computed<ChartViewport>,
        focused: Computed<Option<HitResult<T>>>,
        selected: Computed<Option<HitResult<T>>>,
    ) -> Self {
        Self {
            chart_frame,
            plot_area_frame,
            focused,
            selected,
        }
    }

    /// Returns the full chart frame in the chart view's local coordinate space.
    #[must_use]
    pub fn chart_frame(&self) -> Computed<ChartViewport> {
        self.chart_frame.clone()
    }

    /// Returns the plot area frame in the chart view's local coordinate space.
    #[must_use]
    pub fn plot_area_frame(&self) -> Computed<ChartViewport> {
        self.plot_area_frame.clone()
    }

    /// Returns the currently focused datum, if any.
    #[must_use]
    pub fn focused(&self) -> Computed<Option<HitResult<T>>> {
        self.focused.clone()
    }

    /// Returns the currently selected datum, if any.
    #[must_use]
    pub fn selected(&self) -> Computed<Option<HitResult<T>>> {
        self.selected.clone()
    }

    /// Returns the focused anchor when focus exists.
    #[must_use]
    pub fn focused_anchor(&self) -> Computed<Option<ChartAnchor>> {
        self.focused.map_some(|hit| hit.anchor).computed()
    }

    /// Returns the selected anchor when selection exists.
    #[must_use]
    pub fn selected_anchor(&self) -> Computed<Option<ChartAnchor>> {
        self.selected.map_some(|hit| hit.anchor).computed()
    }
}

#[derive(Clone)]
pub(crate) struct ChartComposition<T: Clone + PartialEq + 'static> {
    background: alloc::vec::Vec<ChartLayerBuilder<T>>,
    overlay: alloc::vec::Vec<ChartLayerBuilder<T>>,
}

impl<T: Clone + PartialEq + 'static> Default for ChartComposition<T> {
    fn default() -> Self {
        Self {
            background: alloc::vec::Vec::new(),
            overlay: alloc::vec::Vec::new(),
        }
    }
}

impl<T: Clone + PartialEq + 'static> ChartComposition<T> {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.background.is_empty() && self.overlay.is_empty()
    }

    #[must_use]
    pub fn with_background(mut self, builder: impl Fn(ChartProxy<T>) -> AnyView + 'static) -> Self {
        self.background.push(Rc::new(builder));
        self
    }

    #[must_use]
    pub fn with_overlay(mut self, builder: impl Fn(ChartProxy<T>) -> AnyView + 'static) -> Self {
        self.overlay.push(Rc::new(builder));
        self
    }

    #[must_use]
    pub fn apply<C: View + 'static>(
        &self,
        chart: C,
        chart_frame: Computed<ChartViewport>,
        plot_area_frame: Computed<ChartViewport>,
        selection: &SelectionBindings<T>,
    ) -> AnyView {
        if self.is_empty() {
            return AnyView::new(chart);
        }

        let proxy = ChartProxy::new(
            chart_frame,
            plot_area_frame,
            selection.focused_signal(),
            selection.selected_signal(),
        );
        let mut content = AnyView::new(chart);
        for background_builder in &self.background {
            content = AnyView::new(background(content, background_builder(proxy.clone())));
        }
        for overlay_builder in &self.overlay {
            content = AnyView::new(Overlay::new(content, overlay_builder(proxy.clone())));
        }
        content
    }
}

macro_rules! chart_composition_methods {
    ($datum:ty) => {
        /// Renders reactive content behind the chart using a [`ChartProxy`].
        #[must_use]
        pub fn chart_background<V, F>(mut self, build: F) -> Self
        where
            V: waterui_core::View + 'static,
            F: Fn(crate::composition::ChartProxy<$datum>) -> V + 'static,
        {
            self.composition = self
                .composition
                .with_background(move |proxy| waterui_core::AnyView::new(build(proxy)));
            self
        }

        /// Renders reactive content above the chart using a [`ChartProxy`].
        #[must_use]
        pub fn chart_overlay<V, F>(mut self, build: F) -> Self
        where
            V: waterui_core::View + 'static,
            F: Fn(crate::composition::ChartProxy<$datum>) -> V + 'static,
        {
            self.composition = self
                .composition
                .with_overlay(move |proxy| waterui_core::AnyView::new(build(proxy)));
            self
        }

        /// Displays a tooltip for the currently focused datum.
        #[must_use]
        pub fn focused_tooltip<V, F>(mut self, build: F) -> Self
        where
            V: waterui_core::View + 'static,
            F: Fn(crate::HitResult<$datum>) -> V + 'static,
        {
            let build = alloc::rc::Rc::new(move |hit| waterui_core::AnyView::new(build(hit)));
            self.composition = self.composition.with_overlay(move |proxy| {
                let build = alloc::rc::Rc::clone(&build);
                waterui_core::AnyView::new(crate::tooltip::chart_tooltip_overlay(
                    proxy.focused(),
                    proxy.chart_frame(),
                    move |hit| build(hit),
                ))
            });
            self
        }

        /// Displays a tooltip for the currently selected datum.
        #[must_use]
        pub fn selected_tooltip<V, F>(mut self, build: F) -> Self
        where
            V: waterui_core::View + 'static,
            F: Fn(crate::HitResult<$datum>) -> V + 'static,
        {
            let build = alloc::rc::Rc::new(move |hit| waterui_core::AnyView::new(build(hit)));
            self.composition = self.composition.with_overlay(move |proxy| {
                let build = alloc::rc::Rc::clone(&build);
                waterui_core::AnyView::new(crate::tooltip::chart_tooltip_overlay(
                    proxy.selected(),
                    proxy.chart_frame(),
                    move |hit| build(hit),
                ))
            });
            self
        }
    };
}

pub(crate) use chart_composition_methods;
