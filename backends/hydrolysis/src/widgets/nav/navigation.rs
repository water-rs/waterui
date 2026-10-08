#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
#[cfg(feature = "accessibility")]
use crate::renderer::ROOT_NAVIGATION_IDENTITY;
use crate::renderer::SafeAreaLayout;
use crate::renderer::bounded_proposal;
use crate::renderer::{
    Edge, HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, RetainedSubview,
    WidgetRenderContext, measure_navigation_view_intrinsic,
    measure_owned_navigation_view_with_proposal, measure_transient_view_with_proposal,
    navigation_back_button_rect, navigation_base_bar_height_for_display_mode,
    normalize_layout_view, split_compact_threshold,
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
    AnyNavigationTransition, Bar, NavigationDestinationState, NavigationSearch,
    NavigationSplitLayout, NavigationStack, NavigationTitleDisplayMode, NavigationToolbarPlacement,
    NavigationTransitionDirection, NavigationView, RetainedNavigationTransition,
    resolve_navigation_root,
};
use waterui::theme::color::{Background, Surface};
use waterui_controls::text_field::TextField;
use waterui_core::id::Id;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Metadata, Native};
use waterui_graphics::color::Color;
use waterui_graphics::draw::{Paint, WorkingColor};

#[derive(Clone, Copy)]
struct NavigationLeadingReserve(f64);

fn navigation_leading_reserve(env: &Environment) -> f64 {
    env.get::<NavigationLeadingReserve>()
        .map_or(0.0, |reserve| reserve.0)
}

fn back_button_title_reserve(theme: &Rc<dyn crate::engine::WidgetTheme>) -> f64 {
    let metrics = theme.navigation_metrics();
    metrics.back_button_size + metrics.title_leading_inset
}

/// The environment a page is presented under: `base`, plus the stack's
/// back-button reserve for any pushed destination. The reserve is a
/// property of the page, not of the stack's current depth — a pushed page
/// always shows the back chrome, the root never does — so a departing or
/// landing page's cached scene is validated and re-recorded against *its*
/// env, not the current top's (water-rs/hydrolysis#325).
fn presented_page_env(
    base: &Environment,
    identity: u64,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> Environment {
    let mut env = base.clone();
    if identity != 0 {
        env.insert(NavigationLeadingReserve(back_button_title_reserve(theme)));
    }
    env
}

/// The retained render state of a `NavigationView`. The bar's semantic title,
/// subtitle, toolbar items and screen content are move-only `AnyView`s, so the persistent
/// `Widget` node holds each as a [`RetainedSubview`] built once and re-flushed at
/// its rect every frame (so reactive descendants inside them stay live). The bar's
/// reactive appearance signals (`color`/`hidden`) are kept and read through
/// `read_signal`; the static `display_mode` and the `search` model (cloneable, used
/// to build a fresh `TextField` each frame) are kept by value.
pub struct NavigationViewRenderState {
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
    color: Computed<WorkingColor>,
    hidden: Computed<bool>,
    display_mode: NavigationTitleDisplayMode,
    subtitle_present: bool,
}

/// Whether a bar slot is bound. An unbound slot is `()`, which resolves to
/// the native-leaf `Native<()>` by the time the backend sees it (`()` is a
/// raw view); waterui may also wrap a slot in `Metadata<Environment>`
/// (`navigation_slot_with_environment`), so the check looks through that wrap
/// too.
#[expect(
    clippy::option_if_let_else,
    reason = "the if-let/else mirrors the control flow more clearly than the combinator chain here"
)]
fn navigation_slot_is_empty(view: &AnyView) -> bool {
    if view.is::<()>() || view.is::<Native<()>>() {
        return true;
    }
    match view.downcast_ref::<Metadata<Environment>>() {
        Some(meta) => navigation_slot_is_empty(&meta.content),
        None => false,
    }
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
            hidden,
            display_mode,
        } = bar;
        let color = color.as_ref().map_or_else(
            || Color::new(Surface).resolve(env),
            |color| color.expect_resolved().clone(),
        );
        let subtitle_present = !navigation_slot_is_empty(&subtitle);
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
            // A search field shows only its placeholder, so the prompt doubles
            // as the accessible name and the visible label is suppressed.
            RetainedSubview::new(AnyView::new(
                TextField::new(search.prompt.clone(), &search.text)
                    .hide_label()
                    .prompt(search.prompt.clone()),
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
    pub(crate) fn prebuild(
        &mut self,
        renderer: &mut crate::renderer::SemanticCore,
        env: &Environment,
    ) {
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
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        measure_navigation_view_intrinsic(view.as_inner(), state, env, theme)
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        if let (Some(width), Some(height)) = (proposal.width, proposal.height) {
            return ViewDimensions::new(LayoutSize::new(width, height));
        }
        ViewDimensions::new(Self::intrinsic(state, view, env, theme))
    }
}

/// Emits a navigation view's bar/title/subtitle accessibility nodes. Shared by
/// the dispatch path and the retained `Widget`-node path. The title and
/// subtitle views are owned by [`RetainedSubview`]s in `state`, which keeps the
/// spoken label each resolved at build time (`default_a11y_label`); both emit
/// as manual children of the bar node — the title a `Header`, the subtitle a
/// `Label` — because their draws are suppressed chrome, not sub-view emissions.
#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn navigation_view_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    theme: Option<&Rc<dyn crate::engine::WidgetTheme>>,
    hidden: &Computed<bool>,
    display_mode: NavigationTitleDisplayMode,
    state: &NavigationViewRenderState,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        if renderer.read_signal(hidden) {
            return;
        }
        let default_title_label = state.title.default_a11y_label();
        let default_subtitle_label = if state.subtitle_present {
            state.subtitle.default_a11y_label()
        } else {
            None
        };
        // Bar/title geometry exists only in the rendered runtime; the semantic
        // emission walk has no bounds and no theme to take metrics from.
        let (bar_bounds, title_bounds, subtitle_bounds) =
            ctx.as_ref()
                .zip(theme)
                .map_or((None, None, None), |(ctx, theme)| {
                    let metrics = theme.navigation_metrics();
                    let bar_height =
                        navigation_base_bar_height_for_display_mode(display_mode, theme);
                    let bar_rect = kurbo::Rect::new(
                        ctx.bounds.x0,
                        ctx.bounds.y0,
                        ctx.bounds.x1,
                        (ctx.bounds.y0 + bar_height).min(ctx.bounds.y1),
                    );
                    let title_height = if matches!(display_mode, NavigationTitleDisplayMode::Large)
                    {
                        metrics.large_title_height
                    } else {
                        metrics.inline_title_height
                    };
                    let title_y0 = if matches!(display_mode, NavigationTitleDisplayMode::Large) {
                        bar_rect.y1 - metrics.large_title_bottom_inset - title_height
                    } else {
                        (bar_height - title_height).mul_add(0.5, bar_rect.y0)
                    };
                    let title_leading = navigation_leading_reserve(env);
                    let title_rect = kurbo::Rect::new(
                        if title_leading > 0.0 {
                            bar_rect.x0 + metrics.horizontal_inset + title_leading
                        } else {
                            bar_rect.x0 + metrics.title_leading_inset
                        },
                        title_y0,
                        bar_rect.x1 - metrics.title_trailing_inset,
                        title_y0 + title_height,
                    );
                    let title_bounds = (title_rect.width() > 0.0 && title_rect.height() > 0.0)
                        .then_some(title_rect);
                    // The subtitle sits under the title inside `title_rect`,
                    // positioned by the same split the suppressed flush draws at.
                    let subtitle_bounds = if state.subtitle_present {
                        let title_size =
                            state.title.measure_built(renderer.state_mut(), env, theme);
                        let subtitle_size =
                            state
                                .subtitle
                                .measure_built(renderer.state_mut(), env, theme);
                        let (_, subtitle_rect) =
                            title_and_subtitle_rects(title_rect, title_size, subtitle_size);
                        (subtitle_rect.width() > 0.0 && subtitle_rect.height() > 0.0)
                            .then_some(subtitle_rect)
                    } else {
                        None
                    };
                    (Some(bar_rect), title_bounds, subtitle_bounds)
                });
        let mut bar_node =
            AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::Navigation,
            ));
        let bar_label = renderer.resolve_accessibility_label(env, None);
        if let Some(label) = bar_label {
            bar_node.set_label(label);
        }
        if let Some(value) = renderer.resolve_accessibility_value(env, None) {
            bar_node.set_value(value);
        }
        let mut title_node =
            AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::Header,
            ));
        let title_label = renderer.resolve_accessibility_label(env, default_title_label);
        if let Some(label) = title_label {
            title_node.set_label(label);
        }
        let title_node_id = match title_bounds {
            Some(title_bounds) => {
                renderer.register_accessibility_child_node(title_node, title_bounds, env, None)
            }
            None => renderer.register_accessibility_child_node_semantic(title_node, env, None),
        };
        if let Some(title_node_id) = title_node_id {
            bar_node.push_child(title_node_id);
        }
        if state.subtitle_present {
            let mut subtitle_node =
                AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                    env,
                    AccessibilityNodeRole::Label,
                ));
            let subtitle_label = renderer.resolve_accessibility_label(env, default_subtitle_label);
            if let Some(label) = subtitle_label {
                subtitle_node.set_label(label);
            }
            let subtitle_node_id = match subtitle_bounds {
                Some(subtitle_bounds) => renderer.register_accessibility_child_node(
                    subtitle_node,
                    subtitle_bounds,
                    env,
                    None,
                ),
                None => {
                    renderer.register_accessibility_child_node_semantic(subtitle_node, env, None)
                }
            };
            if let Some(subtitle_node_id) = subtitle_node_id {
                bar_node.push_child(subtitle_node_id);
            }
        }
        match bar_bounds {
            Some(bar_bounds) => {
                let _ = renderer.register_accessibility_node(bar_node, bar_bounds, env, None);
            }
            None => {
                let _ = renderer.register_accessibility_node_semantic(bar_node, env, None);
            }
        }
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, theme, hidden, display_mode, state, env);
    }
}

