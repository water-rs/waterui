#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::live_toggle::LiveToggleParts;
use crate::renderer::{
    GroupOpacity, HydroNativeView, HydroState, InteractionKey, RenderContext, Track, TrackAction,
    WidgetRenderContext, measure_label_intrinsic,
};
use crate::theme::{
    CheckboxLook, SpecTint, SwitchDrag, SwitchLook, ToggleControl, ToggleSpec, WidgetTheme,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
    Toggled as AccessibilityToggled,
};
use kurbo::{Point, Vec2};
use nami::Signal as _;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use waterui_controls::toggle::{ToggleConfig, ToggleStyle};
use waterui_core::contract_violation;
use waterui_core::interaction::InteractionState;
use waterui_core::layout::{LayoutDirection, Size as LayoutSize};
use waterui_core::layout::{ProposalSize, StretchAxis, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};

use crate::renderer::RetainedSubview;
use crate::widgets::util::{label_beside_control_bounds, mirrored_in, widget_disabled};

/// A toggle's look: the theme's switch or checkbox for a [`ToggleSpec`].
#[derive(Clone)]
enum ToggleLook {
    Switch(SwitchLook),
    Checkbox(CheckboxLook),
}

impl ToggleLook {
    /// The look `theme` gives a toggle of `spec` in `env`.
    fn resolve(theme: &Rc<dyn WidgetTheme>, spec: &ToggleSpec, env: &Environment) -> Self {
        match drawn_style(spec.style()) {
            ToggleStyle::Checkbox => Self::Checkbox(theme.checkbox(spec, env)),
            _ => Self::Switch(theme.switch(spec, env)),
        }
    }

    const fn control(&self) -> &ToggleControl {
        match self {
            Self::Switch(look) => look.control(),
            Self::Checkbox(look) => look.control(),
        }
    }
}

/// The spec of the toggle `config` describes in `env`.
fn toggle_spec(config: &ToggleConfig, env: &Environment) -> ToggleSpec {
    ToggleSpec::new(
        config.style(),
        config.size(),
        waterui_core::layout::layout_direction(env).snapshot(),
        SpecTint::resolve(env),
    )
}

/// The retained render state of a toggle: the cloneable [`ToggleConfig`] drives the
/// control + accessibility, and its main label is held as a [`RetainedSubview`]
/// built once and re-flushed each frame so reactive label content stays live.
/// The look the theme gave, and the parts recorded from it, are held across
/// renders while the toggle's spec and frame stay the same.
pub struct ToggleRenderState<E: crate::text::TextEngine> {
    config: ToggleConfig,
    label_view: RetainedSubview<E>,
    look: Option<(ToggleSpec, ToggleLook)>,
    parts: Option<LiveToggleParts>,
}

impl<E: crate::text::TextEngine> ToggleRenderState<E> {
    pub(crate) fn from_config(config: ToggleConfig) -> Self {
        Self {
            label_view: RetainedSubview::new(AnyView::new(config.label().clone())),
            config,
            look: None,
            parts: None,
        }
    }

    /// Eagerly build the label sub-view (the measure path has only
    /// `&mut HydroState`, no renderer, so it must be built before then).
    pub(crate) fn prebuild(
        &mut self,
        renderer: &mut crate::renderer::SemanticCore<E>,
        env: &Environment,
    ) {
        self.label_view.ensure_built(renderer, env);
    }

