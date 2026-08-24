//! [`StepperConfig`]: the current value beside trailing `[-]` `[+]` boxes.

use std::cell::RefCell;

use kurbo::{Line, Rect, RoundedRect, Stroke};
use nami::{Binding, Computed, Signal};
use waterui_controls::stepper::StepperConfig;
use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, ViewDimensions};
use waterui_text::styled::StyledStr;

use crate::dispatch::{DewNode, DewRenderer, RenderContext, WatchedSignal};
use crate::pointer::{PointerHandler, PointerTargetHandle};
use crate::text::DewState;
use crate::views::{LabelText, emit_styled_text, to_f32};

/// Side length of each square stepper button.
const BUTTON_SIZE: f64 = 28.0;
/// Gap between the two buttons.
const BUTTON_SPACING: f64 = 6.0;
/// Gap between the label and the value text.
const LABEL_SPACING: f64 = 8.0;
/// Gap between the value text and the buttons.
const VALUE_SPACING: f64 = 8.0;
/// Corner radius of the buttons.
const CORNER_RADIUS: f64 = 6.0;
/// Half-length of the `-` / `+` icon arms.
const ICON_ARM: f64 = 5.0;
/// Stroke width of the icon arms.
const ICON_STROKE: f64 = 1.6;
/// Outline width of the buttons.
const BUTTON_BORDER: f64 = 1.0;

/// The intrinsic value text for measurement, without subscribing.
fn value_text_for_measure(config: &StepperConfig) -> StyledStr {
    config.value_formatter.as_ref().map_or_else(
        || StyledStr::plain(config.value.get().to_string()),
        Signal::get,
    )
}

struct StepperNode {
    config: StepperConfig,
    label: LabelText,
    /// The formatted value text signal, subscribed once at build.
    formatter: Option<WatchedSignal<Computed<StyledStr>>>,
    /// The raw value, subscribed once at build.
    value: WatchedSignal<Binding<i32>>,
    /// The step amount, subscribed once at build.
    step: WatchedSignal<Computed<i32>>,
    env: Environment,
    decrement: PointerTargetHandle,
    increment: PointerTargetHandle,
}

impl StepperNode {
    /// The formatted value text: the configured formatter when present,
    /// otherwise the plain decimal value.
    fn value_text(&self) -> StyledStr {
        self.formatter.as_ref().map_or_else(
            || StyledStr::plain(self.value.get().to_string()),
            WatchedSignal::get,
        )
    }
}

struct StepperPointer {
    value: Binding<i32>,
    step: Computed<i32>,
    range: core::ops::RangeInclusive<i32>,
    direction: i32,
    armed: bool,
}

impl PointerHandler for StepperPointer {
    fn pointer_down(&mut self, _point: kurbo::Point, _bounds: Rect) -> bool {
        self.armed = true;
        false
    }

    fn pointer_up(&mut self, point: kurbo::Point, bounds: Rect) -> bool {
        if !core::mem::take(&mut self.armed) || !bounds.contains(point) {
            return false;
        }
        let step = self.step.get();
        assert!(step > 0, "dew stepper requires a positive step");
        let current = self.value.get();
        let next = current
            .saturating_add(step.saturating_mul(self.direction))
            .clamp(*self.range.start(), *self.range.end());
        if next == current {
            return false;
        }
        self.value.set(next);
        true
    }

    fn pointer_cancel(&mut self) -> bool {
        self.armed = false;
        false
    }
}

pub fn build(renderer: &DewRenderer, config: StepperConfig, env: &Environment) -> Box<dyn DewNode> {
    let decrement = PointerTargetHandle::new(StepperPointer {
        value: config.value.clone(),
        step: config.step.clone(),
        range: config.range.clone(),
        direction: -1,
        armed: false,
    });
    let increment = PointerTargetHandle::new(StepperPointer {
        value: config.value.clone(),
        step: config.step.clone(),
        range: config.range.clone(),
        direction: 1,
        armed: false,
    });
    let label = LabelText::new(&config.label, env, renderer.signals());
    let formatter = config
        .value_formatter
        .clone()
        .map(|formatter| WatchedSignal::new(formatter, renderer.signals()));
    let value = WatchedSignal::new(config.value.clone(), renderer.signals());
    let step = WatchedSignal::new(config.step.clone(), renderer.signals());
    Box::new(StepperNode {
        config,
        label,
        formatter,
        value,
        step,
        env: env.clone(),
        decrement,
        increment,
    })
}

