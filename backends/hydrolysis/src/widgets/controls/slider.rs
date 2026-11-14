#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::chrome::{ChromeContent, IndicatorContent, LiveChromeParts, SliderContent};
#[cfg(feature = "accessibility")]
use crate::renderer::slider_step_for_range;
use crate::renderer::{
    GroupOpacity, HydroNativeView, HydroState, HydrolysisRenderer, RenderContext,
    WidgetRenderContext, slider_value_epsilon,
};
use crate::theme::{
    DragMapping, Inputs, SliderLayout, SliderLook, SliderSpec, SpecTint, WidgetTheme,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use core::any::Any;
use core::ops::RangeInclusive;
use kurbo::{Point, Rect, Size};
use nami::{Binding, Computed, Signal, SignalExt as _};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use waterui::ViewExt as _;
use waterui_controls::ControlSize;
use waterui_controls::label::{Label, LabelDisplayMode};
use waterui_controls::slider::{SliderConfig, ValueFormatter, snap};
use waterui_core::AnyView;
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::contract_violation;
use waterui_core::interaction::InteractionState;
use waterui_core::layout::Alignment;
use waterui_core::layout::Size as LayoutSize;
use waterui_core::layout::{HorizontalAlignment, LayoutDirection, ProposalSize, ViewDimensions};
use waterui_text::IntoText as _;
use waterui_text::styled::StyledStr;

use crate::renderer::{PressFilter, RetainedSubview};
use crate::widgets::util::{label_beside_control_bounds, widget_disabled};

/// The retained render state of a slider. A `SliderConfig`'s value-end labels are
/// move-only `AnyView`s (they cannot be re-dispatched twice), so the persistent
/// `Widget` node holds them as [`RetainedSubview`]s built once and re-flushed each
/// frame; the cloneable `label`/`range`/`value` drive the track and accessibility.
pub struct SliderRenderState<E: crate::text::TextEngine> {
    label: Label,
    /// The main label as a retained node sub-view, re-flushed each frame at its
    /// rect (reactive content stays live through the node's own re-flush). The
    /// cloneable `label` is kept alongside for accessibility resolution.
    label_view: RetainedSubview<E>,
    min_value_label: RetainedSubview<E>,
    max_value_label: RetainedSubview<E>,
    /// Whether a label is set beside either end of the track.
    value_labels: bool,
    range: RangeInclusive<f64>,
    step: Option<f64>,
    value: Binding<f64>,
    /// The value normalized to the range: what the look's parts follow.
    normalized: Rc<Computed<f64>>,
    size: ControlSize,
    value_indicator: Option<ValueFormatter>,
    /// The look for the spec it was requested for.
    look: Option<(SliderSpec, SliderLook)>,
    /// The live chrome realizing the look.
    parts: Option<LiveChromeParts>,
    /// The chrome's reported interaction state.
    report: Binding<InteractionState>,
    /// Raised when the value changes, for the accessibility tree to follow
    /// it without this node re-rendering.
    value_changed: Rc<Cell<bool>>,
    /// Keeps the value watcher subscribed.
    value_watch: Option<Box<dyn Any>>,
    /// The value indicator's text, following the value through its own
    /// reactive content; built for the current look.
    indicator_text: Option<RetainedSubview<E>>,
}

/// `range` when it spans a positive distance. A slider needs `start < end`
/// (the value normalizes by the span); anything else is a contract violation
/// by the app, reported, and the slider uses the unit range `0..=1`.
fn valid_range(range: RangeInclusive<f64>) -> RangeInclusive<f64> {
    if range.start() < range.end() {
        return range;
    }
    contract_violation!(
        "hydrolysis slider requires range start < end, got {range:?}; the slider uses the range 0..=1"
    );
    0.0..=1.0
}

impl<E: crate::text::TextEngine> SliderRenderState<E> {
    pub(crate) fn from_config(config: SliderConfig) -> Self {
        let (label, min_value_label, max_value_label, range, value, size, value_indicator, step) =
            config.into_parts();
        let range = valid_range(range);
        let start = *range.start();
        let end = *range.end();
        let normalized = Computed::new(
            value.map(move |value: f64| (value.clamp(start, end) - start) / (end - start)),
        );
        Self {
            label_view: RetainedSubview::new(AnyView::new(label.clone())),
            label,
            value_labels: !min_value_label.is::<()>() || !max_value_label.is::<()>(),
            min_value_label: RetainedSubview::new(min_value_label),
            max_value_label: RetainedSubview::new(max_value_label),
            range,
            step,
            value,
            normalized: Rc::new(normalized),
            size,
            value_indicator,
            look: None,
            parts: None,
            report: Binding::container(InteractionState::empty()),
            value_changed: Rc::new(Cell::new(false)),
            value_watch: None,
            indicator_text: None,
        }
    }

    /// Eagerly build the label sub-views (the measure path has only
    /// `&mut HydroState`, no renderer, so labels must be built before then).
    pub(crate) fn prebuild_labels(
        &mut self,
        renderer: &mut crate::renderer::SemanticCore<E>,
        env: &Environment,
    ) {
        self.label_view.ensure_built(renderer, env);
        self.min_value_label.ensure_built(renderer, env);
        self.max_value_label.ensure_built(renderer, env);
    }

    /// Consume the label sub-views' layout-dirty marks: their measured sizes
    /// feed the slider's, so an enclosing retained sub-view must see them
    /// (water-rs/waterui#1957).
    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        self.label_view.take_layout_dirty_if_built()
            | self.min_value_label.take_layout_dirty_if_built()
            | self.max_value_label.take_layout_dirty_if_built()
    }

    fn spec(&self, env: &Environment) -> SliderSpec {
        slider_spec(
            &self.label,
            self.size,
            &self.range,
            self.step,
            self.value_labels,
            self.value_indicator.is_some(),
            env,
        )
    }

    /// The look for `spec`: the one already held when the spec is unchanged,
    /// otherwise requested again.
    fn look(
        &mut self,
        theme: &Rc<dyn WidgetTheme>,
        spec: &SliderSpec,
        env: &Environment,
    ) -> &SliderLook {
        if self.look.as_ref().is_none_or(|(held, _)| held != spec) {
            self.look = Some((spec.clone(), theme.slider(spec, env)));
            self.indicator_text = None;
        }
        &self.look.as_ref().expect("the look was just ensured").1
    }

    /// The look for `spec` without holding it: the held one, or a fresh one
    /// for a measurement ahead of the render that holds it.
    fn peek(
        &self,
        theme: &Rc<dyn WidgetTheme>,
        spec: &SliderSpec,
        env: &Environment,
    ) -> SliderLook {
        self.look
            .as_ref()
            .filter(|(held, _)| held == spec)
            .map_or_else(|| theme.slider(spec, env), |(_, look)| look.clone())
    }
}