/// Measures a retained navigation view leaf from its [`NavigationViewRenderState`].
/// Fills both axes when a concrete proposal is supplied (matching the dispatch-path
/// `dimensions`), otherwise falls back to the intrinsic size computed from the
/// prebuilt bar/content sub-views (mirroring `measure_navigation_view_intrinsic`).
pub fn measure_navigation_view_node(
    state: &NavigationViewRenderState,
    proposal: ProposalSize,
    hydro: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    if let (Some(width), Some(height)) = (proposal.width, proposal.height) {
        return ViewDimensions::new(LayoutSize::new(width, height));
    }
    let bar_hidden = hydro.measure_signal(&state.hidden);
    let metrics = theme.navigation_metrics();
    let bar_height = if bar_hidden {
        0.0
    } else {
        let base = navigation_base_bar_height_for_display_mode(state.display_mode, theme);
        let search_extra = if state.search.is_some() {
            metrics
                .search_vertical_inset
                .mul_add(2.0, metrics.search_height)
        } else {
            0.0
        };
        base + search_extra
    };
    let title_size = if bar_height > 0.0 && state.principal.is_empty() {
        let title = state.title.measure_built(hydro, env, theme);
        let subtitle = if state.subtitle_present {
            state.subtitle.measure_built(hydro, env, theme)
        } else {
            LayoutSize::zero()
        };
        LayoutSize::new(
            title.width.max(subtitle.width),
            title.height + subtitle.height,
        )
    } else if bar_height > 0.0 {
        measure_retained_toolbar_group(&state.principal, hydro, env, theme)
    } else {
        LayoutSize::zero()
    };
    let leading_size = measure_retained_toolbar_group(&state.leading, hydro, env, theme);
    let trailing_size = measure_retained_toolbar_group(&state.trailing, hydro, env, theme);
    let bottom_size = measure_retained_toolbar_group(&state.bottom, hydro, env, theme);
    // Measure the retained field itself: a throwaway TextField would allocate on
    // every measure and shape its text in a cache the rendered field never sees.
    let search_size = state
        .search_field
        .as_ref()
        .map_or_else(LayoutSize::zero, |field| {
            field.measure_built(hydro, env, theme)
        });
    let content_size = state.content.measure_built(hydro, env, theme);
    let width = f64::from(content_size.width)
        .max(metrics.item_spacing.mul_add(
            2.0,
            metrics.horizontal_inset.mul_add(
                2.0,
                f64::from(leading_size.width)
                    + f64::from(title_size.width)
                    + f64::from(trailing_size.width),
            ),
        ))
        .max(
            metrics
                .horizontal_inset
                .mul_add(2.0, f64::from(search_size.width)),
        )
        .max(
            metrics
                .horizontal_inset
                .mul_add(2.0, f64::from(bottom_size.width)),
        );
    let bottom_height = if state.bottom.is_empty() {
        0.0
    } else {
        metrics.inline_bar_height
    };
    let height = f64::from(content_size.height) + bar_height + bottom_height;
    ViewDimensions::new(LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    ))
}

fn measure_retained_toolbar_group(
    group: &[RetainedSubview],
    hydro: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> LayoutSize {
    let metrics = theme.navigation_metrics();
    let mut width = 0.0_f64;
    let mut height = 0.0_f64;
    for (index, item) in group.iter().enumerate() {
        let size = item.measure_built(hydro, env, theme);
        if index > 0 {
            width += metrics.item_spacing;
        }
        width += f64::from(size.width);
        height = height.max(f64::from(size.height));
    }
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    )
}

/// Renders a retained navigation view leaf every flush: emits the bar/title a11y
/// (unless hidden) then the bar chrome + content, reading the bar's live signals.
pub fn render_navigation_view_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<NavigationViewRenderState>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        let (hidden_signal, display_mode) = {
            let state = state.borrow();
            (state.hidden.clone(), state.display_mode)
        };
        let theme = ctx.theme();
        navigation_view_accessibility(
            ctx.renderer_mut(),
            Some(render_ctx),
            Some(&theme),
            &hidden_signal,
            display_mode,
            &state.borrow(),
            env,
        );
    }
    render_navigation_view_parts(ctx, state, env);
}

