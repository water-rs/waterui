use crate::animation::AnimationKey;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, RetainedSubview,
    WidgetRenderContext, circle_arc_path, measure_progress_intrinsic,
};
#[cfg(feature = "accessibility")]
use accesskit::{Node as AccessibilityNode, Role as AccessibilityNodeRole};
use core::f64::consts::{FRAC_PI_2, TAU};
use nami::{Computed, Signal};
use std::cell::RefCell;
use std::rc::Rc;
use waterui::component::progress::{ProgressConfig, ProgressStyle};
use waterui_backend_core::widget::ProgressIndicatorStyle;
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::layout::Size as LayoutSize;
use waterui_core::layout::{ProposalSize, ViewDimensions};

use crate::widgets::util::widget_theme;

const LINEAR_DETERMINATE_ANIMATION_KEY: usize = 1;
const CIRCULAR_DETERMINATE_ANIMATION_KEY: usize = 2;

/// The retained render state of a progress indicator. Its `label`/`value_label`
/// are move-only `AnyView`s (they cannot be re-dispatched twice), so the
/// persistent `Widget` node holds them as [`RetainedSubview`]s built once and
/// re-flushed each frame; the `value` signal is read through `read_signal`.
pub(crate) struct ProgressRenderState {
    label: RetainedSubview,
    value_label: RetainedSubview,
    value: Computed<f64>,
    style: ProgressStyle,
    four_color: bool,
}

impl ProgressRenderState {
    pub(crate) fn from_config(config: ProgressConfig) -> Self {
        let ProgressConfig {
            label,
            value_label,
            value,
            style,
            four_color,
            ..
        } = config;
        Self {
            label: RetainedSubview::new(label),
            value_label: RetainedSubview::new(value_label),
            value,
            style,
            four_color,
        }
    }

    /// Eagerly build the label sub-views (the measure path has no renderer).
    pub(crate) fn prebuild_labels(
        &mut self,
        renderer: &mut HydrolysisRenderer,
        env: &Environment,
    ) {
        self.label.ensure_built(renderer, env);
        self.value_label.ensure_built(renderer, env);
    }
}

impl HydroNativeView for Native<ProgressConfig> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_progress_intrinsic(view.as_inner(), state, env)
    }
}

/// Emits a progress indicator's accessibility node. Shared by the dispatch path
/// (passing the config label's extracted default text) and the retained node path
/// (passing the [`RetainedSubview`]'s build-time default text).
pub(crate) fn progress_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    default_label: Option<String>,
    value: &Computed<f64>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let mut node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::ProgressIndicator),
        );
        let resolved = renderer.resolve_accessibility_label(env, default_label);
        if let Some(resolved) = resolved {
            node.set_label(resolved);
        }
        node.set_min_numeric_value(0.0);
        node.set_max_numeric_value(1.0);
        let current = renderer.read_signal(value);
        if current.is_finite() {
            node.set_numeric_value(current.clamp(0.0, 1.0));
        }
        let bounds = crate::renderer::transformed_rect(ctx.hit_transform, ctx.bounds);
        let _ = renderer.register_accessibility_node(node, bounds, env, None);
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, default_label, value, env);
    }
}

/// Measures a retained progress leaf from its [`ProgressRenderState`]. Mirrors
/// [`measure_progress_intrinsic`] (which does not measure the label view — it uses
/// the default font height — so the retained labels are not needed here).
pub(crate) fn measure_progress_node(
    render_state: &ProgressRenderState,
    _proposal: ProposalSize,
    _state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    let theme = widget_theme(env);
    let size = match render_state.style {
        ProgressStyle::Linear => {
            let metrics = theme.progress_metrics(ProgressIndicatorStyle::Linear);
            let label_height = f64::from(waterui_text::font::Font::default().resolve(env).get().size)
                .max(metrics.label_height);
            let value_label_height = if render_state.value.get().is_finite() {
                metrics.value_label_top_spacing + label_height
            } else {
                0.0
            };
            let width = metrics.min_track_width + metrics.bar_horizontal_inset * 2.0;
            let height =
                label_height + metrics.bar_top_offset + metrics.bar_height + value_label_height;
            LayoutSize::new(width as f32, height as f32)
        }
        ProgressStyle::Circular => {
            let metrics = theme.progress_metrics(ProgressIndicatorStyle::Circular);
            LayoutSize::new(
                metrics.circular_diameter as f32,
                metrics.circular_diameter as f32,
            )
        }
        _ => panic!("hydrolysis ProgressStyle variant is not implemented"),
    };
    ViewDimensions::new(size)
}

/// Renders a retained progress leaf every flush: emits a11y (unless hidden) then
/// the track/spinner + labels, reading the `value` signal each frame.
pub(crate) fn render_progress_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ProgressRenderState>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        let (default_label, value) = {
            let progress = state.borrow();
            (progress.label.default_a11y_label(), progress.value.clone())
        };
        progress_accessibility(ctx.renderer_mut(), render_ctx, default_label, &value, env);
    }
    render_progress_parts(ctx, state, env);
}