/// The spec of a slider in `env`.
fn slider_spec(
    label: &Label,
    size: ControlSize,
    range: &RangeInclusive<f64>,
    step: Option<f64>,
    value_labels: bool,
    value_indicator: bool,
    env: &Environment,
) -> SliderSpec {
    SliderSpec::new(
        size,
        range.clone(),
        step,
        waterui_core::layout::layout_direction(env).snapshot(),
        SpecTint::resolve(env),
        !matches!(label.effective_display_mode(env), LabelDisplayMode::Hidden),
        value_labels,
        value_indicator,
    )
}

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<SliderConfig> {
    fn intrinsic(
        state: &mut HydroState<E>,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        measure_slider_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// Measures a slider from its config, without a retained node.
pub fn measure_slider_intrinsic<E: crate::text::TextEngine>(
    slider: &SliderConfig,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> LayoutSize {
    let value_labels = !slider.min_value_label().is::<()>() || !slider.max_value_label().is::<()>();
    let spec = slider_spec(
        slider.label(),
        slider.size(),
        slider.range(),
        slider.step(),
        value_labels,
        slider.value_indicator().is_some(),
        env,
    );
    let look = theme.slider(&spec, env);
    let measured = Measured {
        label: crate::renderer::measure_label_intrinsic(slider.label(), state, env, theme),
        min: crate::renderer::measure_view_intrinsic(slider.min_value_label(), state, env, theme),
        max: crate::renderer::measure_view_intrinsic(slider.max_value_label(), state, env, theme),
    };
    intrinsic_size(&look, &spec, &measured)
}

/// The sizes of a slider's labels, as measured.
struct Measured {
    label: LayoutSize,
    min: LayoutSize,
    max: LayoutSize,
}

/// What of the slider's labels the look draws: the main label above the
/// row when the slider has one and the look reserves a band for it, the end
/// labels when it has any. Everything else measures as zero, so the layout
/// reserves nothing for a label that is not drawn.
struct Drawn {
    label: f64,
    min: Size,
    max: Size,
}

impl Drawn {
    fn new(look: &SliderLook, spec: &SliderSpec, measured: &Measured) -> Self {
        let size = |size: LayoutSize| Size::new(f64::from(size.width), f64::from(size.height));
        let label = if spec.has_label() && look.layout().label_spacing.is_some() {
            f64::from(measured.label.height)
        } else {
            0.0
        };
        let (min, max) = if spec.has_value_labels() {
            (size(measured.min), size(measured.max))
        } else {
            (Size::ZERO, Size::ZERO)
        };
        Self { label, min, max }
    }

    /// The height reserved above the control row for the label.
    fn band(&self, layout: &SliderLayout) -> f64 {
        match layout.label_spacing {
            Some(spacing) if self.label > 0.0 => self.label + spacing,
            _ => 0.0,
        }
    }

    /// The control row's height: the knob's travel, no shorter than a label
    /// beside the track.
    const fn row_height(&self, layout: &SliderLayout) -> f64 {
        layout.row_height.max(self.min.height).max(self.max.height)
    }

    /// The room beside the track on its left and right: an end label, its
    /// gap to the track and its inset from the edge, or the track's own
    /// inset where there is no label. The minimum end is on the right in a
    /// right-to-left layout.
    fn sides(&self, layout: &SliderLayout, direction: LayoutDirection) -> (f64, f64) {
        let side = |label: f64| {
            if label > 0.0 {
                layout.label_inset + label + layout.label_gap
            } else {
                layout.track_inset
            }
        };
        match direction {
            LayoutDirection::RightToLeft => (side(self.max.width), side(self.min.width)),
            LayoutDirection::LeftToRight => (side(self.min.width), side(self.max.width)),
        }
    }
}

/// The slider's intrinsic size from its look: the narrowest the track and
/// its end labels allow, and the label band over the control row.
fn intrinsic_size(look: &SliderLook, spec: &SliderSpec, measured: &Measured) -> LayoutSize {
    let layout = look.layout();
    let drawn = Drawn::new(look, spec, measured);
    let (left, right) = drawn.sides(layout, spec.direction());
    let width = f64::from(measured.label.width).max(left + layout.minimum_track + right);
    let height = drawn.band(layout) + drawn.row_height(layout);
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    )
}

/// Where a slider's parts lie in `bounds`.
struct Placed {
    label: Option<Rect>,
    min_label: Option<Rect>,
    max_label: Option<Rect>,
    /// The row between the two ends of the track.
    track: Rect,
}

/// Lays the slider out in `bounds` by its look: the same measures
/// [`intrinsic_size`] reads.
fn place(look: &SliderLook, spec: &SliderSpec, measured: &Measured, bounds: Rect) -> Placed {
    let layout = look.layout();
    let drawn = Drawn::new(look, spec, measured);
    let direction = spec.direction();
    let (left, right) = drawn.sides(layout, direction);
    let band = drawn.band(layout);
    let label = (drawn.label > 0.0).then(|| {
        Rect::new(
            bounds.x0,
            bounds.y0,
            bounds.x1,
            (bounds.y0 + drawn.label).min(bounds.y1),
        )
    });
    let row = Rect::new(
        bounds.x0,
        (bounds.y0 + band).min(bounds.y1),
        bounds.x1,
        bounds.y1,
    );
    let center_y = f64::midpoint(row.y0, row.y1);
    let mut track_width = bounds.width() - left - right;
    if track_width <= 0.0 {
        contract_violation!(
            "hydrolysis slider resolved a non-positive track width ({track_width}); the track collapses to zero width"
        );
        track_width = 0.0;
    }
    let track = Rect::from_center_size(
        Point::new(f64::midpoint(bounds.x0 + left, bounds.x1 - right), center_y),
        Size::new(track_width, layout.row_height),
    );
    let beside = |size: Size, x0: f64| {
        (size.width > 0.0 && row.height() > 0.0)
            .then(|| label_beside_control_bounds(x0, x0 + size.width, row, track, size))
    };
    let left_label = |size: Size| beside(size, bounds.x0 + layout.label_inset);
    let right_label = |size: Size| beside(size, bounds.x1 - layout.label_inset - size.width);
    let (min_label, max_label) = match direction {
        LayoutDirection::RightToLeft => (right_label(drawn.min), left_label(drawn.max)),
        LayoutDirection::LeftToRight => (left_label(drawn.min), right_label(drawn.max)),
    };
    Placed {
        label,
        min_label,
        max_label,
        track,
    }
}

/// Field-level accessibility emission for the [`SliderRenderState`]-based
/// retained node path.
// empty when the accessibility feature is off
#[cfg_attr(not(feature = "accessibility"), allow(clippy::missing_const_for_fn))]
#[expect(
    clippy::too_many_arguments,
    reason = "the node's fields, passed apart because the caller holds the state borrowed"
)]
fn slider_accessibility_parts<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    ctx: Option<RenderContext>,
    label: &Label,
    range: &RangeInclusive<f64>,
    step: Option<f64>,
    value: &Binding<f64>,
    env: &Environment,
    focus_keys: &[crate::renderer::InteractionKey],
) {
    #[cfg(feature = "accessibility")]
    {
        let mut node = AccessibilityNode::new(
            crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::Slider,
            ),
        );
        let default_label =
            crate::renderer::SemanticCore::<E>::accessibility_label_from_label(label, env);
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
        debug_assert!(
            start < end,
            "the retained slider's range was validated by `SliderRenderState::from_config`"
        );
        // Read without watching: the node does not re-render for a value
        // change, the tree follows it through `register_live_numeric`.
        let current = value.snapshot().clamp(start, end);
        let step = step.unwrap_or_else(|| slider_step_for_range(range.clone()));
        node.set_numeric_value(current);
        node.set_min_numeric_value(start);
        node.set_max_numeric_value(end);
        node.set_numeric_value_step(step);
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
                step,
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
        let _ = (renderer, ctx, label, range, step, value, env, focus_keys);
    }
}