    /// Consume the label sub-view's layout-dirty marks: its measured size
    /// feeds the toggle's, so an enclosing retained sub-view must see them
    /// (water-rs/waterui#1957).
    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        self.label_view.take_layout_dirty_if_built()
    }

    /// The look for `spec` in `env`: the one already held when the spec is
    /// unchanged, otherwise requested again.
    fn look(
        &mut self,
        theme: &Rc<dyn WidgetTheme>,
        spec: ToggleSpec,
        env: &Environment,
    ) -> &ToggleLook {
        if self.held(&spec).is_none() {
            let look = ToggleLook::resolve(theme, &spec, env);
            self.look = Some((spec, look));
        }
        &self.look.as_ref().expect("the look was just ensured").1
    }

    fn held(&self, spec: &ToggleSpec) -> Option<&ToggleLook> {
        self.look
            .as_ref()
            .filter(|(held, _)| held == spec)
            .map(|(_, look)| look)
    }

    /// The look for `spec` in `env` without holding it: the held one, or a
    /// fresh one for a measurement ahead of the render that holds it.
    fn peek(
        &self,
        theme: &Rc<dyn WidgetTheme>,
        spec: &ToggleSpec,
        env: &Environment,
    ) -> ToggleLook {
        self.held(spec)
            .cloned()
            .unwrap_or_else(|| ToggleLook::resolve(theme, spec, env))
    }
}

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<ToggleConfig> {
    fn intrinsic(
        state: &mut crate::renderer::HydroState<E>,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        measure_toggle_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// Emits a toggle's accessibility node from its config. Shared by the rendered
/// `Widget`-node flush (which passes its [`RenderContext`]) and the semantic
/// emission walk (which passes `None` — a semantic node carries no bounds).
// empty when the accessibility feature is off
#[cfg_attr(not(feature = "accessibility"), allow(clippy::missing_const_for_fn))]
pub fn toggle_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    ctx: Option<RenderContext>,
    toggle: &ToggleConfig,
    env: &Environment,
    focus_keys: &[crate::renderer::InteractionKey],
) {
    #[cfg(feature = "accessibility")]
    {
        let mut node = AccessibilityNode::new(
            crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
                env,
                match drawn_style(toggle.style()) {
                    ToggleStyle::Checkbox => AccessibilityNodeRole::CheckBox,
                    _ => AccessibilityNodeRole::Switch,
                },
            ),
        );
        let default_label =
            crate::renderer::SemanticCore::<E>::accessibility_label_from_label(toggle.label(), env);
        let label = renderer.resolve_accessibility_label(env, default_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        if let Some(value) = renderer.resolve_accessibility_value(env, None) {
            node.set_value(value);
        }
        let checked = renderer.read_signal(toggle.toggle());
        node.set_toggled(AccessibilityToggled::from(checked));
        node.add_action(AccessibilityAction::Focus);
        // A disabled toggle stays in the tree (focusable, announced as
        // disabled) but exposes no click action and no action target.
        let disabled = renderer.read_signal(&widget_disabled(env));
        let action_target = if disabled {
            node.set_disabled();
            None
        } else {
            node.add_action(AccessibilityAction::Click);
            Some(AccessibilityActionTarget::Toggle {
                binding: toggle.toggle().clone(),
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
        let _ = (renderer, ctx, toggle, env, focus_keys);
    }
}

/// The native payload's style is resolved before a backend sees it
/// (`ToggleConfig`'s resolver owns the platform default), so `Automatic`
/// reaching a match here is a broken contract, not a style to draw.
const UNRESOLVED_STYLE: &str =
    "the toggle payload reached hydrolysis with an unresolved Automatic style";

/// The style a toggle of `style` is drawn in. `Automatic` (unresolved) or a
/// variant this backend does not implement is a contract violation by the
/// producer of the payload, reported; the toggle is then drawn as a switch.
fn drawn_style(style: ToggleStyle) -> ToggleStyle {
    match style {
        ToggleStyle::Switch | ToggleStyle::Checkbox => style,
        ToggleStyle::Automatic => {
            contract_violation!("{UNRESOLVED_STYLE}; the toggle is drawn as a switch");
            ToggleStyle::Switch
        }
        other => {
            contract_violation!(
                "hydrolysis ToggleStyle variant {other:?} is not implemented; the toggle is drawn as a switch"
            );
            ToggleStyle::Switch
        }
    }
}

/// The size a toggle of `control` takes beside a label of `label_size`.
fn toggle_size(control: &ToggleControl, label_size: LayoutSize) -> (f64, f64) {
    let label_width = f64::from(label_size.width);
    let size = control.size();
    let width = if label_width > 0.0 {
        label_width + control.get_label_spacing() + size.width
    } else {
        size.width
    };
    (width, f64::from(label_size.height).max(size.height))
}

/// Measures a retained toggle leaf from its [`ToggleRenderState`], reading the
/// label size from its already-built [`RetainedSubview`] so layout and render agree.
///
/// A switch with a visible label (`StretchAxis::Horizontal`) answers a finite
/// width proposal, never below its intrinsic width; every other toggle
/// answers its intrinsic size (`docs/layout-spec.md` §6).
pub fn measure_toggle_node<E: crate::text::TextEngine>(
    render_state: &ToggleRenderState<E>,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> ViewDimensions {
    let spec = toggle_spec(&render_state.config, env);
    let look = render_state.peek(theme, &spec, env);
    let label_size = render_state.label_view.measure_built(state, env, theme);
    let (width, height) = toggle_size(look.control(), label_size);
    let width = match (
        waterui_core::NativeView::stretch_axis(&render_state.config),
        proposal.width,
    ) {
        (StretchAxis::Horizontal, Some(proposed)) if proposed.is_finite() => {
            f64::from(proposed).max(width)
        }
        _ => width,
    };
    ViewDimensions::new(LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    ))
}

/// Renders a retained toggle leaf every flush: emits a11y (unless hidden) then the
/// control + label + tap target, reading the config's live signals each frame.
pub fn render_toggle_node<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ToggleRenderState<E>>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        toggle_accessibility(
            ctx.renderer_mut(),
            Some(render_ctx),
            &state.borrow().config,
            env,
            &[crate::renderer::InteractionKey::for_rc(state, 0)],
        );
    }
    render_toggle_parts(ctx, state, env);
}

pub fn render_toggle_parts<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ToggleRenderState<E>>>,
    env: &Environment,
) {
    let visual_interaction_key = InteractionKey::for_rc(state, 0);
    let activation_interaction_key = InteractionKey::for_rc(state, 1);
    let theme = ctx.theme();
    let mut state = state.borrow_mut();
    let style = state.config.style();
    let spec = toggle_spec(&state.config, env);
    let direction = spec.direction();
    let look = state.look(&theme, spec, env).clone();
    let control = look.control();
    // Reading the disabled signal watches it, so a change schedules a frame
    // and this persistent node re-renders (and re-registers input) with the
    // new state.
    let disabled = {
        let signal = widget_disabled(env);
        ctx.renderer_mut().read_signal(&signal)
    };
    // The label is a retained node sub-view re-flushed at its rect; reactive
    // content stays live through the node's own per-frame re-flush, with no dispatch.
    let label_size = state.label_view.measure_intrinsic(ctx.renderer_mut(), env);
    let (control_bounds, label_bounds) =
        toggle_control_and_label_bounds(ctx.bounds, style, direction, control, label_size);
    if label_bounds.width() > 0.0 {
        // A disabled control dims its label to the theme's disabled-content
        // alpha (Material: on-surface at 38% for default-colored labels).
        // The label's semantics are merged into the toggle's own node by
        // `toggle_accessibility`, so the sub-view flushes visual-only.
        ctx.with_scope_if(
            disabled,
            crate::renderer::mount::ScopeKey {
                role: "label-dim",
                item: 0,
            },
            theme.disabled_content_alpha(),
            label_bounds,
            |ctx| {
                let render_ctx = ctx.render_context();
                let label_area = ctx.safe_area_for(label_bounds);
                let label_view = &mut state.label_view;
                ctx.renderer_mut()
                    .with_suppressed_accessibility(|renderer| {
                        label_view.place(
                            renderer,
                            render_ctx,
                            env,
                            ProposalSize::UNSPECIFIED,
                            label_bounds,
                            label_area,
                        );
                    });
            },
        );
    }

    let binding = state.config.toggle().clone();
    let target = control.target(control_bounds);
    let (_, press_slot, handles) = ctx.renderer_mut().bind_live_toggle_target(
        visual_interaction_key.clone(),
        target,
        env,
        disabled,
        control,
    );
    let flags = ctx
        .renderer_mut()
        .reported_interaction_state(&visual_interaction_key);
    // The parts read the toggle's value through its binding: a toggle, hover,
    // press or drag moves their operands and records nothing.
    let parts =
        LiveToggleParts::ensure(&mut state.parts, control, control_bounds, flags, &binding).clone();
    handles.attach_live_toggle(Rc::clone(parts.toggle()));
    push_control(ctx, &parts, flags, target);
    // A disabled toggle registers no tap target: the pointer neither presses
    // nor toggles it. Targets are re-registered every flush, so re-enabling
    // restores interactivity on the next frame.
    if disabled {
        return;
    }
    // Keep the label row as an activation target without attaching it to the
    // switch's visual interaction. An associated the Material label toggles the switch
    // but does not originate a switch ripple or pressed thumb from the label's
    // distant pointer coordinate.
    if label_bounds.width() > 0.0 {
        let mut activation_press_slot = press_slot.clone();
        activation_press_slot.key = activation_interaction_key;
        let row = ctx.bounds;
        ctx.renderer_mut()
            .register_interactive_pointer_target_with_keyboard(
                row,
                activation_press_slot,
                false,
                toggle_binding_action(binding.clone(), visual_interaction_key.clone()),
            );
    }
    // Register the visual control last so it wins hit testing where the full-row
    // activation target overlaps it.
    let action = toggle_binding_action(binding.clone(), visual_interaction_key);
    match &look {
        ToggleLook::Switch(switch) => {
            let track = switch_track(
                *switch.drag(),
                parts.toggle().clone(),
                binding,
                press_slot.slop,
            );
            ctx.renderer_mut()
                .register_tracked_pointer_target(target, press_slot, track, action);
        }
        ToggleLook::Checkbox(_) => {
            ctx.renderer_mut()
                .register_interactive_pointer_target(target, press_slot, action);
        }
    }
}

/// Lists a toggle's control as one group. An opacity that varies between
/// renders follows its operand; a fixed one groups the parts only when it
/// fades. The group reaches as far as the parts draw and the touch `target`.
fn push_control<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    parts: &LiveToggleParts,
    flags: InteractionState,
    target: kurbo::Rect,
) {
    let group = crate::renderer::mount::ScopeKey {
        role: "toggle-group",
        item: 0,
    };
    let reach = parts.toggle().visual_bounds().union(target);
    let body = |ctx: &mut WidgetRenderContext<'_, E>| {
        let render_ctx = ctx.render_context();
        parts.push(ctx.renderer_mut(), render_ctx);
    };
    let toggle = parts.toggle();
    let opacity = toggle.live_group_opacity(flags).map_or_else(
        || GroupOpacity::Fixed(toggle.fixed_group_opacity(flags)),
        GroupOpacity::Live,
    );
    let space = toggle.group_blend_space();
    ctx.with_group_scope(group, opacity, space, reach, body);
}

