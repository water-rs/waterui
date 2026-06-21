#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
#[cfg(feature = "accessibility")]
use crate::renderer::accessibility_activation_point;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, RetainedSubview,
    WidgetRenderContext, measure_label_intrinsic, measure_view_intrinsic, popup_menu_nodes,
    transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use nami::Signal;
use std::cell::RefCell;
use std::rc::Rc;
use waterui::ViewExt as _;
use waterui_controls::button::{ButtonConfig, ButtonStyle};
use waterui_controls::label::{Label, LabelDisplayMode};
use waterui_controls::menu::ResolvedMenu;
use waterui_core::layout::Point as LayoutPoint;
use waterui_core::layout::Size as LayoutSize;
use waterui_core::layout::{ProposalSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};
use waterui_graphics::color::Color;
use waterui_text::styled::StyledStr;

use crate::widgets::util::{inset_rect, widget_theme};

/// The retained render state of a button. A `TitleOnly` label is rendered as
/// centered styled text fresh each frame (so its reactive title stays live); any
/// other label is a move-only composite, so it is held as a [`RetainedSubview`]
/// built once (with the theme's button styling + label color applied) and
/// re-flushed each frame. The `config` is kept for the value/style/action and for
/// accessibility resolution.
pub(crate) struct ButtonRenderState {
    config: ButtonConfig,
    /// `Some` for a non-title (general) label held as a retained sub-view; `None`
    /// for a `TitleOnly` label rendered as styled text from `config.label`.
    label_view: Option<RetainedSubview>,
}

impl ButtonRenderState {
    pub(crate) fn from_config(config: ButtonConfig) -> Self {
        Self {
            config,
            label_view: None,
        }
    }

    /// Eagerly build the general (non-title) label sub-view, applying the button's
    /// style-specific label styling + theme label color (mirroring the styled-text
    /// path's `styled_button_label` + `button_label_view`). A `TitleOnly` label
    /// stays `None` and is rendered as styled text each frame. The measure path has
    /// no renderer, so the sub-view must be built here at tree-build time.
    pub(crate) fn prebuild_label(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        if matches!(
            self.config.label.display_mode_preference(),
            LabelDisplayMode::TitleOnly
        ) {
            return;
        }
        let theme = widget_theme(env);
        let style = self.config.style;
        let color = theme.button_label_color(style);
        let styled = styled_button_label(theme, style, self.config.label.clone());
        let mut subview = RetainedSubview::new(button_label_view(color, AnyView::new(styled)));
        subview.ensure_built(renderer, env);
        self.label_view = Some(subview);
    }
}

impl HydroNativeView for Native<ButtonConfig> {
    fn intrinsic(
        state: &mut crate::renderer::HydroState,
        view: &Self,
        env: &Environment,
    ) -> LayoutSize {
        measure_button_intrinsic(view.as_inner(), state, env)
    }
}

/// Emits a button's accessibility node from its config. Shared by the dispatch
/// path ([`Native<ButtonConfig>::accessibility`]) and the retained `Widget`-node
/// path so both produce the same a11y tree.
pub(crate) fn button_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    button: &ButtonConfig,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let mut node = AccessibilityNode::new(renderer.resolve_accessibility_role(
            env,
            match button.style {
                ButtonStyle::Link => AccessibilityNodeRole::Link,
                _ => AccessibilityNodeRole::Button,
            },
        ));
        let default_label = renderer.accessibility_label_from_label(&button.label, env);
        let label = renderer.resolve_accessibility_label(env, default_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        node.add_action(AccessibilityAction::Focus);
        node.add_action(AccessibilityAction::Click);
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let activation_point = accessibility_activation_point(bounds);
        let _ = renderer.register_accessibility_node(
            node,
            bounds,
            env,
            Some(AccessibilityActionTarget::PointerPrimaryClick {
                point: activation_point,
            }),
        );
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, button, env);
    }
}

