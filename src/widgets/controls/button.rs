#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, RetainedSubview,
    WidgetRenderContext, local_interaction_state, measure_label_intrinsic, measure_view_intrinsic,
    popup_menu_nodes, transformed_rect,
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
use waterui_controls::ControlSize;
use waterui_controls::button::{ButtonConfig, ButtonStyle};
use waterui_controls::label::{Label, LabelDisplayMode};
use waterui_controls::menu::ResolvedMenu;
use waterui_core::layout::Point as LayoutPoint;
use waterui_core::layout::Size as LayoutSize;
use waterui_core::layout::{ProposalSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};
use waterui_graphics::color::Color;
use waterui_text::styled::StyledStr;

use crate::widgets::util::{centered_label_rect, inset_rect, widget_disabled};

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

    /// Create the general (non-title) label sub-view at tree-build time. A plain
    /// `TitleOnly` label stays `None` and is rendered as styled text each frame.
    /// The sub-view is created unbuilt and unpainted: the theme supplies the
    /// label font and foreground — paint — so styling is applied in
    /// [`Self::ensure_label_built`] during the layout-time prepare pass, the
    /// first point a theme exists.
    ///
    /// A label carrying custom content also resolves to `TitleOnly`, because it
    /// has no icon — but its content is a view, and rendering its semantic text
    /// instead would throw that view away. Content kind decides here, not
    /// display mode.
    pub(crate) fn init_label(&mut self) {
        if renders_as_plain_title(&self.config.label) {
            return;
        }
        self.label_view = Some(RetainedSubview::new(AnyView::new(
            self.config.label.clone(),
        )));
    }

    /// Apply the theme's label styling and build the label sub-view. Called from
    /// the layout-time prepare pass (`WidgetBehavior::prepare`) — the measure
    /// path has no renderer, so the sub-view must be built before then. A
    /// semantic runtime never prepares, so the label view is never styled or
    /// built there: accessibility reads the label config directly.
    pub(crate) fn ensure_label_built(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
    ) {
        let Some(subview) = &mut self.label_view else {
            return;
        };
        if subview.is_built() {
            return;
        }
        let theme = renderer.theme();
        let style = self.config.style;
        let interaction_style = env.get::<InteractionStyle>();
        let floating_style = env.get::<FloatingScope>().map(|scope| &scope.0);
        let color = if env.get::<ListRowChrome>().is_some() {
            Some(Color::new(waterui::theme::color::Foreground))
        } else {
            disabled_aware_label_color(
                &theme,
                style,
                &widget_disabled(env),
                interaction_style,
                floating_style,
            )
        };
        let styled = styled_button_label(&theme, style, self.config.label.clone());
        let icon_only = label_resolves_icon_only(&self.config.label, env);
        subview.map_source(|_| button_label_view(color, AnyView::new(styled), icon_only));
        subview.ensure_built(renderer, env);
    }
}

impl HydroNativeView for Native<ButtonConfig> {
    fn intrinsic(
        state: &mut crate::renderer::HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        measure_button_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// Emits a button's accessibility node from its retained state. Shared by the
/// rendered `Widget`-node flush and the semantic emission walk so both produce
/// the same a11y tree.
pub(crate) fn button_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    state: &Rc<RefCell<ButtonRenderState>>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let button = &state.borrow().config;
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
        // A disabled button stays in the tree (focusable, announced as
        // disabled) but exposes no click action and no action target.
        let disabled = renderer.read_signal(&widget_disabled(env));
        let action_target = if disabled {
            node.set_disabled();
            None
        } else {
            node.add_action(AccessibilityAction::Click);
            Some(AccessibilityActionTarget::Activate {
                action: button_activation(state, env),
            })
        };
        let node_id = match ctx {
            Some(ctx) => {
                let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
                renderer.register_accessibility_node(node, bounds, env, action_target)
            }
            None => renderer.register_accessibility_node_semantic(node, env, action_target),
        };
        if let Some(node_id) = node_id {
            renderer.register_accessibility_focus_link(
                &crate::renderer::InteractionKey::for_rc(state, 0),
                node_id,
            );
        }
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, state, env);
    }
}