/// The handler of a switch's press: a press that moves farther than `slop`
/// horizontally drags the knob and settles on release, any other tap within
/// reach toggles it.
fn switch_track(
    drag: SwitchDrag,
    toggle: Rc<crate::renderer::live_toggle::LiveToggle>,
    binding: nami::Binding<bool>,
    slop: f64,
) -> TrackAction {
    let start = Cell::new(None::<Point>);
    let dragging = Cell::new(false);
    let offset = move |from: Option<Point>, to: Point| {
        from.map_or(Vec2::ZERO, |from| Vec2::new(to.x - from.x, 0.0))
    };
    Rc::new(RefCell::new(move |step: Track| match step {
        Track::Begin(point) => {
            start.set(Some(point));
            dragging.set(false);
        }
        Track::Move(point) => {
            let offset = offset(start.get(), point);
            if !dragging.get() && offset.x.abs() > slop {
                dragging.set(true);
                toggle.begin_drag();
            }
            if dragging.get() {
                toggle.drag_to(offset);
            }
        }
        Track::Release {
            point,
            within_reach,
        } => {
            let offset = offset(start.take(), point);
            if dragging.replace(false) {
                let value = binding.snapshot();
                let settled = if within_reach {
                    drag.settles_on(value, offset)
                } else {
                    value
                };
                toggle.end_drag(settled, &binding);
            } else if within_reach {
                binding.toggle();
            }
        }
        Track::Cancel => {
            start.set(None);
            if dragging.replace(false) {
                toggle.end_drag(binding.snapshot(), &binding);
            }
        }
    }))
}

