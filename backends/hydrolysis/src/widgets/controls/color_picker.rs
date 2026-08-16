#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
#[cfg(feature = "accessibility")]
use crate::renderer::accessibility_activation_point;
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use nami::Signal;
use std::cell::RefCell;
use std::rc::Rc;
use vello::kurbo::{Rect, RoundedRectRadii};
use waterui_backend_core::widget::{Brush, DrawContext as _};
use waterui_core::layout::{HorizontalAlignment, ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};
use waterui_form::picker::color::ColorPickerConfig;
use waterui_graphics::color::Color;
use waterui_text::styled::StyledStr;

use crate::renderer::RetainedSubview;
use crate::renderer::local_interaction_state;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    measure_label_intrinsic, resolved_color_to_peniko, transformed_rect,
};
#[cfg(feature = "accessibility")]
use crate::widgets::util::widget_disabled;
use crate::widgets::util::{inset_rect, widget_theme};

const COLOR_SWATCH_SIZE: f64 = 32.0;
const COLOR_SWATCH_RADIUS: f64 = 8.0;
const COLOR_PICKER_MIN_WIDTH: f32 = 160.0;

/// The retained render state of a color picker: the cloneable [`ColorPickerConfig`]
/// drives the swatch + accessibility, and its main label is held as a
/// [`RetainedSubview`] built once and re-flushed each frame so reactive label
/// content stays live.
pub(crate) struct ColorPickerRenderState {
    config: ColorPickerConfig,
    label_view: RetainedSubview,
}

impl ColorPickerRenderState {
    pub(crate) fn from_config(config: ColorPickerConfig) -> Self {
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

impl HydroNativeView for Native<ColorPickerConfig> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_color_picker_intrinsic(view.as_inner(), state, env)
    }
}

/// Emits a color picker's accessibility node from its config. Shared by the
/// dispatch path ([`Native<ColorPickerConfig>::accessibility`]) and the retained
/// `Widget`-node path so both produce the same a11y tree.
pub(crate) fn color_picker_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    color_picker: &ColorPickerConfig,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let disabled = renderer.read_signal(&widget_disabled(env));
        let label = color_picker
            .label
            .semantic_text()
            .resolve(env)
            .content
            .get()
            .to_plain()
            .to_string();
        let value = format!("{:?}", renderer.read_signal(&color_picker.value));
        let mut node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Button),
        );
        node.set_label(label);
        node.set_value(value);
        node.add_action(AccessibilityAction::Focus);
        if disabled {
            node.set_disabled();
        } else {
            node.add_action(AccessibilityAction::Click);
        }
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let activation_point = accessibility_activation_point(bounds);
        let _ = renderer.register_accessibility_node(
            node,
            bounds,
            env,
            (!disabled).then(|| AccessibilityActionTarget::PointerPrimaryClick {
                point: activation_point,
            }),
        );
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, color_picker, env);
    }
}

/// Measures a retained color-picker leaf from its [`ColorPickerRenderState`],
/// reading the label size from its already-built [`RetainedSubview`] so layout and
/// render agree.
pub(crate) fn measure_color_picker_node(
    render_state: &ColorPickerRenderState,
    _proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    let theme = widget_theme(env);
    let input_metrics = theme.input_field_metrics();
    let label_size = render_state.label_view.measure_built(state, env);
    let label_height = if label_size.width > 0.0 || label_size.height > 0.0 {
        f64::from(label_size.height).max(input_metrics.label_height)
    } else {
        0.0
    };
    ViewDimensions::new(LayoutSize::new(
        COLOR_PICKER_MIN_WIDTH,
        (label_height + input_metrics.min_height) as f32,
    ))
}

/// Renders a retained color-picker leaf every flush: emits a11y (unless hidden)
/// then the field chrome + swatch + suffix + tap target, reading the value signal
/// each frame.
pub(crate) fn render_color_picker_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ColorPickerRenderState>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        color_picker_accessibility(ctx.renderer_mut(), render_ctx, &state.borrow().config, env);
    }
    render_color_picker_parts(ctx, state, env);
}

fn measure_color_picker_intrinsic(
    color_picker: &ColorPickerConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let theme = widget_theme(env);
    let input_metrics = theme.input_field_metrics();
    let label_size = measure_label_intrinsic(&color_picker.label, state, env);
    let label_height = if label_size.width > 0.0 || label_size.height > 0.0 {
        f64::from(label_size.height).max(input_metrics.label_height)
    } else {
        0.0
    };
    LayoutSize::new(
        COLOR_PICKER_MIN_WIDTH,
        (label_height + input_metrics.min_height) as f32,
    )
}