pub(crate) fn render_progress_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ProgressRenderState>>,
    env: &Environment,
) {
    let theme = widget_theme(env);
    // Stable identity of this progress node: the retained state `Rc`'s address keys
    // the indeterminate repeating phase so it survives structural changes.
    let node_id = Rc::as_ptr(state) as usize;
    let mut progress = state.borrow_mut();
    let style = progress.style;
    let four_color = progress.four_color;
    let value_signal = progress.value.clone();
    let value_identity = value_signal.identity();
    let value = ctx.renderer_mut().read_signal(&value_signal);
    let finite = value.is_finite();
    let clamped = value.clamp(0.0, 1.0) as f32;

    match style {
        ProgressStyle::Linear => {
            let metrics = theme.progress_metrics(ProgressIndicatorStyle::Linear);
            let motion = theme.progress_motion();
            let label_size = progress.label.measure_intrinsic(ctx.renderer_mut(), env);
            let label_height = if label_size.width > 0.0 || label_size.height > 0.0 {
                f64::from(label_size.height).max(metrics.label_height)
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
                let render_ctx = ctx.render_context();
                progress
                    .label
                    .flush_in_rect(ctx.renderer_mut(), render_ctx, env, label_rect);
            }

            let bar_y = ctx.bounds.y0 + label_height + metrics.bar_top_offset;
            let bar_rect = vello::kurbo::Rect::new(
                ctx.bounds.x0 + metrics.bar_horizontal_inset,
                bar_y,
                ctx.bounds.x1 - metrics.bar_horizontal_inset,
                bar_y + metrics.bar_height,
            );
            {
                let mut draw = ctx.draw_context();
                theme.draw_progress_linear_track(&mut draw, bar_rect);
            }
            if finite {
                let animated = if let Some(identity) = value_identity {
                    ctx.renderer_mut().sample_widget_scalar_target(
                        AnimationKey::scalar_with_discriminator(
                            identity,
                            LINEAR_DETERMINATE_ANIMATION_KEY,
                        ),
                        clamped,
                        motion.linear_determinate.clone(),
                    )
                } else {
                    clamped
                };
                let fill_rect = vello::kurbo::Rect::new(
                    bar_rect.x0,
                    bar_rect.y0,
                    bar_rect.x0 + bar_rect.width() * f64::from(animated.clamp(0.0, 1.0)),
                    bar_rect.y1,
                );
                let mut draw = ctx.draw_context();
                theme.draw_progress_linear_fill(&mut draw, fill_rect);
            } else {
                let elapsed = ctx
                    .renderer_mut()
                    .sample_repeating_motion(motion.linear_indeterminate_cycle, node_id);
                let mut draw = ctx.draw_context();
                theme.draw_progress_linear_indeterminate(&mut draw, bar_rect, elapsed, four_color);
            }

            if finite {
                let value_label_rect = vello::kurbo::Rect::new(
                    ctx.bounds.x0,
                    bar_rect.y1 + metrics.value_label_top_spacing,
                    ctx.bounds.x1,
                    ctx.bounds.y1,
                );
                if value_label_rect.height() > 0.0 {
                    let render_ctx = ctx.render_context();
                    progress.value_label.flush_in_rect(
                        ctx.renderer_mut(),
                        render_ctx,
                        env,
                        value_label_rect,
                    );
                }
            }
        }
        ProgressStyle::Circular => {
            let metrics = theme.progress_metrics(ProgressIndicatorStyle::Circular);
            let center = vello::kurbo::Point::new(
                ctx.bounds.x0 + ctx.bounds.width() / 2.0,
                ctx.bounds.y0 + ctx.bounds.height() / 2.0,
            );
            let stroke_width = metrics.circular_stroke_width;
            let radius =
                (ctx.bounds.width().min(ctx.bounds.height()) - stroke_width).max(0.0) / 2.0;
            let motion = theme.progress_motion();
            if finite {
                let animated = if let Some(identity) = value_identity {
                    ctx.renderer_mut().sample_widget_scalar_target(
                        AnimationKey::scalar_with_discriminator(
                            identity,
                            CIRCULAR_DETERMINATE_ANIMATION_KEY,
                        ),
                        clamped,
                        motion.circular_determinate.clone(),
                    )
                } else {
                    clamped
                };
                let arc = circle_arc_path(center, radius, -FRAC_PI_2, TAU * f64::from(animated));
                let mut draw = ctx.draw_context();
                theme.draw_progress_circular_track(&mut draw, center, radius, stroke_width);
                theme.draw_progress_circular_fill(&mut draw, &arc, stroke_width);
            } else {
                let elapsed = ctx
                    .renderer_mut()
                    .sample_repeating_motion(motion.circular_indeterminate_cycle, node_id);
                let mut draw = ctx.draw_context();
                theme.draw_progress_circular_indeterminate(
                    &mut draw,
                    center,
                    radius,
                    stroke_width,
                    elapsed,
                    four_color,
                );
            }
        }
        _ => {
            panic!("hydrolysis ProgressStyle variant is not implemented");
        }
    }
}
