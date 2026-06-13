//! [`StepperConfig`]: the current value beside trailing `[-]` `[+]` boxes.

use std::cell::RefCell;

use kurbo::{Line, Rect, RoundedRect, Stroke};
use nami::Signal;
use waterui_controls::stepper::StepperConfig;
use waterui_core::layout::Size;
use waterui_core::{Environment, Native};
use waterui_text::styled::StyledStr;

use crate::dispatch::{DewRenderer, RenderContext};
use crate::text::DewState;
use crate::theme;
use crate::views::{emit_styled_text, measure_label, render_label, to_f32};

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

/// The formatted value text: the configured formatter when present,
/// otherwise the plain decimal value.
fn value_text(renderer: &mut DewRenderer, config: &StepperConfig) -> StyledStr {
    if let Some(formatter) = &config.value_formatter {
        renderer.read_signal(formatter)
    } else {
        StyledStr::plain(renderer.read_signal(&config.value).to_string())
    }
}

/// The intrinsic value text for measurement, without subscribing.
fn value_text_for_measure(config: &StepperConfig) -> StyledStr {
    config.value_formatter.as_ref().map_or_else(
        || StyledStr::plain(config.value.get().to_string()),
        Signal::get,
    )
}

/// Draws the label leading, the current value trailing before the buttons,
/// and the `[-]` `[+]` boxes at the far end.
pub fn render(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    view: Native<StepperConfig>,
    env: &Environment,
) {
    let config = view.into_inner();
    assert!(
        config.range.start() <= config.range.end(),
        "dew stepper requires an ordered range"
    );
    let bounds = ctx.bounds;

    let button_size = bounds.height().clamp(0.0, BUTTON_SIZE);
    let controls_width = button_size.mul_add(2.0, BUTTON_SPACING);
    let controls_x0 = (bounds.x1 - controls_width).max(bounds.x0);

    let styled_value = value_text(renderer, &config);
    let (value_width, _) =
        renderer
            .state_cell()
            .borrow_mut()
            .measure_styled(&styled_value, env, None);
    let value_x0 = (controls_x0 - VALUE_SPACING - f64::from(value_width)).max(bounds.x0);
    let value_rect = Rect::new(value_x0, bounds.y0, controls_x0 - VALUE_SPACING, bounds.y1);
    if value_rect.width() > 0.0 {
        emit_styled_text(
            renderer,
            ctx,
            value_rect,
            &styled_value,
            env,
            theme::MUTED_FOREGROUND,
        );
    }

    let label_rect = Rect::new(
        bounds.x0,
        bounds.y0,
        (value_x0 - LABEL_SPACING).max(bounds.x0),
        bounds.y1,
    );
    if label_rect.width() > 0.0 {
        render_label(renderer, ctx, label_rect, &config.label, env);
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
}

fn draw_button(renderer: &mut DewRenderer, ctx: RenderContext, rect: Rect, plus: bool) {
    renderer.list_mut().fill(
        &RoundedRect::from_rect(rect, CORNER_RADIUS),
        ctx.transform,
        theme::SURFACE,
    );
    renderer.list_mut().stroke(
        &RoundedRect::from_rect(rect, CORNER_RADIUS),
        ctx.transform,
        Stroke::new(BUTTON_BORDER),
        theme::BORDER,
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
        theme::FOREGROUND,
    );
    if plus {
        renderer.list_mut().stroke(
            &Line::new(
                (center.x, center.y - ICON_ARM),
                (center.x, center.y + ICON_ARM),
            ),
            ctx.transform,
            icon_stroke,
            theme::FOREGROUND,
        );
    }
}

/// Label plus value text plus the two-button block; horizontal stretch
/// pushes the buttons to the trailing edge during placement.
pub fn measure(state: &RefCell<DewState>, config: &StepperConfig, env: &Environment) -> Size {
    assert!(
        config.range.start() <= config.range.end(),
        "dew stepper requires an ordered range"
    );
    let label = measure_label(state, &config.label, env);
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