#[expect(
    clippy::similar_names,
    reason = "the names follow the fixture domain vocabulary; renaming would obscure rather than clarify"
)]
#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn render_navigation_view_parts(
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
    let theme = ctx.theme();
    let metrics = theme.navigation_metrics();
    let top_bar_height = if ctx.renderer_mut().read_signal(&hidden_signal) {
        0.0
    } else {
        let base = navigation_base_bar_height_for_display_mode(display_mode, &theme);
        let search_extra = if search.is_some() {
            metrics
                .search_vertical_inset
                .mul_add(2.0, metrics.search_height)
        } else {
            0.0
        };
        base + search_extra
    };
    // The span clamp in `Edge::split_band` covers a band that outgrows
    // bounds — the extent is not clamped to `bounds.height()` here (a bar
    // the keyboard covers past `bounds` still keeps its height).
    let bottom_bar_height = if has_bottom {
        metrics.inline_bar_height
    } else {
        0.0
    };

    // §7.1's multi-edge chrome split — one derivation for both bars:
    // each bar docks to its edge clear of the container region only, so
    // the keyboard covers it instead of lifting it, while the hosted
    // content keeps the laid-out frame minus both bands — clear of both
    // regions — with each bar's edge docked on its band's inner edge.
    let chrome = ctx.chrome_splits([
        (Edge::Top, top_bar_height),
        (Edge::Bottom, bottom_bar_height),
    ]);
    let [top, bottom] = &chrome.bars;
    let (bar_rect, bottom_rect) = (top.band, bottom.band);

    if top_bar_height > 0.0 {
        let base_bar_height = navigation_base_bar_height_for_display_mode(display_mode, &theme);
        let bar_color = Paint::Solid(ctx.renderer_mut().read_signal(&color_signal));
        // §7.1 "Chrome": the bar's surface extends through the regions of
        // the edges a top bar can touch — top, leading, trailing — to the
        // window edge; the separator stays at the bar's inner edge and
        // everything else keeps `bar_rect`.
        let bar_surface = top.surface(Edge::Top);
        {
            let theme = ctx.theme();
            ctx.draw_context(|draw| {
                theme.draw_navigation_bar(&mut *draw, bar_surface, &bar_color);
                let separator = kurbo::Rect::new(
                    bar_surface.x0,
                    (bar_rect.y1 - 1.0).max(bar_rect.y0),
                    bar_surface.x1,
                    bar_rect.y1,
                );
                theme.draw_navigation_bar_separator(&mut *draw, separator);
            });
        }

        let (leading_size, trailing_size) = {
            let mut state = state.borrow_mut();
            let leading = measure_toolbar_group_intrinsic(
                &mut state.leading,
                ctx.renderer_mut(),
                env,
                &theme,
            );
            let trailing = measure_toolbar_group_intrinsic(
                &mut state.trailing,
                ctx.renderer_mut(),
                env,
                &theme,
            );
            (leading, trailing)
        };
        let leading_width = f64::from(leading_size.width);
        let trailing_width = f64::from(trailing_size.width);
        let leading_rect = kurbo::Rect::new(
            bar_rect.x0 + metrics.horizontal_inset,
            bar_rect.y0,
            (bar_rect.x0 + metrics.horizontal_inset + leading_width).min(bar_rect.x1),
            (bar_rect.y0 + base_bar_height).min(bar_rect.y1),
        );
        let trailing_rect = kurbo::Rect::new(
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
                top,
            );
            flush_toolbar_group(
                ctx,
                &mut state.trailing,
                env,
                trailing_rect,
                ToolbarAlignment::Trailing,
                top,
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
            (base_bar_height - title_height).mul_add(0.5, bar_rect.y0)
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
        let title_rect = kurbo::Rect::new(
            title_x0.min(bar_rect.x1),
            title_y0,
            title_x1.max(bar_rect.x0),
            title_y0 + title_height,
        );
        if title_rect.width() > 0.0 && title_rect.height() > 0.0 {
            let mut state = state.borrow_mut();
            if state.principal.is_empty() {
                // The bar title's and subtitle's a11y are emitted by
                // `navigation_view_accessibility`, so suppress the sub-views'
                // own a11y (matching the dispatch path's
                // `dispatch_in_rect_without_accessibility`). Principal items are
                // not title chrome — they flush unsuppressed so their own nodes
                // (buttons, menus) emit like the other toolbar groups.
                #[cfg(feature = "accessibility")]
                ctx.renderer_mut().push_accessibility_suppression();
                flush_title_and_subtitle(ctx, &mut state, env, title_rect, top);
                #[cfg(feature = "accessibility")]
                ctx.renderer_mut().pop_accessibility_suppression();
            } else {
                flush_toolbar_group(
                    ctx,
                    &mut state.principal,
                    env,
                    title_rect,
                    ToolbarAlignment::Center,
                    top,
                );
            }
        }

        if search.is_some() {
            let search_rect = kurbo::Rect::new(
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
                let field_area = top.area_for(search_rect);
                if let Some(field) = state.borrow_mut().search_field.as_mut() {
                    field.place(
                        ctx.renderer_mut(),
                        render_ctx,
                        env,
                        ProposalSize::UNSPECIFIED,
                        search_rect,
                        field_area,
                    );
                }
            }
        }
    }

    let content_rect = chrome.content;
    if content_rect.width() > 0.0 && content_rect.height() > 0.0 {
        let render_ctx = ctx.render_context();
        // §7.1: navigation content is chrome-hosted — it inherits the
        // widget's boundaries, so a scroll surface inside still extends
        // and clears on the edges the bars leave reachable, and an
        // `.ignore_safe_area` inside still releases there; each bar's
        // edge docks on its band's inner edge.
        state.borrow_mut().content.place(
            ctx.renderer_mut(),
            render_ctx,
            env,
            bounded_proposal(content_rect),
            content_rect,
            chrome.content_area,
        );
    }

    if bottom_bar_height > 0.0 {
        let bar_color = Paint::Solid(ctx.renderer_mut().read_signal(&color_signal));
        // §7.1 "Chrome": the bottom bar's surface extends through the
        // regions of the edges a bottom bar can touch — bottom, leading,
        // trailing — to the window edge.
        let bottom_surface = bottom.surface(Edge::Bottom);
        {
            let theme = ctx.theme();
            ctx.draw_context(|draw| {
                theme.draw_navigation_bar(&mut *draw, bottom_surface, &bar_color);
            });
        }
        flush_toolbar_group(
            ctx,
            &mut state.borrow_mut().bottom,
            env,
            bottom_rect,
            ToolbarAlignment::Center,
            bottom,
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
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> LayoutSize {
    let metrics = theme.navigation_metrics();
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
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    )
}

fn flush_toolbar_group(
    ctx: &mut WidgetRenderContext<'_>,
    group: &mut [RetainedSubview],
    env: &Environment,
    bounds: kurbo::Rect,
    alignment: ToolbarAlignment,
    bar: &crate::renderer::ChromeBar,
) {
    if group.is_empty() || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return;
    }
    let metrics = ctx.theme().navigation_metrics();
    let sizes: Vec<LayoutSize> = group
        .iter_mut()
        .map(|item| item.measure_intrinsic(ctx.renderer_mut(), env))
        .collect();
    let total_width = metrics.item_spacing.mul_add(
        crate::num_cast::usize_as_f64(sizes.len().saturating_sub(1)),
        sizes.iter().map(|size| f64::from(size.width)).sum::<f64>(),
    );
    let mut x = match alignment {
        ToolbarAlignment::Leading => bounds.x0,
        ToolbarAlignment::Center => (bounds.width() - total_width).mul_add(0.5, bounds.x0),
        ToolbarAlignment::Trailing => bounds.x1 - total_width,
    }
    .max(bounds.x0);
    for (item, size) in group.iter_mut().zip(sizes) {
        let width = f64::from(size.width).min((bounds.x1 - x).max(0.0));
        let height = f64::from(size.height).min(bounds.height());
        let y = (bounds.height() - height).mul_add(0.5, bounds.y0);
        let rect = kurbo::Rect::new(x, y, x + width, y + height);
        if rect.width() > 0.0 && rect.height() > 0.0 {
            let render_ctx = ctx.render_context();
            let item_area = bar.area_for(rect);
            item.place(
                ctx.renderer_mut(),
                render_ctx,
                env,
                ProposalSize::UNSPECIFIED,
                rect,
                item_area,
            );
        }
        x += width + metrics.item_spacing;
    }
}

/// Stack the title on the subtitle inside `bounds`, the pair centred as a
/// group — the same split `flush_title_and_subtitle` draws at, so the a11y
/// node's bounds match the painted text.
fn title_and_subtitle_rects(
    bounds: kurbo::Rect,
    title_size: LayoutSize,
    subtitle_size: LayoutSize,
) -> (kurbo::Rect, kurbo::Rect) {
    let total_height =
        (f64::from(title_size.height) + f64::from(subtitle_size.height)).min(bounds.height());
    let mut y = (bounds.height() - total_height).mul_add(0.5, bounds.y0);
    let title_height = f64::from(title_size.height).min((bounds.y1 - y).max(0.0));
    let title_rect = kurbo::Rect::new(bounds.x0, y, bounds.x1, y + title_height);
    y += title_height;
    let subtitle_height = f64::from(subtitle_size.height).min((bounds.y1 - y).max(0.0));
    let subtitle_rect = kurbo::Rect::new(bounds.x0, y, bounds.x1, y + subtitle_height);
    (title_rect, subtitle_rect)
}

fn flush_title_and_subtitle(
    ctx: &mut WidgetRenderContext<'_>,
    state: &mut NavigationViewRenderState,
    env: &Environment,
    bounds: kurbo::Rect,
    bar: &crate::renderer::ChromeBar,
) {
    let title_size = state.title.measure_intrinsic(ctx.renderer_mut(), env);
    let subtitle_size = if state.subtitle_present {
        state.subtitle.measure_intrinsic(ctx.renderer_mut(), env)
    } else {
        LayoutSize::zero()
    };
    let (title_rect, subtitle_rect) = title_and_subtitle_rects(bounds, title_size, subtitle_size);
    if title_rect.height() > 0.0 {
        let render_ctx = ctx.render_context();
        let title_area = bar.area_for(title_rect);
        state.title.place(
            ctx.renderer_mut(),
            render_ctx,
            env,
            ProposalSize::UNSPECIFIED,
            title_rect,
            title_area,
        );
    }
    if state.subtitle_present && subtitle_rect.height() > 0.0 {
        let render_ctx = ctx.render_context();
        let subtitle_area = bar.area_for(subtitle_rect);
        state.subtitle.place(
            ctx.renderer_mut(),
            render_ctx,
            env,
            ProposalSize::UNSPECIFIED,
            subtitle_rect,
            subtitle_area,
        );
    }
}

/// Retained adaptive split state for both two- and three-column configurations.
pub struct NavigationSplitRenderState {
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

    pub(crate) fn prebuild(
        &mut self,
        renderer: &mut crate::renderer::SemanticCore,
        env: &Environment,
    ) {
        self.primary.ensure_built(renderer, env);
        self.placeholder.ensure_built(renderer, env);
    }

    /// Materialize the content/detail columns the live selection demands before
    /// any measure pass reads them. Building belongs to a selection change — a
    /// probe only ever reads what is retained — so this runs in the layout-time
    /// prepare pass, and keeps whichever compact variant the column last built.
    pub(crate) fn prepare_columns(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        self.prebuild(renderer, env);
        let primary_selection = self.primary_selection.snapshot();
        if let Some(selected) = primary_selection.filter(|_| self.content_builder.is_some()) {
            let compact = self
                .content
                .as_ref()
                .is_some_and(|(_, compact, _)| *compact);
            self.ensure_content(selected, compact, renderer, env);
        }
        let detail_selection = self
            .secondary_selection
            .as_ref()
            .map_or(primary_selection, Signal::snapshot);
        if let Some(selected) = detail_selection {
            let compact = self.detail.as_ref().is_some_and(|(_, compact, _)| *compact);
            self.ensure_detail(selected, compact, renderer, env);
        }
    }

    fn ensure_content(
        &mut self,
        id: Id,
        compact: bool,
        renderer: &mut crate::renderer::SemanticCore,
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
        renderer: &mut crate::renderer::SemanticCore,
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
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        measure_navigation_split_layout(
            view.as_inner(),
            ProposalSize::UNSPECIFIED,
            state,
            env,
            theme,
        )
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        ViewDimensions::new(measure_navigation_split_layout(
            view.as_inner(),
            proposal,
            state,
            env,
            theme,
        ))
    }
}

/// The column plan a navigation split resolves under a proposal, mirroring
/// [`render_navigation_split_parts`]: whether the pane is compact, whether a
/// three-column split shows all columns, the resolved fixed-column width, and
/// the proposal each column's rect hands its content — the fixed width for the
/// leading columns, the pane remainder for the detail, and always the full
/// proposal height (waterui `docs/layout-spec.md` §7: a column's rect is the
/// proposal for its content).
struct SplitMeasurePlan {
    compact: bool,
    show_all: bool,
    three_column: bool,
    column_proposal: ProposalSize,
    detail_proposal: ProposalSize,
}

fn split_measure_plan(
    three_column: bool,
    column_width: waterui::navigation::ColumnWidth,
    style: waterui::navigation::NativeNavigationSplitStyle,
    visibility: waterui::navigation::NavigationSplitColumnVisibility,
    proposal: ProposalSize,
) -> SplitMeasurePlan {
    use waterui::navigation::NavigationSplitColumnVisibility as Visibility;
    let desired_column_width = resolved_split_column_width(column_width, style);
    let proposal_width = proposal.width.map(f64::from);

    let compact = matches!(visibility, Visibility::DetailOnly)
        || proposal_width.is_some_and(|width| {
            width
                < split_compact_threshold(
                    desired_column_width * if three_column { 2.0 } else { 1.0 },
                )
        });
    let show_all = !three_column
        || matches!(visibility, Visibility::All)
        || (matches!(visibility, Visibility::Automatic)
            && proposal_width
                .is_none_or(|width| width >= split_compact_threshold(desired_column_width * 2.0)));
    let column_count = if three_column && show_all { 3.0 } else { 2.0 };
    let column_width = desired_column_width
        .clamp(f64::from(column_width.min()), f64::from(column_width.max()))
        .min(proposal_width.unwrap_or(f64::MAX) / column_count);
    let fixed_columns = column_count - 1.0;
    SplitMeasurePlan {
        compact,
        show_all,
        three_column,
        column_proposal: ProposalSize::new(
            Some(crate::num_cast::f64_as_f32(column_width)),
            proposal.height,
        ),
        detail_proposal: ProposalSize::new(
            proposal_width.map(|width| {
                crate::num_cast::f64_as_f32(
                    f64::mul_add(column_width, -fixed_columns, width).max(0.0),
                )
            }),
            proposal.height,
        ),
    }
}

/// The retained column a selection occupies — its mounted view when built,
/// the placeholder otherwise (the placeholder also answers for an absent
/// selection).
fn retained_split_column<'a>(
    selected: Option<Id>,
    column: Option<&'a (Id, bool, RetainedSubview)>,
    placeholder: &'a RetainedSubview,
) -> &'a RetainedSubview {
    if selected.is_some() {
        column.map_or(placeholder, |(_, _, view)| view)
    } else {
        placeholder
    }
}