/// The activation closure a button's pointer target and its accessibility
/// `Click` target share: invokes `config.action` through the retained state
/// cell so both hit the live config.
#[cfg(feature = "accessibility")]
pub(crate) fn button_activation(
    state: &Rc<RefCell<ButtonRenderState>>,
    env: &Environment,
) -> crate::renderer::AccessibilityActivation {
    let state = Rc::clone(state);
    let action_env = env.clone();
    Rc::new(RefCell::new(
        move |_renderer: &mut crate::renderer::SemanticCore, _env: &Environment| {
            (state.borrow_mut().config.action)(&action_env);
            true
        },
    ))
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
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    let metrics = button_metrics(
        theme,
        render_state.config.style,
        render_state.config.size,
        label_resolves_icon_only(&render_state.config.label, env),
        env.get::<InteractionStyle>(),
        env.get::<FloatingScope>().map(|scope| &scope.0),
    );
    let label_size = match &render_state.label_view {
        Some(subview) => subview.measure_built(state, env, theme),
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
        button_accessibility(ctx.renderer_mut(), Some(render_ctx), state, env);
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
    /// Whether the label resolves to an icon-only presentation — decided
    /// once at build from the label's own display-mode resolution, so the
    /// trigger sizes under the icon-button contract like a button.
    icon_only: bool,
    items: nami::Computed<Vec<waterui_controls::menu::ResolvedMenuItem>>,
    #[cfg(feature = "accessibility")]
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
    pub(crate) fn from_resolved(menu: ResolvedMenu, env: &Environment) -> Self {
        let ResolvedMenu {
            label,
            items,
            #[cfg(feature = "accessibility")]
            accessibility_label,
            #[cfg(not(feature = "accessibility"))]
                accessibility_label: _,
        } = menu;
        let icon_only = label
            .downcast_ref::<Label>()
            .is_some_and(|label| label_resolves_icon_only(label, env));
        let label = match label.downcast::<Label>() {
            Ok(label) if renders_as_plain_title(&label) => MenuLabel::Title(*label),
            Ok(label) => MenuLabel::View(RetainedSubview::new(AnyView::new(*label))),
            Err(view) => MenuLabel::View(RetainedSubview::new(view)),
        };
        Self {
            label,
            icon_only,
            items,
            #[cfg(feature = "accessibility")]
            accessibility_label,
        }
    }

    /// Apply the theme's default label foreground and build the label sub-view
    /// (mirroring the dispatch path's `button_label_view`). Called from the
    /// layout-time prepare pass (`WidgetBehavior::prepare`) — the measure path
    /// has no renderer, so the sub-view must be built before then; a semantic
    /// runtime never prepares and never needs the painted label.
    pub(crate) fn ensure_label_built(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
    ) {
        let theme = renderer.theme();
        if let MenuLabel::View(subview) = &mut self.label {
            if subview.is_built() {
                return;
            }
            let color = theme.button_label_color(MENU_TRIGGER_STYLE, false);
            let icon_only = self.icon_only;
            subview.map_source(|view| button_label_view(color, view, icon_only));
            subview.ensure_built(renderer, env);
        }
    }
}

impl HydroNativeView for Native<ResolvedMenu> {
    fn intrinsic(
        state: &mut crate::renderer::HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        measure_menu_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// Emits a menu trigger's accessibility node from its accessibility-label signal.
/// Shared by the rendered `Widget`-node flush (which passes its [`RenderContext`]
/// and theme for the popup anchor and metrics) and the semantic emission walk
/// (which passes `None` for both — activation only marks a menu group active).
pub(crate) fn menu_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    theme: Option<&Rc<dyn crate::engine::WidgetTheme>>,
    state: &Rc<RefCell<MenuRenderState>>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let accessibility_label = state.borrow().accessibility_label.clone();
        let mut node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Button),
        );
        let default_label = Some(
            renderer
                .read_signal(&accessibility_label)
                .to_plain()
                .to_string(),
        );
        let label = renderer.resolve_accessibility_label(env, default_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        node.add_action(AccessibilityAction::Focus);
        node.add_action(AccessibilityAction::Click);
        // Direct activation: show the popup under the trigger's own anchor. The
        // semantic runtime has no trigger rect, so its popup mounts at the
        // origin through `activate_popup_menu_nodes` — the items land in the
        // merged accessibility tree exactly as the rendered popup's do.
        let request = ctx.as_ref().zip(theme).map(|(ctx, theme)| {
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            (
                LayoutPoint::new(bounds.x0 as f32, bounds.y1 as f32),
                theme.text_context_menu_metrics(),
                theme.clone(),
            )
        });
        let items = state.borrow().items.clone();
        // The popup opens in the trigger node's environment layered over the
        // dispatch's, so `.state(&value)` overlays reach the item actions
        // (water-rs/hydrolysis#140).
        let menu_env = env.clone();
        let activation = AccessibilityActionTarget::Activate {
            action: Rc::new(RefCell::new(
                move |renderer: &mut crate::renderer::SemanticCore, env: &Environment| {
                    let nodes = popup_menu_nodes(&items.snapshot());
                    let env = menu_env.layered_on(env);
                    match &request {
                        Some((anchor, metrics, theme)) => {
                            renderer.show_popup_menu_nodes(nodes, *anchor, *metrics, &env, theme);
                        }
                        None => {
                            renderer.activate_popup_menu_nodes(nodes, &env);
                        }
                    }
                    true
                },
            )),
        };
        let node_id = match ctx {
            Some(ctx) => {
                let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
                renderer.register_accessibility_node(node, bounds, env, Some(activation))
            }
            None => renderer.register_accessibility_node_semantic(node, env, Some(activation)),
        };
        if let Some(node_id) = node_id {
            renderer.register_accessibility_focus_link(
                &crate::renderer::InteractionKey::for_rc(state, 0),
                node_id,
            );
        }
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, theme, state, env);
    }
}

