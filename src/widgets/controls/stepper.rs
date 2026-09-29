#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, RenderContext, WidgetRenderContext, local_interaction_state,
    measure_label_intrinsic, transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use nami::Signal;
use std::cell::RefCell;
use std::rc::Rc;
use waterui_backend_core::widget::StepperEnd;
use waterui_controls::stepper::StepperConfig;
use waterui_core::layout::Size as LayoutSize;
use waterui_core::layout::{ProposalSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};

use crate::renderer::RetainedSubview;
use crate::widgets::util::{label_beside_control_bounds, widget_disabled};

/// The retained render state of a stepper: the cloneable [`StepperConfig`] drives the
/// +/- buttons + accessibility, and its main label is held as a [`RetainedSubview`]
/// built once and re-flushed each frame so reactive label content stays live.
pub(crate) struct StepperRenderState {
    config: StepperConfig,
    label_view: RetainedSubview,
}

impl StepperRenderState {
    pub(crate) fn from_config(config: StepperConfig) -> Self {
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

impl HydroNativeView for Native<StepperConfig> {
    fn intrinsic(
        state: &mut crate::renderer::HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        measure_stepper_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// Emits a stepper's accessibility node from its config. Shared by the dispatch
/// path ([`Native<StepperConfig>::accessibility`]) and the retained `Widget`-node
/// path so both produce the same a11y tree.
pub(crate) fn stepper_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    stepper: &StepperConfig,
    env: &Environment,
    focus_keys: &[crate::renderer::InteractionKey],
) {
    #[cfg(feature = "accessibility")]
    {
        let mut node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::SpinButton),
        );
        let default_label = renderer.accessibility_label_from_label(&stepper.label, env);
        let label = renderer.resolve_accessibility_label(env, default_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        let start = *stepper.range.start();
        let end = *stepper.range.end();
        assert!(
            (start <= end),
            "hydrolysis stepper requires an ordered range"
        );
        // Read value/step through `read_signal` so a change schedules a frame and
        // this persistent node re-renders (and re-publishes the a11y value).
        let current = renderer.read_signal(&stepper.value).clamp(start, end);
        node.set_numeric_value(f64::from(current));
        node.set_min_numeric_value(f64::from(start));
        node.set_max_numeric_value(f64::from(end));
        let step = renderer.read_signal(&stepper.step);
        assert!((step > 0), "hydrolysis stepper requires positive step");
        node.set_numeric_value_step(f64::from(step));
        node.add_action(AccessibilityAction::Focus);
        // A disabled stepper stays in the tree (focusable, announced as
        // disabled) but exposes no value actions and no action target.
        let disabled = renderer.read_signal(&widget_disabled(env));
        let action_target = if disabled {
            node.set_disabled();
            None
        } else {
            node.add_action(AccessibilityAction::Increment);
            node.add_action(AccessibilityAction::Decrement);
            node.add_action(AccessibilityAction::SetValue);
            Some(AccessibilityActionTarget::Stepper {
                value: stepper.value.clone(),
                step: stepper.step.clone(),
                range: stepper.range.clone(),
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
        let _ = (renderer, ctx, stepper, env, focus_keys);
    }
}

/// Measures a retained stepper leaf from its [`StepperRenderState`], reading the
/// label size from its already-built [`RetainedSubview`] so layout and render agree.
pub(crate) fn measure_stepper_node(
    render_state: &StepperRenderState,
    _proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    let metrics = theme.stepper_metrics();
    let label_size = render_state.label_view.measure_built(state, env, theme);
    let controls_width = metrics.button_intrinsic_size * 2.0 + metrics.button_spacing;
    let label_width = f64::from(label_size.width);
    let width = if label_width > 0.0 {
        label_width + metrics.label_spacing + controls_width
    } else {
        controls_width
    };
    let height = f64::from(label_size.height).max(metrics.button_intrinsic_size);
    ViewDimensions::new(LayoutSize::new(width as f32, height as f32))
}

/// Renders a retained stepper leaf every flush: emits a11y (unless hidden) then
/// the label + +/- buttons + tap targets, watching the value/step signals so a
/// change schedules a frame.
pub(crate) fn render_stepper_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<StepperRenderState>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        stepper_accessibility(
            ctx.renderer_mut(),
            Some(render_ctx),
            &state.borrow().config,
            env,
            &[
                crate::renderer::InteractionKey::for_rc(state, 0),
                crate::renderer::InteractionKey::for_rc(state, 1),
            ],
        );
    }
    render_stepper_parts(ctx, state, env);
}

pub(crate) fn render_stepper_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<StepperRenderState>>,
    env: &Environment,
) {
    let minus_interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let plus_interaction_key = crate::renderer::InteractionKey::for_rc(state, 1);
    let theme = ctx.theme();
    let mut state = state.borrow_mut();
    let (range, value, step) = {
        let stepper = &state.config;
        (
            stepper.range.clone(),
            stepper.value.clone(),
            stepper.step.clone(),
        )
    };
    // Watch the value/step signals so a change schedules a frame and this
    // persistent node re-renders (the stepper itself shows only the label, but the
    // bound value drives a11y and any value-formatter display).
    let _ = ctx.renderer_mut().read_signal(&value);
    let _ = ctx.renderer_mut().read_signal(&step);
    // Reading the disabled signal watches it, so a change schedules a frame
    // and this persistent node re-renders (and re-registers input).
    let disabled = {
        let signal = widget_disabled(env);
        ctx.renderer_mut().read_signal(&signal)
    };
    let theme_metrics = theme.stepper_metrics();
    let label_size = state.label_view.measure_intrinsic(ctx.renderer_mut(), env);
    let (controls_bounds, label_bounds) =
        stepper_control_and_label_bounds(ctx.bounds, theme_metrics, f64::from(label_size.height));
    let button_size = controls_bounds.height();
    // The label is a retained node sub-view re-flushed at its rect; reactive
    // content stays live through the node's own per-frame re-flush, with no dispatch.
    // Its semantics are merged into the stepper's own node by
    // `stepper_accessibility`, so the sub-view flushes visual-only.
    if label_bounds.width() > 0.0 {
        // A disabled control dims its label to the theme's disabled-content
        // alpha (Material: on-surface at 38% for default-colored labels).
        ctx.with_clip_rect_scope_if(
            disabled,
            theme.disabled_content_alpha(),
            label_bounds,
            |ctx| {
                let render_ctx = ctx.render_context();
                let label_view = &mut state.label_view;
                ctx.renderer_mut()
                    .with_suppressed_accessibility(|renderer| {
                        label_view.flush_in_rect(
                            renderer,
                            render_ctx,
                            env,
                            ProposalSize::UNSPECIFIED,
                            label_bounds,
                        );
                    });
            },
        );
    }

    let minus_bounds = kurbo::Rect::new(
        controls_bounds.x0,
        controls_bounds.y0,
        controls_bounds.x0 + button_size,
        controls_bounds.y1,
    );
    let plus_bounds = kurbo::Rect::new(
        controls_bounds.x1 - button_size,
        controls_bounds.y0,
        controls_bounds.x1,
        controls_bounds.y1,
    );
    let hit_transform = ctx.hit_transform;
    let minus_hit_bounds = transformed_rect(hit_transform, minus_bounds);
    let plus_hit_bounds = transformed_rect(hit_transform, plus_bounds);
    let (minus_interaction, minus_press_slot, _) = ctx
        .renderer_mut()
        .bind_control_interaction_target(minus_interaction_key, minus_hit_bounds, env, disabled);
    let (plus_interaction, plus_press_slot, _) = ctx
        .renderer_mut()
        .bind_control_interaction_target(plus_interaction_key, plus_hit_bounds, env, disabled);
    let minus_interaction = local_interaction_state(minus_interaction, hit_transform);
    let plus_interaction = local_interaction_state(plus_interaction, hit_transform);
    ctx.with_clip_rect_scope_if(
        disabled,
        theme.disabled_content_alpha(),
        controls_bounds,
        |ctx| {
            let mut draw = ctx.draw_context();
            theme.draw_stepper_button(
                &mut draw,
                minus_bounds,
                StepperEnd::Decrement,
                minus_interaction,
            );
            theme.draw_stepper_decrement_icon(&mut draw, minus_bounds);
            theme.draw_stepper_button_state_layer(
                &mut draw,
                minus_bounds,
                StepperEnd::Decrement,
                minus_interaction,
            );
            theme.draw_stepper_button(
                &mut draw,
                plus_bounds,
                StepperEnd::Increment,
                plus_interaction,
            );
            theme.draw_stepper_increment_icon(&mut draw, plus_bounds);
            theme.draw_stepper_button_state_layer(
                &mut draw,
                plus_bounds,
                StepperEnd::Increment,
                plus_interaction,
            );
        },
    );

    // A disabled stepper registers no tap targets: the pointer neither presses
    // nor steps it. Targets are re-registered every flush, so re-enabling
    // restores interactivity on the next frame.
    if disabled {
        return;
    }

    let range_start = *range.start();
    let range_end = *range.end();
    assert!(
        (range_start <= range_end),
        "hydrolysis stepper requires an ordered range"
    );

    let value_binding_minus = value.clone();
    let value_binding_plus = value;
    let step_signal_minus = step.clone();
    let step_signal_plus = step;

    ctx.renderer_mut().register_interactive_pointer_target(
        minus_hit_bounds,
        minus_press_slot,
        move |_renderer, _point, _env| {
            let step = step_signal_minus.snapshot();
            assert!((step > 0), "hydrolysis stepper requires positive step");
            let current = value_binding_minus.snapshot();
            let next = current.saturating_sub(step).clamp(range_start, range_end);
            value_binding_minus.set(next);
            true
        },
    );
    ctx.renderer_mut().register_interactive_pointer_target(
        plus_hit_bounds,
        plus_press_slot,
        move |_renderer, _point, _env| {
            let step = step_signal_plus.snapshot();
            assert!((step > 0), "hydrolysis stepper requires positive step");
            let current = value_binding_plus.snapshot();
            let next = current.saturating_add(step).clamp(range_start, range_end);
            value_binding_plus.set(next);
            true
        },
    );
}

pub(crate) fn measure_stepper_intrinsic(
    stepper: &StepperConfig,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> LayoutSize {
    let metrics = theme.stepper_metrics();
    let label_size = measure_label_intrinsic(&stepper.label, state, env, theme);
    let controls_width = metrics.button_intrinsic_size * 2.0 + metrics.button_spacing;
    let label_width = f64::from(label_size.width);
    let width = if label_width > 0.0 {
        label_width + metrics.label_spacing + controls_width
    } else {
        controls_width
    };
    let height = f64::from(label_size.height).max(metrics.button_intrinsic_size);
    LayoutSize::new(width as f32, height as f32)
}

/// Emits a retained stepper's accessibility node for the semantic walk — the
/// same node `stepper_accessibility` registers, with no bounds. The label
/// sub-view flushes visual-only, so there is nothing else to emit.
#[cfg(feature = "accessibility")]
pub(crate) fn emit_stepper_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<StepperRenderState>>,
    env: &Environment,
) {
    stepper_accessibility(
        renderer,
        None,
        &state.borrow().config,
        env,
        &[
            crate::renderer::InteractionKey::for_rc(state, 0),
            crate::renderer::InteractionKey::for_rc(state, 1),
        ],
    );
}

/// The strip the `-`/`+` buttons occupy and the label's rect beside them. The
/// buttons keep their size, vertically centred in the row; the label keeps its
/// own height on the buttons' centre line — see [`label_beside_control_bounds`].
fn stepper_control_and_label_bounds(
    bounds: kurbo::Rect,
    metrics: waterui_backend_core::widget::StepperMetrics,
    label_height: f64,
) -> (kurbo::Rect, kurbo::Rect) {
    let button_size = bounds
        .height()
        .clamp(metrics.button_min_size, metrics.button_max_size);
    let controls_width = button_size * 2.0 + metrics.button_spacing;
    let controls_x0 = (bounds.x1 - controls_width).max(bounds.x0);
    let button_y0 = bounds.y0 + ((bounds.height() - button_size) / 2.0).max(0.0);
    let controls = kurbo::Rect::new(
        controls_x0,
        button_y0,
        controls_x0 + controls_width,
        button_y0 + button_size,
    );
    let label = label_beside_control_bounds(
        bounds.x0,
        (controls_x0 - metrics.label_spacing).max(bounds.x0),
        bounds,
        controls,
        label_height,
    );
    (controls, label)
}

#[cfg(test)]
mod tests {
    use super::stepper_control_and_label_bounds;
    use kurbo::Rect;
    use waterui_backend_core::widget::StepperMetrics;

    #[test]
    fn label_shares_the_controls_centre_line() {
        let metrics = StepperMetrics::new(24.0, 32.0, 28.0, 8.0, 8.0);
        let (controls, label) =
            stepper_control_and_label_bounds(Rect::new(16.0, 20.0, 320.0, 60.0), metrics, 16.0);
        assert_eq!(controls, Rect::new(248.0, 24.0, 320.0, 56.0));
        assert_eq!(label, Rect::new(16.0, 32.0, 240.0, 48.0));
        assert_eq!(label.center().y, controls.center().y);
    }
}
