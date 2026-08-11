use crate::engine::Brush;
#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
#[cfg(feature = "accessibility")]
use crate::renderer::accessibility_activation_point;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, RetainedSubview,
    WidgetRenderContext, measure_navigation_view_intrinsic,
    measure_owned_navigation_view_intrinsic, measure_view_intrinsic, navigation_back_button_rect,
    navigation_base_bar_height_for_display_mode, normalize_layout_view, resolved_color_to_peniko,
    split_compact_threshold, transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use nami::{Computed, Signal};
use std::cell::RefCell;
use std::rc::Rc;
use waterui::navigation::split::NavigationSplitDetailBuilder;
use waterui::navigation::{
    AnyNavigationTransition, Bar, NavigationDestinationState,
    NavigationSearch, NavigationSplitLayout, NavigationStack, NavigationTitleDisplayMode,
    NavigationToolbarPlacement, NavigationTransitionDirection, NavigationView,
    RetainedNavigationTransition, navigation_back_label, resolve_navigation_root,
};
use waterui::theme::color::{Background, Surface};
use waterui_controls::text_field::TextField;
use waterui_core::id::Id;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};
use waterui_graphics::color::{Color, ResolvedColor};

use crate::widgets::widget_theme;

#[derive(Clone, Copy)]
struct NavigationLeadingReserve(f64);

fn navigation_leading_reserve(env: &Environment) -> f64 {
    env.get::<NavigationLeadingReserve>()
        .map_or(0.0, |reserve| reserve.0)
}

fn back_button_title_reserve(env: &Environment) -> f64 {
    let metrics = widget_theme(env).navigation_metrics();
    metrics.back_button_size + metrics.title_leading_inset
}

/// The retained render state of a `NavigationView`. The bar's semantic title,
/// subtitle, toolbar items and screen content are move-only `AnyView`s, so the persistent
/// `Widget` node holds each as a [`RetainedSubview`] built once and re-flushed at
/// its rect every frame (so reactive descendants inside them stay live). The bar's
/// reactive appearance signals (`color`/`hidden`) are kept and read through
/// `read_signal`; the static `display_mode` and the `search` model (cloneable, used
/// to build a fresh `TextField` each frame) are kept by value.
pub(crate) struct NavigationViewRenderState {
    title: RetainedSubview,
    subtitle: RetainedSubview,
    principal: Vec<RetainedSubview>,
    leading: Vec<RetainedSubview>,
    trailing: Vec<RetainedSubview>,
    bottom: Vec<RetainedSubview>,
    content: RetainedSubview,
    search: Option<NavigationSearch>,
    /// The search field as a retained node sub-view (the `TextField`'s reactive text
    /// binding stays live through the node's own re-flush). `Some` exactly when
    /// `search` is present.
    search_field: Option<RetainedSubview>,
    color: Computed<ResolvedColor>,
    hidden: Computed<bool>,
    display_mode: NavigationTitleDisplayMode,
    subtitle_present: bool,
}

impl NavigationViewRenderState {
    pub(crate) fn from_view(navigation: NavigationView, env: &Environment) -> Self {
        let NavigationView { bar, content, .. } = navigation;
        let Bar {
            title,
            subtitle,
            toolbar,
            search,
            color,
            resolved_color,
            hidden,
            display_mode,
        } = bar;
        let color = color.map_or_else(
            || Color::new(Surface).resolve(env),
            |_| {
                resolved_color
                    .expect("NavigationView explicit bar color was not resolved before rendering")
            },
        );
        let subtitle_present = !subtitle.is::<()>();
        let mut principal = Vec::new();
        let mut leading = Vec::new();
        let mut trailing = Vec::new();
        let mut bottom = Vec::new();
        for item in toolbar.items {
            let retained = RetainedSubview::new(item.content);
            match item.placement {
                NavigationToolbarPlacement::Principal => principal.push(retained),
                NavigationToolbarPlacement::Cancellation
                | NavigationToolbarPlacement::TopBarLeading => leading.push(retained),
                NavigationToolbarPlacement::BottomBar | NavigationToolbarPlacement::Status => {
                    bottom.push(retained);
                }
                NavigationToolbarPlacement::PrimaryAction
                | NavigationToolbarPlacement::SecondaryAction
                | NavigationToolbarPlacement::Confirmation
                | NavigationToolbarPlacement::TopBarTrailing => trailing.push(retained),
            }
        }
        let search_field = search.as_ref().map(|search| {
            RetainedSubview::new(AnyView::new(
                TextField::new(&search.text).prompt(search.prompt.clone()),
            ))
        });
        Self {
            title: RetainedSubview::new(title),
            subtitle: RetainedSubview::new(subtitle),
            principal,
            leading,
            trailing,
            bottom,
            content: RetainedSubview::new(content),
            search,
            search_field,
            color,
            hidden,
            display_mode,
            subtitle_present,
        }
    }

    /// Eagerly build the bar/content sub-views (the measure path has no renderer to
    /// build on), mirroring the dispatch path's normalization.
    pub(crate) fn prebuild(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        self.title.ensure_built(renderer, env);
        if self.subtitle_present {
            self.subtitle.ensure_built(renderer, env);
        }
        for item in self
            .principal
            .iter_mut()
            .chain(&mut self.leading)
            .chain(&mut self.trailing)
            .chain(&mut self.bottom)
        {
            item.ensure_built(renderer, env);
        }
        if let Some(search_field) = &mut self.search_field {
            search_field.ensure_built(renderer, env);
        }
        self.content.ensure_built(renderer, env);
    }
}

impl HydroNativeView for Native<NavigationView> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_navigation_view_intrinsic(view.as_inner(), state, env)
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        if let (Some(width), Some(height)) = (proposal.width, proposal.height) {
            return ViewDimensions::new(LayoutSize::new(width, height));
        }
        ViewDimensions::new(Self::intrinsic(state, view, env))
    }
}

/// Emits a navigation view's bar/title accessibility nodes. Shared by the dispatch
/// path and the retained `Widget`-node path. `default_title_label` is the spoken
/// label resolved from the bar title view (extracted at build time on the node
/// path, where the title view is owned by a [`RetainedSubview`]).
pub(crate) fn navigation_view_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    hidden: &Computed<bool>,
    display_mode: NavigationTitleDisplayMode,
    default_title_label: Option<String>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        if renderer.read_signal(hidden) {
            return;
        }
        let metrics = widget_theme(env).navigation_metrics();
        let bar_height = navigation_base_bar_height_for_display_mode(display_mode, env);
        let bar_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x1,
            (ctx.bounds.y0 + bar_height).min(ctx.bounds.y1),
        );
        let mut bar_node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Navigation),
        );
        let bar_label = renderer.resolve_accessibility_label(env, None);
        if let Some(label) = bar_label {
            bar_node.set_label(label);
        }
        let title_height = if matches!(display_mode, NavigationTitleDisplayMode::Large) {
            metrics.large_title_height
        } else {
            metrics.inline_title_height
        };
        let title_y0 = if matches!(display_mode, NavigationTitleDisplayMode::Large) {
            bar_rect.y1 - metrics.large_title_bottom_inset - title_height
        } else {
            bar_rect.y0 + (bar_height - title_height) * 0.5
        };
        let title_leading = navigation_leading_reserve(env);
        let title_rect = vello::kurbo::Rect::new(
            if title_leading > 0.0 {
                bar_rect.x0 + metrics.horizontal_inset + title_leading
            } else {
                bar_rect.x0 + metrics.title_leading_inset
            },
            title_y0,
            bar_rect.x1 - metrics.title_trailing_inset,
            title_y0 + title_height,
        );
        if title_rect.width() > 0.0 && title_rect.height() > 0.0 {
            let mut title_node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Header),
            );
            let title_label = renderer.resolve_accessibility_label(env, default_title_label);
            if let Some(label) = title_label {
                title_node.set_label(label);
            }
            if let Some(title_node_id) = renderer.register_accessibility_child_node(
                title_node,
                transformed_rect(ctx.hit_transform, title_rect),
                env,
                None,
            ) {
                bar_node.push_child(title_node_id);
            }
        }
        let _ = renderer.register_accessibility_node(
            bar_node,
            transformed_rect(ctx.hit_transform, bar_rect),
            env,
            None,
        );
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, hidden, display_mode, default_title_label, env);
    }
}

