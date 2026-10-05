use crate::animation::AnimationKey;
use crate::platform::TextInputPurpose;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RetainedSubview, TailMark, TextInputModel,
    TextInputTargetRegistration, TextSelectionSlot, WidgetRenderContext, clamp_to_char_boundary,
    measure_label_intrinsic, measure_secure_field_intrinsic,
    measure_secure_field_size_with_label_size, measure_text_field_intrinsic,
    measure_text_field_size_with_label_size, transformed_rect,
};
use core::num::NonZeroUsize;
use nami::Signal;
use std::cell::RefCell;
use std::rc::Rc;
use waterui::cursor::CursorStyle;
use waterui_backend_core::widget::ModalInteraction;
use waterui_controls::text_field::ResolvedTextFieldConfig;
use waterui_core::layout::{HorizontalAlignment, ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native, Str};
use waterui_form::secure::SecureFieldConfig;
use waterui_text::styled::{Style, StyledStr};

/// The retained render state of a text field: the cloneable [`ResolvedTextFieldConfig`]
/// drives the input model + accessibility, and its floating label is held as a
/// [`RetainedSubview`] built once and re-flushed each frame under the animated
/// label transform so reactive label content stays live.
pub struct TextFieldRenderState {
    config: ResolvedTextFieldConfig,
    label_view: RetainedSubview,
    /// The caret/selection anchor+focus for this field, owned by the node so it
    /// persists across frames without a renderer-global, flush-order-indexed slot
    /// pool. Node-owned control state is reset only when this node is dropped.
    selection_slot: Rc<RefCell<TextSelectionSlot>>,
}

