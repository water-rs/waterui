#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, InteractionKey, RenderContext, WidgetRenderContext,
    measure_label_intrinsic,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
    Toggled as AccessibilityToggled,
};
use std::cell::RefCell;
use std::rc::Rc;
use waterui_controls::toggle::{ToggleConfig, ToggleStyle};
use waterui_core::layout::Size as LayoutSize;
use waterui_core::layout::{ProposalSize, StretchAxis, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};

use crate::renderer::local_interaction_state;
use crate::renderer::{RetainedIdentity, RetainedSubview};
use crate::widgets::util::{label_beside_control_bounds, widget_disabled};

/// The retained render state of a toggle: the cloneable [`ToggleConfig`] drives the
/// control + accessibility, and its main label is held as a [`RetainedSubview`]
/// built once and re-flushed each frame so reactive label content stays live.
pub struct ToggleRenderState {
    config: ToggleConfig,
    label_view: RetainedSubview,
}

impl ToggleRenderState {
    pub(crate) fn from_config(config: ToggleConfig) -> Self {
        Self {
            label_view: RetainedSubview::new(AnyView::new(config.label.clone())),
            config,
        }
    }

    /// Eagerly build the label sub-view (the measure path has only
    /// `&mut HydroState`, no renderer, so it must be built before then).
    pub(crate) fn prebuild(
        &mut self,
        renderer: &mut crate::renderer::SemanticCore,
        env: &Environment,
    ) {
        self.label_view.ensure_built(renderer, env);
    }
}

impl HydroNativeView for Native<ToggleConfig> {
    fn intrinsic(
        state: &mut crate::renderer::HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        measure_toggle_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// Emits a toggle's accessibility node from its config. Shared by the rendered
/// `Widget`-node flush (which passes its [`RenderContext`]) and the semantic
/// emission walk (which passes `None` — a semantic node carries no bounds).
// empty when the accessibility feature is off
#[cfg_attr(not(feature = "accessibility"), allow(clippy::missing_const_for_fn))]
pub fn toggle_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    toggle: &ToggleConfig,
    env: &Environment,
    focus_keys: &[crate::renderer::InteractionKey],
) {
    #[cfg(feature = "accessibility")]
    {
        let mut node =
            AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                env,
                match toggle.style {
                    ToggleStyle::Switch => AccessibilityNodeRole::Switch,
                    ToggleStyle::Checkbox => AccessibilityNodeRole::CheckBox,
                    ToggleStyle::Automatic => panic!("{UNRESOLVED_STYLE}"),
                    _ => panic!("hydrolysis ToggleStyle variant is not implemented"),
                },
            ));
        let default_label =
            crate::renderer::SemanticCore::accessibility_label_from_label(&toggle.label, env);
        let label = renderer.resolve_accessibility_label(env, default_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        if let Some(value) = renderer.resolve_accessibility_value(env, None) {
            node.set_value(value);
        }
        let checked = renderer.read_signal(&toggle.toggle);
        node.set_toggled(AccessibilityToggled::from(checked));
        node.add_action(AccessibilityAction::Focus);
        // A disabled toggle stays in the tree (focusable, announced as
        // disabled) but exposes no click action and no action target.
        let disabled = renderer.read_signal(&widget_disabled(env));
        let action_target = if disabled {
            node.set_disabled();
            None
        } else {
            node.add_action(AccessibilityAction::Click);
            Some(AccessibilityActionTarget::Toggle {
                binding: toggle.toggle.clone(),
            })
        };
        if let Some(node_id) = renderer.register_accessibility_leaf(ctx, node, env, action_target) {
            for key in focus_keys {
                renderer.register_accessibility_focus_link(key, node_id);
            }
        }
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, toggle, env, focus_keys);
    }
}

/// The native payload's style is resolved before a backend sees it
/// (`ToggleConfig`'s resolver owns the platform default), so `Automatic`
/// reaching a match here is a broken contract, not a style to draw.
const UNRESOLVED_STYLE: &str =
    "the toggle payload reached hydrolysis with an unresolved Automatic style";