#[expect(
    clippy::option_if_let_else,
    reason = "the if-let/else mirrors the control flow more clearly than the combinator chain here"
)]
// the split layout's column-size resolution is one continuous pass
#[expect(
    clippy::too_many_lines,
    reason = "the layout resolves pane columns and dividers in a single sweep; the length is the enumeration, not logic"
)]
fn measure_navigation_split_layout(
    split: &NavigationSplitLayout,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> LayoutSize {
    let primary_selection = state.measure_signal(split.primary_selection());
    let detail_selection = split
        .secondary_selection()
        .map_or(primary_selection, |selection| {
            state.measure_signal(selection)
        });
    let plan = split_measure_plan(
        split.content_builder().is_some(),
        split.sidebar_width_constraints(),
        split.native_style(),
        state.measure_signal(split.column_visibility_signal()),
        proposal,
    );

    // A dispatch-time measure has no retained columns to read: each column is
    // built transient — normalized then measured — against the proposal its
    // rect would hand it.
    let transient_primary = || normalize_layout_view(split.primary_builder().build(), env);
    let transient_placeholder = || normalize_layout_view(split.placeholder_builder().build(), env);

    if plan.compact {
        let view = if plan.three_column {
            if let Some(selected) = detail_selection {
                normalize_layout_view(AnyView::new(split.detail_builder().build(selected)), env)
            } else if let Some(selected) = primary_selection {
                normalize_layout_view(
                    AnyView::new(
                        split
                            .content_builder()
                            .expect("three-column split must provide a content builder")
                            .build(selected),
                    ),
                    env,
                )
            } else {
                transient_primary()
            }
        } else if let Some(selected) = detail_selection {
            normalize_layout_view(AnyView::new(split.detail_builder().build(selected)), env)
        } else {
            transient_primary()
        };
        return measure_transient_view_with_proposal(&view, proposal, state, env, theme);
    }

    let mut width = 0.0_f64;
    let mut height = 0.0_f64;
    if !plan.three_column || plan.show_all {
        let size = measure_transient_view_with_proposal(
            &transient_primary(),
            plan.column_proposal,
            state,
            env,
            theme,
        );
        width += f64::from(size.width);
        height = height.max(f64::from(size.height));
    }
    if plan.three_column {
        let size = if let Some(selected) = primary_selection {
            measure_owned_navigation_view_with_proposal(
                split
                    .content_builder()
                    .expect("three-column split must provide a content builder")
                    .build(selected),
                plan.column_proposal,
                state,
                env,
                theme,
            )
        } else {
            measure_transient_view_with_proposal(
                &transient_placeholder(),
                plan.column_proposal,
                state,
                env,
                theme,
            )
        };
        width += f64::from(size.width);
        height = height.max(f64::from(size.height));
    }
    let size = if let Some(selected) = detail_selection {
        measure_owned_navigation_view_with_proposal(
            split.detail_builder().build(selected),
            plan.detail_proposal,
            state,
            env,
            theme,
        )
    } else {
        measure_transient_view_with_proposal(
            &transient_placeholder(),
            plan.detail_proposal,
            state,
            env,
            theme,
        )
    };
    width += f64::from(size.width);
    height = height.max(f64::from(size.height));

    LayoutSize::new(
        proposal
            .width
            .unwrap_or_else(|| crate::num_cast::f64_as_f32(width)),
        proposal
            .height
            .unwrap_or_else(|| crate::num_cast::f64_as_f32(height)),
    )
}

pub fn measure_navigation_split_node(
    split: &NavigationSplitRenderState,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    let primary_selection = state.measure_signal(&split.primary_selection);
    let detail_selection = split
        .secondary_selection
        .as_ref()
        .map_or(primary_selection, |selection| {
            state.measure_signal(selection)
        });
    let plan = split_measure_plan(
        split.is_three_column(),
        split.column_width,
        split.style,
        state.measure_signal(&split.visibility),
        proposal,
    );

    // A probe reads the retained, mounted column — `builder.build(selected)`
    // belongs to the selection change, not to measurement — against the
    // proposal the column's rect hands it.
    if plan.compact {
        let column = if plan.three_column {
            if detail_selection.is_some() {
                retained_split_column(detail_selection, split.detail.as_ref(), &split.placeholder)
            } else if primary_selection.is_some() {
                retained_split_column(
                    primary_selection,
                    split.content.as_ref(),
                    &split.placeholder,
                )
            } else {
                &split.primary
            }
        } else {
            retained_split_column(detail_selection, split.detail.as_ref(), &split.placeholder)
        };
        return ViewDimensions::new(
            column.measure_built_with_proposal(state, env, theme, proposal),
        );
    }

    let mut width = 0.0_f64;
    let mut height = 0.0_f64;
    if !plan.three_column || plan.show_all {
        let size =
            split
                .primary
                .measure_built_with_proposal(state, env, theme, plan.column_proposal);
        width += f64::from(size.width);
        height = height.max(f64::from(size.height));
    }
    if plan.three_column {
        let size = retained_split_column(
            primary_selection,
            split.content.as_ref(),
            &split.placeholder,
        )
        .measure_built_with_proposal(state, env, theme, plan.column_proposal);
        width += f64::from(size.width);
        height = height.max(f64::from(size.height));
    }
    let size = retained_split_column(detail_selection, split.detail.as_ref(), &split.placeholder)
        .measure_built_with_proposal(state, env, theme, plan.detail_proposal);
    width += f64::from(size.width);
    height = height.max(f64::from(size.height));

    ViewDimensions::new(LayoutSize::new(
        proposal
            .width
            .unwrap_or_else(|| crate::num_cast::f64_as_f32(width)),
        proposal
            .height
            .unwrap_or_else(|| crate::num_cast::f64_as_f32(height)),
    ))
}

pub fn render_navigation_split_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<NavigationSplitRenderState>>,
    env: &Environment,
) {
    render_navigation_split_parts(ctx, state, env);
}

pub fn render_navigation_split_parts(
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
            &CompactSplitSelection {
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
            kurbo::Rect::new(bounds.x0, bounds.y0, bounds.x0 + column_width, bounds.y1);
        let content_rect = kurbo::Rect::new(
            primary_rect.x1,
            bounds.y0,
            primary_rect.x1 + column_width,
            bounds.y1,
        );
        let detail_rect = kurbo::Rect::new(content_rect.x1, bounds.y0, bounds.x1, bounds.y1);
        (Some(primary_rect), Some(content_rect), detail_rect)
    } else if three_column {
        let content_rect =
            kurbo::Rect::new(bounds.x0, bounds.y0, bounds.x0 + column_width, bounds.y1);
        let detail_rect = kurbo::Rect::new(content_rect.x1, bounds.y0, bounds.x1, bounds.y1);
        (None, Some(content_rect), detail_rect)
    } else {
        let primary_rect =
            kurbo::Rect::new(bounds.x0, bounds.y0, bounds.x0 + column_width, bounds.y1);
        let detail_rect = kurbo::Rect::new(primary_rect.x1, bounds.y0, bounds.x1, bounds.y1);
        (Some(primary_rect), None, detail_rect)
    };

    if let Some(primary_rect) = primary_rect {
        let render_ctx = ctx.render_context();
        // A split column is chrome-hosted content, like navigation content:
        // it inherits the widget's boundaries edge by edge.
        let primary_area = ctx.content_area_for(primary_rect);
        state.borrow_mut().primary.place(
            ctx.renderer_mut(),
            render_ctx,
            env,
            bounded_proposal(primary_rect),
            primary_rect,
            primary_area,
        );
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

/// The pane a collapsed navigation split puts in front — the collapsed
/// mode's single visible column, decided the same way by the rendered and
/// the semantic emit paths.
enum CompactSplitFront {
    /// Only the sidebar shows; there is nothing to navigate back to.
    Sidebar,
    /// The middle column — three-column splits only.
    Content,
    /// The trailing detail column.
    Detail,
}

/// Resolves which pane a collapsed split presents and, when a pane is in
/// front, the selection binding its back button clears to reveal the pane
/// beneath it — the decision `render_compact_split` and the semantic emit
/// share so both presentation paths show the same column.
fn compact_split_front(
    selection: &CompactSplitSelection,
) -> (CompactSplitFront, Option<nami::Binding<Option<Id>>>) {
    if selection.three_column {
        if selection.secondary.is_some() {
            (
                CompactSplitFront::Detail,
                selection.secondary_binding.clone(),
            )
        } else if selection.primary.is_some() {
            (
                CompactSplitFront::Content,
                Some(selection.primary_binding.clone()),
            )
        } else {
            (CompactSplitFront::Sidebar, None)
        }
    } else if selection.primary.is_some() {
        (
            CompactSplitFront::Detail,
            Some(selection.primary_binding.clone()),
        )
    } else {
        (CompactSplitFront::Sidebar, None)
    }
}

/// The back button a collapsed split presents: a Button-role node with the
/// Focus/Click actions every actionable control carries, labeled like the
/// stack's back affordance.
#[cfg(feature = "accessibility")]
fn split_back_accessibility_node(env: &Environment) -> AccessibilityNode {
    let mut node =
        AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
            env,
            AccessibilityNodeRole::Button,
        ));
    node.set_label(crate::localization::text(env, "back"));
    node.add_action(AccessibilityAction::Focus);
    node.add_action(AccessibilityAction::Click);
    node
}

/// The `Activate` target behind a collapsed split's back node: clearing the
/// front pane's selection binding is what presents the pane beneath it —
/// the same navigation the pointer target performs, so `Click` needs no
/// bounds.
#[cfg(feature = "accessibility")]
fn split_back_action_target(selection: nami::Binding<Option<Id>>) -> AccessibilityActionTarget {
    AccessibilityActionTarget::Activate {
        action: Rc::new(RefCell::new(
            move |_renderer: &mut crate::renderer::SemanticCore, _env: &Environment| {
                selection.set(None);
                true
            },
        )),
    }
}