pub(crate) fn render_color_picker_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ColorPickerRenderState>>,
    env: &Environment,
) {
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let theme = widget_theme(env);
    let input_metrics = theme.input_field_metrics();
    let mut state = state.borrow_mut();
    // The value/options are read from the retained config; the label is a retained
    // node sub-view re-flushed at its rect (reactive content stays live).
    let (value_binding, support_alpha, support_hdr) = {
        let color_picker = &state.config;
        (
            color_picker.value.clone(),
            color_picker.support_alpha,
            color_picker.support_hdr,
        )
    };
    let label_size = state.label_view.measure_intrinsic(ctx.renderer_mut(), env);
    let has_label = label_size.width > 0.0 || label_size.height > 0.0;
    let label_height = if has_label {
        f64::from(label_size.height).max(input_metrics.label_height)
    } else {
        0.0
    };
    if label_height > 0.0 {
        let label_bounds = Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x1,
            (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
        );
        // The label's semantics are merged into the picker's own node by
        // `color_picker_accessibility`, so the sub-view flushes visual-only.
        let render_ctx = ctx.render_context();
        let label_view = &mut state.label_view;
        ctx.renderer_mut()
            .with_suppressed_accessibility(|renderer| {
                label_view.flush_in_rect(renderer, render_ctx, env, label_bounds);
            });
    }

    let field_bounds = Rect::new(
        ctx.bounds.x0,
        ctx.bounds.y0 + label_height,
        ctx.bounds.x1,
        ctx.bounds.y1,
    );
    if field_bounds.width() <= 0.0 || field_bounds.height() <= 0.0 {
        return;
    }

    let hit_bounds = transformed_rect(ctx.hit_transform, field_bounds);
    let (interaction, press_slot, _) =
        ctx.renderer_mut()
            .bind_interaction_target(interaction_key, hit_bounds, env);
    {
        let interaction = local_interaction_state(interaction, ctx.hit_transform);
        let mut draw = ctx.draw_context();
        theme.draw_input_field(&mut draw, field_bounds, interaction);
        theme.draw_input_field_state_layer(&mut draw, field_bounds, interaction);
    }
    let content_bounds = inset_rect(
        field_bounds,
        input_metrics.horizontal_inset,
        input_metrics.vertical_inset,
    );
    let swatch_size = COLOR_SWATCH_SIZE.min(content_bounds.height()).max(0.0);
    let swatch_rect = Rect::new(
        content_bounds.x0,
        content_bounds.y0 + (content_bounds.height() - swatch_size) / 2.0,
        content_bounds.x0 + swatch_size,
        content_bounds.y0 + (content_bounds.height() + swatch_size) / 2.0,
    );
    // Reading the value through `read_signal` watches it (registers a
    // retained-refresh watcher), so a value change schedules a frame and this
    // persistent node re-renders the new swatch color.
    let color = ctx.renderer_mut().read_signal(&value_binding);
    let swatch_color = resolved_color_to_peniko(color.resolve(env).get());
    {
        let mut draw = ctx.draw_context();
        draw.fill_rounded_rect(
            swatch_rect,
            RoundedRectRadii::from_single_radius(COLOR_SWATCH_RADIUS),
            &Brush::from(swatch_color),
        );
        draw.stroke_rounded_rect(
            swatch_rect,
            RoundedRectRadii::from_single_radius(COLOR_SWATCH_RADIUS),
            &Brush::from(resolved_color_to_peniko(
                Color::srgb(0, 0, 0).with_opacity(0.16).resolve(env).get(),
            )),
            1.0,
        );
    }

    let text_bounds = Rect::new(
        swatch_rect.x1 + input_metrics.horizontal_inset,
        content_bounds.y0,
        content_bounds.x1,
        content_bounds.y1,
    );
    if text_bounds.width() > 0.0 {
        let suffix = match (support_alpha, support_hdr) {
            (true, true) => "alpha_hdr",
            (true, false) => "alpha",
            (false, true) => "hdr",
            (false, false) => "color",
        };
        ctx.render_styled_text(
            StyledStr::plain(crate::localization::text(env, suffix)),
            HorizontalAlignment::Leading,
            env,
            text_bounds,
        );
    }

    let origin = waterui_core::layout::Point::new(hit_bounds.x0 as f32, hit_bounds.y1 as f32);
    ctx.renderer_mut().register_interactive_pointer_target(
        hit_bounds,
        press_slot,
        move |renderer, _point, env| {
            renderer.show_color_picker(
                value_binding.clone(),
                support_alpha,
                support_hdr,
                origin,
                env,
            )
        },
    );
}