impl TextFieldRenderState {
    pub(crate) fn from_config(config: ResolvedTextFieldConfig) -> Self {
        Self {
            label_view: RetainedSubview::new(AnyView::new(config.label.clone())),
            config,
            selection_slot: Rc::new(RefCell::new(TextSelectionSlot::default())),
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

/// The retained render state of a secure field: the cloneable [`SecureFieldConfig`]
/// drives the input model + accessibility, and its floating label is held as a
/// [`RetainedSubview`] built once and re-flushed each frame under the animated
/// label transform so reactive label content stays live.
pub struct SecureFieldRenderState {
    config: SecureFieldConfig,
    label_view: RetainedSubview,
    /// Node-owned caret/selection state; see [`TextFieldRenderState::selection_slot`].
    selection_slot: Rc<RefCell<TextSelectionSlot>>,
}

impl SecureFieldRenderState {
    pub(crate) fn from_config(config: SecureFieldConfig) -> Self {
        Self {
            label_view: RetainedSubview::new(AnyView::new(config.label.clone())),
            config,
            selection_slot: Rc::new(RefCell::new(TextSelectionSlot::default())),
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

#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};

use crate::renderer::local_interaction_state;
use crate::widgets::util::widget_disabled;

const FLOATING_LABEL_SCALE: f64 = 0.75;
const CONTENT_VISIBLE_PORTION: f32 = 5.0 / 9.0;
const CONTENT_ENTER_DELAY_PORTION: f32 = 1.0 - CONTENT_VISIBLE_PORTION;
const TEXT_FIELD_LABEL_ANIMATION_KEY: usize = 1;
const SECURE_FIELD_LABEL_ANIMATION_KEY: usize = 2;

impl HydroNativeView for Native<ResolvedTextFieldConfig> {
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        measure_text_field_intrinsic(view.as_inner(), state, env, theme)
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let text_field = view.as_inner();
        let label_size = measure_label_intrinsic(&text_field.label, state, env, theme);
        ViewDimensions::new(measure_text_field_size_with_label_size(
            text_field, label_size, state, env, theme, proposal,
        ))
    }
}

impl HydroNativeView for Native<SecureFieldConfig> {
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        measure_secure_field_intrinsic(view.as_inner(), state, env, theme)
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let secure_field = view.as_inner();
        let label_size = measure_label_intrinsic(&secure_field.label, state, env, theme);
        ViewDimensions::new(measure_secure_field_size_with_label_size(
            secure_field,
            label_size,
            state,
            env,
            theme,
            proposal,
        ))
    }
}

/// Renders a retained text-field leaf every flush: text fields are
/// render-driven a11y, so the inline a11y is emitted by `render_text_field_parts`
/// itself; this node suppresses it when the field is accessibility-hidden (the
/// dispatch path's render-driven suppression contract).
pub fn render_text_field_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<TextFieldRenderState>>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    #[cfg(feature = "accessibility")]
    if hidden {
        ctx.renderer_mut().push_accessibility_suppression();
    }
    render_text_field_parts(ctx, state, env);
    #[cfg(feature = "accessibility")]
    if hidden {
        ctx.renderer_mut().pop_accessibility_suppression();
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn render_text_field_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<TextFieldRenderState>>,
    env: &Environment,
) {
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let theme = ctx.theme();
    let input_metrics = theme.input_field_metrics();
    ctx.renderer_mut()
        .set_text_caret_motion(theme.text_caret_motion());
    let mut state = state.borrow_mut();
    // The config's `disabled` already OR-combines the explicit
    // `TextField::disabled` signal with any enclosing `.disabled(...)` scope
    // (`TextFieldConfig` resolves it through `Disabled::resolve`). Reading it
    // watches it, so a change schedules a frame and this persistent node
    // re-renders (and re-registers input) with the new state.
    let disabled = {
        let signal = widget_disabled(env);
        ctx.renderer_mut().read_signal(&signal)
    };
    // Read every retained field from the retained config each frame: the `label`/
    // `value`/`prompt`/`selection_menu` are cloneable signals (the value is read
    // through `read_signal` below so a binding change schedules a frame). The label
    // is a retained node sub-view flushed under the animated transform each frame.
    let (label, value_binding, prompt_signal, selection_menu, line_limit_raw, on_submit) = {
        let text_field = &state.config;
        (
            text_field.label.clone(),
            text_field.value.clone(),
            text_field.prompt.content.clone(),
            text_field.selection_menu.clone(),
            text_field.line_limit,
            text_field.on_submit.clone(),
        )
    };
    #[cfg(feature = "accessibility")]
    let default_accessibility_label =
        crate::renderer::SemanticCore::accessibility_label_from_label(&label, env);
    #[cfg(not(feature = "accessibility"))]
    let _ = label;
    let label_size = state.label_view.measure_intrinsic(ctx.renderer_mut(), env);
    let label_height = material_input_label_height(label_size, input_metrics.label_height);
    let line_limit = line_limit_raw.map(NonZeroUsize::get);
    #[cfg(feature = "accessibility")]
    {
        let prompt = ctx
            .renderer_mut()
            .read_signal(&prompt_signal)
            .to_plain()
            .to_string();
        let value = ctx
            .renderer_mut()
            .read_signal(&value_binding)
            .to_plain()
            .to_string();
        let default_label =
            default_accessibility_label.or_else(|| (!prompt.is_empty()).then_some(prompt.clone()));
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let mut node =
            AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                env,
                if line_limit == Some(1) {
                    AccessibilityNodeRole::TextInput
                } else {
                    AccessibilityNodeRole::MultilineTextInput
                },
            ));
        let label = ctx
            .renderer_mut()
            .resolve_accessibility_label(env, default_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        if !prompt.is_empty() {
            node.set_placeholder(prompt);
        }
        // The field's own text is the default value; an explicit
        // `.a11y_value` wins the same way `.a11y_label` wins the name.
        if let Some(value) = ctx
            .renderer_mut()
            .resolve_accessibility_value(env, (!value.is_empty()).then_some(value))
        {
            node.set_value(value);
        }
        if disabled {
            node.set_disabled();
        } else {
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            node.add_action(AccessibilityAction::SetValue);
        }
        if let Some(node_id) = ctx.renderer_mut().register_accessibility_node(
            node,
            bounds,
            env,
            (!disabled).then_some(AccessibilityActionTarget::TextField {
                value: value_binding.clone(),
                line_limit,
            }),
        ) {
            ctx.renderer_mut()
                .push_pending_text_input_accessibility_node(node_id);
        }
    }
    let field_rect = ctx.bounds;
    let hit_transform = ctx.hit_transform;
    let is_focused = ctx.renderer_mut().is_text_input_focused(&interaction_key);
    let (mut field_interaction, _, _) = ctx.renderer_mut().bind_focused_control_interaction_target(
        interaction_key.clone(),
        transformed_rect(hit_transform, field_rect),
        env,
        is_focused,
        disabled,
    );
    field_interaction = local_interaction_state(field_interaction, hit_transform);
    {
        ctx.draw_context(|draw| {
            theme.draw_input_field(&mut *draw, field_rect, field_interaction);
            theme.draw_input_field_state_layer(&mut *draw, field_rect, field_interaction);
        });
    }
    let selection_slot = Rc::clone(&state.selection_slot);
    let value_identity = value_binding.identity();
    let input_model = TextInputModel::TextField {
        value: value_binding.clone(),
        line_limit,
        selection_menu,
        on_submit,
    };
    let (prompt, value, preedit, preedit_caret) = {
        let (preedit, preedit_caret) = if is_focused {
            (
                ctx.renderer_mut().current_ime_preedit().unwrap_or_default(),
                ctx.renderer_mut().current_ime_preedit_caret(),
            )
        } else {
            (Str::new(), None)
        };
        (
            ctx.renderer_mut().read_signal(&prompt_signal).to_plain(),
            ctx.renderer_mut().read_signal(&value_binding).to_plain(),
            preedit,
            preedit_caret,
        )
    };
    // Normalize the selection first: the composition is inserted at — and
    // replaces — the live selection, so the display string and the caret
    // mapping below both need the clamped range.
    let (selection_start, selection_end) = {
        let mut slot = selection_slot.borrow_mut();
        if !slot.initialized {
            slot.anchor = value.len();
            slot.focus = value.len();
            slot.initialized = true;
        }
        slot.anchor = clamp_to_char_boundary(value.as_str(), slot.anchor);
        slot.focus = clamp_to_char_boundary(value.as_str(), slot.focus);
        (slot.anchor.min(slot.focus), slot.anchor.max(slot.focus))
    };
    let committed_with_preedit = if preedit.is_empty() {
        value.clone()
    } else {
        let mut text = String::with_capacity(value.len() + preedit.len());
        text.push_str(&value[..selection_start]);
        text.push_str(preedit.as_str());
        text.push_str(&value[selection_end..]);
        Str::from(text)
    };
    let use_placeholder = committed_with_preedit.is_empty();
    // With no label view the prompt stands in as the floating label (#85):
    // it rests centred in the container and floats to the top on focus or
    // content under the same Material transition a label view rides.
    let prompt_as_label = label_height == 0.0 && !prompt.is_empty();
    let label_target = if is_focused || !committed_with_preedit.is_empty() {
        1.0
    } else {
        0.0
    };
    let interaction_motion = theme.interaction_motion();
    let label_progress = if let Some(identity) = value_identity {
        ctx.renderer_mut().sample_widget_scalar_target(
            AnimationKey::scalar_with_discriminator(identity, TEXT_FIELD_LABEL_ANIMATION_KEY),
            label_target,
            if label_target > 0.0 {
                interaction_motion.focus_enter
            } else {
                interaction_motion.focus_exit
            },
        )
    } else {
        label_target
    };
    // The prompt-as-label is sized the way a label view is: its own laid-out
    // height floored at the theme's minimum label height. The minimum alone
    // shrinks the Material line box and drops the resting prompt below
    // centre (#1705).
    let prompt_styled = prompt_as_label
        .then(|| StyledStr::plain(prompt.clone()).foreground(theme.input_placeholder_color()));
    let prompt_label_height = prompt_styled.as_ref().map_or(0.0, |styled| {
        material_input_label_height(
            HydrolysisRenderer::measure_text_dimensions(
                ctx.state_mut(),
                styled.clone(),
                HorizontalAlignment::Leading,
                env,
                None,
                Some(1),
            )
            .size,
            input_metrics.label_height,
        )
    });
    if label_height > 0.0 {
        flush_material_label(
            ctx,
            env,
            &mut state.label_view,
            field_rect,
            input_metrics.horizontal_inset,
            label_height,
            label_progress,
        );
    } else if let Some(prompt_styled) = prompt_styled {
        flush_material_prompt_label(
            ctx,
            env,
            prompt_styled,
            field_rect,
            input_metrics.horizontal_inset,
            prompt_label_height,
            label_progress,
        );
    }
    let content_alpha =
        material_input_content_alpha(label_height > 0.0 || prompt_as_label, label_progress);
    let display = if use_placeholder && !prompt_as_label {
        prompt
    } else {
        committed_with_preedit.clone()
    };
    let display_styled = if use_placeholder && !prompt_as_label {
        StyledStr::plain(display).foreground(theme.input_placeholder_color())
    } else if preedit.is_empty() {
        StyledStr::plain(display)
    } else {
        // The pre-edit run carries the composing underline a native text
        // field draws under the IME's composing span; the committed text
        // either side of the splice stays plain.
        let mut styled = StyledStr::empty();
        styled.push_str(value[..selection_start].to_string());
        styled.push(preedit.clone(), Style::new().underline());
        styled.push_str(value[selection_end..].to_string());
        styled
    };
    let effective_label_height = if prompt_as_label {
        prompt_label_height
    } else {
        label_height
    };
    let text_bounds = material_input_text_rect(
        field_rect,
        input_metrics.horizontal_inset,
        input_metrics.vertical_inset,
        effective_label_height,
    );
    let committed_layout = HydrolysisRenderer::build_text_layout(
        ctx.state_mut(),
        StyledStr::plain(value.clone()),
        HorizontalAlignment::Leading,
        env,
        Some(crate::num_cast::f64_as_f32(text_bounds.width())),
    );
    let display_layout = HydrolysisRenderer::build_text_layout(
        ctx.state_mut(),
        display_styled.clone(),
        HorizontalAlignment::Leading,
        env,
        Some(crate::num_cast::f64_as_f32(text_bounds.width())),
    );
    let display_layout_height = display_layout.height();
    // A single-line field carrying no inside label — neither a label view
    // nor a prompt standing in as one — centres its input text vertically
    // in the container, the label's resting spot.
    let text_bounds = if effective_label_height == 0.0 && line_limit == Some(1) {
        material_input_centered_text_rect(
            field_rect,
            text_bounds,
            f64::from(committed_layout.height().max(display_layout_height))
                .max(input_metrics.label_height),
        )
    } else {
        text_bounds
    };
    let text_clip_bounds = material_input_text_clip_rect(
        field_rect,
        text_bounds,
        committed_layout.height().max(display_layout_height),
    );
    let selection = {
        let mut slot = selection_slot.borrow_mut();
        let anchor_layout = input_model.layout_index_from_plain_index(slot.anchor);
        let focus_layout = input_model.layout_index_from_plain_index(slot.focus);
        let anchor_affinity = if anchor_layout >= value.len() {
            parley::Affinity::Upstream
        } else {
            parley::Affinity::Downstream
        };
        let focus_affinity = if focus_layout >= value.len() {
            parley::Affinity::Upstream
        } else {
            parley::Affinity::Downstream
        };
        let selection = parley::Selection::new(
            parley::Cursor::from_byte_index(&committed_layout, anchor_layout, anchor_affinity),
            parley::Cursor::from_byte_index(&committed_layout, focus_layout, focus_affinity),
        )
        .refresh(&committed_layout);
        slot.anchor = input_model.plain_index_from_layout_index(selection.anchor().index());
        slot.focus = input_model.plain_index_from_layout_index(selection.focus().index());
        selection
    };
    if content_alpha > 0.0 {
        ctx.with_clip_rect_scope(content_alpha, text_clip_bounds, |ctx| {
            ctx.render_styled_text_limited(
                display_styled,
                HorizontalAlignment::Leading,
                env,
                text_bounds,
                line_limit,
            );
        });
    }
    // While composing, the caret the platform cares about is the live
    // composition caret inside the marked text, mapped through the display
    // layout — never the committed text's caret, which makes the candidate
    // window refuse to follow the composition (#25).
    let cursor_geometry = if preedit.is_empty() {
        selection.focus().geometry(&committed_layout, 1.0)
    } else {
        let caret = preedit_caret.map_or(preedit.len(), |caret| {
            clamp_to_char_boundary(preedit.as_str(), caret.min(preedit.len()))
        });
        let caret_index = selection_start + caret;
        let affinity = if caret_index >= committed_with_preedit.len() {
            parley::Affinity::Upstream
        } else {
            parley::Affinity::Downstream
        };
        parley::Cursor::from_byte_index(&display_layout, caret_index, affinity)
            .geometry(&display_layout, 1.0)
    };
    let cursor_area = material_input_cursor_rect(
        field_rect,
        text_bounds,
        kurbo::Rect::new(
            cursor_geometry.x0,
            cursor_geometry.y0,
            cursor_geometry.x1,
            cursor_geometry.y1,
        ),
    );
    let hit_transform = ctx.hit_transform;
    if !disabled {
        ctx.renderer_mut().register_cursor_target(
            transformed_rect(hit_transform, field_rect),
            CursorStyle::IBeam,
        );
    }
    tracing::trace!(
        target: "waterui::hydrolysis::hit_region",
        component = "text_field",
        layout_bounds = ?ctx.bounds,
        field_bounds = ?transformed_rect(ctx.hit_transform, field_rect),
        cursor_area = ?transformed_rect(ctx.hit_transform, cursor_area),
        "register text field input region"
    );
    if !disabled {
        ctx.renderer_mut()
            .register_text_input_target(TextInputTargetRegistration {
                interaction_key,
                modal: env
                    .get::<ModalInteraction>()
                    .is_some_and(ModalInteraction::is_active),
                bounds: transformed_rect(hit_transform, field_rect),
                cursor_area: transformed_rect(hit_transform, cursor_area),
                text_bounds: transformed_rect(hit_transform, text_bounds),
                text_clip_bounds: transformed_rect(hit_transform, text_clip_bounds),
                content_alpha,
                layout: committed_layout,
                display_text: committed_with_preedit,
                display_layout,
                purpose: TextInputPurpose::Normal,
                model: input_model,
                selection: selection_slot,
                env: env.clone(),
            });
    }
}

/// Renders a retained secure-field leaf every flush: secure fields are
/// render-driven a11y, so the inline a11y is emitted by `render_secure_field_parts`
/// itself; this node suppresses it when the field is accessibility-hidden (the
/// dispatch path's render-driven suppression contract).
pub fn render_secure_field_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<SecureFieldRenderState>>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    #[cfg(feature = "accessibility")]
    if hidden {
        ctx.renderer_mut().push_accessibility_suppression();
    }
    render_secure_field_parts(ctx, state, env);
    #[cfg(feature = "accessibility")]
    if hidden {
        ctx.renderer_mut().pop_accessibility_suppression();
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn render_secure_field_parts(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<SecureFieldRenderState>>,
    env: &Environment,
) {
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let theme = ctx.theme();
    let input_metrics = theme.input_field_metrics();
    let disabled = {
        let signal = widget_disabled(env);
        ctx.renderer_mut().read_signal(&signal)
    };
    ctx.renderer_mut()
        .set_text_caret_motion(theme.text_caret_motion());
    let mut state = state.borrow_mut();
    // Read the retained label/value from the retained config each frame (the value
    // is a cloneable `Binding<Secure>`; it is read through `read_signal` below so a
    // binding change schedules a frame). The label is a retained node sub-view
    // flushed under the animated transform each frame.
    let (label, value_binding) = {
        let secure_field = &state.config;
        (secure_field.label.clone(), secure_field.value.clone())
    };
    #[cfg(feature = "accessibility")]
    let default_accessibility_label =
        crate::renderer::SemanticCore::accessibility_label_from_label(&label, env);
    #[cfg(not(feature = "accessibility"))]
    let _ = label;
    let label_size = state.label_view.measure_intrinsic(ctx.renderer_mut(), env);
    let label_height = material_input_label_height(label_size, input_metrics.label_height);
    #[cfg(feature = "accessibility")]
    {
        let secure_len = ctx
            .renderer_mut()
            .read_signal(&value_binding)
            .expose()
            .chars()
            .count();
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let mut node =
            AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::PasswordInput,
            ));
        let label = ctx
            .renderer_mut()
            .resolve_accessibility_label(env, default_accessibility_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        if let Some(value) = ctx
            .renderer_mut()
            .resolve_accessibility_value(env, Some("*".repeat(secure_len)))
        {
            node.set_value(value);
        }
        if disabled {
            node.set_disabled();
        } else {
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            node.add_action(AccessibilityAction::SetValue);
        }
        if let Some(node_id) = ctx.renderer_mut().register_accessibility_node(
            node,
            bounds,
            env,
            (!disabled).then_some(AccessibilityActionTarget::SecureField {
                value: value_binding.clone(),
            }),
        ) {
            ctx.renderer_mut()
                .push_pending_text_input_accessibility_node(node_id);
        }
    }
    let field_rect = ctx.bounds;
    let hit_transform = ctx.hit_transform;
    let is_focused = ctx.renderer_mut().is_text_input_focused(&interaction_key);
    let (mut field_interaction, _, _) = ctx.renderer_mut().bind_focused_control_interaction_target(
        interaction_key.clone(),
        transformed_rect(hit_transform, field_rect),
        env,
        is_focused,
        disabled,
    );
    field_interaction = local_interaction_state(field_interaction, hit_transform);
    {
        ctx.draw_context(|draw| {
            theme.draw_input_field(&mut *draw, field_rect, field_interaction);
            theme.draw_input_field_state_layer(&mut *draw, field_rect, field_interaction);
        });
    }
    let selection_slot = Rc::clone(&state.selection_slot);
    let value_identity = value_binding.identity();
    let input_model = TextInputModel::SecureField {
        value: value_binding.clone(),
    };
    let (masked, plain_value) = {
        let plain_value = ctx
            .renderer_mut()
            .read_signal(&value_binding)
            .expose()
            .to_owned();
        let preedit_count = if is_focused {
            ctx.renderer_mut()
                .current_ime_preedit()
                .as_ref()
                .map_or(0, |value| value.chars().count())
        } else {
            0
        };
        let count = plain_value.chars().count() + preedit_count;
        ("*".repeat(count), plain_value)
    };
    let label_target = if is_focused || !plain_value.is_empty() {
        1.0
    } else {
        0.0
    };
    let interaction_motion = theme.interaction_motion();
    let label_progress = if let Some(identity) = value_identity {
        ctx.renderer_mut().sample_widget_scalar_target(
            AnimationKey::scalar_with_discriminator(identity, SECURE_FIELD_LABEL_ANIMATION_KEY),
            label_target,
            if label_target > 0.0 {
                interaction_motion.focus_enter
            } else {
                interaction_motion.focus_exit
            },
        )
    } else {
        label_target
    };
    if label_height > 0.0 {
        flush_material_label(
            ctx,
            env,
            &mut state.label_view,
            field_rect,
            input_metrics.horizontal_inset,
            label_height,
            label_progress,
        );
    }
    let content_alpha = material_input_content_alpha(label_height > 0.0, label_progress);
    let text_bounds = material_input_text_rect(
        field_rect,
        input_metrics.horizontal_inset,
        input_metrics.vertical_inset,
        label_height,
    );
    let masked_display = StyledStr::plain(masked.clone());
    let committed_layout = HydrolysisRenderer::build_text_layout(
        ctx.state_mut(),
        StyledStr::plain(masked.clone()),
        HorizontalAlignment::Leading,
        env,
        Some(crate::num_cast::f64_as_f32(text_bounds.width())),
    );
    // Secure fields are single-line; with no inside label the masked text
    // centres vertically in the container (#85).
    let text_bounds = if label_height == 0.0 {
        material_input_centered_text_rect(
            field_rect,
            text_bounds,
            f64::from(committed_layout.height()).max(input_metrics.label_height),
        )
    } else {
        text_bounds
    };
    let text_clip_bounds =
        material_input_text_clip_rect(field_rect, text_bounds, committed_layout.height());
    let selection = {
        let mut slot = selection_slot.borrow_mut();
        if !slot.initialized {
            slot.anchor = plain_value.len();
            slot.focus = plain_value.len();
            slot.initialized = true;
        }
        slot.anchor = clamp_to_char_boundary(plain_value.as_str(), slot.anchor);
        slot.focus = clamp_to_char_boundary(plain_value.as_str(), slot.focus);
        let text_len = plain_value.chars().count();
        let anchor_layout = input_model.layout_index_from_plain_index(slot.anchor);
        let focus_layout = input_model.layout_index_from_plain_index(slot.focus);
        let anchor_affinity = if anchor_layout >= text_len {
            parley::Affinity::Upstream
        } else {
            parley::Affinity::Downstream
        };
        let focus_affinity = if focus_layout >= text_len {
            parley::Affinity::Upstream
        } else {
            parley::Affinity::Downstream
        };
        let selection = parley::Selection::new(
            parley::Cursor::from_byte_index(&committed_layout, anchor_layout, anchor_affinity),
            parley::Cursor::from_byte_index(&committed_layout, focus_layout, focus_affinity),
        )
        .refresh(&committed_layout);
        slot.anchor = input_model.plain_index_from_layout_index(selection.anchor().index());
        slot.focus = input_model.plain_index_from_layout_index(selection.focus().index());
        selection
    };
    if content_alpha > 0.0 {
        ctx.with_clip_rect_scope(content_alpha, text_clip_bounds, |ctx| {
            ctx.render_styled_text_limited(
                masked_display,
                HorizontalAlignment::Leading,
                env,
                text_bounds,
                Some(1),
            );
        });
    }
    let cursor_geometry = selection.focus().geometry(&committed_layout, 1.0);
    let cursor_area = material_input_cursor_rect(
        field_rect,
        text_bounds,
        kurbo::Rect::new(
            cursor_geometry.x0,
            cursor_geometry.y0,
            cursor_geometry.x1,
            cursor_geometry.y1,
        ),
    );
    let hit_transform = ctx.hit_transform;
    if !disabled {
        ctx.renderer_mut().register_cursor_target(
            transformed_rect(hit_transform, field_rect),
            CursorStyle::IBeam,
        );
    }
    tracing::trace!(
        target: "waterui::hydrolysis::hit_region",
        component = "secure_field",
        layout_bounds = ?ctx.bounds,
        field_bounds = ?transformed_rect(ctx.hit_transform, field_rect),
        cursor_area = ?transformed_rect(ctx.hit_transform, cursor_area),
        "register secure field input region"
    );
    if !disabled {
        ctx.renderer_mut()
            .register_text_input_target(TextInputTargetRegistration {
                interaction_key,
                modal: env
                    .get::<ModalInteraction>()
                    .is_some_and(ModalInteraction::is_active),
                bounds: transformed_rect(hit_transform, field_rect),
                cursor_area: transformed_rect(hit_transform, cursor_area),
                text_bounds: transformed_rect(hit_transform, text_bounds),
                text_clip_bounds: transformed_rect(hit_transform, text_clip_bounds),
                content_alpha,
                layout: committed_layout.clone(),
                display_text: masked.into(),
                display_layout: committed_layout,
                purpose: TextInputPurpose::Password,
                model: input_model,
                selection: selection_slot,
                env: env.clone(),
            });
    }
}

/// Measures a retained text-field leaf from its [`TextFieldRenderState`], mirroring
/// [`measure_text_field_intrinsic`] but reading the label size from its already-built
/// [`RetainedSubview`] so layout and the floating-label render agree.
pub fn measure_text_field_node(
    render_state: &TextFieldRenderState,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    let label_size = render_state.label_view.measure_built(state, env, theme);
    ViewDimensions::new(measure_text_field_size_with_label_size(
        &render_state.config,
        label_size,
        state,
        env,
        theme,
        proposal,
    ))
}

/// Measures a retained secure-field leaf from its [`SecureFieldRenderState`],
/// mirroring [`measure_secure_field_intrinsic`] but reading the label size from its
/// already-built [`RetainedSubview`] so layout and the floating-label render agree.
pub fn measure_secure_field_node(
    render_state: &SecureFieldRenderState,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    let label_size = render_state.label_view.measure_built(state, env, theme);
    ViewDimensions::new(measure_secure_field_size_with_label_size(
        &render_state.config,
        label_size,
        state,
        env,
        theme,
        proposal,
    ))
}

fn material_input_label_height(label_size: LayoutSize, min_label_height: f64) -> f64 {
    if label_size.width > 0.0 || label_size.height > 0.0 {
        f64::from(label_size.height).max(min_label_height)
    } else {
        0.0
    }
}

fn material_input_label_rect(
    field_rect: kurbo::Rect,
    horizontal_inset: f64,
    label_height: f64,
) -> kurbo::Rect {
    let y0 = field_rect.y0 + 4.0;
    kurbo::Rect::new(
        field_rect.x0 + horizontal_inset,
        y0,
        field_rect.x1 - horizontal_inset,
        (y0 + label_height).min(field_rect.y1),
    )
}

fn material_input_resting_label_rect(
    field_rect: kurbo::Rect,
    horizontal_inset: f64,
    label_height: f64,
) -> kurbo::Rect {
    let y0 = field_rect.y0 + ((field_rect.height() - label_height) * 0.5).max(0.0);
    kurbo::Rect::new(
        field_rect.x0 + horizontal_inset,
        y0,
        field_rect.x1 - horizontal_inset,
        (y0 + label_height).min(field_rect.y1),
    )
}

/// Flushes the floating label sub-view under the Material animated transform
/// (translate from resting to floating position + scale). The label is a retained
/// node sub-view re-laid-out and re-flushed each frame, so reactive label content
/// stays live without re-dispatch.
fn flush_material_label(
    ctx: &mut WidgetRenderContext<'_>,
    env: &Environment,
    label_view: &mut RetainedSubview,
    field_rect: kurbo::Rect,
    horizontal_inset: f64,
    label_height: f64,
    progress: f32,
) {
    let progress = f64::from(progress.clamp(0.0, 1.0));
    let resting = material_input_resting_label_rect(field_rect, horizontal_inset, label_height);
    let floating = material_input_label_rect(field_rect, horizontal_inset, label_height);
    let scale = (FLOATING_LABEL_SCALE - 1.0).mul_add(progress, 1.0);
    let x = (floating.x0 - resting.x0).mul_add(progress, resting.x0);
    let y = (floating.y0 - resting.y0).mul_add(progress, resting.y0);
    let width = floating.width() / scale;
    let height = label_height / scale;
    let transform = kurbo::Affine::translate((x, y)) * kurbo::Affine::scale(scale);
    let child = ctx.child(transform, kurbo::Rect::new(0.0, 0.0, width, height));
    #[allow(clippy::cast_possible_truncation)]
    let size = LayoutSize::new(width as f32, height as f32);
    // The label's semantics are merged into the field's own text-input node, so
    // the floating label sub-view flushes visual-only.
    let label_area =
        ctx.safe_area_for(transform.transform_rect_bbox(kurbo::Rect::new(0.0, 0.0, width, height)));
    ctx.renderer_mut()
        .with_suppressed_accessibility(|renderer| {
            label_view.flush_in_ctx(
                renderer,
                child,
                env,
                ProposalSize::UNSPECIFIED,
                size,
                label_area,
            );
        });
}

fn material_input_content_alpha(has_label: bool, progress: f32) -> f32 {
    if !has_label {
        return 1.0;
    }
    let progress = progress.clamp(0.0, 1.0);
    ((progress - CONTENT_ENTER_DELAY_PORTION) / CONTENT_VISIBLE_PORTION).clamp(0.0, 1.0)
}

/// Draws the prompt text under the Material floating-label transform —
/// the resting spot centred in the container, floating to the top scaled
/// down — used when the field has no label view so the prompt stands in as
/// the label (#85).
fn flush_material_prompt_label(
    ctx: &mut WidgetRenderContext<'_>,
    env: &Environment,
    prompt_styled: StyledStr,
    field_rect: kurbo::Rect,
    horizontal_inset: f64,
    label_height: f64,
    progress: f32,
) {
    let progress = f64::from(progress.clamp(0.0, 1.0));
    let resting = material_input_resting_label_rect(field_rect, horizontal_inset, label_height);
    let floating = material_input_label_rect(field_rect, horizontal_inset, label_height);
    let scale = (FLOATING_LABEL_SCALE - 1.0).mul_add(progress, 1.0);
    let x = (floating.x0 - resting.x0).mul_add(progress, resting.x0);
    let y = (floating.y0 - resting.y0).mul_add(progress, resting.y0);
    let width = floating.width() / scale;
    let child = ctx.child(
        kurbo::Affine::translate((x, y)) * kurbo::Affine::scale(scale),
        kurbo::Rect::new(0.0, 0.0, width, label_height / scale),
    );
    let renderer = ctx.renderer_mut();
    let (state, scene) = renderer.state_and_scene_mut();
    HydrolysisRenderer::render_styled_text_limited(
        state,
        scene,
        child,
        prompt_styled,
        HorizontalAlignment::Leading,
        env,
        TailMark::Clip(1),
    );
}

fn material_input_text_rect(
    field_rect: kurbo::Rect,
    horizontal_inset: f64,
    vertical_inset: f64,
    label_height: f64,
) -> kurbo::Rect {
    kurbo::Rect::new(
        field_rect.x0 + horizontal_inset,
        field_rect.y0 + vertical_inset + label_height,
        field_rect.x1 - horizontal_inset,
        field_rect.y1 - vertical_inset,
    )
}

/// Material 3 centres the input text of a single-line field vertically in
/// the container when the field carries no inside label — the label's
/// resting spot — instead of top-aligning it under the vertical inset (#85).
fn material_input_centered_text_rect(
    field_rect: kurbo::Rect,
    text_rect: kurbo::Rect,
    text_height: f64,
) -> kurbo::Rect {
    let y0 = field_rect.y0 + ((field_rect.height() - text_height) * 0.5).max(0.0);
    kurbo::Rect::new(
        text_rect.x0,
        y0,
        text_rect.x1,
        (y0 + text_height).min(field_rect.y1),
    )
}

fn material_input_text_clip_rect(
    field_rect: kurbo::Rect,
    text_rect: kurbo::Rect,
    layout_height: f32,
) -> kurbo::Rect {
    let required_height = f64::from(layout_height).max(text_rect.height());
    if required_height <= text_rect.height() {
        return text_rect;
    }
    let y1 = (text_rect.y0 + required_height).min(field_rect.y1);
    let y0 = (y1 - required_height).max(field_rect.y0);
    kurbo::Rect::new(text_rect.x0, y0, text_rect.x1, y1)
}

fn material_input_cursor_rect(
    field_rect: kurbo::Rect,
    text_rect: kurbo::Rect,
    cursor_geometry: kurbo::Rect,
) -> kurbo::Rect {
    let x0 = text_rect.x0 + cursor_geometry.x0;
    let x1 = text_rect.x0 + cursor_geometry.x1.max(cursor_geometry.x0 + 1.0);
    // `text_rect` is the thin baseline strip the layout sits on — a real
    // cursor already reports the shaped line's block extent (which legitimately
    // rises above the strip's top and sinks below its bottom), so it is never
    // clamped back into the strip. An empty layout reports no line at all:
    // the caret still occupies the first line, which runs from the strip down
    // to the field's bottom edge.
    let (y0, y1) = if cursor_geometry.height() > 1.0 {
        (
            text_rect.y0 + cursor_geometry.y0,
            text_rect.y0 + cursor_geometry.y1,
        )
    } else {
        (text_rect.y0, field_rect.y1.max(text_rect.y0 + 1.0))
    };
    kurbo::Rect::new(x0, y0, x1, y1)
}

/// Emits a retained text field's accessibility node and text-input target for
/// the semantic walk — the same node `render_text_field_parts` registers, with
/// no bounds. The input target gets a real text layout (shaped from the
/// environment's font settings — text shaping needs no theme and no GPU) so
/// keyboard events reaching the focused field edit against a live selection,
/// and zero rects where the rendered path takes its layout's.
#[cfg(feature = "accessibility")]
#[expect(
    clippy::too_many_lines,
    reason = "the emission walks every text-field element in one ordered sequence; splitting it would obscure the node ordering"
)]
pub fn emit_text_field_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<TextFieldRenderState>>,
    env: &Environment,
) {
    if env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden)
    {
        return;
    }
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let disabled = {
        let signal = widget_disabled(env);
        renderer.read_signal(&signal)
    };
    let mut state = state.borrow_mut();
    let (label, value_binding, prompt_signal, selection_menu, line_limit_raw, on_submit) = {
        let text_field = &state.config;
        (
            text_field.label.clone(),
            text_field.value.clone(),
            text_field.prompt.content.clone(),
            text_field.selection_menu.clone(),
            text_field.line_limit,
            text_field.on_submit.clone(),
        )
    };
    let default_accessibility_label =
        crate::renderer::SemanticCore::accessibility_label_from_label(&label, env);
    let line_limit = line_limit_raw.map(NonZeroUsize::get);
    {
        let prompt = renderer.read_signal(&prompt_signal).to_plain().to_string();
        let value = renderer.read_signal(&value_binding).to_plain().to_string();
        let default_label =
            default_accessibility_label.or_else(|| (!prompt.is_empty()).then_some(prompt.clone()));
        let mut node =
            AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                env,
                if line_limit == Some(1) {
                    AccessibilityNodeRole::TextInput
                } else {
                    AccessibilityNodeRole::MultilineTextInput
                },
            ));
        let label = renderer.resolve_accessibility_label(env, default_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        if !prompt.is_empty() {
            node.set_placeholder(prompt);
        }
        if !value.is_empty() {
            node.set_value(value.clone());
        }
        if disabled {
            node.set_disabled();
        } else {
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            node.add_action(AccessibilityAction::SetValue);
        }
        if let Some(node_id) = renderer.register_accessibility_node_semantic(
            node,
            env,
            (!disabled).then_some(AccessibilityActionTarget::TextField {
                value: value_binding.clone(),
                line_limit,
            }),
        ) {
            renderer.push_pending_text_input_accessibility_node(node_id);
        }
        if !disabled {
            let layout = HydrolysisRenderer::build_text_layout(
                renderer.state_mut(),
                StyledStr::plain(value.clone()),
                HorizontalAlignment::Leading,
                env,
                None,
            );
            renderer.register_text_input_target(TextInputTargetRegistration {
                interaction_key,
                modal: env
                    .get::<ModalInteraction>()
                    .is_some_and(ModalInteraction::is_active),
                bounds: kurbo::Rect::ZERO,
                cursor_area: kurbo::Rect::ZERO,
                text_bounds: kurbo::Rect::ZERO,
                text_clip_bounds: kurbo::Rect::ZERO,
                content_alpha: 1.0,
                layout: layout.clone(),
                display_text: value.into(),
                display_layout: layout,
                purpose: TextInputPurpose::Normal,
                model: TextInputModel::TextField {
                    value: value_binding.clone(),
                    line_limit,
                    selection_menu,
                    on_submit,
                },
                selection: Rc::clone(&state.selection_slot),
                env: env.clone(),
            });
        }
    }
    // The floating label is a real node subtree in the retained tree — emit
    // its semantics exactly as the unsuppressed rendered flush does.
    state.label_view.emit_accessibility(renderer, env);
}