fn render_compact_split(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<NavigationSplitRenderState>>,
    env: &Environment,
    selection: &CompactSplitSelection,
) {
    let bounds = ctx.bounds;
    let mut compact_env = env.clone();
    compact_env.insert(NavigationLeadingReserve(back_button_title_reserve(
        &ctx.theme(),
    )));
    let (front, back_selection) = compact_split_front(selection);
    #[cfg(feature = "accessibility")]
    if let Some(back_binding) = back_selection.clone() {
        let back_rect = navigation_back_button_rect(bounds, ctx.theme().navigation_metrics());
        // The back node precedes the front pane's nodes in the tree, the
        // same order the navigation stack emits its back button in.
        let _ = ctx.renderer_mut().register_accessibility_node(
            split_back_accessibility_node(env),
            back_rect,
            env,
            Some(split_back_action_target(back_binding)),
        );
    }
    match front {
        CompactSplitFront::Detail => {
            let detail_selection = if selection.three_column {
                selection.secondary
            } else {
                selection.primary
            };
            render_split_detail(ctx, state, &compact_env, detail_selection, true, bounds);
        }
        CompactSplitFront::Content => {
            render_split_content(ctx, state, &compact_env, selection.primary, true, bounds);
        }
        CompactSplitFront::Sidebar => {
            let render_ctx = ctx.render_context();
            let pane_area = ctx.content_area_for(bounds);
            state.borrow_mut().primary.place(
                ctx.renderer_mut(),
                render_ctx,
                env,
                bounded_proposal(bounds),
                bounds,
                pane_area,
            );
        }
    }

    if let Some(selection) = back_selection {
        let back_rect = navigation_back_button_rect(bounds, ctx.theme().navigation_metrics());
        {
            let theme = ctx.theme();
            ctx.draw_context(|draw| {
                theme.draw_navigation_back_button(&mut *draw, back_rect);
            });
        }
        ctx.renderer_mut()
            .register_pointer_target(back_rect, move |_renderer, _point, _| {
                selection.set(None);
                true
            });
    }
}

fn render_split_content(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<NavigationSplitRenderState>>,
    env: &Environment,
    selected: Option<Id>,
    compact: bool,
    bounds: kurbo::Rect,
) {
    if let Some(selected) = selected {
        let mut state = state.borrow_mut();
        state.ensure_content(selected, compact, ctx.renderer_mut(), env);
        let render_ctx = ctx.render_context();
        let pane_area = ctx.content_area_for(bounds);
        state
            .content
            .as_mut()
            .expect("selected split content must be retained")
            .2
            .place(
                ctx.renderer_mut(),
                render_ctx,
                env,
                bounded_proposal(bounds),
                bounds,
                pane_area,
            );
    } else {
        let render_ctx = ctx.render_context();
        let pane_area = ctx.content_area_for(bounds);
        state.borrow_mut().placeholder.place(
            ctx.renderer_mut(),
            render_ctx,
            env,
            bounded_proposal(bounds),
            bounds,
            pane_area,
        );
    }
}

fn render_split_detail(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<NavigationSplitRenderState>>,
    env: &Environment,
    selected: Option<Id>,
    compact: bool,
    bounds: kurbo::Rect,
) {
    if let Some(selected) = selected {
        let mut state = state.borrow_mut();
        state.ensure_detail(selected, compact, ctx.renderer_mut(), env);
        let render_ctx = ctx.render_context();
        let pane_area = ctx.content_area_for(bounds);
        state
            .detail
            .as_mut()
            .expect("selected split detail must be retained")
            .2
            .place(
                ctx.renderer_mut(),
                render_ctx,
                env,
                bounded_proposal(bounds),
                bounds,
                pane_area,
            );
    } else {
        let render_ctx = ctx.render_context();
        let pane_area = ctx.content_area_for(bounds);
        state.borrow_mut().placeholder.place(
            ctx.renderer_mut(),
            render_ctx,
            env,
            bounded_proposal(bounds),
            bounds,
            pane_area,
        );
    }
}

/// The retained render state of a `NavigationStack`. Both the root and every pushed
/// destination own a persistent render node, so inactive pages retain local widget
/// state and a transition never reconstructs its source or destination subtree.
pub struct NavigationStackRenderState {
    unresolved_root: Option<AnyView>,
    root: Option<RetainedSubview>,
    background: Option<Computed<WorkingColor>>,
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
            // The root is never pushed, so it has no arrival to style.
            transition: _,
        } = resolve_navigation_root(unresolved_root, env);
        self.background = Some(Color::new(Background).resolve(env));
        self.root = Some(RetainedSubview::new(AnyView::new(NavigationView {
            bar,
            content,
            state: NavigationDestinationState::default(),
            transition: None,
        })));
        Some(state)
    }

    const fn root_mut(&mut self) -> &mut RetainedSubview {
        self.root
            .as_mut()
            .expect("Hydrolysis navigation root must be resolved before rendering")
    }

    fn background(&self) -> Computed<WorkingColor> {
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

/// How the stack presents one page this frame.
struct PagePresentation {
    /// The page scope's layer transform in the stack's record space.
    layer: kurbo::Affine,
    alpha: f32,
    clip: Option<kurbo::Rect>,
    /// The space `clip` is given in.
    clip_space: kurbo::Affine,
    /// Whether the page takes input: only the active page does.
    hit: bool,
    /// The matched elements a transition lists outside this page.
    matched: Vec<(bool, Id)>,
}

/// Records one page into its `("page", identity)` scope: its background,
/// the page's retained root, and the back chevron on top of a pushed page.
#[expect(
    clippy::too_many_arguments,
    reason = "the call carries the page record's inputs; the safe-area context is the §7.1 addition that tips the count"
)]
fn record_navigation_page(
    renderer: &mut HydrolysisRenderer,
    state: &Rc<RefCell<NavigationStackRenderState>>,
    slot_key: &crate::renderer::NavigationKey,
    identity: u64,
    env: &Environment,
    size: LayoutSize,
    safe_area: Option<SafeAreaLayout>,
    presented: PagePresentation,
) -> crate::renderer::navigation_state::NavigationRecordedPage {
    let bounds = kurbo::Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height));
    let background = state.borrow().background();
    let background = Paint::Solid(renderer.read_signal(&background));
    let clip = presented.clip.map(crate::renderer::ScopeClip::Rect);
    renderer.open_scope_with(
        crate::renderer::mount::ScopeKey {
            role: "page",
            item: identity,
        },
        presented.layer,
        presented.alpha,
        clip.as_ref(),
        presented.clip_space,
        crate::renderer::mount::ScopeDelta::RECORD_SPACE,
        presented.hit,
    );
    #[cfg(feature = "accessibility")]
    if !presented.hit {
        renderer.push_accessibility_suppression();
    }
    renderer.begin_navigation_page(presented.matched);
    let origin = renderer.program().origin();
    renderer
        .scene_mut()
        .fill_paint(peniko::Fill::NonZero, origin, background, &bounds);
    if identity == 0 {
        state
            .borrow_mut()
            .root_mut()
            .record_built_page(renderer, env, size, safe_area);
    } else {
        let (entries, pending_removed) = {
            let slot = renderer
                .navigation
                .slots
                .get(slot_key)
                .expect("Hydrolysis navigation slot missing during page record");
            (Rc::clone(&slot.entries), Rc::clone(&slot.pending_removed))
        };
        let mut entries = entries.borrow_mut();
        let mut pending_removed = pending_removed.borrow_mut();
        // A pop's departing entry has already moved to `pending_removed`; it
        // stays retained there until the transaction completes, so it is
        // still a valid page.
        let entry = entries
            .iter_mut()
            .chain(pending_removed.iter_mut())
            .find(|entry| entry.identity == identity)
            .unwrap_or_else(|| {
                panic!("Hydrolysis navigation entry identity {identity} is not retained")
            });
        entry
            .content
            .record_built_page(renderer, env, size, safe_area);
        let context = RenderContext {
            local: origin,
            bounds,
        };
        let theme = renderer.theme();
        renderer.draw_context(context, |draw| {
            theme.draw_navigation_back_button(
                draw,
                navigation_back_button_rect(bounds, theme.navigation_metrics()),
            );
        });
    }
    let page = renderer.finish_navigation_page();
    #[cfg(feature = "accessibility")]
    if !presented.hit {
        renderer.pop_accessibility_suppression();
    }
    renderer.close_scope();
    page
}

/// The stack inputs every page record of one frame shares.
struct PageRecorder<'a> {
    state: &'a Rc<RefCell<NavigationStackRenderState>>,
    slot_key: &'a crate::renderer::NavigationKey,
    size: LayoutSize,
}

impl PageRecorder<'_> {
    fn record(
        &self,
        renderer: &mut HydrolysisRenderer,
        identity: u64,
        env: &Environment,
        safe_area: Option<SafeAreaLayout>,
        presented: PagePresentation,
    ) -> crate::renderer::navigation_state::NavigationRecordedPage {
        record_navigation_page(
            renderer,
            self.state,
            self.slot_key,
            identity,
            env,
            self.size,
            safe_area,
            presented,
        )
    }
}

/// One matched-geometry transition frame.
struct MatchedFrame {
    id: Id,
    direction: NavigationTransitionDirection,
    progress: f64,
    /// The stack's record space.
    transform: kurbo::Affine,
    paint_bounds: kurbo::Rect,
    active_identity: u64,
}

/// Presents a matched-geometry frame: both pages fade in their page scopes
/// and the element travels between them in its `matched-from`/`matched-to`
/// scopes.
fn present_matched_transition(
    renderer: &mut HydrolysisRenderer,
    pages: &PageRecorder<'_>,
    page_area: Option<SafeAreaLayout>,
    [(from_identity, from_env), (to_identity, to_env)]: [(u64, &Environment); 2],
    MatchedFrame {
        id,
        direction,
        progress,
        transform,
        paint_bounds,
        active_identity,
    }: MatchedFrame,
) {
    use crate::renderer::interpolate_rect;
    let (from_source, to_source) = match direction {
        NavigationTransitionDirection::Push => (true, false),
        NavigationTransitionDirection::Pop => (false, true),
    };
    let incoming = crate::num_cast::f64_as_f32(progress);
    let fade = |alpha: f32, identity: u64, source: bool| PagePresentation {
        layer: transform,
        alpha,
        clip: Some(paint_bounds),
        clip_space: transform,
        hit: identity == active_identity,
        matched: vec![(source, id)],
    };
    let from_page = pages.record(
        renderer,
        from_identity,
        from_env,
        page_area.clone(),
        fade(1.0 - incoming, from_identity, from_source),
    );
    let to_page = pages.record(
        renderer,
        to_identity,
        to_env,
        page_area,
        fade(incoming, to_identity, to_source),
    );
    let from_element = from_page.element(from_source, id).unwrap_or_else(|| {
        panic!("navigation zoom source {id:?} is not present in the outgoing page")
    });
    let to_element = to_page.element(to_source, id).unwrap_or_else(|| {
        panic!("navigation zoom destination {id:?} is not present in the incoming page")
    });
    assert!(
        from_element.bounds.width() > 0.0
            && from_element.bounds.height() > 0.0
            && to_element.bounds.width() > 0.0
            && to_element.bounds.height() > 0.0,
        "navigation zoom geometry must have a positive size"
    );
    let target = interpolate_rect(from_element.bounds, to_element.bounds, progress);
    let item = matched_scope_item(id);
    present_matched_element(
        renderer,
        crate::renderer::mount::ScopeKey {
            role: "matched-from",
            item,
        },
        transform,
        from_element,
        target,
        1.0 - incoming,
    );
    present_matched_element(
        renderer,
        crate::renderer::mount::ScopeKey {
            role: "matched-to",
            item,
        },
        transform,
        to_element,
        target,
        incoming,
    );
}

