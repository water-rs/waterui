#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    measure_date_picker_intrinsic, transformed_rect,
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
use crate::widgets::util::{inset_rect, widget_theme};

/// The retained render state of a date picker: the clonable [`DatePickerConfig`]
/// drives the field + accessibility, and its main label is held as a
/// [`RetainedSubview`] built once and re-flushed each frame so reactive label
/// content stays live.
pub(crate) struct DatePickerRenderState {
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
    pub(crate) fn prebuild(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        self.label_view.ensure_built(renderer, env);
    }
}

impl HydroNativeView for Native<DatePickerConfig> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_date_picker_intrinsic(view.as_inner(), state, env)
    }
}

/// Emits a date picker's accessibility node from its config. Shared by the dispatch
/// path ([`Native<DatePickerConfig>::accessibility`]) and the retained `Widget`-node
/// path so both produce the same a11y tree.
pub(crate) fn date_picker_accessibility(
    renderer: &mut HydrolysisRenderer,
    ctx: RenderContext,
    date_picker: &DatePickerConfig,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let value = date_picker.ty.format_value(
            renderer
                .read_signal(&date_picker.value)
                .clamp(*date_picker.range.start(), *date_picker.range.end()),
        );
        let default_label = Some(value.clone());
        let mut node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::ComboBox),
        );
        let label = renderer.resolve_accessibility_label(env, default_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        node.set_value(value);
        node.add_action(AccessibilityAction::Focus);
        node.add_action(AccessibilityAction::Click);
        node.add_action(AccessibilityAction::SetValue);
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let origin = waterui_core::layout::Point::new(bounds.x0 as f32, bounds.y1 as f32);
        let _ = renderer.register_accessibility_node(
            node,
            bounds,
            env,
            Some(AccessibilityActionTarget::DatePicker {
                value: date_picker.value.clone(),
                range: date_picker.range.clone(),
                ty: date_picker.ty,
                origin,
            }),
        );
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, date_picker, env);
    }
}

/// Measures a retained date-picker leaf from its [`DatePickerRenderState`],
/// mirroring [`measure_date_picker_intrinsic`] but reading the label size from its
/// already-built [`RetainedSubview`] so layout and render agree.
pub(crate) fn measure_date_picker_node(
    render_state: &DatePickerRenderState,
    _proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    let config = &render_state.config;
    let theme = widget_theme(env);
    let metrics = theme.picker_metrics(PickerStyle::Menu);
    let input_metrics = theme.input_field_metrics();
    let label_size = render_state.label_view.measure_built(state, env);
    let has_label = label_size.width > 0.0 || label_size.height > 0.0;
    let label_height = if has_label {
        f64::from(label_size.height).max(input_metrics.label_height)
    } else {
        0.0
    };
    let current = config
        .value
        .get()
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
    let field_width =
        (field_text_width + input_metrics.horizontal_inset * 2.0 + metrics.indicator_space)
            .max(input_metrics.min_width);
    let field_height =
        (field_text_height + input_metrics.vertical_inset * 2.0).max(input_metrics.min_height);
    let width = f64::from(label_size.width).max(field_width);
    let height = label_height + field_height;
    ViewDimensions::new(LayoutSize::new(width as f32, height as f32))
}

/// Renders a retained date-picker leaf every flush: emits a11y (unless hidden)
/// then the field chrome + value + tap target, reading the value signal each frame.
pub(crate) fn render_date_picker_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<DatePickerRenderState>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        date_picker_accessibility(ctx.renderer_mut(), render_ctx, &state.borrow().config, env);
    }
    render_date_picker_parts(ctx, state, env);
}

pub(crate) fn render_date_picker_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<DatePickerRenderState>>,
    env: &Environment,
) {
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let theme = widget_theme(env);
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
        let label_bounds = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x1,
            (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
        );
        let render_ctx = ctx.render_context();
        state
            .label_view
            .flush_in_rect(ctx.renderer_mut(), render_ctx, env, label_bounds);
    }

    let field_bounds = vello::kurbo::Rect::new(
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

    let hit_bounds = transformed_rect(ctx.hit_transform, field_bounds);
    let (interaction, press_slot, _) =
        ctx.renderer_mut()
            .bind_interaction_target(interaction_key, hit_bounds, env);
    {
        let interaction = local_interaction_state(interaction, ctx.hit_transform);
        let mut draw = ctx.draw_context();
        theme.draw_input_field(&mut draw, field_bounds, interaction);
        theme.draw_picker_indicator(&mut draw, field_bounds);
        theme.draw_picker_state_layer(&mut draw, field_bounds, interaction);
    }
    let text_bounds = inset_rect(
        field_bounds,
        input_metrics.horizontal_inset,
        input_metrics.vertical_inset,
    );
    let text_bounds = vello::kurbo::Rect::new(
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

    let origin = waterui_core::layout::Point::new(hit_bounds.x0 as f32, hit_bounds.y1 as f32);
    ctx.renderer_mut().register_interactive_pointer_target(
        hit_bounds,
        press_slot,
        move |renderer, _point, env| {
            renderer.show_date_picker(value_binding.clone(), range.clone(), ty, origin, env)
        },
    );
}