/// Emits a retained secure field's accessibility node and text-input target
/// for the semantic walk — the same node `render_secure_field_parts`
/// registers, with no bounds.
#[cfg(feature = "accessibility")]
pub fn emit_secure_field_accessibility(
    renderer: &mut crate::renderer::SemanticCore,
    state: &Rc<RefCell<SecureFieldRenderState>>,
    env: &Environment,
) {
    if env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden)
    {
        return;
    }
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let disabled = {
        let signal = widget_disabled(env);
        renderer.read_signal(&signal)
    };
    let mut state = state.borrow_mut();
    let (label, value_binding) = {
        let secure_field = &state.config;
        (secure_field.label.clone(), secure_field.value.clone())
    };
    let default_accessibility_label =
        crate::renderer::SemanticCore::accessibility_label_from_label(&label, env);
    {
        let secure_len = renderer
            .read_signal(&value_binding)
            .expose()
            .chars()
            .count();
        let mut node =
            AccessibilityNode::new(crate::renderer::SemanticCore::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::PasswordInput,
            ));
        let label = renderer.resolve_accessibility_label(env, default_accessibility_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        node.set_value("*".repeat(secure_len));
        if disabled {
            node.set_disabled();
        } else {
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            node.add_action(AccessibilityAction::SetValue);
        }
        if let Some(node_id) = renderer.register_accessibility_node_semantic(
            node,
            env,
            (!disabled).then_some(AccessibilityActionTarget::SecureField {
                value: value_binding.clone(),
            }),
        ) {
            renderer.push_pending_text_input_accessibility_node(node_id);
        }
        if !disabled {
            let masked = "*".repeat(secure_len);
            let layout = HydrolysisRenderer::build_text_layout(
                renderer.state_mut(),
                StyledStr::plain(masked.clone()),
                HorizontalAlignment::Leading,
                env,
                None,
            );
            renderer.register_text_input_target(TextInputTargetRegistration {
                interaction_key,
                modal: env
                    .get::<ModalInteraction>()
                    .is_some_and(ModalInteraction::is_active),
                bounds: kurbo::Rect::ZERO,
                cursor_area: kurbo::Rect::ZERO,
                text_bounds: kurbo::Rect::ZERO,
                text_clip_bounds: kurbo::Rect::ZERO,
                content_alpha: 1.0,
                layout: layout.clone(),
                display_text: masked.into(),
                display_layout: layout,
                purpose: TextInputPurpose::Password,
                model: TextInputModel::SecureField {
                    value: value_binding.clone(),
                },
                selection: Rc::clone(&state.selection_slot),
                env: env.clone(),
            });
        }
    }
    state.label_view.emit_accessibility(renderer, env);
}