/// Measures a retained navigation view leaf from its [`NavigationViewRenderState`].
/// Fills both axes when a concrete proposal is supplied (matching the dispatch-path
/// `dimensions`), otherwise falls back to the intrinsic size computed from the
/// prebuilt bar/content sub-views (mirroring `measure_navigation_view_intrinsic`).
pub(crate) fn measure_navigation_view_node(
    state: &NavigationViewRenderState,
    proposal: ProposalSize,
    hydro: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    if let (Some(width), Some(height)) = (proposal.width, proposal.height) {
        return ViewDimensions::new(LayoutSize::new(width, height));
    }
    let bar_hidden = state.hidden.get();
    let metrics = widget_theme(env).navigation_metrics();
    let bar_height = if bar_hidden {
        0.0
    } else {
        let base = navigation_base_bar_height_for_display_mode(state.display_mode, env);
        let search_extra = if state.search.is_some() {
            metrics.search_height + metrics.search_vertical_inset * 2.0
        } else {
            0.0
        };
        base + search_extra
    };
    let title_size = if bar_height > 0.0 && state.principal.is_empty() {
        let title = state.title.measure_built(hydro, env);
        let subtitle = if state.subtitle_present {
            state.subtitle.measure_built(hydro, env)
        } else {
            LayoutSize::zero()
        };
        LayoutSize::new(
            title.width.max(subtitle.width),
            title.height + subtitle.height,
        )
    } else if bar_height > 0.0 {
        measure_retained_toolbar_group(&state.principal, hydro, env)
    } else {
        LayoutSize::zero()
    };
    let leading_size = measure_retained_toolbar_group(&state.leading, hydro, env);
    let trailing_size = measure_retained_toolbar_group(&state.trailing, hydro, env);
    let bottom_size = measure_retained_toolbar_group(&state.bottom, hydro, env);
    // Measure the retained field itself: a throwaway TextField would allocate on
    // every measure and shape its text in a cache the rendered field never sees.
    let search_size = state
        .search_field
        .as_ref()
        .map_or_else(LayoutSize::zero, |field| field.measure_built(hydro, env));
    let content_size = state.content.measure_built(hydro, env);
    let width = f64::from(content_size.width)
        .max(
            f64::from(leading_size.width)
                + f64::from(title_size.width)
                + f64::from(trailing_size.width)
                + metrics.horizontal_inset * 2.0
                + metrics.item_spacing * 2.0,
        )
        .max(f64::from(search_size.width) + metrics.horizontal_inset * 2.0)
        .max(f64::from(bottom_size.width) + metrics.horizontal_inset * 2.0);
    let bottom_height = if state.bottom.is_empty() {
        0.0
    } else {
        metrics.inline_bar_height
    };
    let height = f64::from(content_size.height) + bar_height + bottom_height;
    ViewDimensions::new(LayoutSize::new(width as f32, height as f32))
}

fn measure_retained_toolbar_group(
    group: &[RetainedSubview],
    hydro: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let metrics = widget_theme(env).navigation_metrics();
    let mut width = 0.0_f64;
    let mut height = 0.0_f64;
    for (index, item) in group.iter().enumerate() {
        let size = item.measure_built(hydro, env);
        if index > 0 {
            width += metrics.item_spacing;
        }
        width += f64::from(size.width);
        height = height.max(f64::from(size.height));
    }
    LayoutSize::new(width as f32, height as f32)
}

/// Renders a retained navigation view leaf every flush: emits the bar/title a11y
/// (unless hidden) then the bar chrome + content, reading the bar's live signals.
pub(crate) fn render_navigation_view_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<NavigationViewRenderState>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        let (hidden_signal, display_mode, default_title_label) = {
            let state = state.borrow();
            (
                state.hidden.clone(),
                state.display_mode,
                state.title.default_a11y_label(),
            )
        };
        navigation_view_accessibility(
            ctx.renderer_mut(),
            render_ctx,
            &hidden_signal,
            display_mode,
            default_title_label,
            env,
        );
    }
    render_navigation_view_parts(ctx, state, env);
}

