use crate::engine::Brush;
#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
#[cfg(feature = "accessibility")]
use crate::renderer::accessibility_activation_point;
use crate::renderer::navigation_state::NavigationTransitionDirection;
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
use waterui::navigation::{
    Bar, NavigationSearch, NavigationSplitLayout, NavigationStack, NavigationTitleDisplayMode,
    NavigationTransition, NavigationView,
};
use waterui::theme::color::Surface;
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

/// The retained render state of a `NavigationView`. The bar's `title`/`leading`/
/// `trailing` and the screen `content` are move-only `AnyView`s, so the persistent
/// `Widget` node holds each as a [`RetainedSubview`] built once and re-flushed at
/// its rect every frame (so reactive descendants inside them stay live). The bar's
/// reactive appearance signals (`color`/`hidden`) are kept and read through
/// `read_signal`; the static `display_mode` and the `search` model (clonable, used
/// to build a fresh `TextField` each frame) are kept by value.
pub(crate) struct NavigationViewRenderState {
    title: RetainedSubview,
    leading: RetainedSubview,
    trailing: RetainedSubview,
    content: RetainedSubview,
    search: Option<NavigationSearch>,
    /// The search field as a retained node sub-view (the `TextField`'s reactive text
    /// binding stays live through the node's own re-flush). `Some` exactly when
    /// `search` is present.
    search_field: Option<RetainedSubview>,
    color: Computed<ResolvedColor>,
    hidden: Computed<bool>,
    display_mode: NavigationTitleDisplayMode,
    /// Whether `leading`/`trailing` were the empty `()` placeholder at build time:
    /// the dispatch path measures/places them only when non-empty.
    leading_present: bool,
    trailing_present: bool,
}

