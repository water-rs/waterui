#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, InteractionKey, RenderContext,
    WidgetRenderContext, measure_label_intrinsic, transformed_rect,
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
use waterui_core::layout::{ProposalSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};

use crate::renderer::RetainedSubview;
use crate::renderer::local_interaction_state;
use crate::widgets::util::widget_theme;

/// The retained render state of a toggle: the cloneable [`ToggleConfig`] drives the
/// control + accessibility, and its main label is held as a [`RetainedSubview`]
/// built once and re-flushed each frame so reactive label content stays live.
pub(crate) struct ToggleRenderState {
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
    pub(crate) fn prebuild(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        self.label_view.ensure_built(renderer, env);
    }
}

impl HydroNativeView for Native<ToggleConfig> {
    fn intrinsic(
        state: &mut crate::renderer::HydroState,
        view: &Self,
        env: &Environment,
    ) -> LayoutSize {
        measure_toggle_intrinsic(view.as_inner(), state, env)
    }
}

/// Emits a toggle's accessibility node from its config. Shared by the dispatch
/// path ([`Native<ToggleConfig>::accessibility`]) and the retained `Widget`-node
/// path so both produce the same a11y tree.
pub(crate) fn toggle_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    toggle: &ToggleConfig,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let mut node = AccessibilityNode::new(renderer.resolve_accessibility_role(
            env,
            match toggle.style {
                ToggleStyle::Automatic | ToggleStyle::Switch => AccessibilityNodeRole::Switch,
                ToggleStyle::Checkbox => AccessibilityNodeRole::CheckBox,
                _ => panic!("hydrolysis ToggleStyle variant is not implemented"),
            },
        ));
        let default_label = renderer.accessibility_label_from_label(&toggle.label, env);
        let label = renderer.resolve_accessibility_label(env, default_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        let checked = renderer.read_signal(&toggle.toggle);
        node.set_toggled(AccessibilityToggled::from(checked));
        node.add_action(AccessibilityAction::Focus);
        // A disabled toggle stays in the tree (focusable, announced as
        // disabled) but exposes no click action and no action target.
        let disabled = renderer.read_signal(&toggle.disabled);
        let action_target = if disabled {
            node.set_disabled();
            None
        } else {
            node.add_action(AccessibilityAction::Click);
            Some(AccessibilityActionTarget::Toggle {
                binding: toggle.toggle.clone(),
            })
        };
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let _ = renderer.register_accessibility_node(node, bounds, env, action_target);
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, toggle, env);
    }
}

/// Measures a retained toggle leaf from its [`ToggleRenderState`], reading the
/// label size from its already-built [`RetainedSubview`] so layout and render agree.
pub(crate) fn measure_toggle_node(
    render_state: &ToggleRenderState,
    _proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    let theme = widget_theme(env);
    let metrics = theme.toggle_metrics(render_state.config.style);
    let label_size = render_state.label_view.measure_built(state, env);
    let label_width = f64::from(label_size.width);
    let width = if label_width > 0.0 {
        label_width + metrics.label_spacing + metrics.width
    } else {
        metrics.width
    };
    let height = f64::from(label_size.height).max(metrics.height);
    ViewDimensions::new(LayoutSize::new(width as f32, height as f32))
}

/// Renders a retained toggle leaf every flush: emits a11y (unless hidden) then the
/// control + label + tap target, reading the config's live signals each frame.
pub(crate) fn render_toggle_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ToggleRenderState>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        toggle_accessibility(ctx.renderer_mut(), render_ctx, &state.borrow().config, env);
    }
    render_toggle_parts(ctx, state, env);
}