pub(crate) fn render_navigation_view_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<NavigationViewRenderState>>,
    env: &Environment,
) {
    let (hidden_signal, color_signal, display_mode, search, has_bottom) = {
        let state = state.borrow();
        (
            state.hidden.clone(),
            state.color.clone(),
            state.display_mode,
            state.search.clone(),
            !state.bottom.is_empty(),
        )
    };
    let top_bar_height = if ctx.renderer_mut().read_signal(&hidden_signal) {
        0.0
    } else {
        let metrics = widget_theme(env).navigation_metrics();
        let base = navigation_base_bar_height_for_display_mode(display_mode, env);
        let search_extra = if search.is_some() {
            metrics.search_height + metrics.search_vertical_inset * 2.0
        } else {
            0.0
        };
        base + search_extra
    };
    let metrics = widget_theme(env).navigation_metrics();
    let bottom_bar_height = if has_bottom {
        metrics.inline_bar_height.min(ctx.bounds.height())
    } else {
        0.0
    };

    if top_bar_height > 0.0 {
        let base_bar_height = navigation_base_bar_height_for_display_mode(display_mode, env);
        let bar_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x1,
            (ctx.bounds.y0 + top_bar_height).min(ctx.bounds.y1),
        );
        let bar_color = resolved_color_to_peniko(ctx.renderer_mut().read_signal(&color_signal));
        {
            let theme = widget_theme(env);
            let mut draw = ctx.draw_context();
            theme.draw_navigation_bar(&mut draw, bar_rect, &Brush::from(bar_color));
            let separator = vello::kurbo::Rect::new(
                bar_rect.x0,
                (bar_rect.y1 - 1.0).max(bar_rect.y0),
                bar_rect.x1,
                bar_rect.y1,
            );
            theme.draw_navigation_bar_separator(&mut draw, separator);
        }

        let (leading_size, trailing_size) = {
            let mut state = state.borrow_mut();
            let leading =
                measure_toolbar_group_intrinsic(&mut state.leading, ctx.renderer_mut(), env);
            let trailing =
                measure_toolbar_group_intrinsic(&mut state.trailing, ctx.renderer_mut(), env);
            (leading, trailing)
        };
        let leading_width = f64::from(leading_size.width);
        let trailing_width = f64::from(trailing_size.width);
        let leading_rect = vello::kurbo::Rect::new(
            bar_rect.x0 + metrics.horizontal_inset,
            bar_rect.y0,
            (bar_rect.x0 + metrics.horizontal_inset + leading_width).min(bar_rect.x1),
            (bar_rect.y0 + base_bar_height).min(bar_rect.y1),
        );
        let trailing_rect = vello::kurbo::Rect::new(
            (bar_rect.x1 - metrics.horizontal_inset - trailing_width).max(bar_rect.x0),
            bar_rect.y0,
            bar_rect.x1 - metrics.horizontal_inset,
            (bar_rect.y0 + base_bar_height).min(bar_rect.y1),
        );
        {
            let mut state = state.borrow_mut();
            flush_toolbar_group(
                ctx,
                &mut state.leading,
                env,
                leading_rect,
                ToolbarAlignment::Leading,
            );
            flush_toolbar_group(
                ctx,
                &mut state.trailing,
                env,
                trailing_rect,
                ToolbarAlignment::Trailing,
            );
        }

        let title_height = if matches!(display_mode, NavigationTitleDisplayMode::Large) {
            metrics.large_title_height
        } else {
            metrics.inline_title_height
        };
        let title_y0 = if matches!(display_mode, NavigationTitleDisplayMode::Large) {
            bar_rect.y0 + base_bar_height - metrics.large_title_bottom_inset - title_height
        } else {
            bar_rect.y0 + (base_bar_height - title_height) * 0.5
        };
        let effective_leading_width = leading_width.max(navigation_leading_reserve(env));
        let title_x0 = if effective_leading_width > 0.0 {
            bar_rect.x0 + metrics.horizontal_inset + effective_leading_width + metrics.item_spacing
        } else {
            bar_rect.x0 + metrics.title_leading_inset
        };
        let title_x1 = if trailing_width > 0.0 {
            bar_rect.x1 - metrics.horizontal_inset - trailing_width - metrics.item_spacing
        } else {
            bar_rect.x1 - metrics.title_trailing_inset
        };
        let title_rect = vello::kurbo::Rect::new(
            title_x0.min(bar_rect.x1),
            title_y0,
            title_x1.max(bar_rect.x0),
            title_y0 + title_height,
        );
        if title_rect.width() > 0.0 && title_rect.height() > 0.0 {
            // The bar title's a11y is emitted by `navigation_view_accessibility`, so
            // suppress the sub-view's own a11y (matching the dispatch path's
            // `dispatch_in_rect_without_accessibility`).
            #[cfg(feature = "accessibility")]
            ctx.renderer_mut().push_accessibility_suppression();
            {
                let mut state = state.borrow_mut();
                if state.principal.is_empty() {
                    flush_title_and_subtitle(ctx, &mut state, env, title_rect);
                } else {
                    flush_toolbar_group(
                        ctx,
                        &mut state.principal,
                        env,
                        title_rect,
                        ToolbarAlignment::Center,
                    );
                }
            }
            #[cfg(feature = "accessibility")]
            ctx.renderer_mut().pop_accessibility_suppression();
        }

        if search.is_some() {
            let search_rect = vello::kurbo::Rect::new(
                bar_rect.x0 + metrics.horizontal_inset,
                bar_rect.y0 + base_bar_height + metrics.search_vertical_inset,
                bar_rect.x1 - metrics.horizontal_inset,
                (bar_rect.y0
                    + base_bar_height
                    + metrics.search_vertical_inset
                    + metrics.search_height)
                    .min(bar_rect.y1 - metrics.search_vertical_inset),
            );
            if search_rect.width() > 0.0 && search_rect.height() > 0.0 {
                let render_ctx = ctx.render_context();
                if let Some(field) = state.borrow_mut().search_field.as_mut() {
                    field.flush_in_rect(ctx.renderer_mut(), render_ctx, env, search_rect);
                }
            }
        }
    }

    let content_rect = vello::kurbo::Rect::new(
        ctx.bounds.x0,
        (ctx.bounds.y0 + top_bar_height).min(ctx.bounds.y1),
        ctx.bounds.x1,
        (ctx.bounds.y1 - bottom_bar_height).max(ctx.bounds.y0),
    );
    if content_rect.width() > 0.0 && content_rect.height() > 0.0 {
        let render_ctx = ctx.render_context();
        state
            .borrow_mut()
            .content
            .flush_in_rect(ctx.renderer_mut(), render_ctx, env, content_rect);
    }

    if bottom_bar_height > 0.0 {
        let bottom_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            (ctx.bounds.y1 - bottom_bar_height).max(ctx.bounds.y0),
            ctx.bounds.x1,
            ctx.bounds.y1,
        );
        let bar_color = resolved_color_to_peniko(ctx.renderer_mut().read_signal(&color_signal));
        {
            let theme = widget_theme(env);
            let mut draw = ctx.draw_context();
            theme.draw_navigation_bar(&mut draw, bottom_rect, &Brush::from(bar_color));
        }
        flush_toolbar_group(
            ctx,
            &mut state.borrow_mut().bottom,
            env,
            bottom_rect,
            ToolbarAlignment::Center,
        );
    }
}

#[derive(Clone, Copy)]
enum ToolbarAlignment {
    Leading,
    Center,
    Trailing,
}

fn measure_toolbar_group_intrinsic(
    group: &mut [RetainedSubview],
    renderer: &mut HydrolysisRenderer,
    env: &Environment,
) -> LayoutSize {
    let metrics = widget_theme(env).navigation_metrics();
    let mut width = 0.0_f64;
    let mut height = 0.0_f64;
    for (index, item) in group.iter_mut().enumerate() {
        let size = item.measure_intrinsic(renderer, env);
        if index > 0 {
            width += metrics.item_spacing;
        }
        width += f64::from(size.width);
        height = height.max(f64::from(size.height));
    }
    LayoutSize::new(width as f32, height as f32)
}

fn flush_toolbar_group(
    ctx: &mut WidgetRenderContext<'_>,
    group: &mut [RetainedSubview],
    env: &Environment,
    bounds: vello::kurbo::Rect,
    alignment: ToolbarAlignment,
) {
    if group.is_empty() || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return;
    }
    let metrics = widget_theme(env).navigation_metrics();
    let sizes: Vec<LayoutSize> = group
        .iter_mut()
        .map(|item| item.measure_intrinsic(ctx.renderer_mut(), env))
        .collect();
    let total_width = sizes.iter().map(|size| f64::from(size.width)).sum::<f64>()
        + metrics.item_spacing * sizes.len().saturating_sub(1) as f64;
    let mut x = match alignment {
        ToolbarAlignment::Leading => bounds.x0,
        ToolbarAlignment::Center => bounds.x0 + (bounds.width() - total_width) * 0.5,
        ToolbarAlignment::Trailing => bounds.x1 - total_width,
    }
    .max(bounds.x0);
    for (item, size) in group.iter_mut().zip(sizes) {
        let width = f64::from(size.width).min((bounds.x1 - x).max(0.0));
        let height = f64::from(size.height).min(bounds.height());
        let y = bounds.y0 + (bounds.height() - height) * 0.5;
        let rect = vello::kurbo::Rect::new(x, y, x + width, y + height);
        if rect.width() > 0.0 && rect.height() > 0.0 {
            let render_ctx = ctx.render_context();
            item.flush_in_rect(ctx.renderer_mut(), render_ctx, env, rect);
        }
        x += width + metrics.item_spacing;
    }
}

