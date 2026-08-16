#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
#[cfg(feature = "accessibility")]
use crate::renderer::accessibility_activation_point;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, RetainedSubview,
    WidgetRenderContext, local_interaction_state, measure_label_intrinsic, measure_view_intrinsic,
    popup_menu_nodes, resolved_color_to_peniko, transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use nami::{Signal, SignalExt};
use std::cell::RefCell;
use std::rc::Rc;
use waterui::ViewExt as _;
use waterui::floating::FloatingScope;
use waterui::style::FloatingStyle;
use waterui_backend_core::widget::{ButtonMetrics, InteractionStyle};
use waterui_controls::button::{ButtonConfig, ButtonSize, ButtonStyle};
use waterui_controls::label::{Label, LabelDisplayMode};
use waterui_controls::menu::ResolvedMenu;
use waterui_core::layout::Point as LayoutPoint;
use waterui_core::layout::Size as LayoutSize;
use waterui_core::layout::{ProposalSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};
use waterui_graphics::color::Color;
use waterui_text::styled::StyledStr;

use crate::widgets::util::{inset_rect, widget_disabled, widget_theme};

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
        let interaction_style = env.get::<InteractionStyle>();
        let floating_style = env.get::<FloatingScope>().map(|scope| &scope.0);
        let color = disabled_aware_label_color(
            theme,
            style,
            &widget_disabled(env),
            interaction_style,
            floating_style,
        );
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
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        // A disabled button stays in the tree (focusable, announced as
        // disabled) but exposes no click action and no action target.
        let disabled = renderer.read_signal(&widget_disabled(env));
        let action_target = if disabled {
            node.set_disabled();
            None
        } else {
            node.add_action(AccessibilityAction::Click);
            let activation_point = accessibility_activation_point(bounds);
            Some(AccessibilityActionTarget::PointerPrimaryClick {
                point: activation_point,
            })
        };
        let _ = renderer.register_accessibility_node(node, bounds, env, action_target);
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, button, env);
    }
}

/// Resolves button-chrome size from the label size and theme metrics. The
/// metric minimums are floors for unconstrained layout; a tighter explicit
/// proposal caps them, so a fixed-size parent (e.g. a 40pt calendar day cell)
/// receives the size it proposed instead of chrome and hit bounds overflowing
/// the assigned slot. Padded label content keeps its intrinsic footprint — the
/// parent decides how any remaining overflow is aligned.
fn button_chrome_size(
    label_size: LayoutSize,
    metrics: &ButtonMetrics,
    proposal: ProposalSize,
) -> LayoutSize {
    let content_width = f64::from(label_size.width) + metrics.padding_x * 2.0;
    let content_height = f64::from(label_size.height) + metrics.padding_y * 2.0;
    let min_width = proposal.width.map_or(metrics.min_width, |width| {
        metrics.min_width.min(f64::from(width))
    });
    let min_height = proposal.height.map_or(metrics.min_height, |height| {
        metrics.min_height.min(f64::from(height))
    });
    LayoutSize::new(
        content_width.max(min_width) as f32,
        content_height.max(min_height) as f32,
    )
}