pub(crate) fn render_toggle_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ToggleRenderState>>,
    env: &Environment,
) {
    let visual_interaction_key = InteractionKey::for_rc(state, 0);
    let activation_interaction_key = InteractionKey::for_rc(state, 1);
    let theme = widget_theme(env);
    let mut state = state.borrow_mut();
    let style = state.config.style;
    let metrics = theme.toggle_metrics(style);
    // Reading the disabled signal watches it, so a change schedules a frame
    // and this persistent node re-renders (and re-registers input) with the
    // new state.
    let disabled = {
        let signal = state.config.disabled.clone();
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
        if disabled {
            ctx.push_layer_rect(theme.disabled_content_alpha(), label_bounds);
        }
        // The label's semantics are merged into the toggle's own node by
        // `toggle_accessibility`, so the sub-view flushes visual-only.
        let render_ctx = ctx.render_context();
        let label_view = &mut state.label_view;
        ctx.renderer_mut()
            .with_suppressed_accessibility(|renderer| {
                label_view.flush_in_rect(renderer, render_ctx, env, label_bounds);
            });
        if disabled {
            ctx.pop_layer();
        }
    }

    // Reading the toggle value through `resolve_toggle_progress` watches the
    // signal (it registers a retained-refresh watcher), so a value change
    // schedules a frame and this persistent node re-renders the new state.
    let (thumb_progress, selected) = {
        let binding = state.config.toggle.clone();
        ctx.renderer_mut()
            .resolve_toggle_progress(&binding, theme.toggle_value_animation())
    };
    let visual_hit_bounds = transformed_rect(ctx.hit_transform, control_bounds);
    let activation_hit_bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
    let (interaction, press_slot, _) = ctx.renderer_mut().bind_control_interaction_target(
        visual_interaction_key.clone(),
        visual_hit_bounds,
        env,
        disabled,
    );
    let interaction = local_interaction_state(interaction, ctx.hit_transform);
    {
        let mut draw = ctx.draw_context();
        match style {
            ToggleStyle::Automatic | ToggleStyle::Switch => {
                theme.draw_toggle_switch(
                    &mut draw,
                    control_bounds,
                    thumb_progress,
                    selected,
                    interaction,
                );
                theme.draw_toggle_switch_state_layer(
                    &mut draw,
                    control_bounds,
                    thumb_progress,
                    selected,
                    interaction,
                );
            }
            ToggleStyle::Checkbox => {
                theme.draw_toggle_checkbox(&mut draw, control_bounds, thumb_progress, interaction);
                theme.draw_toggle_checkbox_state_layer(
                    &mut draw,
                    control_bounds,
                    thumb_progress,
                    interaction,
                );
            }
            _ => panic!("hydrolysis ToggleStyle variant is not implemented"),
        }
    }
    // A disabled toggle registers no tap target: the pointer neither presses
    // nor toggles it. Targets are re-registered every flush, so re-enabling
    // restores interactivity on the next frame.
    if disabled {
        return;
    }
    // Keep the label row as an activation target without attaching it to the
    // switch's visual interaction. An associated mdui label toggles the switch
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
) -> impl FnMut(&mut HydrolysisRenderer, vello::kurbo::Point, &Environment) -> bool {
    move |renderer, _point, _env| {
        // A pointer press on the non-focusable label target temporarily owns an
        // interaction-only key. Restore semantic keyboard focus to the switch
        // itself when the label activates it.
        renderer.set_keyboard_focus(Some(visual_interaction_key.clone()), false);
        binding.set(!binding.get());
        true
    }
}

pub(crate) fn measure_toggle_intrinsic(
    toggle: &ToggleConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    let metrics = theme.toggle_metrics(toggle.style);
    let label_size = measure_label_intrinsic(&toggle.label, state, env);
    let label_width = f64::from(label_size.width);
    let width = if label_width > 0.0 {
        label_width + metrics.label_spacing + metrics.width
    } else {
        metrics.width
    };
    let height = f64::from(label_size.height).max(metrics.height);
    LayoutSize::new(width as f32, height as f32)
}

fn toggle_control_and_label_bounds(
    bounds: vello::kurbo::Rect,
    style: ToggleStyle,
    metrics: waterui_backend_core::widget::ToggleMetrics,
    label_size: LayoutSize,
) -> (vello::kurbo::Rect, vello::kurbo::Rect) {
    let control_y0 = bounds.y0 + ((bounds.height() - metrics.height) / 2.0).max(0.0);
    let control_y1 = control_y0 + metrics.height;
    let has_label = label_size.width > 0.0 || label_size.height > 0.0;
    match style {
        ToggleStyle::Checkbox => {
            let control_x0 = bounds.x0;
            let control_x1 = control_x0 + metrics.width;
            let label_x0 = if has_label {
                (control_x1 + metrics.label_spacing).min(bounds.x1)
            } else {
                control_x1
            };
            let max_label_width = (bounds.x1 - label_x0).max(0.0);
            let label_width = f64::from(label_size.width).min(max_label_width);
            let label_height = f64::from(label_size.height).min(bounds.height());
            let label_y0 = bounds.y0 + (bounds.height() - label_height) * 0.5;
            (
                vello::kurbo::Rect::new(control_x0, control_y0, control_x1, control_y1),
                vello::kurbo::Rect::new(
                    label_x0,
                    label_y0,
                    label_x0 + label_width,
                    label_y0 + label_height,
                ),
            )
        }
        ToggleStyle::Automatic | ToggleStyle::Switch => {
            let control_x0 = (bounds.x1 - metrics.width).max(bounds.x0);
            let label_x1 = if has_label {
                (control_x0 - metrics.label_spacing).max(bounds.x0)
            } else {
                bounds.x0
            };
            (
                vello::kurbo::Rect::new(
                    control_x0,
                    control_y0,
                    control_x0 + metrics.width,
                    control_y1,
                ),
                vello::kurbo::Rect::new(bounds.x0, bounds.y0, label_x1, bounds.y1),
            )
        }
        _ => panic!("hydrolysis ToggleStyle variant is not implemented"),
    }
}

#[cfg(test)]
mod tests {
    use super::toggle_control_and_label_bounds;
    use vello::kurbo::Rect;
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
        assert_eq!(label, Rect::new(16.0, 20.0, 260.0, 60.0));
    }
}