fn flush_title_and_subtitle(
    ctx: &mut WidgetRenderContext<'_>,
    state: &mut NavigationViewRenderState,
    env: &Environment,
    bounds: vello::kurbo::Rect,
) {
    let title_size = state.title.measure_intrinsic(ctx.renderer_mut(), env);
    let subtitle_size = if state.subtitle_present {
        state.subtitle.measure_intrinsic(ctx.renderer_mut(), env)
    } else {
        LayoutSize::zero()
    };
    let total_height =
        (f64::from(title_size.height) + f64::from(subtitle_size.height)).min(bounds.height());
    let mut y = bounds.y0 + (bounds.height() - total_height) * 0.5;
    let title_height = f64::from(title_size.height).min((bounds.y1 - y).max(0.0));
    let title_rect = vello::kurbo::Rect::new(bounds.x0, y, bounds.x1, y + title_height);
    if title_rect.height() > 0.0 {
        let render_ctx = ctx.render_context();
        state
            .title
            .flush_in_rect(ctx.renderer_mut(), render_ctx, env, title_rect);
    }
    y += title_height;
    if state.subtitle_present {
        let subtitle_height = f64::from(subtitle_size.height).min((bounds.y1 - y).max(0.0));
        let subtitle_rect = vello::kurbo::Rect::new(bounds.x0, y, bounds.x1, y + subtitle_height);
        if subtitle_rect.height() > 0.0 {
            let render_ctx = ctx.render_context();
            state
                .subtitle
                .flush_in_rect(ctx.renderer_mut(), render_ctx, env, subtitle_rect);
        }
    }
}

/// Retained adaptive split state for both two- and three-column configurations.
pub(crate) struct NavigationSplitRenderState {
    primary_selection: nami::Binding<Option<Id>>,
    content_builder: Option<NavigationSplitDetailBuilder>,
    secondary_selection: Option<nami::Binding<Option<Id>>>,
    detail_builder: NavigationSplitDetailBuilder,
    visibility: Computed<waterui::navigation::NavigationSplitColumnVisibility>,
    column_width: waterui::navigation::ColumnWidth,
    style: waterui::navigation::NativeNavigationSplitStyle,
    primary: RetainedSubview,
    placeholder: RetainedSubview,
    content: Option<(Id, bool, RetainedSubview)>,
    detail: Option<(Id, bool, RetainedSubview)>,
}

impl NavigationSplitRenderState {
    pub(crate) fn from_layout(split: NavigationSplitLayout) -> Self {
        let (
            primary,
            placeholder,
            primary_selection,
            content_builder,
            secondary_selection,
            detail_builder,
            visibility,
            column_width,
            style,
        ) = split.into_parts();
        Self {
            primary_selection,
            content_builder,
            secondary_selection,
            detail_builder,
            visibility,
            column_width,
            style,
            primary: RetainedSubview::new(primary.build()),
            placeholder: RetainedSubview::new(placeholder.build()),
            content: None,
            detail: None,
        }
    }

    pub(crate) fn prebuild(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        self.primary.ensure_built(renderer, env);
        self.placeholder.ensure_built(renderer, env);
    }

    fn ensure_content(
        &mut self,
        id: Id,
        compact: bool,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
    ) {
        let needs_rebuild = self
            .content
            .as_ref()
            .is_none_or(|(cached_id, cached_compact, _)| {
                *cached_id != id || *cached_compact != compact
            });
        if needs_rebuild {
            let builder = self
                .content_builder
                .as_ref()
                .expect("three-column split must provide a content builder");
            let mut subview = RetainedSubview::new(AnyView::new(builder.build(id)));
            subview.ensure_built(renderer, env);
            self.content = Some((id, compact, subview));
        }
    }

    fn ensure_detail(
        &mut self,
        id: Id,
        compact: bool,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
    ) {
        let needs_rebuild = self
            .detail
            .as_ref()
            .is_none_or(|(cached_id, cached_compact, _)| {
                *cached_id != id || *cached_compact != compact
            });
        if needs_rebuild {
            let mut subview = RetainedSubview::new(AnyView::new(self.detail_builder.build(id)));
            subview.ensure_built(renderer, env);
            self.detail = Some((id, compact, subview));
        }
    }

    const fn is_three_column(&self) -> bool {
        self.content_builder.is_some()
    }
}

impl HydroNativeView for Native<NavigationSplitLayout> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_navigation_split_layout(view.as_inner(), state, env)
    }
}

fn measure_navigation_split_layout(
    split: &NavigationSplitLayout,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let primary_view = normalize_layout_view(split.primary_builder().build(), env);
    let primary = measure_view_intrinsic(&primary_view, state, env);
    let primary_selection = split.primary_selection().get();
    let content = split.content_builder().and_then(|builder| {
        primary_selection.map(|selected| {
            measure_owned_navigation_view_intrinsic(builder.build(selected), state, env)
        })
    });
    let detail_selection = split
        .secondary_selection()
        .map_or(primary_selection, Signal::get);
    let detail = match detail_selection {
        None => {
            let placeholder = normalize_layout_view(split.placeholder_builder().build(), env);
            measure_view_intrinsic(&placeholder, state, env)
        }
        Some(selected) => measure_owned_navigation_view_intrinsic(
            split.detail_builder().build(selected),
            state,
            env,
        ),
    };
    let column_width =
        resolved_split_column_width(split.sidebar_width_constraints(), split.native_style());
    let content_width = content.map_or(0.0, |_| column_width);
    let height = content.map_or(primary.height.max(detail.height), |content| {
        primary.height.max(content.height).max(detail.height)
    });
    LayoutSize::new(
        (column_width + content_width + f64::from(detail.width)) as f32,
        height,
    )
}

pub(crate) fn measure_navigation_split_node(
    split: &NavigationSplitRenderState,
    _proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    let primary = split.primary.measure_built(state, env);
    let primary_selection = split.primary_selection.get();
    let content = split.content_builder.as_ref().and_then(|builder| {
        primary_selection.map(|selected| {
            measure_owned_navigation_view_intrinsic(builder.build(selected), state, env)
        })
    });
    let detail_selection = split
        .secondary_selection
        .as_ref()
        .map_or(primary_selection, Signal::get);
    let detail = match detail_selection {
        None => split.placeholder.measure_built(state, env),
        Some(selected) => measure_owned_navigation_view_intrinsic(
            split.detail_builder.build(selected),
            state,
            env,
        ),
    };
    let column_width = resolved_split_column_width(split.column_width, split.style);
    let content_width = content.map_or(0.0, |_| column_width);
    let height = content.map_or(primary.height.max(detail.height), |content| {
        primary.height.max(content.height).max(detail.height)
    });
    ViewDimensions::new(LayoutSize::new(
        (column_width + content_width + f64::from(detail.width)) as f32,
        height,
    ))
}

