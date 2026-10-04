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
use nami::{Binding, Signal};
use std::cell::RefCell;
use std::rc::Rc;
use waterui_controls::ControlSize;
use waterui_controls::label::Label;
use waterui_controls::slider::{SliderConfig, ValueFormatter};
use waterui_core::AnyView;
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::interaction::InteractionState;
use waterui_core::layout::Size as LayoutSize;
use waterui_core::layout::{HorizontalAlignment, ProposalSize, ViewDimensions};
use waterui_text::styled::StyledStr;

use crate::renderer::RetainedSubview;
use crate::renderer::local_interaction_state;
use crate::widgets::util::{label_beside_control_bounds, widget_disabled};

/// The retained render state of a slider. A `SliderConfig`'s value-end labels are
/// move-only `AnyView`s (they cannot be re-dispatched twice), so the persistent
/// `Widget` node holds them as [`RetainedSubview`]s built once and re-flushed each
/// frame; the cloneable `label`/`range`/`value` drive the track and accessibility.
pub struct SliderRenderState {
    label: Label,
    /// The main label as a retained node sub-view, re-flushed each frame at its
    /// rect (reactive content stays live through the node's own re-flush). The
    /// cloneable `label` is kept alongside for accessibility resolution.
    label_view: RetainedSubview,
    min_value_label: RetainedSubview,
    max_value_label: RetainedSubview,
    range: RangeInclusive<f64>,
    value: Binding<f64>,
    size: ControlSize,
    value_indicator: Option<ValueFormatter>,
}

impl SliderRenderState {
    pub(crate) fn from_config(config: SliderConfig) -> Self {
        let SliderConfig {
            label,
            min_value_label,
            max_value_label,
            range,
            value,
            size,
            value_indicator,
            ..
        } = config;
        Self {
            label_view: RetainedSubview::new(AnyView::new(label.clone())),
            label,
            min_value_label: RetainedSubview::new(min_value_label),
            max_value_label: RetainedSubview::new(max_value_label),
            range,
            value,
            size,
            value_indicator,
        }
    }

    /// Eagerly build the label sub-views (the measure path has only
    /// `&mut HydroState`, no renderer, so labels must be built before then).
    pub(crate) fn prebuild_labels(
        &mut self,
        renderer: &mut crate::renderer::SemanticCore,
        env: &Environment,
    ) {
        self.label_view.ensure_built(renderer, env);
        self.min_value_label.ensure_built(renderer, env);
        self.max_value_label.ensure_built(renderer, env);
    }
}

impl HydroNativeView for Native<SliderConfig> {
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        measure_slider_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// Field-level accessibility emission for the [`SliderRenderState`]-based
/// retained node path.
// empty when the accessibility feature is off
#[cfg_attr(not(feature = "accessibility"), allow(clippy::missing_const_for_fn))]
fn slider_accessibility_parts(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    label: &Label,
    range: &RangeInclusive<f64>,
    value: &Binding<f64>,
    env: &Environment,
    focus_keys: &[crate::renderer::InteractionKey],
) {
    #[cfg(feature = "accessibility")]
    {
        let mut node =
            AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::Slider,
            ));
        let default_label =
            crate::renderer::SemanticCore::accessibility_label_from_label(label, env);
        let resolved = renderer.resolve_accessibility_label(env, default_label);
        if let Some(resolved) = resolved {
            node.set_label(resolved);
        }
        // The string value is the spoken form of the numeric value: an
        // explicit `.a11y_value` overrides it while `set_numeric_value` keeps
        // the raw position, matching `aria-valuetext` beside `aria-valuenow`.
        if let Some(value) = renderer.resolve_accessibility_value(env, None) {
            node.set_value(value);
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
        let action_target = if renderer.read_signal(&widget_disabled(env)) {
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
        if let Some(node_id) = renderer.register_accessibility_leaf(ctx, node, env, action_target) {
            for key in focus_keys {
                renderer.register_accessibility_focus_link(key, node_id);
            }
        }
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, label, range, value, env, focus_keys);
    }
}