/// Lists a matched element in its own scope (§D.2): the element keeps its
/// structural placement for hit testing while its paint parent moves to the
/// scope, which flies it from its page frame onto `target`.
fn present_matched_element(
    renderer: &mut HydrolysisRenderer,
    key: crate::renderer::mount::ScopeKey,
    transform: kurbo::Affine,
    element: &crate::renderer::navigation_state::NavigationMatchedElement,
    target: kurbo::Rect,
    alpha: f32,
) {
    let local = crate::renderer::matched_element_transform(element.bounds, target);
    renderer.open_scope_with(
        key,
        transform * local * element.to_page * element.frame.inverse(),
        alpha,
        Some(&crate::renderer::ScopeClip::Rect(target)),
        transform,
        crate::renderer::mount::ScopeDelta::RECORD_SPACE,
        false,
    );
    let placement = renderer.current_placement();
    element
        .cell
        .placement()
        .set_paint_parent(Some(Rc::clone(&placement)));
    *element.cell.retained().anchor.borrow_mut() = Some(placement);
    renderer.program().push_node(Rc::clone(&element.cell));
    renderer.close_scope();
}

fn matched_scope_item(id: Id) -> u64 {
    use core::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    id.hash(&mut hasher);
    hasher.finish()
}

type NavigationPresentationFrame = (
    AnyNavigationTransition,
    NavigationTransitionDirection,
    f64,
    crate::renderer::navigation_state::NavigationPage,
    crate::renderer::navigation_state::NavigationPage,
);

/// Records the pages the stack shows this frame, each in its page scope:
/// the active page alone, or a transition's two pages under their layers.
#[expect(
    clippy::too_many_arguments,
    reason = "the call carries the stack's per-frame presentation inputs"
)]
fn present_navigation_pages(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<NavigationStackRenderState>>,
    slot_key: &crate::renderer::NavigationKey,
    stack_env: &Environment,
    active_identity: u64,
    size: LayoutSize,
    page_area: Option<SafeAreaLayout>,
    motion: waterui_backend_core::widget::NavigationMotion,
    presentation: Option<NavigationPresentationFrame>,
) {
    use crate::renderer::{
        NavigationPresentation, navigation_presentation, transition_layer_transform,
    };
    let transform = ctx.local;
    let bounds = ctx.bounds;
    // The transition's page clips and the stack's backdrop fill cover the
    // same reach — `chrome_paint_bounds` — so an extended bar surface is
    // never clipped mid-animation. `bounds` stays the scale-centre reference.
    let paint_bounds = ctx.chrome_paint_bounds();
    let theme = ctx.theme();
    let renderer = ctx.renderer_mut();
    let Some((style, direction, progress, from, to)) = presentation else {
        let env = presented_page_env(stack_env, active_identity, &theme);
        record_navigation_page(
            renderer,
            state,
            slot_key,
            active_identity,
            &env,
            size,
            page_area,
            PagePresentation {
                layer: transform,
                alpha: 1.0,
                clip: None,
                clip_space: transform,
                hit: true,
                matched: Vec::new(),
            },
        );
        return;
    };
    let from_env = presented_page_env(stack_env, from.identity, &theme);
    let to_env = presented_page_env(stack_env, to.identity, &theme);
    match navigation_presentation(&style, motion, direction, progress, paint_bounds.width()) {
        NavigationPresentation::Matched(id) => present_matched_transition(
            renderer,
            &PageRecorder {
                state,
                slot_key,
                size,
            },
            page_area,
            [(from.identity, &from_env), (to.identity, &to_env)],
            MatchedFrame {
                id,
                direction,
                progress,
                transform,
                paint_bounds,
                active_identity,
            },
        ),
        NavigationPresentation::Layers(resolved) => {
            let outgoing = (&from, &from_env, resolved.outgoing);
            let incoming = (&to, &to_env, resolved.incoming);
            let order = match direction {
                NavigationTransitionDirection::Push => [outgoing, incoming],
                NavigationTransitionDirection::Pop => [incoming, outgoing],
            };
            for (page, env, layer) in order {
                let local = transition_layer_transform(bounds, paint_bounds, layer);
                record_navigation_page(
                    renderer,
                    state,
                    slot_key,
                    page.identity,
                    env,
                    size,
                    page_area.clone(),
                    PagePresentation {
                        layer: transform * local,
                        alpha: layer.opacity,
                        clip: Some(local.transform_rect_bbox(paint_bounds)),
                        clip_space: transform,
                        hit: page.identity == active_identity,
                        matched: Vec::new(),
                    },
                );
            }
        }
    }
}

impl HydroNativeView for Native<NavigationStack<(), ()>> {
    fn intrinsic(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        LayoutSize::zero()
    }
}

/// Binds the navigation stack by retained semantic identity and emits the
/// back-button accessibility node when the stack is non-empty. The rendered
/// `Widget`-node path passes its [`RenderContext`]; the semantic emission walk
/// passes `None` — the back button's `Click` pops the stack directly, so it
/// needs no bounds.
pub fn navigation_stack_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    theme: Option<&Rc<dyn crate::engine::WidgetTheme>>,
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
        let mut back_node =
            AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::Button,
            ));
        back_node.set_label(crate::localization::text(env, "back"));
        back_node.add_action(AccessibilityAction::Focus);
        back_node.add_action(AccessibilityAction::Click);
        // Direct semantic activation: pop through the slot's controller, the
        // same path the pointer target takes, so `Click` works with no bounds.
        let action_target = renderer
            .navigation
            .slots
            .get(&slot_key)
            .map(|slot| slot.controller.clone())
            .map(|controller| {
                let back_slot_key = slot_key;
                AccessibilityActionTarget::Activate {
                    action: Rc::new(RefCell::new(
                        move |renderer: &mut crate::renderer::SemanticCore, env: &Environment| {
                            // A denied pop is a handled activation whose
                            // outcome is "attempt reported, destination
                            // stays" — `attempt_pop` already fired
                            // `pop_attempted`. `request_pop` runs only when
                            // the destination allows it.
                            if renderer.attempt_navigation_pop(&back_slot_key, env) {
                                controller.request_pop(1);
                            }
                            true
                        },
                    )),
                }
            });
        match ctx {
            Some(ctx) => {
                let metrics = theme
                    .expect("rendered navigation stack accessibility passes the theme")
                    .navigation_metrics();
                let back_bounds = navigation_back_button_rect(ctx.bounds, metrics);
                let _ = renderer.register_accessibility_node(
                    back_node,
                    back_bounds,
                    env,
                    action_target,
                );
            }
            None => {
                let _ =
                    renderer.register_accessibility_node_semantic(back_node, env, action_target);
            }
        }
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, theme, depth, env);
    }
}

/// Measures a navigation stack leaf (zero intrinsic, matching the dispatch path).
pub fn measure_navigation_stack_node(
    _state: &NavigationStackRenderState,
    _proposal: ProposalSize,
    _hydro: &mut HydroState,
    _env: &Environment,
    _theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    ViewDimensions::new(LayoutSize::zero())
}

/// Renders a retained navigation stack leaf every flush. Stack accessibility is not
/// render-driven and additionally binds the navigation entries the render consumes,
/// so this node always runs `navigation_stack_accessibility` first (mirroring the
/// dispatch wrapper's `accessibility`-then-`render` order); when the stack is
/// accessibility-hidden it runs that step inside a suppression scope so the entries
/// are still bound while the a11y nodes are suppressed.
pub fn render_navigation_stack_node(
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
        let theme = ctx.theme();
        let render_ctx = ctx.render_context();
        navigation_stack_accessibility(
            ctx.renderer_mut(),
            Some(render_ctx),
            Some(&theme),
            state,
            env,
        );
    }
    #[cfg(feature = "accessibility")]
    if hidden {
        ctx.renderer_mut().pop_accessibility_suppression();
    }
    render_navigation_stack_parts(ctx, state, env);
}