/// Measures a retained slider leaf from its [`SliderRenderState`], reading
/// the value-end labels from their already-built [`RetainedSubview`]s (the
/// measure path has no renderer to build on, so the labels are pre-built at
/// tree-build time).
pub fn measure_slider_node<E: crate::text::TextEngine>(
    render_state: &SliderRenderState<E>,
    _proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> ViewDimensions {
    let spec = render_state.spec(env);
    let look = render_state.peek(theme, &spec, env);
    let measured = Measured {
        label: render_state.label_view.measure_built(state, env, theme),
        min: render_state
            .min_value_label
            .measure_built(state, env, theme),
        max: render_state
            .max_value_label
            .measure_built(state, env, theme),
    };
    ViewDimensions::new(intrinsic_size(&look, &spec, &measured))
}

/// Renders a retained slider leaf every flush: emits a11y (unless hidden) then the
/// track + knob + labels + drag target.
pub fn render_slider_node<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<SliderRenderState<E>>>,
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
            slider.step,
            &slider.value,
            env,
            &[crate::renderer::InteractionKey::for_rc(state, 0)],
        );
    }
    render_slider_parts(ctx, state, env);
}

/// The knob's centre for the normalized `value` on `track`.
fn knob_center(track: Rect, value: f64, direction: LayoutDirection) -> Point {
    let along = value * track.width();
    Point::new(
        match direction {
            LayoutDirection::RightToLeft => track.x1 - along,
            LayoutDirection::LeftToRight => track.x0 + along,
        },
        track.center().y,
    )
}