/// Measures a retained button leaf from its [`ButtonRenderState`]: a general label
/// is measured from its built [`RetainedSubview`], a title from its styled text —
/// mirroring the render path so layout and render agree.
pub(crate) fn measure_button_node(
    render_state: &ButtonRenderState,
    _proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    let theme = widget_theme(env);
    let metrics = theme.button_metrics(render_state.config.style);
    let label_size = match &render_state.label_view {
        Some(subview) => subview.measure_built(state, env),
        None => {
            let styled =
                styled_button_title(theme, render_state.config.style, &render_state.config.label, env);
            HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env)
        }
    };
    let content_width = f64::from(label_size.width) + metrics.padding_x * 2.0;
    let content_height = f64::from(label_size.height) + metrics.padding_y * 2.0;
    ViewDimensions::new(LayoutSize::new(
        content_width.max(metrics.min_width) as f32,
        content_height.max(metrics.min_height) as f32,
    ))
}

/// Renders a retained button leaf every flush: emits a11y (unless hidden) then the
/// chrome + label + tap target, reading the config's live signals each frame.
pub(crate) fn render_button_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ButtonRenderState>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        button_accessibility(ctx.renderer_mut(), render_ctx, &state.borrow().config, env);
    }
    render_button_parts(ctx, state, env);
}

/// The retained render state of a menu trigger. `ResolvedMenu::label` is a
/// move-only `AnyView`, so the persistent `Widget` node holds it as a
/// [`RetainedSubview`] built once and re-flushed each frame (unless it is a
/// `Label`/`TitleOnly`, which is rendered as styled text fresh each frame like a
/// button); the `items`/`accessibility_label` signals are kept and read through
/// `read_signal`.
pub(crate) struct MenuRenderState {
    /// The build-time decision of how to render the label.
    label: MenuLabel,
    items: nami::Computed<Vec<waterui_controls::menu::ResolvedMenuItem>>,
    accessibility_label: nami::Computed<StyledStr>,
}

/// How a menu trigger renders its label, decided once at build time from the
/// label `AnyView`.
enum MenuLabel {
    /// A `Label` with `TitleOnly` display: rendered as centered styled text fresh
    /// each frame (clonable, like a button title).
    Title(Label),
    /// Any other view: re-flushed from a retained sub-view each frame.
    View(RetainedSubview),
}

impl MenuRenderState {
    pub(crate) fn from_resolved(menu: ResolvedMenu) -> Self {
        let ResolvedMenu {
            label,
            items,
            accessibility_label,
        } = menu;
        let label = match label.downcast::<Label>() {
            Ok(label) if matches!(label.display_mode_preference(), LabelDisplayMode::TitleOnly) => {
                MenuLabel::Title(*label)
            }
            Ok(label) => MenuLabel::View(RetainedSubview::new(AnyView::new(*label))),
            Err(view) => MenuLabel::View(RetainedSubview::new(view)),
        };
        Self {
            label,
            items,
            accessibility_label,
        }
    }

    /// Eagerly build the label sub-view (the measure path has no renderer),
    /// applying the theme's default label foreground first (mirroring the dispatch
    /// path's `button_label_view`).
    pub(crate) fn prebuild_label(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
    ) {
        if let MenuLabel::View(subview) = &mut self.label {
            let color = widget_theme(env).button_label_color(ButtonStyle::Borderless);
            subview.map_source(|view| button_label_view(color, view));
            subview.ensure_built(renderer, env);
        }
    }
}

impl HydroNativeView for Native<ResolvedMenu> {
    fn intrinsic(
        state: &mut crate::renderer::HydroState,
        view: &Self,
        env: &Environment,
    ) -> LayoutSize {
        measure_menu_intrinsic(view.as_inner(), state, env)
    }
}

/// Emits a menu trigger's accessibility node from its accessibility-label signal.
/// Shared by the dispatch path and the retained node path.
pub(crate) fn menu_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    accessibility_label: &nami::Computed<StyledStr>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let mut node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Button),
        );
        let default_label = Some(
            renderer
                .read_signal(accessibility_label)
                .to_plain()
                .to_string(),
        );
        let label = renderer.resolve_accessibility_label(env, default_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        node.add_action(AccessibilityAction::Focus);
        node.add_action(AccessibilityAction::Click);
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let activation_point = accessibility_activation_point(bounds);
        let _ = renderer.register_accessibility_node(
            node,
            bounds,
            env,
            Some(AccessibilityActionTarget::PointerPrimaryClick {
                point: activation_point,
            }),
        );
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, accessibility_label, env);
    }
}

