#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use kurbo::Rect;
use nami::Signal;
use std::cell::RefCell;
use std::rc::Rc;
#[cfg(feature = "accessibility")]
use waterui_core::layout::Point as LayoutPoint;
use waterui_core::layout::{HorizontalAlignment, ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};
use waterui_form::picker::color::ColorPickerConfig;
use waterui_graphics::cherenkov::Draw as _;
use waterui_graphics::color::Color;
use waterui_text::styled::StyledStr;

use crate::renderer::RetainedSubview;
use crate::renderer::local_interaction_state;
use crate::renderer::{
    HydroNativeView, HydroState, RenderContext, WidgetRenderContext, measure_label_intrinsic,
    transformed_rect,
};
use crate::widgets::util::inset_rect;
#[cfg(feature = "accessibility")]
use crate::widgets::util::widget_disabled;

const COLOR_SWATCH_SIZE: f64 = 32.0;
const COLOR_SWATCH_RADIUS: f64 = 8.0;
const COLOR_PICKER_MIN_WIDTH: f32 = 160.0;

/// The retained render state of a color picker: the cloneable [`ColorPickerConfig`]
/// drives the swatch + accessibility, and its main label is held as a
/// [`RetainedSubview`] built once and re-flushed each frame so reactive label
/// content stays live.
pub struct ColorPickerRenderState {
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
    pub(crate) fn prebuild(
        &mut self,
        renderer: &mut crate::renderer::SemanticCore,
        env: &Environment,
    ) {
        self.label_view.ensure_built(renderer, env);
    }
}

impl HydroNativeView for Native<ColorPickerConfig> {
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        measure_color_picker_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// Emits a color picker's accessibility node from its config. Shared by the
/// dispatch path ([`Native<ColorPickerConfig>::accessibility`]) and the retained
/// `Widget`-node path so both produce the same a11y tree.
// empty when the accessibility feature is off
#[cfg_attr(not(feature = "accessibility"), allow(clippy::missing_const_for_fn))]
pub fn color_picker_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    color_picker: &ColorPickerConfig,
    env: &Environment,
    focus_keys: &[crate::renderer::InteractionKey],
) {
    #[cfg(feature = "accessibility")]
    {
        let disabled = renderer.read_signal(&widget_disabled(env));
        let label = color_picker
            .label
            .semantic_text()
            .resolve(env)
            .content
            .snapshot()
            .to_plain()
            .to_string();
        let value = format!("{:?}", renderer.read_signal(&color_picker.value));
        let mut node =
            AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::Button,
            ));
        node.set_label(label);
        // The formatted color is the default value; an explicit `.a11y_value`
        // wins the same way `.a11y_label` wins the name.
        if let Some(value) = renderer.resolve_accessibility_value(env, Some(value)) {
            node.set_value(value);
        }
        node.add_action(AccessibilityAction::Focus);
        if disabled {
            node.set_disabled();
        } else {
            node.add_action(AccessibilityAction::Click);
        }
        // Direct semantic activation: `Click` shows the color picker under the
        // trigger's own anchor — the pointer path's exact handler. A rendered
        // node carries the anchor; a semantic node carries none and mounts the
        // same window with no placement at all.
        let origin = ctx.map(|ctx| {
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            LayoutPoint::new(
                crate::num_cast::f64_as_f32(bounds.x0),
                crate::num_cast::f64_as_f32(bounds.y1),
            )
        });
        let action_target = (!disabled).then(|| {
            let value = color_picker.value.clone();
            let support_alpha = color_picker.support_alpha;
            let support_hdr = color_picker.support_hdr;
            // The popup opens in the picker's environment layered over the
            // dispatch's (water-rs/hydrolysis#140).
            let picker_env = env.clone();
            AccessibilityActionTarget::Activate {
                action: Rc::new(RefCell::new(
                    move |renderer: &mut crate::renderer::SemanticCore, env: &Environment| {
                        let env = picker_env.layered_on(env);
                        match origin {
                            Some(origin) => {
                                renderer.show_color_picker(
                                    value.clone(),
                                    support_alpha,
                                    support_hdr,
                                    origin,
                                    &env,
                                );
                            }
                            None => {
                                renderer.activate_color_picker(
                                    value.clone(),
                                    support_alpha,
                                    support_hdr,
                                    &env,
                                );
                            }
                        }
                        true
                    },
                )),
            }
        });
        let node_id = match ctx {
            Some(ctx) => {
                let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
                renderer.register_accessibility_node(node, bounds, env, action_target)
            }
            None => renderer.register_accessibility_node_semantic(node, env, action_target),
        };
        if let Some(node_id) = node_id {
            for key in focus_keys {
                renderer.register_accessibility_focus_link(key, node_id);
            }
        }
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, color_picker, env, focus_keys);
    }
}