/// Measures a retained menu leaf from its [`MenuRenderState`].
pub(crate) fn measure_menu_node(
    state: &MenuRenderState,
    proposal: ProposalSize,
    hydro: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    let metrics = button_metrics(
        theme,
        MENU_TRIGGER_STYLE,
        ControlSize::Small,
        state.icon_only,
        env.get::<InteractionStyle>(),
        env.get::<FloatingScope>().map(|scope| &scope.0),
    );
    let label_size = match &state.label {
        MenuLabel::Title(label) => {
            let styled = styled_button_title(theme, MENU_TRIGGER_STYLE, label, env);
            HydrolysisRenderer::measure_text_intrinsic_size(hydro, styled, env)
        }
        MenuLabel::View(subview) => subview.measure_built(hydro, env, theme),
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
        let theme = ctx.theme();
        let render_ctx = ctx.render_context();
        menu_accessibility(
            ctx.renderer_mut(),
            Some(render_ctx),
            Some(&theme),
            state,
            env,
        );
    }
    render_menu_parts(ctx, state, env);
}

pub(crate) fn render_button_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ButtonRenderState>>,
    env: &Environment,
) {
    let theme = ctx.theme();
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
    // The label's own resolved display mode selects the presentation: an
    // icon-only button lays out at the theme's icon-button touch target and
    // its bounds are the hit area — the theme draws the smaller icon-button
    // container centred inside them.
    let icon_only = label_resolves_icon_only(&state.borrow().config.label, env);
    let metrics = button_metrics(
        &theme,
        style,
        size,
        icon_only,
        interaction_style.as_ref(),
        floating_style.as_ref(),
    );
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
        theme.draw_button_chrome(&mut draw, bounds, style, icon_only, interaction);
    }

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
            // sub-view flushes visual-only. The label is placed centred in the
            // content rect, so a label smaller than the chrome sits in the middle.
            let proposal = ProposalSize::new(
                Some(label_target.width() as f32),
                Some(label_target.height() as f32),
            );
            let (label_size, _) = subview.patch_and_measure(ctx.renderer_mut(), env, proposal);
            let label_target = centered_label_rect(label_target, label_size);
            let render_ctx = ctx.render_context();
            ctx.renderer_mut()
                .with_suppressed_accessibility(|renderer| {
                    subview.flush_in_rect(
                        renderer,
                        render_ctx,
                        env,
                        ProposalSize::UNSPECIFIED,
                        label_target,
                    );
                });
        } else if label_target.width() > 0.0 && label_target.height() > 0.0 {
            // Title label: centered styled text rendered fresh each frame,
            // picking the enabled or disabled label color for this frame.
            let mut styled = styled_button_title(&theme, style, &state_mut.config.label, env);
            let title_color = if env.get::<ListRowChrome>().is_some() {
                Some(Color::new(waterui::theme::color::Foreground))
            } else {
                button_label_color(
                    &theme,
                    style,
                    disabled,
                    interaction_style.as_ref(),
                    floating_style.as_ref(),
                )
            };
            if let Some(color) = title_color {
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
            let color = ctx.renderer_mut().read_signal(&color_signal);
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
            let color = ctx.renderer_mut().read_signal(&color_signal);
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
            theme.draw_button_state_layer(&mut draw, bounds, style, icon_only, interaction);
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
    let theme = ctx.theme();
    let style = MENU_TRIGGER_STYLE;
    let bounds = ctx.bounds;
    let hit_bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let (interaction, press_slot, _) =
        ctx.renderer_mut()
            .bind_interaction_target(interaction_key, hit_bounds, env);
    let icon_only = state.borrow().icon_only;
    {
        let mut draw = ctx.draw_context();
        theme.draw_button_chrome(&mut draw, bounds, style, icon_only, interaction);
    }

    let metrics = button_metrics(
        &theme,
        style,
        ControlSize::Small,
        icon_only,
        env.get::<InteractionStyle>(),
        env.get::<FloatingScope>().map(|scope| &scope.0),
    );
    let label_bounds = inset_rect(bounds, metrics.padding_x, metrics.padding_y);
    {
        let mut state = state.borrow_mut();
        match &mut state.label {
            MenuLabel::Title(label)
                if label_bounds.width() > 0.0 && label_bounds.height() > 0.0 =>
            {
                let mut styled = styled_button_title(&theme, style, label, env);
                if let Some(color) = theme.button_label_color(style, false) {
                    styled = styled_with_default_foreground(styled, color);
                }
                ctx.render_styled_text_single_line_centered(styled, env, label_bounds);
            }
            MenuLabel::Title(_) => {}
            MenuLabel::View(subview) => {
                // The trigger's semantics are merged into the menu's own node by
                // `menu_accessibility`, so the label sub-view flushes visual-only.
                // Like a button's label, it sits centred in the content rect.
                let proposal = ProposalSize::new(
                    Some(label_bounds.width() as f32),
                    Some(label_bounds.height() as f32),
                );
                let (label_size, _) = subview.patch_and_measure(ctx.renderer_mut(), env, proposal);
                let label_bounds = centered_label_rect(label_bounds, label_size);
                let render_ctx = ctx.render_context();
                ctx.renderer_mut()
                    .with_suppressed_accessibility(|renderer| {
                        subview.flush_in_rect(
                            renderer,
                            render_ctx,
                            env,
                            ProposalSize::UNSPECIFIED,
                            label_bounds,
                        );
                    });
            }
        }
    }
    {
        // Hover/focus/press state layers over the menu trigger chrome.
        let interaction = local_interaction_state(interaction, ctx.hit_transform);
        let mut draw = ctx.draw_context();
        theme.draw_button_state_layer(&mut draw, bounds, style, icon_only, interaction);
    }

    let items = state.borrow().items.clone();
    // Watch the items so a change schedules a frame (the popup re-reads live items
    // on open, but a change still re-presents the trigger).
    let _ = ctx.renderer_mut().read_signal(&items);
    // A mounted `Menu` arms its commands' chords on the window's scope while
    // mounted (water-rs/hydrolysis#247); unmount drops the render state and
    // the registration with it.
    ctx.renderer_mut()
        .register_menu_shortcuts(Rc::downgrade(state), items.clone(), env.clone());
    let anchor = LayoutPoint::new(hit_bounds.x0 as f32, hit_bounds.y1 as f32);
    let menu_metrics = theme.text_context_menu_metrics();
    let menu_theme = theme.clone();
    // The popup opens in the trigger node's environment layered over the
    // dispatch's (water-rs/hydrolysis#140).
    let menu_env = env.clone();
    ctx.renderer_mut().register_interactive_pointer_target(
        hit_bounds,
        press_slot,
        move |renderer, _point, env| {
            let env = menu_env.layered_on(env);
            renderer.show_popup_menu_nodes(
                popup_menu_nodes(&items.snapshot()),
                anchor,
                menu_metrics,
                &env,
                &menu_theme,
            )
        },
    );
}

pub(crate) fn measure_button_intrinsic(
    button: &ButtonConfig,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> LayoutSize {
    let metrics = button_metrics(
        theme,
        button.style,
        button.size,
        label_resolves_icon_only(&button.label, env),
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
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> LayoutSize {
    let icon_only = menu
        .label
        .downcast_ref::<Label>()
        .is_some_and(|label| label_resolves_icon_only(label, env));
    let metrics = button_metrics(
        theme,
        MENU_TRIGGER_STYLE,
        ControlSize::Small,
        icon_only,
        env.get::<InteractionStyle>(),
        env.get::<FloatingScope>().map(|scope| &scope.0),
    );
    let label_size = if let Some(label) = menu.label.downcast_ref::<Label>()
        && renders_as_plain_title(label)
    {
        let styled = styled_button_title(theme, MENU_TRIGGER_STYLE, label, env);
        HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env)
    } else {
        measure_view_intrinsic(&menu.label, state, env, theme)
    };
    button_chrome_size(label_size, &metrics, ProposalSize::UNSPECIFIED)
}

fn button_label_view(color: Option<Color>, label: AnyView, icon_only: bool) -> AnyView {
    // `.foreground` resolves its colour eagerly while installing, so the
    // icon-only marker must wrap it: a theme's resolvable
    // `WidgetTheme::button_label_color` then sees the marker and paints the
    // standard icon button's content colour rather than the filled text
    // button's.
    let label = match color {
        Some(color) => AnyView::new(label.foreground(color)),
        None => label,
    };
    if icon_only {
        AnyView::new(label.install(crate::IconOnlyButtonLabel))
    } else {
        label
    }
}

/// The theme label color for a general (retained) button label, switching
/// reactively between the enabled and disabled label colors so the retained
/// sub-view recolors without being rebuilt.
/// Marks the subtree of a list row.
///
/// A row's content is body text that happens to be tappable, so it keeps the
/// ordinary foreground colour. Themes paint a container-less button in their
/// accent colour, which is right for a text button on a screen and wrong for
/// every row of a list.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ListRowChrome;

fn disabled_aware_label_color(
    theme: &Rc<dyn crate::engine::WidgetTheme>,
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
    theme: &Rc<dyn crate::engine::WidgetTheme>,
    style: ButtonStyle,
    size: ControlSize,
    icon_only: bool,
    interaction_style: Option<&InteractionStyle>,
    floating_style: Option<&FloatingStyle>,
) -> ButtonMetrics {
    interaction_style.map_or_else(
        || {
            floating_style.map_or_else(
                || {
                    if icon_only {
                        theme.icon_button_metrics(style, size)
                    } else {
                        theme.button_metrics(style, size)
                    }
                },
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
    theme: &Rc<dyn crate::engine::WidgetTheme>,
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
    type Resolved = waterui_graphics::color::WorkingColor;

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
    theme: &Rc<dyn crate::engine::WidgetTheme>,
    style: ButtonStyle,
    label: &Label,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    if renders_as_plain_title(label) {
        let styled = styled_button_title(theme, style, label, env);
        HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env)
    } else {
        let label = styled_button_label(theme, style, label.clone());
        measure_label_intrinsic(&label, state, env, theme)
    }
}

/// Whether a label is nothing but its own title, and can therefore take the
/// cheap path that draws styled text instead of retaining a sub-view.
///
/// A label built from caller-supplied views also reports `TitleOnly` once
/// resolved — it has no icon — so the display mode alone is not the question;
/// answering it with the mode alone drew the spoken text in place of the
/// content.
fn renders_as_plain_title(label: &Label) -> bool {
    matches!(label.display_mode_preference(), LabelDisplayMode::TitleOnly)
        && !label.has_custom_content()
}

/// Whether the label's own configuration resolves to an icon-only
/// presentation. The label resolves its effective display mode — its
/// preference, the `LabelDisplayMode` it inherits from the environment, and
/// whether it carries an icon — so the chrome agrees with what the label
/// draws without inspecting rendered children.
pub(crate) fn label_resolves_icon_only(label: &Label, env: &Environment) -> bool {
    matches!(
        label.effective_display_mode(env),
        LabelDisplayMode::IconOnly
    )
}

fn styled_button_title(
    theme: &Rc<dyn crate::engine::WidgetTheme>,
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
    title.resolve(env).content.snapshot()
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

/// Applies the theme's button label font to a label that owns its text. A
/// label built from caller-supplied views carries its own typography, and
/// `Label::font` rejects it outright.
fn styled_button_label(
    theme: &Rc<dyn crate::engine::WidgetTheme>,
    style: ButtonStyle,
    label: Label,
) -> Label {
    if label.has_custom_content() {
        return label;
    }
    if let Some(font) = theme.button_label_font(style) {
        label.font(font)
    } else {
        label
    }
}

/// Emits a retained button's accessibility node for the semantic walk — the
/// same node `button_accessibility` registers, with no bounds. The label
/// sub-view flushes visual-only (its semantics are merged into the button's
/// node), so there is nothing else to emit.
#[cfg(feature = "accessibility")]
pub(crate) fn emit_button_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<ButtonRenderState>>,
    env: &Environment,
) {
    button_accessibility(renderer, None, state, env);
}

/// Emits a retained menu trigger's accessibility node for the semantic walk —
/// the same node `menu_accessibility` registers, with no bounds.
#[cfg(feature = "accessibility")]
pub(crate) fn emit_menu_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<MenuRenderState>>,
    env: &Environment,
) {
    menu_accessibility(renderer, None, None, state, env);
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
