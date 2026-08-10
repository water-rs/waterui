#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
#[cfg(feature = "accessibility")]
use crate::renderer::slider_step_for_range;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    measure_slider_intrinsic, slider_value_epsilon, transformed_rect,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use core::ops::RangeInclusive;
use nami::Binding;
use std::cell::RefCell;
use std::rc::Rc;
use waterui_controls::label::Label;
use waterui_controls::slider::SliderConfig;
use waterui_core::AnyView;
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::layout::Size as LayoutSize;
use waterui_core::layout::{ProposalSize, ViewDimensions};

use crate::renderer::RetainedSubview;
use crate::renderer::local_interaction_state;
use crate::widgets::util::{widget_disabled, widget_theme};

/// The retained render state of a slider. A `SliderConfig`'s value-end labels are
/// move-only `AnyView`s (they cannot be re-dispatched twice), so the persistent
/// `Widget` node holds them as [`RetainedSubview`]s built once and re-flushed each
/// frame; the cloneable `label`/`range`/`value` drive the track and accessibility.
pub(crate) struct SliderRenderState {
    label: Label,
    /// The main label as a retained node sub-view, re-flushed each frame at its
    /// rect (reactive content stays live through the node's own re-flush). The
    /// cloneable `label` is kept alongside for accessibility resolution.
    label_view: RetainedSubview,
    min_value_label: RetainedSubview,
    max_value_label: RetainedSubview,
    range: RangeInclusive<f64>,
    value: Binding<f64>,
}

impl SliderRenderState {
    pub(crate) fn from_config(config: SliderConfig) -> Self {
        let SliderConfig {
            label,
            min_value_label,
            max_value_label,
            range,
            value,
            ..
        } = config;
        Self {
            label_view: RetainedSubview::new(AnyView::new(label.clone())),
            label,
            min_value_label: RetainedSubview::new(min_value_label),
            max_value_label: RetainedSubview::new(max_value_label),
            range,
            value,
        }
    }

    /// Eagerly build the label sub-views (the measure path has only
    /// `&mut HydroState`, no renderer, so labels must be built before then).
    pub(crate) fn prebuild_labels(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        self.label_view.ensure_built(renderer, env);
        self.min_value_label.ensure_built(renderer, env);
        self.max_value_label.ensure_built(renderer, env);
    }
}

impl HydroNativeView for Native<SliderConfig> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_slider_intrinsic(view.as_inner(), state, env)
    }
}

/// Field-level accessibility emission for the [`SliderRenderState`]-based
/// retained node path.
fn slider_accessibility_parts(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    label: &Label,
    range: &RangeInclusive<f64>,
    value: &Binding<f64>,
    disabled: &nami::Computed<bool>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let mut node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Slider),
        );
        let default_label = renderer.accessibility_label_from_label(label, env);
        let resolved = renderer.resolve_accessibility_label(env, default_label);
        if let Some(resolved) = resolved {
            node.set_label(resolved);
        }
        let start = *range.start();
        let end = *range.end();
        assert!(start < end, "hydrolysis slider requires range start < end");
        let current = renderer.read_signal(value).clamp(start, end);
        node.set_numeric_value(current);
        node.set_min_numeric_value(start);
        node.set_max_numeric_value(end);
        node.set_numeric_value_step(slider_step_for_range(range.clone()));
        node.add_action(AccessibilityAction::Focus);
        // A disabled slider stays in the tree (focusable, announced as
        // disabled) but exposes no value actions and no action target.
        let action_target = if renderer.read_signal(disabled) {
            node.set_disabled();
            None
        } else {
            node.add_action(AccessibilityAction::Increment);
            node.add_action(AccessibilityAction::Decrement);
            node.add_action(AccessibilityAction::SetValue);
            Some(AccessibilityActionTarget::Slider {
                value: value.clone(),
                range: range.clone(),
                step: slider_step_for_range(range.clone()),
            })
        };
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let _ = renderer.register_accessibility_node(node, bounds, env, action_target);
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, label, range, value, disabled, env);
    }
}