fn toggle_binding_action<E: crate::text::TextEngine>(
    binding: nami::Binding<bool>,
    visual_interaction_key: InteractionKey,
) -> impl FnMut(&mut crate::renderer::SemanticCore<E>, kurbo::Point, &Environment) -> bool {
    move |renderer, _point, _env| {
        // A pointer press on the non-focusable label target temporarily owns an
        // interaction-only key. Restore semantic keyboard focus to the switch
        // itself when the label activates it.
        renderer.set_keyboard_focus(Some(visual_interaction_key.clone()), false);
        binding.toggle();
        true
    }
}

pub fn measure_toggle_intrinsic<E: crate::text::TextEngine>(
    toggle: &ToggleConfig,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> LayoutSize {
    let look = ToggleLook::resolve(theme, &toggle_spec(toggle, env), env);
    let label_size = measure_label_intrinsic(toggle.label(), state, env, theme);
    let (width, height) = toggle_size(look.control(), label_size);
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    )
}

/// Where a toggle of `style` places its control and its label in `bounds`.
/// The label lies at the leading edge of a switch's frame and the control at
/// its trailing edge; a checkbox's control leads its label. A right-to-left
/// layout mirrors both about the frame's centre line.
fn toggle_control_and_label_bounds(
    bounds: kurbo::Rect,
    style: ToggleStyle,
    direction: LayoutDirection,
    toggle: &ToggleControl,
    label_size: LayoutSize,
) -> (kurbo::Rect, kurbo::Rect) {
    let (control, label) = left_to_right_bounds(bounds, style, toggle, label_size);
    match direction {
        LayoutDirection::LeftToRight => (control, label),
        LayoutDirection::RightToLeft => (mirrored_in(control, bounds), mirrored_in(label, bounds)),
    }
}