/// Measures a retained toggle leaf from its [`ToggleRenderState`], reading the
/// label size from its already-built [`RetainedSubview`] so layout and render agree.
///
/// A switch with a visible label (`StretchAxis::Horizontal`) answers a finite
/// width proposal, never below its intrinsic width; every other toggle
/// answers its intrinsic size (`docs/layout-spec.md` §6).
pub fn measure_toggle_node(
    render_state: &ToggleRenderState,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    let metrics = theme.toggle_metrics(render_state.config.style);
    let label_size = render_state.label_view.measure_built(state, env, theme);
    let label_width = f64::from(label_size.width);
    let width = if label_width > 0.0 {
        label_width + metrics.label_spacing + metrics.width
    } else {
        metrics.width
    };
    let height = f64::from(label_size.height).max(metrics.height);
    let width = match (
        waterui_core::NativeView::stretch_axis(&render_state.config),
        proposal.width,
    ) {
        (StretchAxis::Horizontal, Some(proposed)) if proposed.is_finite() => {
            f64::from(proposed).max(width)
        }
        _ => width,
    };
    ViewDimensions::new(LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    ))
}

/// Renders a retained toggle leaf every flush: emits a11y (unless hidden) then the
/// control + label + tap target, reading the config's live signals each frame.
pub fn render_toggle_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ToggleRenderState>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        toggle_accessibility(
            ctx.renderer_mut(),
            Some(render_ctx),
            &state.borrow().config,
            env,
            &[crate::renderer::InteractionKey::for_rc(state, 0)],
        );
    }
    render_toggle_parts(ctx, state, env);
}

#[expect(
    clippy::too_many_lines,
    reason = "the render sequence is one continuous scenario; splitting it would obscure the order"
)]
pub fn render_toggle_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ToggleRenderState>>,
    env: &Environment,
) {
    let visual_interaction_key = InteractionKey::for_rc(state, 0);
    let activation_interaction_key = InteractionKey::for_rc(state, 1);
    // The thumb-progress slot keys off the retained node, not the toggle
    // signal: `Binding::mapping`s minted at one call site share a
    // `SignalIdentity`, so signal-keyed slots would collapse every same-site
    // mapping onto one thumb position. The interaction key stored in the
    // interaction engine holds the owner `Rc` for as long as the control
    // binds, so the address cannot be reused while the slot lives.
    let toggle_identity = RetainedIdentity::for_rc(state);
    let theme = ctx.theme();
    let mut state = state.borrow_mut();
    let style = state.config.style;
    let metrics = theme.toggle_metrics(style);
    // Reading the disabled signal watches it, so a change schedules a frame
    // and this persistent node re-renders (and re-registers input) with the
    // new state.
    let disabled = {
        let signal = widget_disabled(env);
        ctx.renderer_mut().read_signal(&signal)
    };
    // The label is a retained node sub-view re-flushed at its rect; reactive
    // content stays live through the node's own per-frame re-flush, with no dispatch.
    let label_size = state.label_view.measure_intrinsic(ctx.renderer_mut(), env);
    let (control_bounds, label_bounds) =
        toggle_control_and_label_bounds(ctx.bounds, style, metrics, label_size);
    if label_bounds.width() > 0.0 {
        // A disabled control dims its label to the theme's disabled-content
        // alpha (Material: on-surface at 38% for default-colored labels).
        // The label's semantics are merged into the toggle's own node by
        // `toggle_accessibility`, so the sub-view flushes visual-only.
        ctx.with_scope_if(
            disabled,
            crate::renderer::mount::ScopeKey {
                role: "label-dim",
                item: 0,
            },
            theme.disabled_content_alpha(),
            label_bounds,
            |ctx| {
                let render_ctx = ctx.render_context();
                let label_area = ctx.safe_area_for(label_bounds);
                let label_view = &mut state.label_view;
                ctx.renderer_mut()
                    .with_suppressed_accessibility(|renderer| {
                        label_view.place(
                            renderer,
                            render_ctx,
                            env,
                            ProposalSize::UNSPECIFIED,
                            label_bounds,
                            label_area,
                        );
                    });
            },
        );
    }

    // Reading the toggle value through `resolve_toggle_progress` watches the
    // signal (it registers a retained-refresh watcher), so a value change
    // schedules a frame and this persistent node re-renders the new state.
    let (thumb_progress, selected) = {
        let binding = state.config.toggle.clone();
        ctx.renderer_mut().resolve_toggle_progress(
            &binding,
            &toggle_identity,
            theme.toggle_value_animation(),
        )
    };
    let visual_hit_bounds = control_bounds;
    let activation_hit_bounds = ctx.bounds;
    let hit_transform = ctx.renderer_mut().current_hit_transform();
    let (interaction, press_slot, _) = ctx.renderer_mut().bind_control_interaction_target(
        visual_interaction_key.clone(),
        visual_hit_bounds,
        env,
        disabled,
    );
    let interaction = local_interaction_state(interaction, hit_transform);
    {
        ctx.draw_context(|draw| match style {
            ToggleStyle::Switch => {
                theme.draw_toggle_switch(
                    &mut *draw,
                    control_bounds,
                    thumb_progress,
                    selected,
                    interaction,
                );
                theme.draw_toggle_switch_state_layer(
                    &mut *draw,
                    control_bounds,
                    thumb_progress,
                    selected,
                    interaction,
                );
            }
            ToggleStyle::Checkbox => {
                theme.draw_toggle_checkbox(&mut *draw, control_bounds, thumb_progress, interaction);
                theme.draw_toggle_checkbox_state_layer(
                    &mut *draw,
                    control_bounds,
                    thumb_progress,
                    interaction,
                );
            }
            ToggleStyle::Automatic => panic!("{UNRESOLVED_STYLE}"),
            _ => panic!("hydrolysis ToggleStyle variant is not implemented"),
        });
    }
    // A disabled toggle registers no tap target: the pointer neither presses
    // nor toggles it. Targets are re-registered every flush, so re-enabling
    // restores interactivity on the next frame.
    if disabled {
        return;
    }
    // Keep the label row as an activation target without attaching it to the
    // switch's visual interaction. An associated the Material label toggles the switch
    // but does not originate a switch ripple or pressed thumb from the label's
    // distant pointer coordinate.
    let binding = state.config.toggle.clone();
    if label_bounds.width() > 0.0 {
        let mut activation_press_slot = press_slot.clone();
        activation_press_slot.key = activation_interaction_key;
        ctx.renderer_mut()
            .register_interactive_pointer_target_with_keyboard(
                activation_hit_bounds,
                activation_press_slot,
                false,
                toggle_binding_action(binding.clone(), visual_interaction_key.clone()),
            );
    }
    // Register the visual control last so it wins hit testing where the full-row
    // activation target overlaps it.
    ctx.renderer_mut().register_interactive_pointer_target(
        visual_hit_bounds,
        press_slot,
        toggle_binding_action(binding, visual_interaction_key),
    );
}