/// Measures a retained slider leaf from its [`SliderRenderState`], mirroring
/// [`measure_slider_intrinsic`] but reading the value-end labels from their
/// already-built [`RetainedSubview`]s (the measure path has no renderer to build
/// on, so the labels are pre-built at tree-build time).
pub(crate) fn measure_slider_node(
    render_state: &SliderRenderState,
    _proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    let theme = widget_theme(env);
    let metrics = theme.slider_metrics();
    let label_size = render_state.label_view.measure_built(state, env);
    let min_label_size = render_state.min_value_label.measure_built(state, env);
    let max_label_size = render_state.max_value_label.measure_built(state, env);

    let control_row_height = (metrics.thumb_radius * 2.0)
        .max(f64::from(min_label_size.height))
        .max(f64::from(max_label_size.height));
    let label_height = f64::from(label_size.height);
    let intrinsic_height = if label_height > 0.0 {
        label_height + metrics.vertical_spacing + control_row_height
    } else {
        control_row_height
    };

    let min_width = f64::from(label_size.width).max(
        f64::from(min_label_size.width)
            + metrics.horizontal_spacing
            + metrics.min_track_width
            + metrics.horizontal_spacing
            + f64::from(max_label_size.width)
            + metrics.horizontal_inset * 2.0,
    );
    ViewDimensions::new(LayoutSize::new(min_width as f32, intrinsic_height as f32))
}

/// Renders a retained slider leaf every flush: emits a11y (unless hidden) then the
/// track + thumb + labels + drag target, reading the value signal each frame.
pub(crate) fn render_slider_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<SliderRenderState>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        let slider = state.borrow();
        slider_accessibility_parts(
            ctx.renderer_mut(),
            render_ctx,
            &slider.label,
            &slider.range,
            &slider.value,
            &widget_disabled(env),
            env,
        );
    }
    render_slider_parts(ctx, state, env);
}