impl NavigationViewRenderState {
    pub(crate) fn from_view(navigation: NavigationView, env: &Environment) -> Self {
        let NavigationView { bar, content } = navigation;
        let Bar {
            title,
            leading,
            trailing,
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
        let leading_present = !leading.is::<()>();
        let trailing_present = !trailing.is::<()>();
        let search_field = search.as_ref().map(|search| {
            RetainedSubview::new(AnyView::new(
                TextField::new(&search.text).prompt(search.prompt.clone()),
            ))
        });
        Self {
            title: RetainedSubview::new(title),
            leading: RetainedSubview::new(leading),
            trailing: RetainedSubview::new(trailing),
            content: RetainedSubview::new(content),
            search,
            search_field,
            color,
            hidden,
            display_mode,
            leading_present,
            trailing_present,
        }
    }

    /// Eagerly build the bar/content sub-views (the measure path has no renderer to
    /// build on), mirroring the dispatch path's normalization.
    pub(crate) fn prebuild(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        self.title.ensure_built(renderer, env);
        if self.leading_present {
            self.leading.ensure_built(renderer, env);
        }
        if self.trailing_present {
            self.trailing.ensure_built(renderer, env);
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
    bar: &Bar,
    default_title_label: Option<String>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let bar_hidden = renderer.read_signal(&bar.hidden);
        if bar_hidden {
            return;
        }
        let metrics = widget_theme(env).navigation_metrics();
        let bar_height = navigation_base_bar_height_for_display_mode(bar.display_mode, env);
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
        let title_height = if matches!(bar.display_mode, NavigationTitleDisplayMode::Large) {
            metrics.large_title_height
        } else {
            metrics.inline_title_height
        };
        let title_y0 = if matches!(bar.display_mode, NavigationTitleDisplayMode::Large) {
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
        let _ = (renderer, ctx, bar, default_title_label, env);
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
    let title_size = if bar_height > 0.0 {
        state.title.measure_built(hydro, env)
    } else {
        LayoutSize::zero()
    };
    let leading_size = if state.leading_present {
        state.leading.measure_built(hydro, env)
    } else {
        LayoutSize::zero()
    };
    let trailing_size = if state.trailing_present {
        state.trailing.measure_built(hydro, env)
    } else {
        LayoutSize::zero()
    };
    let search_size = if let Some(search) = state.search.as_ref() {
        let body_env = env.clone();
        let search_field = TextField::new(&search.text).prompt(search.prompt.clone());
        let search_body = normalize_layout_view(
            AnyView::new(waterui_core::View::body(search_field, &body_env)),
            &body_env,
        );
        measure_view_intrinsic(&search_body, hydro, &body_env)
    } else {
        LayoutSize::zero()
    };
    let content_size = state.content.measure_built(hydro, env);
    let width = f64::from(content_size.width)
        .max(
            f64::from(leading_size.width)
                + f64::from(title_size.width)
                + f64::from(trailing_size.width)
                + metrics.horizontal_inset * 2.0
                + metrics.item_spacing * 2.0,
        )
        .max(f64::from(search_size.width) + metrics.horizontal_inset * 2.0);
    let height = f64::from(content_size.height) + bar_height;
    ViewDimensions::new(LayoutSize::new(width as f32, height as f32))
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
        let bar = Bar {
            title: AnyView::default(),
            leading: AnyView::default(),
            trailing: AnyView::default(),
            search: None,
            color: None,
            resolved_color: None,
            hidden: hidden_signal,
            display_mode,
        };
        navigation_view_accessibility(
            ctx.renderer_mut(),
            render_ctx,
            &bar,
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
    let (hidden_signal, color_signal, display_mode, search, leading_present, trailing_present) = {
        let state = state.borrow();
        (
            state.hidden.clone(),
            state.color.clone(),
            state.display_mode,
            state.search.clone(),
            state.leading_present,
            state.trailing_present,
        )
    };
    let bar_height = if ctx.renderer_mut().read_signal(&hidden_signal) {
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

    if bar_height > 0.0 {
        let metrics = widget_theme(env).navigation_metrics();
        let base_bar_height = navigation_base_bar_height_for_display_mode(display_mode, env);
        let bar_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x1,
            (ctx.bounds.y0 + bar_height).min(ctx.bounds.y1),
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

        let leading_width = if leading_present {
            f64::from(
                state
                    .borrow_mut()
                    .leading
                    .measure_intrinsic(ctx.renderer_mut(), env)
                    .width,
            )
        } else {
            0.0
        };
        let trailing_width = if trailing_present {
            f64::from(
                state
                    .borrow_mut()
                    .trailing
                    .measure_intrinsic(ctx.renderer_mut(), env)
                    .width,
            )
        } else {
            0.0
        };
        let leading_rect = vello::kurbo::Rect::new(
            bar_rect.x0 + metrics.horizontal_inset,
            bar_rect.y0 + (base_bar_height - metrics.inline_title_height) * 0.5,
            (bar_rect.x0 + metrics.horizontal_inset + leading_width).min(bar_rect.x1),
            bar_rect.y0 + (base_bar_height + metrics.inline_title_height) * 0.5,
        );
        let trailing_rect = vello::kurbo::Rect::new(
            (bar_rect.x1 - metrics.horizontal_inset - trailing_width).max(bar_rect.x0),
            bar_rect.y0 + (base_bar_height - metrics.inline_title_height) * 0.5,
            bar_rect.x1 - metrics.horizontal_inset,
            bar_rect.y0 + (base_bar_height + metrics.inline_title_height) * 0.5,
        );
        if leading_present && leading_rect.width() > 0.0 && leading_rect.height() > 0.0 {
            let render_ctx = ctx.render_context();
            state.borrow_mut().leading.flush_in_rect(
                ctx.renderer_mut(),
                render_ctx,
                env,
                leading_rect,
            );
        }
        if trailing_present && trailing_rect.width() > 0.0 && trailing_rect.height() > 0.0 {
            let render_ctx = ctx.render_context();
            state.borrow_mut().trailing.flush_in_rect(
                ctx.renderer_mut(),
                render_ctx,
                env,
                trailing_rect,
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
            let render_ctx = ctx.render_context();
            state
                .borrow_mut()
                .title
                .flush_in_rect(ctx.renderer_mut(), render_ctx, env, title_rect);
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
        (ctx.bounds.y0 + bar_height).min(ctx.bounds.y1),
        ctx.bounds.x1,
        ctx.bounds.y1,
    );
    if content_rect.width() > 0.0 && content_rect.height() > 0.0 {
        let render_ctx = ctx.render_context();
        state
            .borrow_mut()
            .content
            .flush_in_rect(ctx.renderer_mut(), render_ctx, env, content_rect);
    }
}

/// The retained render state of a `NavigationSplitLayout`. The layout's sidebar/
/// placeholder/detail are clonable, `Rc`-backed builders that produce a fresh view
/// each frame, and the selection is a clonable `Binding`, so the whole config is
/// kept by value and re-read every flush — sidebar/detail/placeholder are rebuilt
/// and re-dispatched live, and the selection drives a fresh detail each frame.
pub(crate) struct NavigationSplitRenderState {
    pub(crate) split: NavigationSplitLayout,
    /// The sidebar as a retained node sub-view, built once (reactive descendants
    /// stay live through the node's own re-flush).
    sidebar: RetainedSubview,
    /// The empty-selection placeholder as a retained node sub-view, built once.
    placeholder: RetainedSubview,
    /// The detail content keyed by `(selected id, compact)` — rebuilt when either
    /// changes, since both the detail view (selection) and its env (compact adds a
    /// back-button leading reserve) depend on them; re-flushed each frame otherwise.
    detail: Option<(Id, bool, RetainedSubview)>,
}

impl NavigationSplitRenderState {
    pub(crate) fn from_layout(split: NavigationSplitLayout) -> Self {
        let sidebar = RetainedSubview::new(split.sidebar().build());
        let placeholder = RetainedSubview::new(split.placeholder().build());
        Self {
            split,
            sidebar,
            placeholder,
            detail: None,
        }
    }

    /// Eagerly build the sidebar + placeholder sub-views (the measure path has no
    /// renderer); the detail is built lazily when a selection first resolves.
    pub(crate) fn prebuild(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        self.sidebar.ensure_built(renderer, env);
        self.placeholder.ensure_built(renderer, env);
    }

    /// Ensure the detail sub-view is built for `(id, compact)`, rebuilding it when
    /// the selection or compact mode changes. `env` must carry the compact-mode
    /// leading reserve when `compact` is true (it scopes the built node).
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
            let view = AnyView::new(self.split.detail_builder().build(id));
            let mut subview = RetainedSubview::new(view);
            subview.ensure_built(renderer, env);
            self.detail = Some((id, compact, subview));
        }
    }
}

impl HydroNativeView for Native<NavigationSplitLayout> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let split = view.as_inner();
        let sidebar = {
            let sidebar_view = normalize_layout_view(split.sidebar().build(), env);
            measure_view_intrinsic(&sidebar_view, state, env)
        };
        let detail = if let Some(selected) = split.selection().get() {
            measure_owned_navigation_view_intrinsic(
                split.detail_builder().build(selected),
                state,
                env,
            )
        } else {
            let placeholder_view = normalize_layout_view(split.placeholder().build(), env);
            measure_view_intrinsic(&placeholder_view, state, env)
        };
        LayoutSize::new(
            (f64::from(split.sidebar_width()) + f64::from(detail.width)) as f32,
            f64::from(sidebar.height.max(detail.height)) as f32,
        )
    }
}

/// Measures a navigation split leaf from its layout (intrinsic-sized).
pub(crate) fn measure_navigation_split_node(
    split: &NavigationSplitLayout,
    _proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    let sidebar = {
        let sidebar_view = normalize_layout_view(split.sidebar().build(), env);
        measure_view_intrinsic(&sidebar_view, state, env)
    };
    let detail = if let Some(selected) = split.selection().get() {
        measure_owned_navigation_view_intrinsic(split.detail_builder().build(selected), state, env)
    } else {
        let placeholder_view = normalize_layout_view(split.placeholder().build(), env);
        measure_view_intrinsic(&placeholder_view, state, env)
    };
    ViewDimensions::new(LayoutSize::new(
        (f64::from(split.sidebar_width()) + f64::from(detail.width)) as f32,
        f64::from(sidebar.height.max(detail.height)) as f32,
    ))
}

/// Renders a retained navigation split leaf every flush. The split has no a11y of
/// its own (its children carry their own), so this is just the parts render.
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
    let (selection, sidebar_width_raw) = {
        let st = state.borrow();
        (
            st.split.selection().clone(),
            f64::from(st.split.sidebar_width()),
        )
    };
    let compact = bounds.width() < split_compact_threshold(sidebar_width_raw);
    let selected = ctx.renderer_mut().read_signal(&selection);

    if compact && let Some(selected_id) = selected {
        let mut detail_env = env.clone();
        detail_env.insert(NavigationLeadingReserve(back_button_title_reserve(env)));
        {
            let mut st = state.borrow_mut();
            st.ensure_detail(selected_id, true, ctx.renderer_mut(), &detail_env);
            let render_ctx = ctx.render_context();
            if let Some((_, _, subview)) = st.detail.as_mut() {
                subview.flush_in_rect(ctx.renderer_mut(), render_ctx, &detail_env, bounds);
            }
        }
        let back_button_rect =
            navigation_back_button_rect(bounds, widget_theme(env).navigation_metrics());
        {
            let theme = widget_theme(env);
            let mut draw = ctx.draw_context();
            theme.draw_navigation_back_button(&mut draw, back_button_rect);
        }
        let hit_transform = ctx.hit_transform;
        ctx.renderer_mut().register_pointer_target(
            transformed_rect(hit_transform, back_button_rect),
            move |_renderer, _point, _| {
                selection.set(None);
                true
            },
        );
        return;
    }

    let sidebar_width = sidebar_width_raw.min(bounds.width() * 0.5);
    let sidebar_rect =
        vello::kurbo::Rect::new(bounds.x0, bounds.y0, bounds.x0 + sidebar_width, bounds.y1);
    let detail_rect = vello::kurbo::Rect::new(sidebar_rect.x1, bounds.y0, bounds.x1, bounds.y1);
    {
        let mut st = state.borrow_mut();
        let render_ctx = ctx.render_context();
        st.sidebar
            .flush_in_rect(ctx.renderer_mut(), render_ctx, env, sidebar_rect);
    }
    if let Some(selected_id) = selected {
        let mut st = state.borrow_mut();
        st.ensure_detail(selected_id, false, ctx.renderer_mut(), env);
        let render_ctx = ctx.render_context();
        if let Some((_, _, subview)) = st.detail.as_mut() {
            subview.flush_in_rect(ctx.renderer_mut(), render_ctx, env, detail_rect);
        }
    } else {
        let mut st = state.borrow_mut();
        let render_ctx = ctx.render_context();
        st.placeholder
            .flush_in_rect(ctx.renderer_mut(), render_ctx, env, detail_rect);
    }
}

/// The retained render state of a `NavigationStack`. The stack root is a move-only
/// `AnyView`, so it is held as a [`RetainedSubview`] (re-rendered into a fresh
/// scene each frame for the transition cross-fade); the active pushed destinations
/// come from the navigation controller's entries (rebuilt from their `Rc`-backed
/// builders each flush), so they need no retention. The transition style is copied.
pub(crate) struct NavigationStackRenderState {
    root: RetainedSubview,
    transition_style: NavigationTransition,
}

impl NavigationStackRenderState {
    pub(crate) fn from_stack(stack: NavigationStack<(), ()>) -> Self {
        let transition_style = stack.transition_style();
        let root = stack.into_inner();
        Self {
            root: RetainedSubview::new(root),
            transition_style,
        }
    }
}

impl HydroNativeView for Native<NavigationStack<(), ()>> {
    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }
}

/// Binds the navigation entries (pushing them pending for the subsequent render to
/// consume) and emits the back-button a11y node when the stack is non-empty. Shared
/// by the dispatch path and the retained `Widget`-node path; the binding side
/// effect runs regardless of the `accessibility` feature.
pub(crate) fn navigation_stack_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    env: &Environment,
) {
    let entries = {
        let (slot_index, entries) = renderer.bind_navigation_entries();
        renderer.push_pending_navigation_entries(slot_index, Rc::clone(&entries));
        entries
    };
    let depth = entries.borrow().len();
    #[cfg(feature = "accessibility")]
    {
        if depth == 0 {
            return;
        }
        let mut back_node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Button),
        );
        back_node.set_label("Back".to_owned());
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
        navigation_stack_accessibility(ctx.renderer_mut(), render_ctx, env);
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
    let transition_style = state.borrow().transition_style;
    let transition_motion = widget_theme(env).navigation_motion();
    let (slot_index, entries) = ctx
        .renderer_mut()
        .take_pending_navigation_entries("render_navigation_stack");

