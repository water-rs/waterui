use crate::renderer::lazy::{LazyStackAxisConfig, lazy_stack_axis_config};
use crate::renderer::{
    HydroNativeView, HydroState, estimate_layout_intrinsic, measure_layout_dimensions,
    measure_view_intrinsic, normalize_layout_view,
};
use nami::Signal;
use waterui::views::Views;
use waterui_core::layout::{ProposalSize, Size as LayoutSize};
use waterui_core::views::AnyViews;
use waterui_core::{AnyView, Environment, Native};
use waterui_layout::container::{FixedContainer, LazyContainer};

/// Materializes every child view of a non-virtualized lazy collection in order.
/// Used for measurement of `AbsoluteLayout`/`ZStackLayout` collections, which
/// (unlike scroll-virtualized stacks) lay out their whole membership.
fn materialize_all(children: &AnyViews<AnyView>, env: &Environment) -> Vec<AnyView> {
    let count = children.len().get();
    let mut views = Vec::with_capacity(count);
    for index in 0..count {
        let view = children.get_view(index).unwrap_or_else(|| {
            panic!("LazyContainer failed to materialize child at index {index}")
        });
        views.push(normalize_layout_view(view, env));
    }
    views
}

impl HydroNativeView for Native<FixedContainer> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let (layout, children) = view.as_inner().as_parts();
        estimate_layout_intrinsic(layout, children.iter(), state, env)
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> waterui_core::layout::ViewDimensions {
        let (layout, children) = view.as_inner().as_parts();
        measure_layout_dimensions(layout, children.iter(), proposal, state, env)
    }
}

impl HydroNativeView for Native<LazyContainer> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let (layout, children) = view.as_inner().as_parts();
        let child_count = children.len().get();
        if child_count == 0 {
            return LayoutSize::zero();
        }
        let Some(axis) = lazy_stack_axis_config(layout) else {
            // Non-virtualized collection (AbsoluteLayout/ZStackLayout overlay):
            // measure like a FixedContainer over its whole materialized membership.
            let views = materialize_all(children, env);
            return estimate_layout_intrinsic(layout, views.iter(), state, env);
        };
        let sample = children
            .get_view(0)
            .map(|view| normalize_layout_view(view, env))
            .map(|view| measure_view_intrinsic(&view, state, env))
            .unwrap_or_else(|| panic!("LazyContainer failed to materialize child at index 0"));
        let count = child_count as f64;
        match axis {
            LazyStackAxisConfig::Vertical { spacing, .. } => {
                let width = f64::from(sample.width);
                let height = f64::from(sample.height) * count
                    + f64::from(spacing.get()) * (count - 1.0).max(0.0);
                LayoutSize::new(width as f32, height as f32)
            }
            LazyStackAxisConfig::Horizontal { spacing, .. } => {
                let width = f64::from(sample.width) * count
                    + f64::from(spacing.get()) * (count - 1.0).max(0.0);
                let height = f64::from(sample.height);
                LayoutSize::new(width as f32, height as f32)
            }
        }
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> waterui_core::layout::ViewDimensions {
        let (layout, children) = view.as_inner().as_parts();
        if lazy_stack_axis_config(layout).is_some() {
            // Virtualized stacks size from the intrinsic estimate (proposal is
            // applied per-row at render time, not to the whole stack here).
            return waterui_core::layout::ViewDimensions::new(Self::intrinsic(state, view, env));
        }
        // Non-virtualized collection: proposal-aware sizing via the layout, so a
        // stretch-both overlay (AbsoluteLayout) reports the offered window size.
        let views = materialize_all(children, env);
        measure_layout_dimensions(layout, views.iter(), proposal, state, env)
    }
}