#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn render_navigation_stack_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<NavigationStackRenderState>>,
    env: &Environment,
) {
    let transition_style = state.borrow().transition_style.clone();
    let transition_motion = ctx.theme().navigation_motion();
    let slot_key = crate::renderer::NavigationKey::for_rc(state);
    let entries = ctx.renderer_mut().bind_navigation_entries(&slot_key);

    let mut stack_env = env.clone();
    let controller = ctx
        .renderer_mut()
        .navigation
        .slots
        .get(&slot_key)
        .expect("hydrolysis navigation slot missing")
        .controller
        .clone();
    if let Some(retained_env) = controller.retained_environment() {
        // The retained environment was captured when the stack's subtree was
        // built; the keys it scoped there (the `State<Navigator>` the path
        // wiring installs, anything the caller scoped the stack under) must
        // keep precedence — an enclosing context must not be able to shadow
        // the stack's own navigator. The live render environment carries
        // whatever the current flush injects — the compact split's
        // `NavigationLeadingReserve` is one — so it supplies the keys the
        // retained scope lacks. Replaying the retained overlays on top of the
        // live environment gives exactly that precedence, for every depth
        // and for path-backed and plain stacks alike.
        stack_env = retained_env.layered_on(env);
    }
    stack_env.insert(controller);

    if let Some(root_state) = state.borrow_mut().resolve_root(&stack_env) {
        ctx.renderer_mut()
            .install_navigation_root_state(&slot_key, root_state);
    }

    let depth = entries.borrow().len();
    let active_identity = if depth == 0 {
        0
    } else {
        navigation_entry_identity(&entries, depth - 1)
    };
    let local_env = presented_page_env(&stack_env, active_identity, &ctx.theme());

    // Pages are recorded in their own local space and replayed at the stack's
    // transform; the hit targets and accessibility bounds a page registers
    // while it records are live, so they anchor at the stack's placement —
    // the inactive ones under their unhittable scope, which gates them out.
    let page_size = LayoutSize::new(
        crate::num_cast::f64_as_f32(ctx.bounds.width()),
        crate::num_cast::f64_as_f32(ctx.bounds.height()),
    );
    // A captured page is chrome-hosted content: it inherits the stack's
    // boundaries, so a scroll surface inside a page extends to the window
    // edge on the edges it touches and `.ignore_safe_area` inside releases
    // there.
    let page_area = ctx.content_area_for(ctx.bounds);
    let background = state.borrow().background();
    let background = Paint::Solid(ctx.renderer_mut().read_signal(&background));
    let transform = ctx.local;
    // §7.1 "Chrome": the stack's backdrop paints what the pages paint —
    // `chrome_paint_bounds`, the same reach the transition page clips
    // cover — so a bar surface extended to the window edge never lands on
    // the window background.
    let paint_bounds = ctx.chrome_paint_bounds();
    ctx.renderer_mut().scene_mut().fill_paint(
        peniko::Fill::NonZero,
        transform,
        background,
        &paint_bounds,
    );

    let navigation_change =
        ctx.renderer_mut()
            .apply_navigation_events(&slot_key, active_identity, &local_env);

    ctx.renderer_mut()
        .activate_navigation_root_if_needed(&slot_key, &local_env);

    let active_page = crate::renderer::navigation_state::NavigationPage {
        identity: active_identity,
    };

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
            if transition_style.retained() == RetainedNavigationTransition::None || skip_transition
            {
                slot.transition = None;
            } else {
                let direction = if depth >= previous_depth {
                    NavigationTransitionDirection::Push
                } else {
                    NavigationTransitionDirection::Pop
                };
                let from_page = crate::renderer::navigation_state::NavigationPage {
                    identity: previous_identity,
                };
                // A destination may declare how it arrives, and a matched
                // transition has to: the pair it names differs per destination,
                // so the stack cannot name it once for all of them. The moving
                // destination owns the motion in both directions — arriving on a
                // push, leaving on a pop — so a pop replays what the departing
                // one declared, which is what makes the two halves symmetric.
                let moving_identity = if direction == NavigationTransitionDirection::Push {
                    active_identity
                } else {
                    previous_identity
                };
                let declared = entries
                    .borrow()
                    .iter()
                    .find(|entry| entry.identity == moving_identity)
                    .and_then(|entry| entry.transition.clone());
                let style = declared
                    .or_else(|| {
                        // On a pop the departing entry has already left
                        // `entries`; it is held here until its teardown runs,
                        // which is the only place its declaration survives.
                        slot.pending_removed
                            .borrow()
                            .iter()
                            .find(|entry| entry.identity == moving_identity)
                            .and_then(|entry| entry.transition.clone())
                    })
                    .unwrap_or_else(|| transition_style.clone());
                slot.transition = Some(
                    crate::renderer::navigation_state::NavigationTransitionState::new(
                        style,
                        direction,
                        from_page,
                        active_page.clone(),
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
                transition.from_page.clone(),
                transition.to_page.clone(),
            )
        });
        slot.last_page = Some(active_page.clone());
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
                interactive.from_page.clone(),
                interactive.to_page.clone(),
            )
        })
    };

    let presentation = interactive_frame
        .as_ref()
        .map(|(progress, _, _, from_page, to_page)| {
            (
                transition_style.clone(),
                NavigationTransitionDirection::Pop,
                *progress,
                from_page.clone(),
                to_page.clone(),
            )
        })
        .or(transition_frame);
    present_navigation_pages(
        ctx,
        state,
        &slot_key,
        &stack_env,
        active_identity,
        page_size,
        page_area,
        transition_motion,
        presentation,
    );
    if let Some((_, completed, cancelled, _, _)) = interactive_frame {
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
    }

    if depth == 0 {
        return;
    }

    let previous_identity = if depth == 1 {
        0
    } else {
        navigation_entry_identity(&entries, depth - 2)
    };
    let previous_page = crate::renderer::navigation_state::NavigationPage {
        identity: previous_identity,
    };

    let metrics = ctx.theme().navigation_metrics();
    let edge_rect = kurbo::Rect::new(
        ctx.bounds.x0,
        ctx.bounds.y0,
        (ctx.bounds.x0 + metrics.back_button_size).min(ctx.bounds.x1),
        ctx.bounds.y1,
    );
    // The edge-gesture point lands in window space and `edge_rect` is
    // node-local: resolve the inverse through the region's live placement
    // at invoke time — a mid-transition re-record leaves the recorded
    // transform stale.
    let placement = ctx.renderer_mut().current_placement();
    let active_page_for_gesture = active_page;
    let previous_page_for_gesture = previous_page;
    let navigation_width = ctx.bounds.width();
    let drag_slot_key = slot_key.clone();
    let back_from_page = active_page_for_gesture.clone();
    let back_to_page = previous_page_for_gesture.clone();
    let controller = ctx
        .renderer_mut()
        .navigation
        .slots
        .get(&slot_key)
        .expect("Hydrolysis navigation slot missing")
        .controller
        .clone();
    ctx.renderer_mut()
        .register_pointer_drag_target(edge_rect, move |renderer, point, pop_env| {
            let point = placement.resolved_transform(true).inverse() * point;
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
                        active_page_for_gesture.clone(),
                        previous_page_for_gesture.clone(),
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
        });
    ctx.renderer_mut().register_back_target(
        crate::renderer::navigation_state::NavigationBackTarget {
            owner: std::rc::Weak::new(),
            slot_key: slot_key.clone(),
            width: navigation_width,
            from_page: back_from_page,
            to_page: back_to_page,
            controller: controller.clone(),
        },
    );

    let back_button_rect = navigation_back_button_rect(ctx.bounds, metrics);
    let back_hit_rect = back_button_rect;
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

/// Emits a retained navigation view's accessibility nodes for the semantic
/// walk: the bar, title, and subtitle nodes `navigation_view_accessibility`
/// registers, then the sub-views that flush unsuppressed in the rendered path —
/// toolbar items, the search field, and the screen content. The title and
/// subtitle sub-views flush under suppression (their semantics live on the
/// bar's own nodes), so they emit nothing here.
#[cfg(feature = "accessibility")]
pub fn emit_navigation_view_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<NavigationViewRenderState>>,
    env: &Environment,
) {
    if env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden)
    {
        return;
    }
    let mut state = state.borrow_mut();
    let (hidden_signal, display_mode) = (state.hidden.clone(), state.display_mode);
    navigation_view_accessibility(
        renderer,
        None,
        None,
        &hidden_signal,
        display_mode,
        &state,
        env,
    );
    for item in &mut state.leading {
        item.emit_accessibility(renderer, env);
    }
    for item in &mut state.principal {
        item.emit_accessibility(renderer, env);
    }
    for item in &mut state.trailing {
        item.emit_accessibility(renderer, env);
    }
    for item in &mut state.bottom {
        item.emit_accessibility(renderer, env);
    }
    if let Some(field) = state.search_field.as_mut() {
        field.emit_accessibility(renderer, env);
    }
    state.content.emit_accessibility(renderer, env);
}

/// Emits a retained navigation split's accessibility nodes for the semantic
/// walk — the panes the platform presents, and only those. Expanded, the
/// sidebar, the selected content, and the selected detail or placeholder all
/// emit. Collapsed, only the frontmost pane emits, with the back button that
/// presents the pane beneath it — the shape `render_compact_split` produces
/// by placing a single pane. A pane that is never emitted stays unplaced, so
/// its subtree retires and never reaches the accessibility tree, exactly as
/// in the rendered path.
#[cfg(feature = "accessibility")]
pub fn emit_navigation_split_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<NavigationSplitRenderState>>,
    env: &Environment,
) {
    let (selection, compact) = {
        let state = state.borrow();
        let three_column = state.is_three_column();
        let visibility = renderer.read_signal(&state.visibility);
        let primary_selection = renderer.read_signal(&state.primary_selection);
        let secondary_binding = state.secondary_selection.clone();
        let secondary_selection = secondary_binding
            .as_ref()
            .and_then(|binding| renderer.read_signal(binding));
        // The rendered path resolves the mode from the split's laid-out
        // width; the semantic walk has no layout, so the declared window
        // frame stands in — a root split reads identically either way.
        let desired_column_width = resolved_split_column_width(state.column_width, state.style);
        let automatic_compact = renderer.window_frame().width()
            < split_compact_threshold(desired_column_width * if three_column { 2.0 } else { 1.0 });
        let compact = automatic_compact
            || matches!(
                visibility,
                waterui::navigation::NavigationSplitColumnVisibility::DetailOnly
            );
        (
            CompactSplitSelection {
                primary: primary_selection,
                secondary: secondary_selection,
                primary_binding: state.primary_selection.clone(),
                secondary_binding,
                three_column,
            },
            compact,
        )
    };
    let mut state = state.borrow_mut();
    if compact {
        let (front, back_binding) = compact_split_front(&selection);
        // The back node precedes the front pane's nodes in the tree, the
        // same order the rendered path registers it in.
        if let Some(back_binding) = back_binding {
            let _ = renderer.register_accessibility_node_semantic(
                split_back_accessibility_node(env),
                env,
                Some(split_back_action_target(back_binding)),
            );
        }
        match front {
            CompactSplitFront::Detail => {
                let selected = if selection.three_column {
                    selection.secondary
                } else {
                    selection.primary
                }
                .expect("a detail front implies a selection");
                state.ensure_detail(selected, true, renderer, env);
                state
                    .detail
                    .as_mut()
                    .expect("selected split detail must be retained")
                    .2
                    .emit_accessibility(renderer, env);
            }
            CompactSplitFront::Content => {
                let selected = selection
                    .primary
                    .expect("a content front implies a primary selection");
                state.ensure_content(selected, true, renderer, env);
                state
                    .content
                    .as_mut()
                    .expect("selected split content must be retained")
                    .2
                    .emit_accessibility(renderer, env);
            }
            CompactSplitFront::Sidebar => {
                state.primary.emit_accessibility(renderer, env);
            }
        }
        return;
    }
    state.primary.emit_accessibility(renderer, env);
    // The middle column exists only on a three-column split; a two-column
    // split's primary selection drives the detail column, exactly as the
    // rendered path routes it.
    if state.is_three_column()
        && let Some(selected) = selection.primary
    {
        state.ensure_content(selected, false, renderer, env);
        state
            .content
            .as_mut()
            .expect("selected split content must be retained")
            .2
            .emit_accessibility(renderer, env);
    }
    let detail_selection = if state.is_three_column() {
        selection.secondary
    } else {
        selection.primary
    };
    if let Some(selected) = detail_selection {
        state.ensure_detail(selected, false, renderer, env);
        state
            .detail
            .as_mut()
            .expect("selected split detail must be retained")
            .2
            .emit_accessibility(renderer, env);
    } else {
        state.placeholder.emit_accessibility(renderer, env);
    }
}