fn toggle_binding_action(
    binding: nami::Binding<bool>,
    visual_interaction_key: InteractionKey,
) -> impl FnMut(&mut crate::renderer::SemanticCore, kurbo::Point, &Environment) -> bool {
    move |renderer, _point, _env| {
        // A pointer press on the non-focusable label target temporarily owns an
        // interaction-only key. Restore semantic keyboard focus to the switch
        // itself when the label activates it.
        renderer.set_keyboard_focus(Some(visual_interaction_key.clone()), false);
        binding.toggle();
        true
    }
}

pub fn measure_toggle_intrinsic(
    toggle: &ToggleConfig,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> LayoutSize {
    let metrics = theme.toggle_metrics(toggle.style);
    let label_size = measure_label_intrinsic(&toggle.label, state, env, theme);
    let label_width = f64::from(label_size.width);
    let width = if label_width > 0.0 {
        label_width + metrics.label_spacing + metrics.width
    } else {
        metrics.width
    };
    let height = f64::from(label_size.height).max(metrics.height);
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    )
}

#[expect(
    clippy::similar_names,
    reason = "the names follow the fixture domain vocabulary; renaming would obscure rather than clarify"
)]
fn toggle_control_and_label_bounds(
    bounds: kurbo::Rect,
    style: ToggleStyle,
    metrics: waterui_backend_core::widget::ToggleMetrics,
    label_size: LayoutSize,
) -> (kurbo::Rect, kurbo::Rect) {
    let control_y0 = bounds.y0 + ((bounds.height() - metrics.height) / 2.0).max(0.0);
    let control_y1 = control_y0 + metrics.height;
    let has_label = label_size.width > 0.0 || label_size.height > 0.0;
    match style {
        ToggleStyle::Checkbox => {
            let control_x0 = bounds.x0;
            let control_x1 = control_x0 + metrics.width;
            let control = kurbo::Rect::new(control_x0, control_y0, control_x1, control_y1);
            let label_x0 = if has_label {
                (control_x1 + metrics.label_spacing).min(bounds.x1)
            } else {
                control_x1
            };
            let max_label_width = (bounds.x1 - label_x0).max(0.0);
            let label_width = f64::from(label_size.width).min(max_label_width);
            (
                control,
                label_beside_control_bounds(
                    label_x0,
                    label_x0 + label_width,
                    bounds,
                    control,
                    f64::from(label_size.height),
                ),
            )
        }
        ToggleStyle::Switch => {
            let control_x0 = (bounds.x1 - metrics.width).max(bounds.x0);
            let control = kurbo::Rect::new(
                control_x0,
                control_y0,
                control_x0 + metrics.width,
                control_y1,
            );
            let label_x1 = if has_label {
                (control_x0 - metrics.label_spacing).max(bounds.x0)
            } else {
                bounds.x0
            };
            (
                control,
                label_beside_control_bounds(
                    bounds.x0,
                    label_x1,
                    bounds,
                    control,
                    f64::from(label_size.height),
                ),
            )
        }
        ToggleStyle::Automatic => panic!("{UNRESOLVED_STYLE}"),
        _ => panic!("hydrolysis ToggleStyle variant is not implemented"),
    }
}