impl DewNode for StepperNode {
    fn measure(&self, state: &RefCell<DewState>, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(measure(state, &self.config, &self.label, &self.env))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        assert!(self.step.get() > 0, "dew stepper requires a positive step");
        let styled_value = self.value_text();
        let (minus, plus) = render(
            renderer,
            ctx,
            &self.config,
            &self.label,
            &self.env,
            &styled_value,
        );
        renderer.register_pointer_target(
            ctx.transform.transform_rect_bbox(minus),
            self.decrement.clone(),
        );
        renderer.register_pointer_target(
            ctx.transform.transform_rect_bbox(plus),
            self.increment.clone(),
        );
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }
}

fn render(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    config: &StepperConfig,
    label: &LabelText,
    env: &Environment,
    styled_value: &StyledStr,
) -> (Rect, Rect) {
    assert!(
        config.range.start() <= config.range.end(),
        "dew stepper requires an ordered range"
    );
    let bounds = ctx.bounds;

    let button_size = bounds.height().clamp(0.0, BUTTON_SIZE);
    let controls_width = button_size.mul_add(2.0, BUTTON_SPACING);
    let controls_x0 = (bounds.x1 - controls_width).max(bounds.x0);

    let (value_width, _) =
        renderer
            .state_cell()
            .borrow_mut()
            .measure_styled(styled_value, env, None);
    let value_x0 = (controls_x0 - VALUE_SPACING - f64::from(value_width)).max(bounds.x0);
    let value_rect = Rect::new(value_x0, bounds.y0, controls_x0 - VALUE_SPACING, bounds.y1);
    if value_rect.width() > 0.0 {
        let muted_foreground = renderer.theme().muted_foreground();
        emit_styled_text(
            renderer,
            ctx,
            value_rect,
            styled_value,
            env,
            muted_foreground,
        );
    }

    let label_rect = Rect::new(
        bounds.x0,
        bounds.y0,
        (value_x0 - LABEL_SPACING).max(bounds.x0),
        bounds.y1,
    );
    if label_rect.width() > 0.0 {
        label.render(renderer, ctx, label_rect, env);
    }

    let button_y0 = bounds.y0 + ((bounds.height() - button_size) / 2.0).max(0.0);
    let minus = Rect::new(
        controls_x0,
        button_y0,
        controls_x0 + button_size,
        button_y0 + button_size,
    );
    let plus = Rect::new(
        minus.x1 + BUTTON_SPACING,
        button_y0,
        minus.x1 + BUTTON_SPACING + button_size,
        button_y0 + button_size,
    );
    draw_button(renderer, ctx, minus, false);
    draw_button(renderer, ctx, plus, true);
    (minus, plus)
}

fn draw_button(renderer: &mut DewRenderer, ctx: RenderContext, rect: Rect, plus: bool) {
    let surface = renderer.theme().surface();
    let border = renderer.theme().border();
    let foreground = renderer.theme().foreground();
    renderer.list_mut().fill(
        &RoundedRect::from_rect(rect, CORNER_RADIUS),
        ctx.transform,
        surface,
    );
    renderer.list_mut().stroke(
        &RoundedRect::from_rect(rect, CORNER_RADIUS),
        ctx.transform,
        Stroke::new(BUTTON_BORDER),
        border,
    );
    let center = rect.center();
    let icon_stroke = Stroke::new(ICON_STROKE);
    renderer.list_mut().stroke(
        &Line::new(
            (center.x - ICON_ARM, center.y),
            (center.x + ICON_ARM, center.y),
        ),
        ctx.transform,
        icon_stroke.clone(),
        foreground,
    );
    if plus {
        renderer.list_mut().stroke(
            &Line::new(
                (center.x, center.y - ICON_ARM),
                (center.x, center.y + ICON_ARM),
            ),
            ctx.transform,
            icon_stroke,
            foreground,
        );
    }
}

/// Label plus value text plus the two-button block; horizontal stretch
/// pushes the buttons to the trailing edge during placement.
fn measure(
    state: &RefCell<DewState>,
    config: &StepperConfig,
    label: &LabelText,
    env: &Environment,
) -> Size {
    assert!(
        config.range.start() <= config.range.end(),
        "dew stepper requires an ordered range"
    );
    assert!(
        config.step.get() > 0,
        "dew stepper requires a positive step"
    );
    let label = label.measure(state, env);
    let styled_value = value_text_for_measure(config);
    let (value_width, value_height) = state.borrow_mut().measure_styled(&styled_value, env, None);

    let controls_width = BUTTON_SIZE.mul_add(2.0, BUTTON_SPACING);
    let mut width = controls_width + VALUE_SPACING + f64::from(value_width);
    if label.width > 0.0 {
        width += f64::from(label.width) + LABEL_SPACING;
    }
    let height = f64::from(label.height.max(value_height)).max(BUTTON_SIZE);
    Size::new(to_f32(width), to_f32(height))
}