/// Emits a retained navigation stack's accessibility nodes for the semantic
/// walk: the back button `navigation_stack_accessibility` registers (which
/// also binds the slot's entries), then the active page's subtree — the root
/// when the stack is empty, the topmost pushed destination otherwise.
/// Transition scenes are presentation and emit nothing.
#[cfg(feature = "accessibility")]
pub fn emit_navigation_stack_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<NavigationStackRenderState>>,
    env: &Environment,
) {
    navigation_stack_accessibility(renderer, None, None, state, env);
    let slot_key = crate::renderer::NavigationKey::for_rc(state);
    let entries = renderer.bind_navigation_entries(&slot_key);
    let mut local_env = env.clone();
    let controller = renderer
        .navigation
        .slots
        .get(&slot_key)
        .expect("hydrolysis navigation slot missing")
        .controller
        .clone();
    if let Some(retained_env) = controller.retained_environment() {
        // Same merge as the render path: stack-scoped keys keep precedence,
        // keys injected for this walk (e.g. `NavigationLeadingReserve`) fill
        // the gaps.
        local_env = retained_env.layered_on(&local_env);
    }
    local_env.insert(controller);
    if let Some(root_state) = state.borrow_mut().resolve_root(&local_env) {
        renderer.install_navigation_root_state(&slot_key, root_state);
    }
    let depth = entries.borrow().len();
    let active_identity = if depth == 0 {
        ROOT_NAVIGATION_IDENTITY
    } else {
        navigation_entry_identity(&entries, depth - 1)
    };
    renderer.apply_navigation_events(&slot_key, active_identity, &local_env);
    renderer.activate_navigation_root_if_needed(&slot_key, &local_env);
    // A semantic emit completes the transaction in place — there is no
    // scene transition to await, so `popped`/`appeared` and the controller
    // acknowledgement fire with the emit that shows the result.
    renderer.complete_navigation_transaction(&slot_key, &local_env);
    if depth == 0 {
        state
            .borrow_mut()
            .root_mut()
            .emit_accessibility(renderer, &local_env);
        return;
    }
    let mut entries = entries.borrow_mut();
    let entry = entries
        .iter_mut()
        .find(|entry| entry.identity == active_identity)
        .unwrap_or_else(|| {
            panic!("Hydrolysis navigation entry identity {active_identity} is not retained")
        });
    entry.content.emit_accessibility(renderer, &local_env);
}
#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::resolved_split_column_width;
    use crate::HeadlessRuntime;
    use crate::renderer::tests::{MinimalTestTheme, test_environment};
    use kurbo::Point;
    use nami::Binding;
    use waterui::component::text;
    use waterui::navigation::{
        ColumnWidth, NativeNavigationSplitStyle, NavigationPath, NavigationSplitView,
        NavigationStack, NavigationView,
    };
    use waterui_core::handler::AnyViewBuilder;

    /// The sidebar's width is a policy of the split style, not of the column
    /// constraints alone: a detail-first layout squeezes the sidebar to its
    /// minimum so the detail column gets the remaining space, while the
    /// balanced styles honour the author's ideal.
    #[test]
    fn prominent_detail_squeezes_the_sidebar_to_its_minimum() {
        let width = ColumnWidth::new(180.0, 260.0, 400.0);

        approx::assert_relative_eq!(
            resolved_split_column_width(width, NativeNavigationSplitStyle::ProminentDetail),
            180.0
        );
        for balanced in [
            NativeNavigationSplitStyle::Automatic,
            NativeNavigationSplitStyle::Balanced,
        ] {
            approx::assert_relative_eq!(resolved_split_column_width(width, balanced), 260.0);
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
            approx::assert_relative_eq!(resolved_split_column_width(fixed, style), 240.0);
        }
    }

    /// `NavigationLeadingReserve` is the leftmost x the bar title may paint at
    /// while the back chevron is up: `back_button_size + title_leading_inset`.
    const LEADING_RESERVE: f64 = 40.0 + 16.0;

    /// The minimum painted x of every glyph in the frame — the position the
    /// renderer actually put the ink at, not the layout the a11y tree reports.
    fn painted_text_leading_edge(runtime: &HeadlessRuntime) -> f64 {
        let renderer = runtime.renderer();
        let mut leftmost = f64::INFINITY;
        for recording in std::iter::once(&renderer.painted_scene()) {
            for (transform, glyphs) in recording.glyph_runs() {
                for glyph in glyphs {
                    let point = transform * Point::new(f64::from(glyph.x), f64::from(glyph.y));
                    leftmost = leftmost.min(point.x);
                }
            }
        }
        leftmost
    }

    fn pump_until_settled(runtime: &mut HeadlessRuntime) {
        for _ in 0..64 {
            let _ = runtime.pump_at(true, Instant::now());
            if runtime.is_settled() {
                break;
            }
        }
    }

    /// A compact split draws its own chevron over the pushed pane and injects
    /// `NavigationLeadingReserve` so the pushed page's title paints after it.
    /// A path-backed sidebar `NavigationStack` snapshots its environment into
    /// `retain_environment` before the split injects that key, and
    /// `render_navigation_stack_parts` then installs the snapshot wholesale —
    /// dropping the reserve (water-rs/hydrolysis#325). The painted title must
    /// start at or after the reserve, matching where layout and accessibility
    /// already put it.
    #[test]
    fn compact_split_pushed_title_paints_after_the_leading_reserve() {
        // The detail pane is built at prepare time under the split's ambient
        // environment — before the compact style injects the reserve — so a
        // selection made after the first frame exercises exactly the stale-env
        // path watergram hits when a chat opens.
        let selection = Binding::container(Some(1_i64));
        let path = NavigationPath::<i64>::new();
        path.push(9);
        let sel = selection.clone();
        let mut runtime = HeadlessRuntime::new_for_tests(
            test_environment(),
            AnyViewBuilder::new(move || {
                let path = path.clone();
                waterui_core::AnyView::new(NavigationSplitView::new(
                    &sel,
                    move || {
                        NavigationStack::with_path(
                            path.clone(),
                            NavigationView::new("Chats", text("")),
                        )
                        .destination(|route: i64| {
                            NavigationView::new(format!("Chat {route}"), text(""))
                        })
                    },
                    |id: i64| NavigationView::new(format!("Chat {id}"), text("")),
                ))
            }),
            600,
            800,
            MinimalTestTheme::default(),
        );
        pump_until_settled(&mut runtime);
        selection.set(Some(7));
        pump_until_settled(&mut runtime);

        let leading_edge = painted_text_leading_edge(&runtime);
        assert!(
            leading_edge.is_finite() && leading_edge >= LEADING_RESERVE,
            "pushed title paints under the back chevron at {leading_edge} \
             (reserve is {LEADING_RESERVE})"
        );
    }

    /// A pop's departing entry has already moved to `pending_removed` when
    /// its cached scene is validated, and that scene was recorded under the
    /// departing page's own env — a pushed page's reserve — not the new
    /// top's. Popping to root must replay it, not invalidate the cache and
    /// panic re-recording a page `entries` no longer lists
    /// (water-rs/hydrolysis#325).
    #[test]
    fn pop_to_root_replays_the_departing_scene() {
        let path = NavigationPath::<i64>::new();
        let builder_path = path.clone();
        let mut runtime = HeadlessRuntime::new_for_tests(
            test_environment(),
            AnyViewBuilder::new(move || {
                let path = builder_path.clone();
                waterui_core::AnyView::new(
                    NavigationStack::with_path(path, NavigationView::new("Root", text("")))
                        .destination(|route: i64| {
                            NavigationView::new(format!("Page {route}"), text(""))
                        }),
                )
            }),
            600,
            800,
            MinimalTestTheme::default(),
        );
        pump_until_settled(&mut runtime);
        path.push(7);
        pump_until_settled(&mut runtime);
        path.push(8);
        pump_until_settled(&mut runtime);

        // Pop through depth 1, then to root. The first pop proves the
        // transition replays the departing scene; the second exercises the
        // reserve gate under the root's reserve-less env.
        let _ = path.pop();
        let _ = runtime.pump_at(true, Instant::now());
        assert!(
            runtime
                .renderer()
                .navigation
                .slots
                .values()
                .any(|slot| slot.transition.is_some()),
            "pop must install a transition replaying the departing scene"
        );
        pump_until_settled(&mut runtime);
        let _ = path.pop();
        // Drive the frame clock forward past the transition duration instead
        // of waiting on real time.
        let mut instant = Instant::now();
        for _ in 0..64 {
            instant += Duration::from_millis(500);
            let _ = runtime.pump_at(true, instant);
            if runtime.is_settled() {
                break;
            }
        }
        assert!(runtime.is_settled());
    }
}