pub(crate) fn render_navigation_split_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<NavigationSplitRenderState>>,
    env: &Environment,
) {
    render_navigation_split_parts(ctx, state, env);
}

pub(crate) fn render_navigation_split_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<NavigationSplitRenderState>>,
    env: &Environment,
) {
    let bounds = ctx.bounds;
    let (
        primary_selection,
        secondary_selection,
        visibility_signal,
        column_width,
        style,
        three_column,
    ) = {
        let state = state.borrow();
        (
            state.primary_selection.clone(),
            state.secondary_selection.clone(),
            state.visibility.clone(),
            state.column_width,
            state.style,
            state.is_three_column(),
        )
    };
    let primary = ctx.renderer_mut().read_signal(&primary_selection);
    let secondary = secondary_selection
        .as_ref()
        .and_then(|selection| ctx.renderer_mut().read_signal(selection));
    let visibility = ctx.renderer_mut().read_signal(&visibility_signal);
    let desired_column_width = resolved_split_column_width(column_width, style);
    let automatic_compact = bounds.width()
        < split_compact_threshold(desired_column_width * if three_column { 2.0 } else { 1.0 });

    if automatic_compact
        || matches!(
            visibility,
            waterui::navigation::NavigationSplitColumnVisibility::DetailOnly
        )
    {
        render_compact_split(
            ctx,
            state,
            env,
            CompactSplitSelection {
                primary,
                secondary,
                primary_binding: primary_selection,
                secondary_binding: secondary_selection,
                three_column,
            },
        );
        return;
    }

    let show_all = !three_column
        || matches!(
            visibility,
            waterui::navigation::NavigationSplitColumnVisibility::All
        )
        || (matches!(
            visibility,
            waterui::navigation::NavigationSplitColumnVisibility::Automatic
        ) && bounds.width() >= split_compact_threshold(desired_column_width * 2.0));
    let column_count = if three_column && show_all { 3.0 } else { 2.0 };
    let column_width = desired_column_width
        .clamp(f64::from(column_width.min()), f64::from(column_width.max()))
        .min(bounds.width() / column_count);

    let (primary_rect, content_rect, detail_rect) = if three_column && show_all {
        let primary_rect =
            vello::kurbo::Rect::new(bounds.x0, bounds.y0, bounds.x0 + column_width, bounds.y1);
        let content_rect = vello::kurbo::Rect::new(
            primary_rect.x1,
            bounds.y0,
            primary_rect.x1 + column_width,
            bounds.y1,
        );
        let detail_rect = vello::kurbo::Rect::new(content_rect.x1, bounds.y0, bounds.x1, bounds.y1);
        (Some(primary_rect), Some(content_rect), detail_rect)
    } else if three_column {
        let content_rect =
            vello::kurbo::Rect::new(bounds.x0, bounds.y0, bounds.x0 + column_width, bounds.y1);
        let detail_rect = vello::kurbo::Rect::new(content_rect.x1, bounds.y0, bounds.x1, bounds.y1);
        (None, Some(content_rect), detail_rect)
    } else {
        let primary_rect =
            vello::kurbo::Rect::new(bounds.x0, bounds.y0, bounds.x0 + column_width, bounds.y1);
        let detail_rect = vello::kurbo::Rect::new(primary_rect.x1, bounds.y0, bounds.x1, bounds.y1);
        (Some(primary_rect), None, detail_rect)
    };

    if let Some(primary_rect) = primary_rect {
        let render_ctx = ctx.render_context();
        state
            .borrow_mut()
            .primary
            .flush_in_rect(ctx.renderer_mut(), render_ctx, env, primary_rect);
    }
    if let Some(content_rect) = content_rect {
        render_split_content(ctx, state, env, primary, false, content_rect);
    }
    let detail_selection = if three_column { secondary } else { primary };
    render_split_detail(ctx, state, env, detail_selection, false, detail_rect);
}

fn resolved_split_column_width(
    width: waterui::navigation::ColumnWidth,
    style: waterui::navigation::NativeNavigationSplitStyle,
) -> f64 {
    match style {
        waterui::navigation::NativeNavigationSplitStyle::Automatic
        | waterui::navigation::NativeNavigationSplitStyle::Balanced => f64::from(width.ideal()),
        waterui::navigation::NativeNavigationSplitStyle::ProminentDetail => f64::from(width.min()),
    }
}

struct CompactSplitSelection {
    primary: Option<Id>,
    secondary: Option<Id>,
    primary_binding: nami::Binding<Option<Id>>,
    secondary_binding: Option<nami::Binding<Option<Id>>>,
    three_column: bool,
}

fn render_compact_split(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<NavigationSplitRenderState>>,
    env: &Environment,
    selection: CompactSplitSelection,
) {
    let bounds = ctx.bounds;
    let mut back_selection = None;
    let mut compact_env = env.clone();
    compact_env.insert(NavigationLeadingReserve(back_button_title_reserve(env)));
    if selection.three_column {
        if selection.secondary.is_some() {
            render_split_detail(ctx, state, &compact_env, selection.secondary, true, bounds);
            back_selection = selection.secondary_binding;
        } else if selection.primary.is_some() {
            render_split_content(ctx, state, &compact_env, selection.primary, true, bounds);
            back_selection = Some(selection.primary_binding);
        } else {
            let render_ctx = ctx.render_context();
            state
                .borrow_mut()
                .primary
                .flush_in_rect(ctx.renderer_mut(), render_ctx, env, bounds);
        }
    } else if selection.primary.is_some() {
        render_split_detail(ctx, state, &compact_env, selection.primary, true, bounds);
        back_selection = Some(selection.primary_binding);
    } else {
        let render_ctx = ctx.render_context();
        state
            .borrow_mut()
            .primary
            .flush_in_rect(ctx.renderer_mut(), render_ctx, env, bounds);
    }

    if let Some(selection) = back_selection {
        let back_rect = navigation_back_button_rect(bounds, widget_theme(env).navigation_metrics());
        {
            let theme = widget_theme(env);
            let mut draw = ctx.draw_context();
            theme.draw_navigation_back_button(&mut draw, back_rect);
        }
        let hit_transform = ctx.hit_transform;
        ctx.renderer_mut().register_pointer_target(
            transformed_rect(hit_transform, back_rect),
            move |_renderer, _point, _| {
                selection.set(None);
                true
            },
        );
    }
}