/// Measures a retained slider leaf from its [`SliderRenderState`], mirroring
/// [`measure_slider_intrinsic`] but reading the value-end labels from their
/// already-built [`RetainedSubview`]s (the measure path has no renderer to build
/// on, so the labels are pre-built at tree-build time).
pub fn measure_slider_node(
    render_state: &SliderRenderState,
    _proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    let metrics = theme.slider_metrics(render_state.size);
    let label_size = render_state.label_view.measure_built(state, env, theme);
    let min_label_size = render_state
        .min_value_label
        .measure_built(state, env, theme);
    let max_label_size = render_state
        .max_value_label
        .measure_built(state, env, theme);

    let control_row_height = metrics
        .handle_height
        .max(f64::from(min_label_size.height))
        .max(f64::from(max_label_size.height));
    let label_height = f64::from(label_size.height);
    let intrinsic_height = if label_height > 0.0 {
        label_height + metrics.vertical_spacing + control_row_height
    } else {
        control_row_height
    };

    let min_width = f64::from(label_size.width).max(metrics.horizontal_inset.mul_add(
        2.0,
        f64::from(min_label_size.width)
            + metrics.horizontal_spacing
            + metrics.min_track_width
            + metrics.horizontal_spacing
            + f64::from(max_label_size.width),
    ));
    ViewDimensions::new(LayoutSize::new(
        crate::num_cast::f64_as_f32(min_width),
        crate::num_cast::f64_as_f32(intrinsic_height),
    ))
}

/// Renders a retained slider leaf every flush: emits a11y (unless hidden) then the
/// track + thumb + labels + drag target, reading the value signal each frame.
pub fn render_slider_node(
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
            Some(render_ctx),
            &slider.label,
            &slider.range,
            &slider.value,
            env,
            &[crate::renderer::InteractionKey::for_rc(state, 0)],
        );
    }
    render_slider_parts(ctx, state, env);
}