#[cfg(test)]
mod tests {
    use super::{
        CONTENT_ENTER_DELAY_PORTION, CONTENT_VISIBLE_PORTION, material_input_content_alpha,
        material_input_cursor_rect, material_input_text_clip_rect,
    };

    #[test]
    fn material_input_content_enter_matches_material_web_delay() {
        assert_eq!(material_input_content_alpha(true, 0.0), 0.0);
        assert_eq!(
            material_input_content_alpha(true, CONTENT_ENTER_DELAY_PORTION),
            0.0
        );
        assert_eq!(material_input_content_alpha(true, 1.0), 1.0);
    }

    #[test]
    fn material_input_content_exit_matches_material_web_visible_window() {
        assert_eq!(material_input_content_alpha(true, 1.0), 1.0);
        assert_eq!(
            material_input_content_alpha(
                true,
                CONTENT_VISIBLE_PORTION.mul_add(0.5, CONTENT_ENTER_DELAY_PORTION),
            ),
            0.5
        );
        assert_eq!(
            material_input_content_alpha(true, CONTENT_ENTER_DELAY_PORTION),
            0.0
        );
        assert_eq!(material_input_content_alpha(true, 0.0), 0.0);
    }

    #[test]
    fn material_input_without_label_keeps_content_visible() {
        assert_eq!(material_input_content_alpha(false, 0.0), 1.0);
    }