/// Measures a retained button leaf from its [`ButtonRenderState`]: a general label
/// is measured from its built [`RetainedSubview`], a title from its styled text —
/// mirroring the render path so layout and render agree.
pub(crate) fn measure_button_node(
    render_state: &ButtonRenderState,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    let theme = widget_theme(env);
    let metrics = button_metrics(
        theme,
        render_state.config.style,
        render_state.config.size,
        env.get::<InteractionStyle>(),
        env.get::<FloatingScope>().map(|scope| &scope.0),
    );
    let label_size = match &render_state.label_view {
        Some(subview) => subview.measure_built(state, env),
        None => {
            let styled = styled_button_title(
                theme,
                render_state.config.style,
                &render_state.config.label,
                env,
            );
            HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env)
        }
    };
    ViewDimensions::new(button_chrome_size(label_size, &metrics, proposal))
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
const MENU_TRIGGER_STYLE: ButtonStyle = ButtonStyle::Automatic;

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
    /// each frame (cloneable, like a button title).
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
    pub(crate) fn prebuild_label(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        if let MenuLabel::View(subview) = &mut self.label {
            let color = widget_theme(env).button_label_color(MENU_TRIGGER_STYLE, false);
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
    proposal: ProposalSize,
    hydro: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    let theme = widget_theme(env);
    let metrics = theme.button_metrics(MENU_TRIGGER_STYLE, ButtonSize::default());
    let label_size = match &state.label {
        MenuLabel::Title(label) => {
            let styled = styled_button_title(theme, MENU_TRIGGER_STYLE, label, env);
            HydrolysisRenderer::measure_text_intrinsic_size(hydro, styled, env)
        }
        MenuLabel::View(subview) => subview.measure_built(hydro, env),
    };
    ViewDimensions::new(button_chrome_size(label_size, &metrics, proposal))
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
    let size = state.borrow().config.size;
    let interaction_style = env.get::<InteractionStyle>().cloned();
    let floating_style = env.get::<FloatingScope>().map(|scope| scope.0.clone());
    // Subscribe to the (possibly reactive) title so a label change schedules a frame
    // and this persistent node re-renders the new text.
    {
        let label = state.borrow().config.label.clone();
        watch_button_title(ctx.renderer_mut(), &label, env);
    }
    // Reading the disabled signal watches it, so a change schedules a frame
    // and this persistent node re-renders (and re-registers input) with the
    // new state.
    let disabled = {
        let signal = widget_disabled(env);
        ctx.renderer_mut().read_signal(&signal)
    };
    let bounds = ctx.bounds;
    let hit_bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let (interaction, press_slot, _) = ctx.renderer_mut().bind_control_interaction_target(
        interaction_key,
        hit_bounds,
        env,
        disabled,
    );
    if interaction_style.is_none() && floating_style.is_none() {
        let mut draw = ctx.draw_context();
        theme.draw_button_chrome(&mut draw, bounds, style, interaction);
    }

    let metrics = button_metrics(
        theme,
        style,
        size,
        interaction_style.as_ref(),
        floating_style.as_ref(),
    );
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
            // content stays live through the node's own re-flush). Its semantics are
            // merged into the button's own node by `button_accessibility`, so the
            // sub-view flushes visual-only.
            let render_ctx = ctx.render_context();
            ctx.renderer_mut()
                .with_suppressed_accessibility(|renderer| {
                    subview.flush_in_rect(renderer, render_ctx, env, label_target);
                });
        } else if label_target.width() > 0.0 && label_target.height() > 0.0 {
            // Title label: centered styled text rendered fresh each frame,
            // picking the enabled or disabled label color for this frame.
            let mut styled = styled_button_title(theme, style, &state_mut.config.label, env);
            if let Some(color) = button_label_color(
                theme,
                style,
                disabled,
                interaction_style.as_ref(),
                floating_style.as_ref(),
            ) {
                styled = styled_with_default_foreground(styled, color);
            }
            ctx.render_styled_text_single_line_centered(styled, env, label_target);
        }
    }
    {
        // Hover/focus/press state layers, drawn fresh each flush from the sampled
        // interaction state (the press/hover animations keep frames pumping).
        let interaction = local_interaction_state(interaction, ctx.hit_transform);
        if let Some(interaction_style) = interaction_style {
            let color_signal = interaction_style.state_layer_color.resolve(env);
            let color = resolved_color_to_peniko(ctx.renderer_mut().read_signal(&color_signal));
            let mut draw = ctx.draw_context();
            theme.draw_interaction_state_layer(
                &mut draw,
                interaction_style.state_layer_bounds(bounds),
                interaction_style.state_layer_radii,
                color,
                interaction,
            );
        } else if let Some(floating_style) = floating_style {
            let color_signal = floating_style.state_layer_color.resolve(env);
            let color = resolved_color_to_peniko(ctx.renderer_mut().read_signal(&color_signal));
            let corner_radius =
                bounds.width().min(bounds.height()) * f64::from(floating_style.clip_radius);
            let mut draw = ctx.draw_context();
            theme.draw_interaction_state_layer(
                &mut draw,
                bounds,
                corner_radius.into(),
                color,
                interaction,
            );
        } else {
            let mut draw = ctx.draw_context();
            theme.draw_button_state_layer(&mut draw, bounds, style, interaction);
        }
    }

    // A disabled button registers no tap target: the pointer neither presses
    // it nor invokes its action. Targets are re-registered every flush, so
    // re-enabling restores interactivity on the next frame.
    if disabled {
        return;
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
            true
        },
    );
}