fn render_split_content(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<NavigationSplitRenderState>>,
    env: &Environment,
    selected: Option<Id>,
    compact: bool,
    bounds: vello::kurbo::Rect,
) {
    if let Some(selected) = selected {
        let mut state = state.borrow_mut();
        state.ensure_content(selected, compact, ctx.renderer_mut(), env);
        let render_ctx = ctx.render_context();
        state
            .content
            .as_mut()
            .expect("selected split content must be retained")
            .2
            .flush_in_rect(ctx.renderer_mut(), render_ctx, env, bounds);
    } else {
        let render_ctx = ctx.render_context();
        state
            .borrow_mut()
            .placeholder
            .flush_in_rect(ctx.renderer_mut(), render_ctx, env, bounds);
    }
}

fn render_split_detail(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<NavigationSplitRenderState>>,
    env: &Environment,
    selected: Option<Id>,
    compact: bool,
    bounds: vello::kurbo::Rect,
) {
    if let Some(selected) = selected {
        let mut state = state.borrow_mut();
        state.ensure_detail(selected, compact, ctx.renderer_mut(), env);
        let render_ctx = ctx.render_context();
        state
            .detail
            .as_mut()
            .expect("selected split detail must be retained")
            .2
            .flush_in_rect(ctx.renderer_mut(), render_ctx, env, bounds);
    } else {
        let render_ctx = ctx.render_context();
        state
            .borrow_mut()
            .placeholder
            .flush_in_rect(ctx.renderer_mut(), render_ctx, env, bounds);
    }
}

/// The retained render state of a `NavigationStack`. Both the root and every pushed
/// destination own a persistent render node, so inactive pages retain local widget
/// state and a transition never reconstructs its source or destination subtree.
pub(crate) struct NavigationStackRenderState {
    unresolved_root: Option<AnyView>,
    root: Option<RetainedSubview>,
    background: Option<Computed<ResolvedColor>>,
    transition_style: AnyNavigationTransition,
}

impl NavigationStackRenderState {
    pub(crate) fn from_stack(stack: NavigationStack<(), ()>) -> Self {
        let transition_style = stack.transition_style().clone();
        Self {
            unresolved_root: Some(stack.into_inner()),
            root: None,
            background: None,
            transition_style,
        }
    }

    fn resolve_root(&mut self, env: &Environment) -> Option<NavigationDestinationState> {
        let unresolved_root = self.unresolved_root.take()?;
        let NavigationView {
            bar,
            content,
            state,
        } = resolve_navigation_root(unresolved_root, env);
        self.background = Some(Color::new(Background).resolve(env));
        self.root = Some(RetainedSubview::new(AnyView::new(NavigationView {
            bar,
            content,
            state: NavigationDestinationState::default(),
        })));
        Some(state)
    }

    fn root_mut(&mut self) -> &mut RetainedSubview {
        self.root
            .as_mut()
            .expect("Hydrolysis navigation root must be resolved before rendering")
    }

    fn background(&self) -> Computed<ResolvedColor> {
        self.background
            .clone()
            .expect("Hydrolysis navigation background must be resolved before rendering")
    }
}

fn navigation_entry_identity(
    entries: &crate::renderer::navigation_state::NavigationEntries,
    index: usize,
) -> u64 {
    entries
        .borrow()
        .get(index)
        .unwrap_or_else(|| panic!("Hydrolysis navigation entry {index} is missing"))
        .identity
}

fn render_navigation_page_scene(
    renderer: &mut HydrolysisRenderer,
    state: &Rc<RefCell<NavigationStackRenderState>>,
    entries: &crate::renderer::navigation_state::NavigationEntries,
    identity: u64,
    env: &Environment,
    size: LayoutSize,
    inactive: bool,
) -> crate::renderer::navigation_state::NavigationCapturedScene {
    let background = state.borrow().background();
    let mut captured = if identity == 0 {
        let mut state = state.borrow_mut();
        if inactive {
            state
                .root_mut()
                .render_built_navigation_scene_inactive(renderer, env, size)
        } else {
            state.root_mut().render_built_scene(renderer, env, size)
        }
    } else {
        let mut entries = entries.borrow_mut();
        let entry = entries
            .iter_mut()
            .find(|entry| entry.identity == identity)
            .unwrap_or_else(|| {
                panic!("Hydrolysis navigation entry identity {identity} is not retained")
            });
        if inactive {
            entry
                .content
                .render_built_navigation_scene_inactive(renderer, env, size)
        } else {
            entry.content.render_built_scene(renderer, env, size)
        }
    };
    let mut scene = vello::Scene::new();
    let bounds = vello::kurbo::Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height));
    scene.fill(
        vello::peniko::Fill::NonZero,
        vello::kurbo::Affine::IDENTITY,
        resolved_color_to_peniko(renderer.read_signal(&background)),
        None,
        &bounds,
    );
    scene.append(&captured.scene, None);
    if identity != 0 {
        core::mem::swap(renderer.scene_mut(), &mut scene);
        let context = RenderContext::with_transforms(
            bounds,
            vello::kurbo::Affine::IDENTITY,
            vello::kurbo::Affine::IDENTITY,
        );
        {
            let theme = widget_theme(env);
            let mut draw = renderer.draw_context(context);
            theme.draw_navigation_back_button(
                &mut draw,
                navigation_back_button_rect(bounds, theme.navigation_metrics()),
            );
        }
        core::mem::swap(renderer.scene_mut(), &mut scene);
    }
    captured.scene = scene;
    captured
}

impl HydroNativeView for Native<NavigationStack<(), ()>> {
    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }
}

/// Binds the navigation stack by retained semantic identity and emits the
/// back-button accessibility node when the stack is non-empty.
pub(crate) fn navigation_stack_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    state: &Rc<RefCell<NavigationStackRenderState>>,
    env: &Environment,
) {
    let slot_key = crate::renderer::NavigationKey::for_rc(state);
    let entries = renderer.bind_navigation_entries(&slot_key);
    let depth = entries.borrow().len();
    #[cfg(feature = "accessibility")]
    {
        if depth == 0 {
            return;
        }
        let mut back_node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Button),
        );
        back_node.set_label(crate::localization::text(env, "back"));
        back_node.add_action(AccessibilityAction::Focus);
        back_node.add_action(AccessibilityAction::Click);
        let back_bounds = transformed_rect(
            ctx.hit_transform,
            navigation_back_button_rect(ctx.bounds, widget_theme(env).navigation_metrics()),
        );
        let _ = renderer.register_accessibility_node(
            back_node,
            back_bounds,
            env,
            Some(AccessibilityActionTarget::PointerPrimaryClick {
                point: accessibility_activation_point(back_bounds),
            }),
        );
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (ctx, depth, env);
    }
}

/// Measures a navigation stack leaf (zero intrinsic, matching the dispatch path).
pub(crate) fn measure_navigation_stack_node(
    _state: &NavigationStackRenderState,
    _proposal: ProposalSize,
    _hydro: &mut HydroState,
    _env: &Environment,
) -> ViewDimensions {
    ViewDimensions::new(LayoutSize::zero())
}