/// The value a drag to `local` sets, normalized to the range, under
/// `mapping`; `None` when the mapping needs a press point and there is none.
fn dragged_value(
    mapping: DragMapping,
    track: Rect,
    direction: LayoutDirection,
    local: Point,
    press: Option<(Point, f64)>,
) -> Option<f64> {
    if track.width() <= 0.0 {
        // A collapsed track (reported when it was placed) has no positions to map to.
        return None;
    }
    let rtl = matches!(direction, LayoutDirection::RightToLeft);
    let toward_end = |along: f64| if rtl { -along } else { along };
    let value = match mapping {
        DragMapping::Absolute => {
            toward_end((local.x - track.x0) / track.width() - if rtl { 1.0 } else { 0.0 })
        }
        DragMapping::Relative { .. } => {
            let (press, value) = press?;
            value + toward_end((local.x - press.x) / track.width())
        }
    };
    Some(value.clamp(0.0, 1.0))
}

/// Binds the slider's interaction, lays its look out and lists it: the
/// labels, the live parts of the look and the drag target.
#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn render_slider_parts<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<SliderRenderState<E>>>,
    env: &Environment,
) {
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let theme = ctx.theme();
    let mut state = state.borrow_mut();
    let spec = state.spec(env);
    let look = state.look(&theme, &spec, env).clone();
    // Reading the disabled signal watches it, so a change schedules a frame
    // and this persistent node re-renders (and re-registers input) with the
    // new state.
    let disabled = {
        let signal = widget_disabled(env);
        ctx.renderer_mut().read_signal(&signal)
    };
    // The value is deliberately not watched: a value change re-renders
    // nothing. The look's parts and the indicator's text follow it live, the
    // accessibility tree through the watcher below, and a drag reads it when
    // the press lands.
    let value_binding = state.value.clone();
    let range_start = *state.range.start();
    let range_end = *state.range.end();
    let span = range_end - range_start;
    let direction = spec.direction();
    if state.value_watch.is_none() {
        let changed = Rc::clone(&state.value_changed);
        let frames = ctx.renderer_mut().frame_signals();
        let armed = Rc::new(Cell::new(false));
        let armed_for_watch = Rc::clone(&armed);
        state.value_watch = Some(Box::new(value_binding.watch(move |_| {
            // The registration-time emission echoes the value read now.
            if !armed_for_watch.get() {
                return;
            }
            changed.set(true);
            // Wakes the pump for the accessibility publish. A redraw is the
            // cheapest request: it marks nothing and re-encodes nothing.
            frames.request_redraw();
        })));
        armed.set(true);
    }
    #[cfg(feature = "accessibility")]
    ctx.renderer_mut().register_live_numeric(
        interaction_key.clone(),
        Rc::clone(&state.value_changed),
        value_binding.clone(),
        state.range.clone(),
    );

    // Every label is a retained node sub-view re-flushed at its rect; reactive
    // content stays live through the node's own per-frame re-flush, with no
    // dispatch. A disabled control dims every label to the theme's
    // disabled-content alpha (Material: on-surface at 38%).
    let measured = Measured {
        label: layout_size(state.label_view.measure_intrinsic(ctx.renderer_mut(), env)),
        min: layout_size(
            state
                .min_value_label
                .measure_intrinsic(ctx.renderer_mut(), env),
        ),
        max: layout_size(
            state
                .max_value_label
                .measure_intrinsic(ctx.renderer_mut(), env),
        ),
    };
    let placed = place(&look, &spec, &measured, ctx.bounds);
    let dim = |ctx: &mut WidgetRenderContext<'_, E>,
               role: &'static str,
               rect: Rect,
               place: &mut dyn FnMut(&mut WidgetRenderContext<'_, E>)| {
        ctx.with_scope_if(
            disabled,
            crate::renderer::mount::ScopeKey { role, item: 0 },
            theme.disabled_content_alpha(),
            rect,
            place,
        );
    };
    if let Some(label_rect) = placed.label {
        // The label's semantics are merged into the slider's own node by
        // `slider_accessibility_parts`, so the sub-view flushes visual-only.
        // The min/max value labels below stay exposed: their text (e.g.
        // "Dark"/"Bright") is not carried by the slider node.
        dim(ctx, "label-dim", label_rect, &mut |ctx| {
            let render_ctx = ctx.render_context();
            let label_area = ctx.safe_area_for(label_rect);
            let label_view = &mut state.label_view;
            ctx.renderer_mut()
                .with_suppressed_accessibility(|renderer| {
                    label_view.place(
                        renderer,
                        render_ctx,
                        env,
                        ProposalSize::UNSPECIFIED,
                        label_rect,
                        label_area,
                    );
                });
        });
    }
    if let Some(rect) = placed.min_label {
        dim(ctx, "min-label-dim", rect, &mut |ctx| {
            let render_ctx = ctx.render_context();
            let label_area = ctx.safe_area_for(rect);
            state.min_value_label.place(
                ctx.renderer_mut(),
                render_ctx,
                env,
                ProposalSize::UNSPECIFIED,
                rect,
                label_area,
            );
        });
    }
    if let Some(rect) = placed.max_label {
        dim(ctx, "max-label-dim", rect, &mut |ctx| {
            let render_ctx = ctx.render_context();
            let label_area = ctx.safe_area_for(rect);
            state.max_value_label.place(
                ctx.renderer_mut(),
                render_ctx,
                env,
                ProposalSize::UNSPECIFIED,
                rect,
                label_area,
            );
        });
    }

    let track = placed.track;
    let knob = *look.knob();
    // Where a press lands: the track and the room for the knob beyond its
    // ends. A drag that holds the knob alone declines a press off the knob
    // when it lands, against the knob's position then.
    let hit_bounds = {
        let target = knob.target(track.center());
        Rect::new(
            track.x0 - target.width() / 2.0,
            track.y0.min(target.y0),
            track.x1 + target.width() / 2.0,
            track.y1.max(target.y1),
        )
    };
    let chrome = look.chrome().clone();
    let (_, press_slot, handles) = ctx.renderer_mut().bind_live_chrome_target(
        interaction_key.clone(),
        hit_bounds,
        env,
        disabled,
        &chrome,
    );
    let flags = ctx
        .renderer_mut()
        .reported_interaction_state(&interaction_key);

    // The value indicator: a bubble resting above the start of the track
    // and carried along with the knob while the slider is held. It is as
    // wide as the widest of the range's ends needs, so it holds its shape
    // through a drag.
    let indicator = look
        .get_value_indicator()
        .filter(|_| spec.has_value_indicator())
        .zip(state.value_indicator.clone())
        .map(|(indicator, formatter)| {
            let font = indicator.get_font().clone();
            let color = indicator.get_color().clone();
            let text: Rc<dyn Fn(f64) -> StyledStr> = Rc::new(move |value: f64| {
                StyledStr::plain(formatter.format(value))
                    .font(font.clone())
                    .foreground(color.clone())
            });
            let measure = |renderer: &mut WidgetRenderContext<'_, E>, value: f64| {
                HydrolysisRenderer::measure_text_dimensions(
                    renderer.state_mut(),
                    text(value),
                    HorizontalAlignment::Center,
                    env,
                    None,
                    Some(1),
                )
                .size
            };
            let start_size = measure(ctx, range_start);
            let end_size = measure(ctx, range_end);
            let text_size = LayoutSize::new(
                start_size.width.max(end_size.width),
                start_size.height.max(end_size.height),
            );
            let padding = indicator.get_padding();
            let minimum = indicator.get_minimum_size();
            let width = padding
                .x
                .mul_add(2.0, f64::from(text_size.width))
                .max(minimum.width);
            let height = padding
                .y
                .mul_add(2.0, f64::from(text_size.height))
                .max(minimum.height);
            let start = knob_center(track, 0.0, direction);
            let bottom = start.y - knob.size.height / 2.0 - indicator.get_gap();
            let rest = Rect::new(
                start.x - width / 2.0,
                bottom - height,
                start.x + width / 2.0,
                bottom,
            );
            let least = ctx.bounds.x0 + width / 2.0;
            let greatest = (ctx.bounds.x1 - width / 2.0).max(least);
            (
                IndicatorContent {
                    container: Rc::clone(indicator.container()),
                    rest,
                    limits: (least, greatest),
                },
                text,
            )
        });
    // The indicator's text is its own retained node following the value
    // reactively; the bubble's rect centres it.
    let mut indicator_text = state.indicator_text.take();
    if let Some((_, text)) = &indicator
        && indicator_text.is_none()
    {
        let text = Rc::clone(text);
        let styled: Computed<StyledStr> =
            Computed::new(value_binding.map(move |value: f64| text(value)));
        let mut subview = RetainedSubview::new(AnyView::new(
            styled.into_text().alignment(Alignment::Center),
        ));
        subview.ensure_built(ctx.renderer_mut(), env);
        indicator_text = Some(subview);
    }
    let content = ChromeContent {
        group_opacity: Some(look.get_group_opacity().clone()),
        label_color: None,
        slider: Some(SliderContent {
            parts: look.parts().to_vec(),
            value: Rc::clone(&state.normalized),
            mirrored: matches!(direction, LayoutDirection::RightToLeft),
            indicator: indicator.as_ref().map(|(content, ..)| content.clone()),
        }),
    };
    let report = state.report.clone();
    let parts = LiveChromeParts::ensure(
        &mut state.parts,
        &chrome,
        &content,
        track,
        flags,
        env,
        &report,
    )
    .clone();
    handles.attach_live(Rc::clone(parts.chrome()));
    let live = Rc::clone(parts.chrome());

    // The slider as one group at the look's group opacity: its parts, then
    // the value indicator over them.
    let clip = indicator
        .as_ref()
        .map_or(ctx.bounds, |(content, ..)| ctx.bounds.union(content.rest));
    let clip = clip.union(track.inflate(knob.size.width, knob.size.height / 2.0));
    let body = |ctx: &mut WidgetRenderContext<'_, E>| {
        let render_ctx = ctx.render_context();
        parts.push_below(ctx.renderer_mut(), render_ctx);
        if let (Some((content, _)), Some(indicator_text), Some((opacity, offset))) = (
            &indicator,
            indicator_text.as_mut(),
            live.indicator_operands(),
        ) {
            let operands = crate::renderer::ScopeOperands {
                opacity: Some(opacity),
                transform: Some(offset),
            };
            ctx.with_live_scope(
                crate::renderer::mount::ScopeKey {
                    role: "slider-indicator",
                    item: 0,
                },
                operands,
                clip,
                |ctx| {
                    let render_ctx = ctx.render_context();
                    parts.push_indicator(ctx.renderer_mut(), render_ctx);
                    let text_rect = content.rest;
                    let label_area = ctx.safe_area_for(text_rect);
                    let render_ctx = ctx.render_context();
                    indicator_text.place(
                        ctx.renderer_mut(),
                        render_ctx,
                        env,
                        ProposalSize::UNSPECIFIED,
                        text_rect,
                        label_area,
                    );
                },
            );
        }
        let render_ctx = ctx.render_context();
        parts.push_above(ctx.renderer_mut(), render_ctx);
    };
    // An opacity that varies between renders draws the slider as one group
    // following the chrome's operand; a fixed one groups it only when it
    // fades.
    let key = crate::renderer::mount::ScopeKey {
        role: "slider-group",
        item: 0,
    };
    let opacity = live.group_opacity().map_or_else(
        || {
            GroupOpacity::Fixed(look.get_group_opacity().target(&Inputs::at_rest(
                track,
                flags,
                &(),
            )))
        },
        GroupOpacity::Live,
    );
    ctx.with_group_scope(key, opacity, look.get_group_blend_space(), clip, body);
    state.indicator_text = indicator_text;

    // A disabled slider registers no drag target: the pointer neither presses
    // nor drags it. Targets are re-registered every flush, so re-enabling
    // restores interactivity on the next frame.
    if disabled {
        return;
    }
    // The drag's point lands in window space and the track is node-local:
    // resolve the inverse through the region's live placement at invoke
    // time. The registration outlives this record, so a transform captured
    // here goes stale the moment an ancestor moves the slider without
    // re-recording it (a scroll, a transition).
    let placement = ctx.renderer_mut().current_placement();
    let value_epsilon = slider_value_epsilon(span, track.width());
    let mapping = look.get_drag();
    let step = state.step;
    let drag_value = value_binding.clone();
    let drag_chrome = Rc::clone(&live);
    // A drag that holds the knob alone takes only a press on the knob, at
    // the knob's position as the press lands; any other press reaches
    // whatever lies beneath, a scroll view's drag among them. The value is
    // read for that one press, not watched.
    let press_filter: Option<PressFilter> = (mapping
        == (DragMapping::Relative { knob_only: true }))
    .then(|| {
        let placement = placement.clone();
        let value = value_binding.clone();
        Rc::new(move |point: Point| {
            let local = placement.resolved_transform(true).inverse() * point;
            let normalized = (value.snapshot().clamp(range_start, range_end) - range_start) / span;
            knob.target(knob_center(track, normalized, direction))
                .contains(local)
        }) as PressFilter
    });
    let drag_range = state.range.clone();
    let keyboard_range = state.range.clone();
    let keyboard_value = value_binding;
    let keyboard_step = step.unwrap_or(span / 100.0);
    ctx.renderer_mut().register_interactive_pointer_drag_target(
        hit_bounds,
        press_slot,
        press_filter,
        move |_renderer, point, _env| {
            let local = placement.resolved_transform(true).inverse() * point;
            let press = drag_chrome
                .press_point()
                .map(|press| (press, drag_chrome.press_value()));
            let Some(along) = dragged_value(mapping, track, direction, local, press) else {
                return false;
            };
            let next = snap(span.mul_add(along, range_start), &drag_range, step);
            if let Some((press, _)) = press {
                drag_chrome.set_drag(Some(local - press));
            }
            if (drag_value.snapshot() - next).abs() <= value_epsilon {
                return false;
            }
            // The parts follow the pointer at once, the value change that
            // follows finding them already there. Nothing re-records, so the
            // drag asks for no layout.
            drag_chrome.track_value((next - range_start) / span);
            drag_value.set(next);
            false
        },
        move |forward| {
            let current = keyboard_value.snapshot();
            let delta = if forward {
                keyboard_step
            } else {
                -keyboard_step
            };
            let next = snap(current + delta, &keyboard_range, step);
            if (current - next).abs() > f64::EPSILON {
                keyboard_value.set(next);
            }
            // The parts, the indicator and the accessibility tree follow the
            // value on their own: nothing asks for layout.
            false
        },
    );
}