    let mut local_env = env.clone();
    let controller = ctx
        .renderer_mut()
        .navigation
        .slots
        .get(slot_index)
        .expect("hydrolysis navigation slot missing")
        .controller
        .clone();
    if let Some(retained_env) = controller.retained_environment() {
        local_env = retained_env;
    }
    local_env.insert(controller);

    let depth = entries.borrow().len();
    if depth > 0 {
        local_env.insert(NavigationLeadingReserve(back_button_title_reserve(env)));
    }

    // The active screen is the top pushed destination (rebuilt fresh from its
    // `Rc`-backed builder), or the retained root at depth 0. Both are rendered into
    // a standalone scene so the transition can cross-fade `from`/`to` without
    // re-dispatch.
    #[allow(clippy::cast_possible_truncation)]
    let scene_size = LayoutSize::new(ctx.bounds.width() as f32, ctx.bounds.height() as f32);
    let active = entries
        .borrow()
        .last()
        .map(|builder| AnyView::new(builder.build()));
    let active_scene = if let Some(active) = active {
        // Pushed destinations are rebuilt fresh from their `Rc`-backed builders each
        // flush, so the freshly built view is wrapped in a transient `RetainedSubview`
        // and rendered into a standalone scene through the node path (the same
        // mechanism the retained root uses), keeping the transition cross-fade able to
        // replay `from`/`to` scenes.
        let mut active_subview = RetainedSubview::new(active);
        active_subview.render_built_scene(ctx.renderer_mut(), &local_env, scene_size)
    } else {
        state
            .borrow_mut()
            .root
            .render_built_scene(ctx.renderer_mut(), &local_env, scene_size)
    };

