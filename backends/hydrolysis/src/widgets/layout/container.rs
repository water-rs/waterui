use crate::renderer::lazy::{LazyStackAxisConfig, lazy_stack_axis_config};
use crate::renderer::{
    HydroNativeView, HydroState, estimate_layout_intrinsic, measure_layout_dimensions,
    measure_transient_view_with_proposal, normalize_layout_view,
};
use nami::Signal;
use std::rc::Rc;
use waterui::views::Views;
use waterui_core::layout::{ProposalSize, Size as LayoutSize};
use waterui_core::views::AnyViews;
use waterui_core::{AnyView, Environment, Native};
use waterui_layout::container::{FixedContainer, LazyContainer};

/// Materializes every child view of a non-virtualized lazy collection in order.
/// Used for measurement of `AbsoluteLayout`/`ZStackLayout` collections, which
/// (unlike scroll-virtualized stacks) lay out their whole membership.
fn materialize_all(children: &AnyViews<AnyView>, env: &Environment) -> Vec<AnyView> {
    let count = children.len().snapshot();
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
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        let (layout, children) = view.as_inner().as_parts();
        estimate_layout_intrinsic(layout, children.iter(), state, env, theme)
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> waterui_core::layout::ViewDimensions {
        let (layout, children) = view.as_inner().as_parts();
        measure_layout_dimensions(layout, children.iter(), proposal, state, env, theme)
    }
}

/// Sizes a virtualized lazy stack from the measurement its first item gives
/// under the offered cross-axis extent — the same sample the retained
/// [`LazyStackNode`](crate::renderer::tree::LazyStackNode) reads in
/// `measure_item`. `cross` is the axis-negotiated extent on the stack's cross
/// axis (`None` when the caller left it open); the item is measured with the
/// main axis unspecified either way, since a virtualized stack lays items out
/// at their intrinsic main extent. `main` is the offered main-axis extent:
/// a finite offer caps the reported extent (the stack virtualizes — it fits
/// by showing fewer items), while an open axis reads the full extent.
fn lazy_stack_sample_size(
    state: &mut HydroState,
    view: &Native<LazyContainer>,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
    main: Option<f32>,
    cross: Option<f32>,
) -> LayoutSize {
    let (layout, children) = view.as_inner().as_parts();
    let child_count = children.len().snapshot();
    if child_count == 0 {
        return LayoutSize::zero();
    }
    let Some(axis) = lazy_stack_axis_config(layout, view.as_inner().direction()) else {
        // Non-virtualized collection (AbsoluteLayout/ZStackLayout overlay):
        // measure like a FixedContainer over its whole materialized membership.
        let views = materialize_all(children, env);
        return estimate_layout_intrinsic(layout, views.iter(), state, env, theme);
    };
    let item_proposal = match &axis {
        LazyStackAxisConfig::Vertical { .. } => ProposalSize::new(cross, None),
        LazyStackAxisConfig::Horizontal { .. } => ProposalSize::new(None, cross),
    };
    let sample = children
        .get_view(0)
        .map(|view| normalize_layout_view(view, env))
        .map(|view| measure_transient_view_with_proposal(&view, item_proposal, state, env, theme))
        .unwrap_or_else(|| panic!("LazyContainer failed to materialize child at index 0"));
    let count = child_count as f64;
    // The sampled extent is the membership's intrinsic ideal; a finite
    // main-axis offer caps it (the stack virtualizes onto the offered
    // viewport) while an open axis reads the full extent — but never below
    // the items' summed minima: content that cannot shrink keeps its
    // extent, the eager stack's `minima_overflow` answer.
    let finite_main = main.filter(|offer| offer.is_finite());
    let min_sample = finite_main.map(|_| {
        let min_proposal = match &axis {
            LazyStackAxisConfig::Vertical { .. } => ProposalSize::new(cross, Some(0.0)),
            LazyStackAxisConfig::Horizontal { .. } => ProposalSize::new(Some(0.0), cross),
        };
        children
            .get_view(0)
            .map(|view| normalize_layout_view(view, env))
            .map(|view| {
                measure_transient_view_with_proposal(&view, min_proposal, state, env, theme)
            })
            .unwrap_or_else(|| panic!("LazyContainer failed to materialize child at index 0"))
    });
    let cap = |extent: f64, floor: f64| match finite_main {
        Some(offer) => extent.min(f64::from(offer)).max(floor),
        _ => extent,
    };
    match axis {
        LazyStackAxisConfig::Vertical { spacing, .. } => {
            let gaps = f64::from(spacing.snapshot()) * (count - 1.0).max(0.0);
            let floor = min_sample.map_or(0.0, |min| f64::from(min.height) * count + gaps);
            let width = f64::from(sample.width);
            let height = cap(f64::from(sample.height) * count + gaps, floor);
            LayoutSize::new(width as f32, height as f32)
        }
        LazyStackAxisConfig::Horizontal { spacing, .. } => {
            let gaps = f64::from(spacing.snapshot()) * (count - 1.0).max(0.0);
            let floor = min_sample.map_or(0.0, |min| f64::from(min.width) * count + gaps);
            let width = cap(f64::from(sample.width) * count + gaps, floor);
            let height = f64::from(sample.height);
            LayoutSize::new(width as f32, height as f32)
        }
    }
}

impl HydroNativeView for Native<LazyContainer> {
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        lazy_stack_sample_size(state, view, env, theme, None, None)
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> waterui_core::layout::ViewDimensions {
        let (layout, children) = view.as_inner().as_parts();
        match lazy_stack_axis_config(layout, view.as_inner().direction()) {
            Some(LazyStackAxisConfig::Vertical { .. }) => {
                // The cross answer is what the first row measures under the
                // offered width — an intrinsic measure would report the row's
                // unwrapped natural width, which exceeds the proposal whenever
                // the content is wider than it.
                waterui_core::layout::ViewDimensions::new(lazy_stack_sample_size(
                    state,
                    view,
                    env,
                    theme,
                    proposal.height,
                    proposal.width,
                ))
            }
            Some(LazyStackAxisConfig::Horizontal { .. }) => {
                waterui_core::layout::ViewDimensions::new(lazy_stack_sample_size(
                    state,
                    view,
                    env,
                    theme,
                    proposal.width,
                    proposal.height,
                ))
            }
            // Non-virtualized collection: proposal-aware sizing via the layout,
            // so a stretch-both overlay (AbsoluteLayout) reports the offered
            // window size.
            None => {
                let views = materialize_all(children, env);
                measure_layout_dimensions(layout, views.iter(), proposal, state, env, theme)
            }
        }
    }
}
