#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::chrome::{ChromeContent, LiveChromeParts};
use crate::renderer::{
    HydroNativeView, HydroState, InteractionKey, RenderContext, SemanticCore, WidgetRenderContext,
    measure_label_intrinsic,
};
use crate::theme::{Chrome, SpecTint, StepperLook, StepperSpec, WidgetTheme};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use kurbo::{Point, Rect};
use nami::Signal;
use std::cell::RefCell;
use std::ops::RangeInclusive;
use std::rc::Rc;
use waterui_controls::stepper::StepperConfig;
use waterui_core::contract_violation;
use waterui_core::interaction::InteractionState;
use waterui_core::layout::Size as LayoutSize;
use waterui_core::layout::{LayoutDirection, ProposalSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};

use crate::renderer::RetainedSubview;
use crate::widgets::util::{label_beside_control_bounds, mirrored_in, widget_disabled};

/// The retained render state of a stepper: the cloneable [`StepperConfig`] drives the
/// +/- buttons + accessibility, and its main label is held as a [`RetainedSubview`]
/// built once and re-flushed each frame so reactive label content stays live.
/// The look the theme gave, and the parts recorded from it, are held across
/// renders while the stepper's spec and frame stay the same.
pub struct StepperRenderState<E: crate::text::TextEngine> {
    config: StepperConfig,
    label_view: RetainedSubview<E>,
    look: Option<(StepperSpec, StepperLook)>,
    decrement: EndParts,
    increment: EndParts,
}

/// What one end of a stepper keeps across renders: its live chrome, and the
/// reported-state binding the chrome keeps current.
struct EndParts {
    parts: Option<LiveChromeParts>,
    report: nami::Binding<InteractionState>,
}

impl EndParts {
    fn new() -> Self {
        Self {
            parts: None,
            report: nami::Binding::container(InteractionState::empty()),
        }
    }
}

/// `range` when it is ordered. An unordered range is a contract violation by
/// the app, reported, and the stepper holds its value at `range`'s start.
fn ordered_range(range: &RangeInclusive<i32>) -> RangeInclusive<i32> {
    if range.start() <= range.end() {
        return range.clone();
    }
    contract_violation!(
        "hydrolysis stepper requires an ordered range, got {range:?}; the stepper holds its value at the range's start"
    );
    *range.start()..=*range.start()
}

/// Whether `step` is positive. A step that is not is a contract violation by
/// the app, reported; the stepper then ignores it.
fn positive_step(step: i32) -> bool {
    let positive = step > 0;
    if !positive {
        contract_violation!(
            "hydrolysis stepper requires positive step, got {step}; the step is ignored"
        );
    }
    positive
}

