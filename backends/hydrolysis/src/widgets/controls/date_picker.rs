#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    measure_date_picker_intrinsic,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use std::cell::RefCell;
use std::rc::Rc;
use waterui_core::layout::{HorizontalAlignment, ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};
use waterui_form::picker::PickerStyle;
use waterui_form::picker::date::DatePickerConfig;
use waterui_text::styled::StyledStr;

use crate::renderer::RetainedSubview;
use crate::renderer::local_interaction_state;
use crate::widgets::util::inset_rect;
#[cfg(feature = "accessibility")]
use crate::widgets::util::widget_disabled;

/// The retained render state of a date picker: the cloneable [`DatePickerConfig`]
/// drives the field + accessibility, and its main label is held as a
/// [`RetainedSubview`] built once and re-flushed each frame so reactive label
/// content stays live.
pub struct DatePickerRenderState {
    config: DatePickerConfig,
    label_view: RetainedSubview,
}

impl DatePickerRenderState {
    pub(crate) fn from_config(config: DatePickerConfig) -> Self {
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

impl HydroNativeView for Native<DatePickerConfig> {
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        measure_date_picker_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// Emits a date picker's accessibility node from its config. Shared by the dispatch
/// path ([`Native<DatePickerConfig>::accessibility`]) and the retained `Widget`-node
/// path so both produce the same a11y tree.
// empty when the accessibility feature is off
#[cfg_attr(not(feature = "accessibility"), allow(clippy::missing_const_for_fn))]
pub fn date_picker_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    ctx: Option<RenderContext>,
    date_picker: &DatePickerConfig,
    env: &Environment,
    focus_keys: &[crate::renderer::InteractionKey],
) {
    #[cfg(feature = "accessibility")]
    {
        let disabled = renderer.read_signal(&widget_disabled(env));
        let value = date_picker.ty.format_value(
            renderer
                .read_signal(&date_picker.value)
                .clamp(*date_picker.range.start(), *date_picker.range.end()),
        );
        let default_label = Some(value.clone());
        let mut node =
            AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::ComboBox,
            ));
        let label = renderer.resolve_accessibility_label(env, default_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        // The formatted date is the default value; an explicit `.a11y_value`
        // wins the same way `.a11y_label` wins the name.
        if let Some(value) = renderer.resolve_accessibility_value(env, Some(value)) {
            node.set_value(value);
        }
        node.add_action(AccessibilityAction::Focus);
        if disabled {
            node.set_disabled();
        } else {
            node.add_action(AccessibilityAction::Click);
            node.add_action(AccessibilityAction::SetValue);
        }
        let origin = ctx.map(|ctx| {
            let bounds = ctx.bounds;
            waterui_core::layout::Point::new(
                crate::num_cast::f64_as_f32(bounds.x0),
                crate::num_cast::f64_as_f32(bounds.y1),
            )
        });
        if let Some(node_id) = renderer.register_accessibility_leaf(
            ctx,
            node,
            env,
            (!disabled).then(|| AccessibilityActionTarget::DatePicker {
                value: date_picker.value.clone(),
                range: date_picker.range.clone(),
                ty: date_picker.ty,
                origin,
                // The popup opens in the picker's own environment
                // (water-rs/hydrolysis#140).
                env: env.clone(),
            }),
        ) {
            for key in focus_keys {
                renderer.register_accessibility_focus_link(key, node_id);
            }
        }
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, date_picker, env, focus_keys);
    }
}

/// Measures a retained date-picker leaf from its [`DatePickerRenderState`],
/// mirroring [`measure_date_picker_intrinsic`] but reading the label size from its
/// already-built [`RetainedSubview`] so layout and render agree.
pub fn measure_date_picker_node(
    render_state: &DatePickerRenderState,
    _proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    let config = &render_state.config;
    let metrics = theme.picker_metrics(PickerStyle::Menu);
    let input_metrics = theme.input_field_metrics();
    let label_size = render_state.label_view.measure_built(state, env, theme);
    let has_label = label_size.width > 0.0 || label_size.height > 0.0;
    let label_height = if has_label {
        f64::from(label_size.height).max(input_metrics.label_height)
    } else {
        0.0
    };
    let current = state
        .measure_signal(&config.value)
        .clamp(*config.range.start(), *config.range.end());
    let candidates = [
        config.ty.format_value(*config.range.start()),
        config.ty.format_value(current),
        config.ty.format_value(*config.range.end()),
    ];
    let mut field_text_width: f64 = 0.0;
    let mut field_text_height: f64 = 0.0;
    for candidate in candidates {
        let size = HydrolysisRenderer::measure_text_intrinsic_size(
            state,
            StyledStr::plain(candidate),
            env,
        );
        field_text_width = field_text_width.max(f64::from(size.width));
        field_text_height = field_text_height.max(f64::from(size.height));
    }
    let field_width = (input_metrics
        .horizontal_inset
        .mul_add(2.0, field_text_width)
        + metrics.indicator_space)
        .max(input_metrics.min_width);
    let field_height = input_metrics
        .vertical_inset
        .mul_add(2.0, field_text_height)
        .max(input_metrics.min_height);
    let width = f64::from(label_size.width).max(field_width);
    let height = label_height + field_height;
    ViewDimensions::new(LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    ))
}