/// Renders a retained navigation stack leaf every flush. Stack accessibility is not
/// render-driven and additionally binds the navigation entries the render consumes,
/// so this node always runs `navigation_stack_accessibility` first (mirroring the
/// dispatch wrapper's `accessibility`-then-`render` order); when the stack is
/// accessibility-hidden it runs that step inside a suppression scope so the entries
/// are still bound while the a11y nodes are suppressed.
pub(crate) fn render_navigation_stack_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<NavigationStackRenderState>>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    #[cfg(feature = "accessibility")]
    if hidden {
        ctx.renderer_mut().push_accessibility_suppression();
    }
    {
        let render_ctx = ctx.render_context();
        navigation_stack_accessibility(ctx.renderer_mut(), render_ctx, state, env);
    }
    #[cfg(feature = "accessibility")]
    if hidden {
        ctx.renderer_mut().pop_accessibility_suppression();
    }
    render_navigation_stack_parts(ctx, state, env);
}

pub(crate) fn render_navigation_stack_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<NavigationStackRenderState>>,
    env: &Environment,
) {
    let transition_style = state.borrow().transition_style.clone();
    let transition_motion = widget_theme(env).navigation_motion();
    let slot_key = crate::renderer::NavigationKey::for_rc(state);
    let entries = ctx.renderer_mut().bind_navigation_entries(&slot_key);

    let mut local_env = env.clone();
    let controller = ctx
        .renderer_mut()
        .navigation
        .slots
        .get(&slot_key)
        .expect("hydrolysis navigation slot missing")
        .controller
        .clone();
    if let Some(retained_env) = controller.retained_environment() {
        local_env = retained_env;
    }
    local_env.insert(controller);

    if let Some(root_state) = state.borrow_mut().resolve_root(&local_env) {
        ctx.renderer_mut()
            .install_navigation_root_state(&slot_key, root_state);
    }

    let depth = entries.borrow().len();
    let active_identity = if depth == 0 {
        0
    } else {
        navigation_entry_identity(&entries, depth - 1)
    };
    if depth > 0 {
        local_env.insert(NavigationLeadingReserve(back_button_title_reserve(env)));
    }

    #[allow(clippy::cast_possible_truncation)]
    let scene_size = LayoutSize::new(ctx.bounds.width() as f32, ctx.bounds.height() as f32);
    let background = state.borrow().background();
    let background = resolved_color_to_peniko(ctx.renderer_mut().read_signal(&background));
    let transform = ctx.transform;
    let bounds = ctx.bounds;
    ctx.renderer_mut().scene_mut().fill(
        vello::peniko::Fill::NonZero,
        transform,
        background,
        None,
        &bounds,
    );

    let events = {
        let slot = ctx
            .renderer_mut()
            .navigation
            .slots
            .get_mut(&slot_key)
            .expect("Hydrolysis navigation slot missing");
        slot.events.borrow_mut().drain(..).collect::<Vec<_>>()
    };
    let mut navigation_change = None;
    if !events.is_empty() {
        for pair in events.windows(2) {
            assert_eq!(
                pair[0].current_identity, pair[1].previous_identity,
                "Hydrolysis navigation events must form one atomic transaction chain"
            );
        }
        let final_identity = events
            .last()
            .expect("non-empty navigation event batch must have a last event")
            .current_identity;
        assert_eq!(
            final_identity, active_identity,
            "Hydrolysis navigation event result must match retained stack entries"
        );
        let final_transaction_id = events
            .last()
            .expect("non-empty navigation event batch must have a last event")
            .transaction_id;
        let previous_identity = {
            let slot = ctx
                .renderer_mut()
                .navigation
                .slots
                .get_mut(&slot_key)
                .expect("Hydrolysis navigation slot missing");
            assert_eq!(
                events
                    .first()
                    .expect("non-empty navigation event batch must have a first event")
                    .previous_identity,
                slot.active_identity,
                "Hydrolysis navigation event must start from the rendered destination"
            );
            if let Some(previous_transaction_id) = slot.pending_transaction_id.take() {
                let _ = slot
                    .controller
                    .transition_cancelled(previous_transaction_id);
            }
            let previous_identity = slot.active_identity;
            let previous_depth = slot.last_depth;
            for event in events {
                slot.pending_removed.extend(event.removed);
            }
            slot.transition = None;
            slot.interactive_pop = None;
            slot.pending_transaction_id = Some(final_transaction_id);
            slot.pending_appearance = previous_identity != final_identity;
            slot.active_identity = final_identity;
            navigation_change = Some((previous_identity, previous_depth));
            previous_identity
        };
        if previous_identity != active_identity {
            ctx.renderer_mut().navigation_destination_disappeared(
                &slot_key,
                previous_identity,
                &local_env,
            );
        }
    }

    ctx.renderer_mut()
        .activate_navigation_root_if_needed(&slot_key, &local_env);

    if let Some((previous_identity, _)) = navigation_change {
        let previous_scene_is_cached = ctx
            .renderer_mut()
            .navigation
            .slots
            .get(&slot_key)
            .expect("Hydrolysis navigation slot missing")
            .scene_cache
            .contains_key(&previous_identity);
        if !previous_scene_is_cached {
            assert_eq!(
                previous_identity, 0,
                "Hydrolysis must retain the previously rendered navigation scene"
            );
            let previous_scene = render_navigation_page_scene(
                ctx.renderer_mut(),
                state,
                &entries,
                previous_identity,
                &local_env,
                scene_size,
                true,
            );
            ctx.renderer_mut()
                .navigation
                .slots
                .get_mut(&slot_key)
                .expect("Hydrolysis navigation slot missing")
                .scene_cache
                .insert(previous_identity, previous_scene);
        }
    }

    let active_scene = render_navigation_page_scene(
        ctx.renderer_mut(),
        state,
        &entries,
        active_identity,
        &local_env,
        scene_size,
        false,
    );

    let now = ctx.renderer_mut().frame_instant();
    let (transition_frame, complete_transaction) = {
        let slot = ctx
            .renderer_mut()
            .navigation
            .slots
            .get_mut(&slot_key)
            .expect("hydrolysis navigation slot missing");
        if let Some((previous_identity, previous_depth)) = navigation_change {
            let skip_transition = slot.skip_next_pop_transition && depth < previous_depth;
            slot.skip_next_pop_transition = false;
            if transition_style.retained() == RetainedNavigationTransition::None || skip_transition {
                slot.transition = None;
            } else {
                let direction = if depth >= previous_depth {
                    NavigationTransitionDirection::Push
                } else {
                    NavigationTransitionDirection::Pop
                };
                let from_scene = slot
                    .scene_cache
                    .get(&previous_identity)
                    .cloned()
                    .expect("hydrolysis navigation transition requires previous scene");
                slot.transition = Some(
                    crate::renderer::navigation_state::NavigationTransitionState::new(
                        transition_style.clone(),
                        direction,
                        from_scene,
                        active_scene.clone(),
                        now,
                        transition_motion.transition_duration,
                    ),
                );
            }
            slot.last_depth = depth;
        } else if let Some(transition) = slot.transition.as_ref()
            && !transition.is_active(now)
        {
            slot.transition = None;
        }
        let frame = slot.transition.as_ref().map(|transition| {
            (
                transition.style.clone(),
                transition.direction,
                transition.eased_progress(now, transition_motion),
                transition.from_scene.clone(),
                transition.to_scene.clone(),
            )
        });
        slot.scene_cache
            .insert(active_identity, active_scene.clone());
        slot.last_scene = Some(active_scene.clone());
        let complete = slot.transition.is_none() && slot.pending_transaction_id.is_some();
        (frame, complete)
    };

    if complete_transaction {
        ctx.renderer_mut()
            .complete_navigation_transaction(&slot_key, &local_env);
    }

    let interactive_frame = {
        let slot = ctx
            .renderer_mut()
            .navigation
            .slots
            .get_mut(&slot_key)
            .expect("Hydrolysis navigation slot missing");
        slot.interactive_pop.as_mut().map(|interactive| {
            let (progress, completed, cancelled) = interactive.sample(now, transition_motion);
            (
                progress,
                completed,
                cancelled,
                interactive.from_scene.clone(),
                interactive.to_scene.clone(),
            )
        })
    };

    if let Some((progress, completed, cancelled, from_scene, to_scene)) = interactive_frame {
        ctx.draw_navigation_transition(
            transition_style.clone(),
            transition_motion,
            NavigationTransitionDirection::Pop,
            progress,
            &from_scene,
            &to_scene,
        );
        if completed || cancelled {
            let slot = ctx
                .renderer_mut()
                .navigation
                .slots
                .get_mut(&slot_key)
                .expect("Hydrolysis navigation slot missing");
            slot.interactive_pop = None;
            if completed {
                slot.skip_next_pop_transition = true;
            }
        }
        if completed {
            let controller = ctx
                .renderer_mut()
                .navigation
                .slots
                .get(&slot_key)
                .expect("Hydrolysis navigation slot missing")
                .controller
                .clone();
            controller.request_pop(1);
        }
    } else if let Some((style, direction, progress, from_scene, to_scene)) = transition_frame {
        ctx.draw_navigation_transition(
            style,
            transition_motion,
            direction,
            progress,
            &from_scene,
            &to_scene,
        );
    } else {
        ctx.append_scene(&active_scene.composed());
    }

    if depth == 0 {
        return;
    }

    let previous_identity = if depth == 1 {
        0
    } else {
        navigation_entry_identity(&entries, depth - 2)
    };
    let previous_scene = ctx
        .renderer_mut()
        .navigation
        .slots
        .get(&slot_key)
        .expect("Hydrolysis navigation slot missing")
        .scene_cache
        .get(&previous_identity)
        .cloned();
    let previous_scene = previous_scene.unwrap_or_else(|| {
        let scene = render_navigation_page_scene(
            ctx.renderer_mut(),
            state,
            &entries,
            previous_identity,
            &local_env,
            scene_size,
            true,
        );
        ctx.renderer_mut()
            .navigation
            .slots
            .get_mut(&slot_key)
            .expect("Hydrolysis navigation slot missing")
            .scene_cache
            .insert(previous_identity, scene.clone());
        scene
    });

    let metrics = widget_theme(env).navigation_metrics();
    let edge_rect = vello::kurbo::Rect::new(
        ctx.bounds.x0,
        ctx.bounds.y0,
        (ctx.bounds.x0 + metrics.back_button_size).min(ctx.bounds.x1),
        ctx.bounds.y1,
    );
    let edge_hit_rect = transformed_rect(ctx.hit_transform, edge_rect);
    let inverse_hit_transform = ctx.hit_transform.inverse();
    let active_scene_for_gesture = active_scene.clone();
    let previous_scene_for_gesture = previous_scene;
    let navigation_width = ctx.bounds.width();
    let drag_slot_key = slot_key.clone();
    ctx.renderer_mut().register_pointer_drag_target(
        edge_hit_rect,
        move |renderer, point, pop_env| {
            let point = inverse_hit_transform * point;
            let starting = renderer
                .navigation
                .slots
                .get(&drag_slot_key)
                .expect("Hydrolysis navigation slot missing")
                .interactive_pop
                .is_none();
            if starting {
                if !renderer.attempt_navigation_pop(&drag_slot_key, pop_env) {
                    return false;
                }
                let slot = renderer
                    .navigation
                    .slots
                    .get_mut(&drag_slot_key)
                    .expect("Hydrolysis navigation slot missing");
                slot.transition = None;
                slot.interactive_pop = Some(
                    crate::renderer::navigation_state::NavigationInteractivePop::new(
                        point.x,
                        navigation_width,
                        active_scene_for_gesture.clone(),
                        previous_scene_for_gesture.clone(),
                    ),
                );
                return true;
            }
            renderer
                .navigation
                .slots
                .get_mut(&drag_slot_key)
                .expect("Hydrolysis navigation slot missing")
                .interactive_pop
                .as_mut()
                .expect("Hydrolysis interactive pop must exist while dragging")
                .update(point.x)
        },
    );

    let back_button_rect = navigation_back_button_rect(ctx.bounds, metrics);
    let controller = ctx
        .renderer_mut()
        .navigation
        .slots
        .get(&slot_key)
        .expect("Hydrolysis navigation slot missing")
        .controller
        .clone();
    let hit_transform = ctx.hit_transform;
    let back_hit_rect = transformed_rect(hit_transform, back_button_rect);
    let back_interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let (_, back_press_slot, _) = ctx.renderer_mut().bind_control_interaction_target(
        back_interaction_key,
        back_hit_rect,
        env,
        false,
    );
    let back_slot_key = slot_key;
    ctx.renderer_mut().register_interactive_pointer_target(
        back_hit_rect,
        back_press_slot,
        move |renderer, _point, pop_env| {
            if !renderer.attempt_navigation_pop(&back_slot_key, pop_env) {
                return false;
            }
            controller.request_pop(1);
            true
        },
    );
}