/// Measures a retained menu leaf from its [`MenuRenderState`].
pub(crate) fn measure_menu_node(
    state: &MenuRenderState,
    _proposal: ProposalSize,
    hydro: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    let theme = widget_theme(env);
    let metrics = theme.button_metrics(ButtonStyle::Borderless);
    let label_size = match &state.label {
        MenuLabel::Title(label) => {
            let styled = styled_button_title(theme, ButtonStyle::Borderless, label, env);
            HydrolysisRenderer::measure_text_intrinsic_size(hydro, styled, env)
        }
        MenuLabel::View(subview) => subview.measure_built(hydro, env),
    };
    let content_width = f64::from(label_size.width) + metrics.padding_x * 2.0;
    let content_height = f64::from(label_size.height) + metrics.padding_y * 2.0;
    ViewDimensions::new(LayoutSize::new(
        content_width.max(metrics.min_width) as f32,
        content_height.max(metrics.min_height) as f32,
    ))
}

/// Renders a retained menu leaf every flush: emits a11y (unless hidden) then the
/// chrome + label + tap target, reading the accessibility-label/items signals.
pub(crate) fn render_menu_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<MenuRenderState>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        let accessibility_label = state.borrow().accessibility_label.clone();
        menu_accessibility(ctx.renderer_mut(), render_ctx, &accessibility_label, env);
    }
    render_menu_parts(ctx, state, env);
}

pub(crate) fn render_button_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ButtonRenderState>>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let style = state.borrow().config.style;
    // Subscribe to the (possibly reactive) title so a label change schedules a frame
    // and this persistent node re-renders the new text.
    {
        let label = state.borrow().config.label.clone();
        watch_button_title(ctx.renderer_mut(), &label, env);
    }
    let bounds = ctx.bounds;
    let hit_bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
    let (_, press_slot, _) = ctx.renderer_mut().bind_interaction_target(hit_bounds, env);
    {
        let mut draw = ctx.draw_context();
        theme.draw_button_chrome(&mut draw, bounds, style);
    }

    let metrics = theme.button_metrics(style);
    let label_bounds = inset_rect(bounds, metrics.padding_x, metrics.padding_y);
    // A degenerate padded rect (a button smaller than its padding) falls back to the
    // full bounds so the label still shows — matching the old dispatch fallback.
    let label_target = if label_bounds.width() > 0.0 && label_bounds.height() > 0.0 {
        label_bounds
    } else {
        bounds
    };
    {
        let mut state_mut = state.borrow_mut();
        if let Some(subview) = &mut state_mut.label_view {
            // General label: a retained node sub-view re-flushed at its rect (reactive
            // content stays live through the node's own re-flush).
            let render_ctx = ctx.render_context();
            subview.flush_in_rect(ctx.renderer_mut(), render_ctx, env, label_target);
        } else if label_target.width() > 0.0 && label_target.height() > 0.0 {
            // Title label: centered styled text rendered fresh each frame.
            let mut styled = styled_button_title(theme, style, &state_mut.config.label, env);
            if let Some(color) = theme.button_label_color(style) {
                styled = styled_with_default_foreground(styled, color);
            }
            ctx.render_styled_text_single_line_centered(styled, env, label_target);
        }
    }

    // Invoke the action through the shared state cell so the retained node and the
    // dispatch path register equivalent tap targets without moving the action out.
    let state = Rc::clone(state);
    let action_env = env.clone();
    ctx.renderer_mut().register_interactive_pointer_target(
        hit_bounds,
        press_slot,
        move |_renderer, _point, _env| {
            (state.borrow_mut().config.action)(&action_env);
            false
        },
    );
}

pub(crate) fn render_menu_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<MenuRenderState>>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let style = ButtonStyle::Borderless;
    let bounds = ctx.bounds;
    let hit_bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
    let (_, press_slot, _) = ctx.renderer_mut().bind_interaction_target(hit_bounds, env);
    {
        let mut draw = ctx.draw_context();
        theme.draw_button_chrome(&mut draw, bounds, style);
    }

    let metrics = theme.button_metrics(style);
    let label_bounds = inset_rect(bounds, metrics.padding_x, metrics.padding_y);
    {
        let mut state = state.borrow_mut();
        match &mut state.label {
            MenuLabel::Title(label)
                if label_bounds.width() > 0.0 && label_bounds.height() > 0.0 =>
            {
                let mut styled = styled_button_title(theme, style, label, env);
                if let Some(color) = theme.button_label_color(style) {
                    styled = styled_with_default_foreground(styled, color);
                }
                ctx.render_styled_text_single_line_centered(styled, env, label_bounds);
            }
            MenuLabel::Title(_) => {}
            MenuLabel::View(subview) => {
                let render_ctx = ctx.render_context();
                subview.flush_in_rect(ctx.renderer_mut(), render_ctx, env, label_bounds);
            }
        }
    }

    let items = state.borrow().items.clone();
    // Watch the items so a change schedules a frame (the popup re-reads live items
    // on open, but a change still re-presents the trigger).
    let _ = ctx.renderer_mut().read_signal(&items);
    let anchor = LayoutPoint::new(hit_bounds.x0 as f32, hit_bounds.y1 as f32);
    ctx.renderer_mut().register_interactive_pointer_target(
        hit_bounds,
        press_slot,
        move |renderer, _point, env| {
            renderer.show_popup_menu_nodes(popup_menu_nodes(&items.get()), anchor, env)
        },
    );
}