    #[test]
    fn material_input_text_clip_expands_for_tall_fallback_glyphs() {
        let field = kurbo::Rect::new(0.0, 0.0, 200.0, 56.0);
        let text = kurbo::Rect::new(16.0, 26.0, 184.0, 48.0);

        let clip = material_input_text_clip_rect(field, text, 30.0);

        assert_eq!(clip.x0, text.x0);
        assert_eq!(clip.x1, text.x1);
        assert!(clip.height() >= 30.0);
        assert!(clip.y0 >= field.y0);
        assert!(clip.y1 <= field.y1);
    }

    #[test]
    fn material_input_text_clip_expands_for_placeholder_layout() {
        let field = kurbo::Rect::new(0.0, 0.0, 200.0, 56.0);
        let text = kurbo::Rect::new(16.0, 26.0, 184.0, 48.0);

        let clip = material_input_text_clip_rect(field, text, 34.0);

        assert_eq!(clip.x0, text.x0);
        assert_eq!(clip.x1, text.x1);
        assert!(clip.height() >= 34.0);
        assert!(clip.y1 > text.y1);
    }

    #[test]
    fn material_input_cursor_spans_the_line_for_empty_layout_geometry() {
        // A material field's text rect is the thin baseline strip; with no
        // shaped line the caret still spans the strip down to the field's
        // bottom edge rather than collapsing into the strip.
        let field = kurbo::Rect::new(0.0, 0.0, 200.0, 56.0);
        let text = kurbo::Rect::new(16.0, 24.75, 184.0, 26.0);
        let empty_geometry = kurbo::Rect::new(0.0, 0.0, 0.0, 1.0);

        let cursor = material_input_cursor_rect(field, text, empty_geometry);

        assert_eq!(cursor.x0, text.x0);
        assert_eq!(cursor.x1, text.x0 + 1.0);
        assert_eq!(cursor.y0, text.y0);
        assert_eq!(cursor.y1, field.y1);
    }

    #[test]
    fn material_input_cursor_preserves_non_empty_layout_geometry() {
        let field = kurbo::Rect::new(0.0, 0.0, 200.0, 56.0);
        let text = kurbo::Rect::new(16.0, 26.0, 184.0, 60.0);
        let geometry = kurbo::Rect::new(42.0, 3.0, 43.0, 25.0);

        let cursor = material_input_cursor_rect(field, text, geometry);

        assert_eq!(cursor.x0, text.x0 + 42.0);
        assert_eq!(cursor.x1, text.x0 + 43.0);
        assert_eq!(cursor.y0, text.y0 + 3.0);
        assert_eq!(cursor.y1, text.y0 + 25.0);
    }
}