/// `size` as a layout size.
const fn layout_size(size: LayoutSize) -> LayoutSize {
    size
}

/// Emits a retained slider's accessibility nodes for the semantic walk: the
/// slider node `slider_accessibility_parts` registers, plus the minimum and
/// maximum value labels — they flush unsuppressed in the rendered path, so
/// they emit their own nodes here too. The main label flushes visual-only and
/// emits nothing.
#[cfg(feature = "accessibility")]
pub fn emit_slider_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    state: &Rc<RefCell<SliderRenderState<E>>>,
    env: &Environment,
) {
    let interaction_key = crate::renderer::InteractionKey::for_rc(state, 0);
    let mut state = state.borrow_mut();
    slider_accessibility_parts(
        renderer,
        None,
        &state.label,
        &state.range,
        state.step,
        &state.value,
        env,
        &[interaction_key],
    );
    state.min_value_label.emit_accessibility(renderer, env);
    state.max_value_label.emit_accessibility(renderer, env);
}

#[cfg(test)]
mod tests {
    use super::{dragged_value, valid_range};
    use crate::theme::DragMapping;
    use kurbo::{Point, Rect};
    use waterui_core::layout::LayoutDirection;

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "slider requires range start < end")
    )]
    fn an_empty_range_is_reported_and_replaced_by_the_unit_range() {
        assert_eq!(valid_range(2.0..=2.0), 0.0..=1.0);
    }

    #[test]
    fn a_positive_range_is_kept() {
        assert_eq!(valid_range(-5.0..=5.0), -5.0..=5.0);
    }

    #[test]
    fn a_collapsed_track_maps_no_drag_position() {
        let track = Rect::new(10.0, 0.0, 10.0, 20.0);
        assert_eq!(
            dragged_value(
                DragMapping::Absolute,
                track,
                LayoutDirection::LeftToRight,
                Point::new(10.0, 10.0),
                None,
            ),
            None
        );
    }
}