impl<E: crate::text::TextEngine> StepperRenderState<E> {
    pub(crate) fn from_config(config: StepperConfig) -> Self {
        Self {
            label_view: RetainedSubview::new(AnyView::new(config.label().clone())),
            config,
            look: None,
            decrement: EndParts::new(),
            increment: EndParts::new(),
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
    /// feeds the stepper's, so an enclosing retained sub-view must see them
    /// (water-rs/waterui#1957).
    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        self.label_view.take_layout_dirty_if_built()
    }

    /// The look for the stepper's spec in `env`: the one already held when
    /// the spec is unchanged, otherwise requested again.
    fn look(&mut self, theme: &Rc<dyn WidgetTheme>, env: &Environment) -> &StepperLook {
        let spec = stepper_spec(env);
        if self.held(&spec).is_none() {
            let look = theme.stepper(&spec, env);
            self.look = Some((spec, look));
        }
        &self.look.as_ref().expect("the look was just ensured").1
    }

    fn held(&self, spec: &StepperSpec) -> Option<&StepperLook> {
        self.look
            .as_ref()
            .filter(|(held, _)| held == spec)
            .map(|(_, look)| look)
    }

    /// The look for the stepper's spec in `env` without holding it: the held
    /// one, or a fresh one for a measurement ahead of the render that holds
    /// it.
    fn peek(&self, theme: &Rc<dyn WidgetTheme>, env: &Environment) -> StepperLook {
        let spec = stepper_spec(env);
        self.held(&spec)
            .cloned()
            .unwrap_or_else(|| theme.stepper(&spec, env))
    }
}

/// The spec of a stepper in `env`.
fn stepper_spec(env: &Environment) -> StepperSpec {
    StepperSpec::new(
        waterui_core::layout::layout_direction(env).snapshot(),
        SpecTint::resolve(env),
    )
}

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<StepperConfig> {
    fn intrinsic(
        state: &mut crate::renderer::HydroState<E>,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        measure_stepper_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// Emits a stepper's accessibility node from its config. Shared by the dispatch
/// path ([`Native<StepperConfig>::accessibility`]) and the retained `Widget`-node
/// path so both produce the same a11y tree.
// empty when the accessibility feature is off
#[cfg_attr(not(feature = "accessibility"), allow(clippy::missing_const_for_fn))]
pub fn stepper_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    ctx: Option<RenderContext>,
    stepper: &StepperConfig,
    env: &Environment,
    focus_keys: &[crate::renderer::InteractionKey],
) {
    #[cfg(feature = "accessibility")]
    {
        let mut node = AccessibilityNode::new(
            crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::SpinButton,
            ),
        );
        let default_label = crate::renderer::SemanticCore::<E>::accessibility_label_from_label(
            stepper.label(),
            env,
        );
        let label = renderer.resolve_accessibility_label(env, default_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        if let Some(value) = renderer.resolve_accessibility_value(env, None) {
            node.set_value(value);
        }
        let range = ordered_range(stepper.range());
        let start = *range.start();
        let end = *range.end();
        // Read value/step through `read_signal` so a change schedules a frame and
        // this persistent node re-renders (and re-publishes the a11y value).
        let current = renderer.read_signal(stepper.value()).clamp(start, end);
        node.set_numeric_value(f64::from(current));
        node.set_min_numeric_value(f64::from(start));
        node.set_max_numeric_value(f64::from(end));
        let step = renderer.read_signal(stepper.step());
        if positive_step(step) {
            node.set_numeric_value_step(f64::from(step));
        }
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
                value: stepper.value().clone(),
                step: stepper.step().clone(),
                range,
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
/// The proposal changes nothing: the look fixes the size of the controls, so
/// the stepper measures the same at every height it is proposed.
pub fn measure_stepper_node<E: crate::text::TextEngine>(
    render_state: &StepperRenderState<E>,
    _proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> ViewDimensions {
    let look = render_state.peek(theme, env);
    let label_size = render_state.label_view.measure_built(state, env, theme);
    ViewDimensions::new(stepper_size(&look, label_size))
}

/// Renders a retained stepper leaf every flush: emits a11y (unless hidden) then
/// the label + +/- buttons + tap targets, watching the value/step signals so a
/// change schedules a frame.
pub fn render_stepper_node<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<StepperRenderState<E>>>,
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

/// Which way a stepper's end moves the value.
#[derive(Clone, Copy)]
enum End {
    Decrement,
    Increment,
}

impl End {
    /// The interaction key discriminator of the end.
    const fn index(self) -> usize {
        match self {
            Self::Decrement => 0,
            Self::Increment => 1,
        }
    }

    /// Whether `value` is at this end's limit of `range`.
    const fn at_limit(self, value: i32, range: &RangeInclusive<i32>) -> bool {
        match self {
            Self::Decrement => value <= *range.start(),
            Self::Increment => value >= *range.end(),
        }
    }

    /// The value one `step` from `value` towards this end, held in `range`.
    fn stepped(self, value: i32, step: i32, range: &RangeInclusive<i32>) -> i32 {
        match self {
            Self::Decrement => value.saturating_sub(step),
            Self::Increment => value.saturating_add(step),
        }
        .clamp(*range.start(), *range.end())
    }
}

/// The action of a stepper's `end`: one step towards it, held in the range.
fn stepping_action<E: crate::text::TextEngine>(
    end: End,
    stepping: Stepping,
) -> impl FnMut(&mut SemanticCore<E>, Point, &Environment) -> bool {
    move |_renderer, _point, _env| {
        let step = stepping.step.snapshot();
        if !positive_step(step) {
            return false;
        }
        let next = end.stepped(stepping.value.snapshot(), step, &stepping.range);
        stepping.value.set(next);
        true
    }
}

/// What stepping a stepper reads: its value, its step and its range.
#[derive(Clone)]
struct Stepping {
    value: nami::Binding<i32>,
    step: nami::Computed<i32>,
    range: RangeInclusive<i32>,
}

/// One end of a stepper as a render realizes it: which end, its interaction
/// key, its chrome, the rect it lies in and whether it is disabled.
struct EndPlan<'a> {
    end: End,
    key: InteractionKey,
    chrome: &'a Chrome,
    bounds: Rect,
    disabled: bool,
}

pub fn render_stepper_parts<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<StepperRenderState<E>>>,
    env: &Environment,
) {
    let decrement_key = InteractionKey::for_rc(state, End::Decrement.index());
    let increment_key = InteractionKey::for_rc(state, End::Increment.index());
    let theme = ctx.theme();
    let mut state = state.borrow_mut();
    let state = &mut *state;
    let (range, value, step) = {
        let stepper = &state.config;
        (
            ordered_range(stepper.range()),
            stepper.value().clone(),
            stepper.step().clone(),
        )
    };
    // Watch the value/step signals so a change schedules a frame and this
    // persistent node re-renders: the value decides which ends are at a
    // limit, and drives a11y and any value-formatter display.
    let current = ctx.renderer_mut().read_signal(&value);
    let _ = ctx.renderer_mut().read_signal(&step);
    // Reading the disabled signal watches it, so a change schedules a frame
    // and this persistent node re-renders (and re-registers input).
    let disabled = {
        let signal = widget_disabled(env);
        ctx.renderer_mut().read_signal(&signal)
    };
    let look = state.look(&theme, env).clone();
    let direction = waterui_core::layout::layout_direction(env).snapshot();
    let label_size = state.label_view.measure_intrinsic(ctx.renderer_mut(), env);
    let rects = stepper_rects(ctx.bounds, &look, direction, f64::from(label_size.height));
    // The label is a retained node sub-view re-flushed at its rect; reactive
    // content stays live through the node's own per-frame re-flush, with no dispatch.
    // Its semantics are merged into the stepper's own node by
    // `stepper_accessibility`, so the sub-view flushes visual-only.
    if rects.label.width() > 0.0 {
        // A disabled control dims its label to the theme's disabled-content
        // alpha (Material: on-surface at 38% for default-colored labels).
        ctx.with_scope_if(
            disabled,
            crate::renderer::mount::ScopeKey {
                role: "label-dim",
                item: 0,
            },
            theme.disabled_content_alpha(),
            rects.label,
            |ctx| {
                let render_ctx = ctx.render_context();
                let label_area = ctx.safe_area_for(rects.label);
                let label_view = &mut state.label_view;
                ctx.renderer_mut()
                    .with_suppressed_accessibility(|renderer| {
                        label_view.place(
                            renderer,
                            render_ctx,
                            env,
                            ProposalSize::UNSPECIFIED,
                            rects.label,
                            label_area,
                        );
                    });
            },
        );
    }

    // Each end binds its own interaction target and realizes its own chrome;
    // an end at the limit of the range is disabled, and the chrome sees it
    // as its state.
    let stepping = Stepping { value, step, range };
    let decrement = EndPlan {
        end: End::Decrement,
        key: decrement_key,
        chrome: look.decrement(),
        bounds: rects.decrement,
        disabled: disabled || End::Decrement.at_limit(current, &stepping.range),
    };
    let increment = EndPlan {
        end: End::Increment,
        key: increment_key,
        chrome: look.increment(),
        bounds: rects.increment,
        disabled: disabled || End::Increment.at_limit(current, &stepping.range),
    };
    render_end(ctx, decrement, &mut state.decrement, &stepping, env);
    render_end(ctx, increment, &mut state.increment, &stepping, env);
}

/// Binds one end's interaction target, realizes its chrome on its rect,
/// lists its layers around the icon and, unless the end is disabled,
/// registers its tap target.
fn render_end<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    plan: EndPlan<'_>,
    kept: &mut EndParts,
    stepping: &Stepping,
    env: &Environment,
) {
    let EndPlan {
        end,
        key,
        chrome,
        bounds,
        disabled,
    } = plan;
    let target = chrome.target(bounds);
    let (_, press_slot, handles) =
        ctx.renderer_mut()
            .bind_live_chrome_target(key.clone(), target, env, disabled, chrome);
    let flags = ctx.renderer_mut().reported_interaction_state(&key);
    let parts = LiveChromeParts::ensure(
        &mut kept.parts,
        chrome,
        &ChromeContent::default(),
        bounds,
        flags,
        env,
        &kept.report,
    )
    .clone();
    handles.attach_live(Rc::clone(parts.chrome()));
    let render_ctx = ctx.render_context();
    parts.push_below(ctx.renderer_mut(), render_ctx);
    parts.push_above(ctx.renderer_mut(), render_ctx);

    // A disabled end registers no tap target: the pointer neither presses
    // nor steps it. Targets are re-registered every flush, so an end leaving
    // its limit takes taps again on the next frame.
    if disabled {
        return;
    }
    ctx.renderer_mut()
        .register_interactive_pointer_target_with_keyboard(
            target,
            press_slot,
            chrome.keyboard_focusable(),
            stepping_action(end, stepping.clone()),
        );
}

/// The size of a stepper from its look and its label: the label, the space
/// to the controls and the controls, the row as tall as the taller of the
/// label and the controls. Measurement and drawing both read it.
fn stepper_size(look: &StepperLook, label: LayoutSize) -> LayoutSize {
    let controls = look.controls();
    let label_width = f64::from(label.width);
    let width = if label_width > 0.0 {
        label_width + look.get_label_spacing() + controls.width
    } else {
        controls.width
    };
    let height = f64::from(label.height).max(controls.height);
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    )
}

pub fn measure_stepper_intrinsic<E: crate::text::TextEngine>(
    stepper: &StepperConfig,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> LayoutSize {
    let look = theme.stepper(&stepper_spec(env), env);
    let label_size = measure_label_intrinsic(stepper.label(), state, env, theme);
    stepper_size(&look, label_size)
}

/// Emits a retained stepper's accessibility node for the semantic walk — the
/// same node `stepper_accessibility` registers, with no bounds. The label
/// sub-view flushes visual-only, so there is nothing else to emit.
#[cfg(feature = "accessibility")]
pub fn emit_stepper_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    state: &Rc<RefCell<StepperRenderState<E>>>,
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

/// Where a stepper places its two ends and its label in its bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
struct StepperRects {
    decrement: Rect,
    increment: Rect,
    label: Rect,
}

/// Where a stepper in `direction` places the ends of `look` and its label in
/// `bounds`: the ends side by side at the trailing edge, each the look's size
/// and centred in the row — a row shorter than the buttons caps their height
/// and never their width — and the label on their centre line, keeping its
/// own height, in the room before them. A right-to-left layout mirrors all
/// of it about the bounds' centre line, so the decrement end leads the pair
/// in either direction.
fn stepper_rects(
    bounds: Rect,
    look: &StepperLook,
    direction: LayoutDirection,
    label_height: f64,
) -> StepperRects {
    let controls = look.controls();
    let button = look.button();
    let height = button.height.min(bounds.height());
    let y0 = bounds.y0 + (bounds.height() - height) / 2.0;
    let controls_x0 = (bounds.x1 - controls.width).max(bounds.x0);
    let decrement = Rect::new(controls_x0, y0, controls_x0 + button.width, y0 + height);
    let increment = Rect::new(
        controls_x0 + controls.width - button.width,
        y0,
        controls_x0 + controls.width,
        y0 + height,
    );
    let label = label_beside_control_bounds(
        bounds.x0,
        (controls_x0 - look.get_label_spacing()).max(bounds.x0),
        bounds,
        decrement.union(increment),
        label_height,
    );
    let rects = StepperRects {
        decrement,
        increment,
        label,
    };
    match direction {
        LayoutDirection::LeftToRight => rects,
        LayoutDirection::RightToLeft => StepperRects {
            decrement: mirrored_in(rects.decrement, bounds),
            increment: mirrored_in(rects.increment, bounds),
            label: mirrored_in(rects.label, bounds),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{ordered_range, positive_step, stepper_rects, stepper_size};
    use crate::testing::plain_stepper_look;
    use kurbo::Rect;
    use std::ops::RangeInclusive;
    use waterui_core::layout::{LayoutDirection, Size as LayoutSize};

    /// The plain look's ends are 14 by 14, 4 apart, 8 from the label.
    const LABEL: LayoutSize = LayoutSize::new(60.0, 16.0);

    /// The controls measure at the width the look declares and draw at it
    /// too, whatever the height the stepper lands in.
    #[test]
    fn controls_are_as_wide_as_they_were_measured_at_every_height() {
        let look = plain_stepper_look();
        let measured = stepper_size(&look, LABEL);
        let width = f64::from(measured.width);
        // The label, 8 points, two 14 point ends and 4 points between them.
        assert!((width - (60.0 + 8.0 + 14.0 + 4.0 + 14.0)).abs() < 1e-4);
        for height in [4.0, 10.0, 14.0, 16.0, 17.0, 30.0, 100.0] {
            let bounds = Rect::new(10.0, 20.0, 10.0 + width, 20.0 + height);
            let rects = stepper_rects(bounds, &look, LayoutDirection::LeftToRight, 16.0);
            assert!(
                (rects.decrement.width() - 14.0).abs() < 1e-9
                    && (rects.increment.width() - 14.0).abs() < 1e-9,
                "height {height}: the ends keep the look's width"
            );
            assert!(
                (rects.increment.x1 - bounds.x1).abs() < 1e-9,
                "height {height}: the controls end at the trailing edge"
            );
            assert!(
                (rects.increment.x0 - rects.decrement.x1 - 4.0).abs() < 1e-9,
                "height {height}: the ends keep the look's spacing"
            );
            assert!(
                (rects.decrement.x0 - rects.label.x1 - 8.0).abs() < 1e-9,
                "height {height}: the label keeps the look's spacing"
            );
            assert!(
                (rects.label.width() - 60.0).abs() < 1e-9,
                "height {height}: the label keeps its measured width"
            );
            let expected = 14.0_f64.min(height);
            assert!(
                (rects.decrement.height() - expected).abs() < 1e-9
                    && (rects.increment.height() - expected).abs() < 1e-9,
                "height {height}: the ends keep the look's height, capped to the row"
            );
            assert!(
                (rects.decrement.center().y - bounds.center().y).abs() < 1e-9,
                "height {height}: the ends are centred in the row"
            );
            assert!(
                (rects.label.center().y - bounds.center().y).abs() < 1e-9,
                "height {height}: the label shares the controls' centre line"
            );
        }
    }

    /// A right-to-left layout mirrors the row: the label leads on the right
    /// and the decrement end leads the pair.
    #[test]
    fn right_to_left_mirrors_the_row() {
        let look = plain_stepper_look();
        let measured = stepper_size(&look, LABEL);
        let bounds = Rect::new(
            0.0,
            0.0,
            f64::from(measured.width),
            f64::from(measured.height),
        );
        let ltr = stepper_rects(bounds, &look, LayoutDirection::LeftToRight, 16.0);
        let rtl = stepper_rects(bounds, &look, LayoutDirection::RightToLeft, 16.0);
        assert!(ltr.decrement.x1 < ltr.increment.x0 && ltr.label.x1 < ltr.decrement.x0);
        assert!(rtl.increment.x1 < rtl.decrement.x0 && rtl.decrement.x1 < rtl.label.x0);
        assert!((rtl.decrement.x0 - (bounds.x1 - ltr.decrement.x1)).abs() < 1e-9);
        assert!((rtl.label.x1 - (bounds.x1 - ltr.label.x0)).abs() < 1e-9);
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "stepper requires an ordered range")
    )]
    fn an_unordered_range_is_reported_and_holds_the_value_at_its_start() {
        assert_eq!(ordered_range(&RangeInclusive::new(5, 1)), 5..=5);
    }

    #[test]
    fn an_ordered_range_is_kept() {
        assert_eq!(ordered_range(&(1..=5)), 1..=5);
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "stepper requires positive step")
    )]
    fn a_non_positive_step_is_reported_and_ignored() {
        assert!(!positive_step(0));
    }

    #[test]
    fn a_positive_step_is_accepted() {
        assert!(positive_step(2));
    }
}