#[expect(
    clippy::similar_names,
    reason = "the names follow the fixture domain vocabulary; renaming would obscure rather than clarify"
)]
fn left_to_right_bounds(
    bounds: kurbo::Rect,
    style: ToggleStyle,
    toggle: &ToggleControl,
    label_size: LayoutSize,
) -> (kurbo::Rect, kurbo::Rect) {
    let size = toggle.size();
    let spacing = toggle.get_label_spacing();
    let control_y0 = bounds.y0 + ((bounds.height() - size.height) / 2.0).max(0.0);
    let control_y1 = control_y0 + size.height;
    let has_label = label_size.width > 0.0 || label_size.height > 0.0;
    // `drawn_style` only answers `Checkbox` or `Switch`.
    if matches!(drawn_style(style), ToggleStyle::Checkbox) {
        let control_x0 = bounds.x0;
        let control_x1 = control_x0 + size.width;
        let control = kurbo::Rect::new(control_x0, control_y0, control_x1, control_y1);
        let label_x0 = if has_label {
            (control_x1 + spacing).min(bounds.x1)
        } else {
            control_x1
        };
        let max_label_width = (bounds.x1 - label_x0).max(0.0);
        let label_width = f64::from(label_size.width).min(max_label_width);
        (
            control,
            label_beside_control_bounds(
                label_x0,
                label_x0 + label_width,
                bounds,
                control,
                f64::from(label_size.height),
            ),
        )
    } else {
        let control_x0 = (bounds.x1 - size.width).max(bounds.x0);
        let control = kurbo::Rect::new(control_x0, control_y0, control_x0 + size.width, control_y1);
        let label_x1 = if has_label {
            (control_x0 - spacing).max(bounds.x0)
        } else {
            bounds.x0
        };
        (
            control,
            label_beside_control_bounds(
                bounds.x0,
                label_x1,
                bounds,
                control,
                f64::from(label_size.height),
            ),
        )
    }
}

/// Emits a retained toggle's accessibility node for the semantic walk — the
/// same node `toggle_accessibility` registers, with no bounds. The label
/// sub-view flushes visual-only, so there is nothing else to emit.
#[cfg(feature = "accessibility")]
pub fn emit_toggle_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    state: &Rc<RefCell<ToggleRenderState<E>>>,
    env: &Environment,
) {
    toggle_accessibility(
        renderer,
        None,
        &state.borrow().config,
        env,
        &[crate::renderer::InteractionKey::for_rc(state, 0)],
    );
}