pub(crate) fn render_menu_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<MenuRenderState>>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    let style = MENU_TRIGGER_STYLE;
    let bounds = ctx.bounds;
    let hit_bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let (interaction, press_slot, _) =
        ctx.renderer_mut()
            .bind_interaction_target(interaction_key, hit_bounds, env);
    {
        let mut draw = ctx.draw_context();
        theme.draw_button_chrome(&mut draw, bounds, style, interaction);
    }

    let metrics = theme.button_metrics(style, ButtonSize::default());
    let label_bounds = inset_rect(bounds, metrics.padding_x, metrics.padding_y);
    {
        let mut state = state.borrow_mut();
        match &mut state.label {
            MenuLabel::Title(label)
                if label_bounds.width() > 0.0 && label_bounds.height() > 0.0 =>
            {
                let mut styled = styled_button_title(theme, style, label, env);
                if let Some(color) = theme.button_label_color(style, false) {
                    styled = styled_with_default_foreground(styled, color);
                }
                ctx.render_styled_text_single_line_centered(styled, env, label_bounds);
            }
            MenuLabel::Title(_) => {}
            MenuLabel::View(subview) => {
                // The trigger's semantics are merged into the menu's own node by
                // `menu_accessibility`, so the label sub-view flushes visual-only.
                let render_ctx = ctx.render_context();
                ctx.renderer_mut()
                    .with_suppressed_accessibility(|renderer| {
                        subview.flush_in_rect(renderer, render_ctx, env, label_bounds);
                    });
            }
        }
    }
    {
        // Hover/focus/press state layers over the menu trigger chrome.
        let interaction = local_interaction_state(interaction, ctx.hit_transform);
        let mut draw = ctx.draw_context();
        theme.draw_button_state_layer(&mut draw, bounds, style, interaction);
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
    let metrics = button_metrics(
        theme,
        button.style,
        button.size,
        env.get::<InteractionStyle>(),
        env.get::<FloatingScope>().map(|scope| &scope.0),
    );
    let label_size = measure_button_label_intrinsic(theme, button.style, &button.label, state, env);
    button_chrome_size(label_size, &metrics, ProposalSize::UNSPECIFIED)
}

pub(crate) fn measure_menu_intrinsic(
    menu: &ResolvedMenu,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    let metrics = theme.button_metrics(MENU_TRIGGER_STYLE, ButtonSize::default());
    let label_size = if let Some(label) = menu.label.downcast_ref::<Label>()
        && matches!(label.display_mode_preference(), LabelDisplayMode::TitleOnly)
    {
        let styled = styled_button_title(theme, MENU_TRIGGER_STYLE, label, env);
        HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env)
    } else {
        measure_view_intrinsic(&menu.label, state, env)
    };
    button_chrome_size(label_size, &metrics, ProposalSize::UNSPECIFIED)
}

fn button_label_view(color: Option<Color>, label: AnyView) -> AnyView {
    match color {
        Some(color) => AnyView::new(label.foreground(color)),
        None => label,
    }
}

/// The theme label color for a general (retained) button label, switching
/// reactively between the enabled and disabled label colors so the retained
/// sub-view recolors without being rebuilt.
fn disabled_aware_label_color(
    theme: &dyn waterui_backend_core::widget::WidgetTheme,
    style: ButtonStyle,
    disabled: &nami::Computed<bool>,
    interaction_style: Option<&InteractionStyle>,
    floating_style: Option<&FloatingStyle>,
) -> Option<Color> {
    let enabled_color = button_label_color(theme, style, false, interaction_style, floating_style);
    let disabled_color = button_label_color(theme, style, true, interaction_style, floating_style);
    match (enabled_color, disabled_color) {
        (None, None) => None,
        (Some(when_false), Some(when_true)) => Some(Color::new(SelectResolvedColor {
            condition: disabled.clone(),
            when_true,
            when_false,
        })),
        (enabled_color, disabled_color) => panic!(
            "theme must override the button label color for both the enabled and \
             disabled states, or neither (enabled: {enabled_color:?}, disabled: {disabled_color:?})"
        ),
    }
}