pub(crate) fn measure_button_intrinsic(
    button: &ButtonConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    let metrics = theme.button_metrics(button.style);
    let label_size = measure_button_label_intrinsic(theme, button.style, &button.label, state, env);
    let content_width = f64::from(label_size.width) + metrics.padding_x * 2.0;
    let content_height = f64::from(label_size.height) + metrics.padding_y * 2.0;
    LayoutSize::new(
        content_width.max(metrics.min_width) as f32,
        content_height.max(metrics.min_height) as f32,
    )
}

pub(crate) fn measure_menu_intrinsic(
    menu: &ResolvedMenu,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    let metrics = theme.button_metrics(ButtonStyle::Borderless);
    let label_size = if let Some(label) = menu.label.downcast_ref::<Label>()
        && matches!(label.display_mode_preference(), LabelDisplayMode::TitleOnly)
    {
        let styled = styled_button_title(theme, ButtonStyle::Borderless, label, env);
        HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env)
    } else {
        measure_view_intrinsic(&menu.label, state, env)
    };
    let content_width = f64::from(label_size.width) + metrics.padding_x * 2.0;
    let content_height = f64::from(label_size.height) + metrics.padding_y * 2.0;
    LayoutSize::new(
        content_width.max(metrics.min_width) as f32,
        content_height.max(metrics.min_height) as f32,
    )
}

fn button_label_view(color: Option<Color>, label: AnyView) -> AnyView {
    match color {
        Some(color) => AnyView::new(label.foreground(color)),
        None => label,
    }
}

fn measure_button_label_intrinsic(
    theme: &dyn waterui_backend_core::widget::WidgetTheme,
    style: ButtonStyle,
    label: &Label,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    if matches!(label.display_mode_preference(), LabelDisplayMode::TitleOnly) {
        let styled = styled_button_title(theme, style, label, env);
        HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env)
    } else {
        let label = styled_button_label(theme, style, label.clone());
        measure_label_intrinsic(&label, state, env)
    }
}

fn styled_button_title(
    theme: &dyn waterui_backend_core::widget::WidgetTheme,
    style: ButtonStyle,
    label: &Label,
    env: &Environment,
) -> StyledStr {
    let title = label.semantic_text().clone();
    let title = if let Some(font) = theme.button_label_font(style) {
        title.font(font)
    } else {
        title
    };
    title.resolve_reactive(env).content.get()
}

/// Subscribes to a label's reactive title content so a change schedules a frame
/// (the persistent `Widget` node then re-renders it live). The returned value is
/// ignored; the side effect is the watch registration.
fn watch_button_title(renderer: &mut HydrolysisRenderer, label: &Label, env: &Environment) {
    let _ = renderer.read_signal(&label.semantic_text().resolve_reactive(env).content);
}

/// Fills `color` into chunks that have no explicit foreground: the theme's
/// button label color is a default, never an override of caller styling.
fn styled_with_default_foreground(styled: StyledStr, color: Color) -> StyledStr {
    let mut out = StyledStr::empty();
    for (chunk, style) in styled.chunks() {
        let mut style = style.clone();
        if style.foreground.is_none() {
            style.foreground = Some(color.clone());
        }
        out.push(chunk.clone(), style);
    }
    out
}

fn styled_button_label(
    theme: &dyn waterui_backend_core::widget::WidgetTheme,
    style: ButtonStyle,
    label: Label,
) -> Label {
    if let Some(font) = theme.button_label_font(style) {
        label.font(font)
    } else {
        label
    }
}