/// Measures a retained color-picker leaf from its [`ColorPickerRenderState`],
/// reading the label size from its already-built [`RetainedSubview`] so layout and
/// render agree.
pub fn measure_color_picker_node(
    render_state: &ColorPickerRenderState,
    _proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    let input_metrics = theme.input_field_metrics();
    let label_size = render_state.label_view.measure_built(state, env, theme);
    let label_height = if label_size.width > 0.0 || label_size.height > 0.0 {
        f64::from(label_size.height).max(input_metrics.label_height)
    } else {
        0.0
    };
    ViewDimensions::new(LayoutSize::new(
        COLOR_PICKER_MIN_WIDTH,
        crate::num_cast::f64_as_f32(label_height + input_metrics.min_height),
    ))
}

/// Renders a retained color-picker leaf every flush: emits a11y (unless hidden)
/// then the field chrome + swatch + suffix + tap target, reading the value signal
/// each frame.
pub fn render_color_picker_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ColorPickerRenderState>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        color_picker_accessibility(
            ctx.renderer_mut(),
            Some(render_ctx),
            &state.borrow().config,
            env,
            &[crate::renderer::InteractionKey::for_rc(state, 0)],
        );
    }
    render_color_picker_parts(ctx, state, env);
}

fn measure_color_picker_intrinsic(
    color_picker: &ColorPickerConfig,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> LayoutSize {
    let input_metrics = theme.input_field_metrics();
    let label_size = measure_label_intrinsic(&color_picker.label, state, env, theme);
    let label_height = if label_size.width > 0.0 || label_size.height > 0.0 {
        f64::from(label_size.height).max(input_metrics.label_height)
    } else {
        0.0
    };
    LayoutSize::new(
        COLOR_PICKER_MIN_WIDTH,
        crate::num_cast::f64_as_f32(label_height + input_metrics.min_height),
    )
}

#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn render_color_picker_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<ColorPickerRenderState>>,
    env: &Environment,
) {
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let theme = ctx.theme();
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
                label_view.flush_in_rect(
                    renderer,
                    render_ctx,
                    env,
                    ProposalSize::UNSPECIFIED,
                    label_bounds,
                );
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
        content_bounds.y0 + f64::midpoint(content_bounds.height(), swatch_size),
    );
    // Reading the value through `read_signal` watches it (registers a
    // retained-refresh watcher), so a value change schedules a frame and this
    // persistent node re-renders the new swatch color.
    let color = ctx.renderer_mut().read_signal(&value_binding);
    let swatch_color = color.resolve(env).snapshot();
    {
        let mut draw = ctx.draw_context();
        let swatch_shape = kurbo::RoundedRect::from_rect(swatch_rect, COLOR_SWATCH_RADIUS);
        draw.fill(swatch_shape, swatch_color);
        draw.stroke(
            swatch_shape,
            kurbo::Stroke::new(1.0),
            Color::srgb(0, 0, 0)
                .with_opacity(0.16)
                .resolve(env)
                .snapshot(),
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

    let origin = waterui_core::layout::Point::new(
        crate::num_cast::f64_as_f32(hit_bounds.x0),
        crate::num_cast::f64_as_f32(hit_bounds.y1),
    );
    // The popup opens in the picker's environment layered over the
    // dispatch's (water-rs/hydrolysis#140).
    let picker_env = env.clone();
    ctx.renderer_mut().register_interactive_pointer_target(
        hit_bounds,
        press_slot,
        move |renderer, _point, env| {
            let env = picker_env.layered_on(env);
            renderer.show_color_picker(
                value_binding.clone(),
                support_alpha,
                support_hdr,
                origin,
                &env,
            )
        },
    );
}

/// Emits a retained color picker's accessibility node for the semantic walk —
/// the same node `color_picker_accessibility` registers, with no bounds. The
/// label sub-view flushes visual-only, so there is nothing else to emit.
#[cfg(feature = "accessibility")]
pub fn emit_color_picker_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<ColorPickerRenderState>>,
    env: &Environment,
) {
    color_picker_accessibility(
        renderer,
        None,
        &state.borrow().config,
        env,
        &[crate::renderer::InteractionKey::for_rc(state, 0)],
    );
}