/// Renders a retained date-picker leaf every flush: emits a11y (unless hidden)
/// then the field chrome + value + tap target, reading the value signal each frame.
pub fn render_date_picker_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<DatePickerRenderState>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        date_picker_accessibility(
            ctx.renderer_mut(),
            Some(render_ctx),
            &state.borrow().config,
            env,
            &[crate::renderer::InteractionKey::for_rc(state, 0)],
        );
    }
    render_date_picker_parts(ctx, state, env);
}

pub fn render_date_picker_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<DatePickerRenderState>>,
    env: &Environment,
) {
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let theme = ctx.theme();
    let metrics = theme.picker_metrics(PickerStyle::Menu);
    let input_metrics = theme.input_field_metrics();
    let mut state = state.borrow_mut();
    // The value/range/ty are read from the retained config; the label is a retained
    // node sub-view re-flushed at its rect (reactive content stays live).
    let (value_binding, range, ty) = {
        let date_picker = &state.config;
        (
            date_picker.value.clone(),
            date_picker.range.clone(),
            date_picker.ty,
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
        let label_bounds = kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x1,
            (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
        );
        let render_ctx = ctx.render_context();
        let label_area = ctx.safe_area_for(label_bounds);
        state.label_view.flush_in_rect(
            ctx.renderer_mut(),
            render_ctx,
            env,
            ProposalSize::UNSPECIFIED,
            label_bounds,
            label_area,
        );
    }

    let field_bounds = kurbo::Rect::new(
        ctx.bounds.x0,
        ctx.bounds.y0 + label_height,
        ctx.bounds.x1,
        ctx.bounds.y1,
    );
    if field_bounds.width() <= 0.0 || field_bounds.height() <= 0.0 {
        return;
    }

    // Reading the value through `read_signal` watches it (registers a
    // retained-refresh watcher), so a value change schedules a frame and this
    // persistent node re-renders the new formatted value.
    let value = ty.format_value(
        ctx.renderer_mut()
            .read_signal(&value_binding)
            .clamp(*range.start(), *range.end()),
    );

    let hit_bounds = field_bounds;
    let (interaction, press_slot, _) =
        ctx.renderer_mut()
            .bind_interaction_target(interaction_key, hit_bounds, env);
    {
        let interaction =
            local_interaction_state(interaction, ctx.renderer_mut().current_hit_transform());
        ctx.draw_context(|draw| {
            theme.draw_input_field(&mut *draw, field_bounds, interaction);
            theme.draw_picker_indicator(&mut *draw, field_bounds);
            theme.draw_picker_state_layer(&mut *draw, field_bounds, interaction);
        });
    }
    let text_bounds = inset_rect(
        field_bounds,
        input_metrics.horizontal_inset,
        input_metrics.vertical_inset,
    );
    let text_bounds = kurbo::Rect::new(
        text_bounds.x0,
        text_bounds.y0,
        (text_bounds.x1 - metrics.indicator_space).max(text_bounds.x0),
        text_bounds.y1,
    );
    ctx.render_styled_text(
        StyledStr::plain(value),
        HorizontalAlignment::Leading,
        env,
        text_bounds,
    );

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
            renderer.show_date_picker(value_binding.clone(), range.clone(), ty, origin, &env)
        },
    );
}

/// Emits a retained date picker's accessibility nodes for the semantic walk:
/// the field node `date_picker_accessibility` registers, plus the label
/// sub-view — it flushes unsuppressed in the rendered path, so it emits its
/// own nodes here too.
#[cfg(feature = "accessibility")]
pub fn emit_date_picker_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<DatePickerRenderState>>,
    env: &Environment,
) {
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let mut state = state.borrow_mut();
    date_picker_accessibility(renderer, None, &state.config, env, &[interaction_key]);
    state.label_view.emit_accessibility(renderer, env);
}