pub(crate) fn render_slider_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<SliderRenderState>>,
    env: &Environment,
) {
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let theme = widget_theme(env);
    let metrics = theme.slider_metrics();
    let mut state = state.borrow_mut();
    // Reading the disabled signal watches it, so a change schedules a frame
    // and this persistent node re-renders (and re-registers input) with the
    // new state.
    let disabled = {
        let signal = widget_disabled(env);
        ctx.renderer_mut().read_signal(&signal)
    };
    // Every label is a retained node sub-view re-flushed at its rect; reactive
    // content stays live through the node's own per-frame re-flush, with no
    // dispatch. The main label sizes the top row, the value-end labels flank the
    // track. A disabled control dims every label to the theme's
    // disabled-content alpha (Material: on-surface at 38%).
    let label_height = if ctx.bounds.height() >= 36.0 {
        f64::from(
            state
                .label_view
                .measure_intrinsic(ctx.renderer_mut(), env)
                .height,
        )
        .max(20.0)
    } else {
        0.0
    };
    if label_height > 0.0 {
        let label_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x1,
            (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
        );
        if disabled {
            ctx.push_layer_rect(theme.disabled_content_alpha(), label_rect);
        }
        // The label's semantics are merged into the slider's own node by
        // `slider_accessibility_parts`, so the sub-view flushes visual-only.
        // The min/max value labels below stay exposed: their text (e.g.
        // "Dark"/"Bright") is not carried by the slider node.
        let render_ctx = ctx.render_context();
        let label_view = &mut state.label_view;
        ctx.renderer_mut()
            .with_suppressed_accessibility(|renderer| {
                label_view.flush_in_rect(renderer, render_ctx, env, label_rect);
            });
        if disabled {
            ctx.pop_layer();
        }
    }

    let min_label_size = state
        .min_value_label
        .measure_intrinsic(ctx.renderer_mut(), env);
    let max_label_size = state
        .max_value_label
        .measure_intrinsic(ctx.renderer_mut(), env);
    let min_label_width = f64::from(min_label_size.width);
    let max_label_width = f64::from(max_label_size.width);
    let min_label_x0 = ctx.bounds.x0 + metrics.horizontal_inset;
    let min_label_x1 = min_label_x0 + min_label_width;
    let max_label_x1 = ctx.bounds.x1 - metrics.horizontal_inset;
    let max_label_x0 = max_label_x1 - max_label_width;
    let control_top = ctx.bounds.y0 + label_height;
    let control_bottom = ctx.bounds.y1;
    let control_height = control_bottom - control_top;

    if min_label_width > 0.0 && control_height > 0.0 {
        let min_label_rect =
            vello::kurbo::Rect::new(min_label_x0, control_top, min_label_x1, control_bottom);
        if disabled {
            ctx.push_layer_rect(theme.disabled_content_alpha(), min_label_rect);
        }
        let render_ctx = ctx.render_context();
        state
            .min_value_label
            .flush_in_rect(ctx.renderer_mut(), render_ctx, env, min_label_rect);
        if disabled {
            ctx.pop_layer();
        }
    }
    if max_label_width > 0.0 && control_height > 0.0 {
        let max_label_rect =
            vello::kurbo::Rect::new(max_label_x0, control_top, max_label_x1, control_bottom);
        if disabled {
            ctx.push_layer_rect(theme.disabled_content_alpha(), max_label_rect);
        }
        let render_ctx = ctx.render_context();
        state
            .max_value_label
            .flush_in_rect(ctx.renderer_mut(), render_ctx, env, max_label_rect);
        if disabled {
            ctx.pop_layer();
        }
    }

    let range_start = *state.range.start();
    let range_end = *state.range.end();
    let span = range_end - range_start;
    assert!(span > 0.0, "hydrolysis slider requires range start < end");

    let track_left = if min_label_width > 0.0 {
        min_label_x1 + metrics.horizontal_spacing
    } else {
        ctx.bounds.x0 + metrics.horizontal_inset
    };
    let track_right = if max_label_width > 0.0 {
        max_label_x0 - metrics.horizontal_spacing
    } else {
        ctx.bounds.x1 - metrics.horizontal_inset
    };
    let track_center_y = control_top + control_height / 2.0;
    let track_rect = vello::kurbo::Rect::new(
        track_left,
        track_center_y - metrics.track_height / 2.0,
        track_right,
        track_center_y + metrics.track_height / 2.0,
    );

    // Reading the value through `read_signal` watches it (registers a
    // retained-refresh watcher), so a value change schedules a frame and this
    // persistent node re-renders the new fill/thumb position.
    let value_binding = state.value.clone();
    let clamped = ctx
        .renderer_mut()
        .read_signal(&value_binding)
        .clamp(range_start, range_end);
    let progress = (clamped - range_start) / span;
    let fill_right = track_left + (track_right - track_left) * progress;
    let fill_rect = vello::kurbo::Rect::new(
        track_left,
        track_center_y - metrics.track_height / 2.0,
        fill_right,
        track_center_y + metrics.track_height / 2.0,
    );
    let hit_bounds = transformed_rect(
        ctx.hit_transform,
        vello::kurbo::Rect::new(
            track_left - metrics.thumb_radius,
            control_top,
            track_right + metrics.thumb_radius,
            control_bottom,
        ),
    );
    let (interaction, press_slot, _) = ctx.renderer_mut().bind_control_interaction_target(
        interaction_key,
        hit_bounds,
        env,
        disabled,
    );
    let thumb_center = vello::kurbo::Point::new(fill_right, track_center_y);
    {
        let interaction = local_interaction_state(interaction, ctx.hit_transform);
        let mut draw = ctx.draw_context();
        theme.draw_slider_track(&mut draw, track_rect, fill_rect, interaction);
        theme.draw_slider_thumb(&mut draw, thumb_center, metrics.thumb_radius, interaction);
        theme.draw_slider_thumb_state_layer(
            &mut draw,
            thumb_center,
            metrics.thumb_radius,
            interaction,
        );
    }
    let usable_track = track_right - track_left;
    assert!(
        usable_track > 0.0,
        "hydrolysis slider resolved a non-positive track width"
    );
    // A disabled slider registers no drag target: the pointer neither presses
    // nor drags it. Targets are re-registered every flush, so re-enabling
    // restores interactivity on the next frame.
    if disabled {
        return;
    }
    let inverse_transform = ctx.hit_transform.inverse();
    let value_epsilon = slider_value_epsilon(span, usable_track);
    let keyboard_value = value_binding.clone();
    let keyboard_step = span / 100.0;
    ctx.renderer_mut().register_interactive_pointer_drag_target(
        hit_bounds,
        press_slot,
        move |_renderer, point, _env| {
            let local_point = inverse_transform * point;
            let x = local_point.x.clamp(track_left, track_right);
            let t = (x - track_left) / usable_track;
            let next = range_start + span * t;
            if (value_binding.get() - next).abs() <= value_epsilon {
                return false;
            }
            value_binding.set(next);
            true
        },
        move |forward| {
            let current = keyboard_value.get();
            let delta = if forward {
                keyboard_step
            } else {
                -keyboard_step
            };
            let next = (current + delta).clamp(range_start, range_end);
            if (current - next).abs() <= f64::EPSILON {
                return false;
            }
            keyboard_value.set(next);
            true
        },
    );
}