fn button_metrics(
    theme: &dyn waterui_backend_core::widget::WidgetTheme,
    style: ButtonStyle,
    size: ButtonSize,
    interaction_style: Option<&InteractionStyle>,
    floating_style: Option<&FloatingStyle>,
) -> ButtonMetrics {
    interaction_style.map_or_else(
        || {
            floating_style.map_or_else(
                || theme.button_metrics(style, size),
                |style| {
                    ButtonMetrics::new(
                        style.content_inset_x,
                        style.content_inset_y,
                        style.minimum_width,
                        style.minimum_height,
                    )
                },
            )
        },
        |style| style.metrics,
    )
}

fn button_label_color(
    theme: &dyn waterui_backend_core::widget::WidgetTheme,
    style: ButtonStyle,
    disabled: bool,
    interaction_style: Option<&InteractionStyle>,
    floating_style: Option<&FloatingStyle>,
) -> Option<Color> {
    interaction_style.map_or_else(
        || {
            floating_style.map_or_else(
                || theme.button_label_color(style, disabled),
                |style| {
                    Some(if disabled {
                        style
                            .content_color
                            .clone()
                            .with_opacity(style.disabled_content_opacity)
                    } else {
                        style.content_color.clone()
                    })
                },
            )
        },
        |style| style.resolved_label_color(disabled),
    )
}

/// A [`Resolvable`] color that follows `condition`: it resolves to
/// `when_true` while the signal is `true` and `when_false` otherwise, so a
/// retained label recolors reactively (e.g. on disable) without being rebuilt.
#[derive(Debug, Clone)]
struct SelectResolvedColor {
    condition: nami::Computed<bool>,
    when_true: Color,
    when_false: Color,
}

impl waterui_core::resolve::Resolvable for SelectResolvedColor {
    type Resolved = waterui_graphics::color::ResolvedColor;

    fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
        let when_true = self.when_true.resolve(env);
        let when_false = self.when_false.resolve(env);
        nami::zip::zip(
            nami::zip::zip(self.condition.clone(), when_true),
            when_false,
        )
        .map(
            |((condition, when_true), when_false)| {
                if condition { when_true } else { when_false }
            },
        )
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
    title.resolve(env).content.get()
}

/// Subscribes to a label's reactive title content so a change schedules a frame
/// (the persistent `Widget` node then re-renders it live). The returned value is
/// ignored; the side effect is the watch registration.
fn watch_button_title(renderer: &mut HydrolysisRenderer, label: &Label, env: &Environment) {
    let _ = renderer.read_signal(&label.semantic_text().resolve(env).content);
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

#[cfg(test)]
mod tests {
    use super::{MENU_TRIGGER_STYLE, button_chrome_size};
    use waterui_backend_core::widget::ButtonMetrics;
    use waterui_controls::button::ButtonStyle;
    use waterui_core::layout::{ProposalSize, Size};

    #[test]
    fn menu_trigger_uses_material_default_button_style() {
        assert_eq!(MENU_TRIGGER_STYLE, ButtonStyle::Automatic);
    }

    #[test]
    fn button_chrome_measurement_uses_metrics_and_proposal() {
        let label = Size::new(11.0, 13.0);
        let metrics = ButtonMetrics::new(3.0, 5.0, 37.0, 41.0);

        assert_eq!(
            button_chrome_size(label, &metrics, ProposalSize::UNSPECIFIED),
            Size::new(37.0, 41.0)
        );
        assert_eq!(
            button_chrome_size(label, &metrics, ProposalSize::new(Some(31.0), Some(29.0))),
            Size::new(31.0, 29.0)
        );
    }
}