#[cfg(test)]
mod tests {
    use super::{drawn_style, toggle_control_and_label_bounds};
    use crate::theme::{Motion, ToggleControl};
    use kurbo::Rect;
    use std::time::Duration;
    use waterui_controls::toggle::ToggleStyle;
    use waterui_core::animation::Animation;
    use waterui_core::layout::{LayoutDirection, Size};

    fn control(width: f64, height: f64, spacing: f64) -> ToggleControl {
        ToggleControl::new(
            kurbo::Size::new(width, height),
            Motion::uniform(Animation::linear(Duration::ZERO)),
        )
        .label_spacing(spacing)
    }

    #[test]
    fn checkbox_layout_places_control_before_label() {
        let (control, label) = toggle_control_and_label_bounds(
            Rect::new(16.0, 20.0, 320.0, 60.0),
            ToggleStyle::Checkbox,
            LayoutDirection::LeftToRight,
            &control(18.0, 18.0, 8.0),
            Size::new(64.0, 16.0),
        );

        assert_eq!(control, Rect::new(16.0, 31.0, 34.0, 49.0));
        assert_eq!(label, Rect::new(42.0, 32.0, 106.0, 48.0));
    }

    /// A right-to-left layout mirrors both placements about the frame.
    #[test]
    fn right_to_left_mirrors_the_placement() {
        let bounds = Rect::new(16.0, 20.0, 320.0, 60.0);
        let label_size = Size::new(64.0, 16.0);
        let (control, label) = toggle_control_and_label_bounds(
            bounds,
            ToggleStyle::Switch,
            LayoutDirection::RightToLeft,
            &self::control(52.0, 32.0, 8.0),
            label_size,
        );
        assert_eq!(control, Rect::new(16.0, 24.0, 68.0, 56.0));
        assert_eq!(label, Rect::new(76.0, 32.0, 320.0, 48.0));

        let (control, label) = toggle_control_and_label_bounds(
            bounds,
            ToggleStyle::Checkbox,
            LayoutDirection::RightToLeft,
            &self::control(18.0, 18.0, 8.0),
            label_size,
        );
        assert_eq!(control, Rect::new(302.0, 31.0, 320.0, 49.0));
        assert_eq!(label, Rect::new(230.0, 32.0, 294.0, 48.0));
    }

    #[test]
    fn switch_layout_keeps_control_trailing() {
        let (control, label) = toggle_control_and_label_bounds(
            Rect::new(16.0, 20.0, 320.0, 60.0),
            ToggleStyle::Switch,
            LayoutDirection::LeftToRight,
            &self::control(52.0, 32.0, 8.0),
            Size::new(64.0, 16.0),
        );

        assert_eq!(control, Rect::new(268.0, 24.0, 320.0, 56.0));
        assert_eq!(label, Rect::new(16.0, 32.0, 260.0, 48.0));
    }

    #[test]
    fn label_shares_the_control_centre_line() {
        let bounds = Rect::new(16.0, 20.0, 320.0, 60.0);
        for style in [ToggleStyle::Switch, ToggleStyle::Checkbox] {
            let toggle = match style {
                ToggleStyle::Checkbox => control(18.0, 18.0, 8.0),
                _ => control(52.0, 32.0, 8.0),
            };
            let (control, label) = toggle_control_and_label_bounds(
                bounds,
                style,
                LayoutDirection::LeftToRight,
                &toggle,
                Size::new(64.0, 16.0),
            );
            assert!(
                (label.center().y - control.center().y).abs() < 1e-9,
                "{style:?}: label centre {} != control centre {}",
                label.center().y,
                control.center().y,
            );
        }
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "unresolved Automatic style")
    )]
    fn an_unresolved_automatic_style_is_reported_and_drawn_as_a_switch() {
        assert!(matches!(
            drawn_style(ToggleStyle::Automatic),
            ToggleStyle::Switch
        ));
    }

    #[test]
    fn a_resolved_style_is_drawn_as_it_is() {
        assert!(matches!(
            drawn_style(ToggleStyle::Checkbox),
            ToggleStyle::Checkbox
        ));
        assert!(matches!(
            drawn_style(ToggleStyle::Switch),
            ToggleStyle::Switch
        ));
    }
}