    let now = ctx.renderer_mut().frame_instant();
    let transition_frame = {
        let slot = ctx
            .renderer_mut()
            .navigation
            .slots
            .get_mut(slot_index)
            .expect("hydrolysis navigation slot missing");
        if depth != slot.last_depth {
            if transition_style == NavigationTransition::None || slot.last_scene.is_none() {
                slot.transition = None;
            } else {
                let direction = if depth > slot.last_depth {
                    NavigationTransitionDirection::Push
                } else {
                    NavigationTransitionDirection::Pop
                };
                let from_scene = slot
                    .last_scene
                    .take()
                    .expect("hydrolysis navigation transition requires previous scene");
                slot.transition = Some(
                    crate::renderer::navigation_state::NavigationTransitionState::new(
                        transition_style,
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
                transition.style,
                transition.direction,
                transition.progress(now),
                transition_motion.pushpop_parallax_factor,
                transition.from_scene.clone(),
                transition.to_scene.clone(),
            )
        });
        slot.last_scene = Some(active_scene.clone());
        frame
    };

    if let Some((style, direction, progress, parallax_factor, from_scene, to_scene)) =
        transition_frame
    {
        ctx.draw_navigation_transition(
            style,
            direction,
            progress,
            parallax_factor,
            &from_scene,
            &to_scene,
        );
    } else {
        ctx.append_scene(&active_scene);
    }

    if depth == 0 {
        return;
    }

    let back_button_rect =
        navigation_back_button_rect(ctx.bounds, widget_theme(env).navigation_metrics());
    {
        let theme = widget_theme(env);
        let mut draw = ctx.draw_context();
        theme.draw_navigation_back_button(&mut draw, back_button_rect);
    }

    let entries_for_pop = Rc::clone(&entries);
    let signals = ctx.renderer_mut().frame_signals();
    let hit_transform = ctx.hit_transform;
    ctx.renderer_mut().register_pointer_target(
        transformed_rect(hit_transform, back_button_rect),
        move |_renderer, _point, _env| {
            if entries_for_pop.borrow_mut().pop().is_some() {
                signals.request_rebuild();
                return true;
            }
            false
        },
    );
}