#[cfg(test)]
mod tests {
    use super::resolved_split_column_width;
    use waterui::navigation::{ColumnWidth, NativeNavigationSplitStyle};

    /// The sidebar's width is a policy of the split style, not of the column
    /// constraints alone: a detail-first layout squeezes the sidebar to its
    /// minimum so the detail column gets the remaining space, while the
    /// balanced styles honour the author's ideal.
    #[test]
    fn prominent_detail_squeezes_the_sidebar_to_its_minimum() {
        let width = ColumnWidth::new(180.0, 260.0, 400.0);

        assert_eq!(
            resolved_split_column_width(width, NativeNavigationSplitStyle::ProminentDetail),
            180.0
        );
        for balanced in [
            NativeNavigationSplitStyle::Automatic,
            NativeNavigationSplitStyle::Balanced,
        ] {
            assert_eq!(resolved_split_column_width(width, balanced), 260.0);
        }
    }

    /// Column constraints that leave no room to choose must resolve the same way
    /// under every style, so a fixed-width sidebar cannot drift between them.
    #[test]
    fn a_fixed_width_sidebar_resolves_identically_under_every_style() {
        let fixed = ColumnWidth::new(240.0, 240.0, 240.0);

        for style in [
            NativeNavigationSplitStyle::Automatic,
            NativeNavigationSplitStyle::Balanced,
            NativeNavigationSplitStyle::ProminentDetail,
        ] {
            assert_eq!(resolved_split_column_width(fixed, style), 240.0);
        }
    }
}