/// Emits a retained toggle's accessibility node for the semantic walk — the
/// same node `toggle_accessibility` registers, with no bounds. The label
/// sub-view flushes visual-only, so there is nothing else to emit.
#[cfg(feature = "accessibility")]
pub fn emit_toggle_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<ToggleRenderState>>,
    env: &Environment,
) {
    toggle_accessibility(
        renderer,
        None,
        &state.borrow().config,
        env,
        &[crate::renderer::InteractionKey::for_rc(state, 0)],
    );
}

#[cfg(test)]
mod tests {
    use super::toggle_control_and_label_bounds;
    use kurbo::Rect;
    use waterui_backend_core::widget::ToggleMetrics;
    use waterui_controls::toggle::ToggleStyle;
    use waterui_core::layout::Size;

    #[test]
    fn checkbox_layout_places_control_before_label() {
        let metrics = ToggleMetrics::new(18.0, 18.0, 8.0);
        let (control, label) = toggle_control_and_label_bounds(
            Rect::new(16.0, 20.0, 320.0, 60.0),
            ToggleStyle::Checkbox,
            metrics,
            Size::new(64.0, 16.0),
        );

        assert_eq!(control, Rect::new(16.0, 31.0, 34.0, 49.0));
        assert_eq!(label, Rect::new(42.0, 32.0, 106.0, 48.0));
    }

    #[test]
    fn switch_layout_keeps_control_trailing() {
        let metrics = ToggleMetrics::new(52.0, 32.0, 8.0);
        let (control, label) = toggle_control_and_label_bounds(
            Rect::new(16.0, 20.0, 320.0, 60.0),
            ToggleStyle::Switch,
            metrics,
            Size::new(64.0, 16.0),
        );

        assert_eq!(control, Rect::new(268.0, 24.0, 320.0, 56.0));
        assert_eq!(label, Rect::new(16.0, 32.0, 260.0, 48.0));
    }

    #[test]
    fn label_shares_the_control_centre_line() {
        let metrics = ToggleMetrics::new(52.0, 32.0, 8.0);
        let bounds = Rect::new(16.0, 20.0, 320.0, 60.0);
        for style in [ToggleStyle::Switch, ToggleStyle::Checkbox] {
            let metrics = match style {
                ToggleStyle::Checkbox => ToggleMetrics::new(18.0, 18.0, 8.0),
                _ => metrics,
            };
            let (control, label) =
                toggle_control_and_label_bounds(bounds, style, metrics, Size::new(64.0, 16.0));
            assert!(
                (label.center().y - control.center().y).abs() < 1e-9,
                "{style:?}: label centre {} != control centre {}",
                label.center().y,
                control.center().y,
            );
        }
    }
}