#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn render_slider_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<SliderRenderState>>,
    env: &Environment,
) {
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let theme = ctx.theme();
    let mut state = state.borrow_mut();
    let metrics = theme.slider_metrics(state.size);
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
        let label_rect = kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x1,
            (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
        );
        // The label's semantics are merged into the slider's own node by
        // `slider_accessibility_parts`, so the sub-view flushes visual-only.
        // The min/max value labels below stay exposed: their text (e.g.
        // "Dark"/"Bright") is not carried by the slider node.
        ctx.with_clip_rect_scope_if(
            disabled,
            theme.disabled_content_alpha(),
            label_rect,
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
                            label_rect,
                        );
                    });
            },
        );
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
    let controls_row = kurbo::Rect::new(ctx.bounds.x0, control_top, ctx.bounds.x1, control_bottom);
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
    let track_rect = kurbo::Rect::new(
        track_left,
        track_center_y - metrics.track_height / 2.0,
        track_right,
        track_center_y + metrics.track_height / 2.0,
    );

    if min_label_width > 0.0 && control_height > 0.0 {
        let min_label_rect = label_beside_control_bounds(
            min_label_x0,
            min_label_x1,
            controls_row,
            track_rect,
            f64::from(min_label_size.height),
        );
        ctx.with_clip_rect_scope_if(
            disabled,
            theme.disabled_content_alpha(),
            min_label_rect,
            |ctx| {
                let render_ctx = ctx.render_context();
                state.min_value_label.flush_in_rect(
                    ctx.renderer_mut(),
                    render_ctx,
                    env,
                    ProposalSize::UNSPECIFIED,
                    min_label_rect,
                );
            },
        );
    }
    if max_label_width > 0.0 && control_height > 0.0 {
        let max_label_rect = label_beside_control_bounds(
            max_label_x0,
            max_label_x1,
            controls_row,
            track_rect,
            f64::from(max_label_size.height),
        );
        ctx.with_clip_rect_scope_if(
            disabled,
            theme.disabled_content_alpha(),
            max_label_rect,
            |ctx| {
                let render_ctx = ctx.render_context();
                state.max_value_label.flush_in_rect(
                    ctx.renderer_mut(),
                    render_ctx,
                    env,
                    ProposalSize::UNSPECIFIED,
                    max_label_rect,
                );
            },
        );
    }

    let range_start = *state.range.start();
    let range_end = *state.range.end();
    let span = range_end - range_start;
    assert!(span > 0.0, "hydrolysis slider requires range start < end");

    // Reading the value through `read_signal` watches it (registers a
    // retained-refresh watcher), so a value change schedules a frame and this
    // persistent node re-renders the new fill/thumb position.
    let value_binding = state.value.clone();
    let clamped = ctx
        .renderer_mut()
        .read_signal(&value_binding)
        .clamp(range_start, range_end);
    let progress = (clamped - range_start) / span;
    let fill_right = f64::mul_add(track_right - track_left, progress, track_left);
    let fill_rect = kurbo::Rect::new(
        track_left,
        track_center_y - metrics.track_height / 2.0,
        fill_right,
        track_center_y + metrics.track_height / 2.0,
    );
    let hit_bounds = transformed_rect(
        ctx.hit_transform,
        kurbo::Rect::new(
            track_left - metrics.handle_overhang(),
            control_top,
            track_right + metrics.handle_overhang(),
            control_bottom,
        ),
    );
    let (interaction, press_slot, _) = ctx.renderer_mut().bind_control_interaction_target(
        interaction_key,
        hit_bounds,
        env,
        disabled,
    );
    let thumb_center = kurbo::Point::new(fill_right, track_center_y);
    let interaction = local_interaction_state(interaction, ctx.hit_transform);
    {
        ctx.draw_context(|draw| {
            theme.draw_slider_track(&mut *draw, track_rect, fill_rect, state.size, interaction);
            theme.draw_slider_thumb(
                &mut *draw,
                thumb_center,
                metrics.handle_overhang(),
                state.size,
                interaction,
            );
            theme.draw_slider_thumb_state_layer(
                &mut *draw,
                thumb_center,
                metrics.handle_overhang(),
                state.size,
                interaction,
            );
        });
    }

    // The value indicator floats above the thumb while the pointer holds the
    // drag; the theme draws the chrome and the renderer lays the formatted
    // value inside it. It stays visual-only — the slider node already carries
    // the live numeric value to assistive technology.
    if interaction.state.contains(InteractionState::PRESSED)
        && let Some(formatter) = &state.value_indicator
    {
        let indicator_metrics = theme.slider_value_indicator_metrics();
        let label = StyledStr::plain(formatter.format(clamped))
            .font(theme.slider_value_indicator_font())
            .foreground(theme.slider_value_indicator_color());
        let text_size = HydrolysisRenderer::measure_text_dimensions(
            ctx.state_mut(),
            label.clone(),
            HorizontalAlignment::Center,
            env,
            None,
            Some(1),
        )
        .size;
        let bubble_width = indicator_metrics
            .padding_x
            .mul_add(2.0, f64::from(text_size.width))
            .max(indicator_metrics.min_width);
        let bubble_height = indicator_metrics
            .padding_y
            .mul_add(2.0, f64::from(text_size.height))
            .max(indicator_metrics.min_height);
        let thumb_top = track_center_y - metrics.handle_height / 2.0;
        let bubble_bottom = thumb_top - indicator_metrics.thumb_gap;
        let bubble_center_x = thumb_center.x.clamp(
            ctx.bounds.x0 + bubble_width / 2.0,
            ctx.bounds.x1 - bubble_width / 2.0,
        );
        let bubble = kurbo::Rect::new(
            bubble_center_x - bubble_width / 2.0,
            bubble_bottom - bubble_height,
            bubble_center_x + bubble_width / 2.0,
            bubble_bottom,
        );
        {
            ctx.draw_context(|draw| {
                theme.draw_slider_value_indicator(&mut *draw, bubble);
            });
        }
        let text_rect = kurbo::Rect::new(
            bubble.x0,
            (bubble.height() - f64::from(text_size.height)).mul_add(0.5, bubble.y0),
            bubble.x1,
            bubble.y1,
        );
        let text_ctx = RenderContext {
            transform: ctx.transform,
            hit_transform: ctx.hit_transform,
            bounds: text_rect,
        };
        let (hydro, scene) = ctx.renderer_mut().state_and_scene_mut();
        HydrolysisRenderer::render_styled_text(
            hydro,
            scene,
            text_ctx,
            label,
            HorizontalAlignment::Center,
            env,
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
            let next = span.mul_add(t, range_start);
            if (value_binding.snapshot() - next).abs() <= value_epsilon {
                return false;
            }
            value_binding.set(next);
            true
        },
        move |forward| {
            let current = keyboard_value.snapshot();
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

/// Emits a retained slider's accessibility nodes for the semantic walk: the
/// slider node `slider_accessibility_parts` registers, plus the minimum and
/// maximum value labels — they flush unsuppressed in the rendered path, so
/// they emit their own nodes here too. The main label flushes visual-only and
/// emits nothing.
#[cfg(feature = "accessibility")]
pub fn emit_slider_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<SliderRenderState>>,
    env: &Environment,
) {
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let mut state = state.borrow_mut();
    slider_accessibility_parts(
        renderer,
        None,
        &state.label,
        &state.range,
        &state.value,
        env,
        &[interaction_key],
    );
    state.min_value_label.emit_accessibility(renderer, env);
    state.max_value_label.emit_accessibility(renderer, env);
}
