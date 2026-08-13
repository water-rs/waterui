//! [`ResolvedTextFieldConfig`]: a bordered input box showing the bound
//! value, or the placeholder prompt while the value is empty.
//!
//! Dew has no focus or key-input runtime yet, so the field is a faithful
//! visual: the box, the label row above it, and the value/prompt text.

use std::cell::RefCell;

use kurbo::{Rect, RoundedRect, Stroke};
use nami::Signal;
use waterui_controls::text_field::ResolvedTextFieldConfig;
use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, ViewDimensions};

use crate::dispatch::{DewNode, DewRenderer, RenderContext, WatchedSignal};
use crate::text::DewState;
use crate::views::{LabelText, emit_styled_text, to_f32};

/// Minimum input box width.
const MIN_WIDTH: f64 = 96.0;
/// Minimum input box height.
const MIN_HEIGHT: f64 = 36.0;
/// Gap between the label row and the box.
const LABEL_SPACING: f64 = 4.0;
/// Horizontal text inset inside the box.
const HORIZONTAL_INSET: f64 = 10.0;
/// Vertical text inset inside the box.
const VERTICAL_INSET: f64 = 8.0;
/// Corner radius of the box.
const CORNER_RADIUS: f64 = 6.0;
/// Outline width of the box.
const BOX_BORDER: f64 = 1.0;

struct TextFieldNode {
    config: ResolvedTextFieldConfig,
    label: LabelText,
    /// The bound value text, subscribed once at build.
    value: WatchedSignal<nami::Binding<waterui_text::styled::StyledStr>>,
    /// The placeholder prompt, subscribed once at build.
    prompt: WatchedSignal<nami::Computed<waterui_text::styled::StyledStr>>,
    env: Environment,
}

pub fn build(
    renderer: &DewRenderer,
    config: ResolvedTextFieldConfig,
    env: &Environment,
) -> Box<dyn DewNode> {
    let label = LabelText::new(&config.label, env, renderer.signals());
    let value = WatchedSignal::new(config.value.clone(), renderer.signals());
    let prompt = WatchedSignal::new(config.prompt.content.clone(), renderer.signals());
    Box::new(TextFieldNode {
        config,
        label,
        value,
        prompt,
        env: env.clone(),
    })
}

impl DewNode for TextFieldNode {
    fn measure(&self, state: &RefCell<DewState>, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(measure(state, &self.config, &self.label, &self.env))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        render(
            renderer,
            ctx,
            &self.label,
            &self.env,
            &self.value.get(),
            &self.prompt.get(),
        );
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }
}

fn render(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    label: &LabelText,
    env: &Environment,
    value: &waterui_text::styled::StyledStr,
    prompt: &waterui_text::styled::StyledStr,
) {
    let bounds = ctx.bounds;

    let label_size = label.measure(renderer.state_cell(), env);
    let label_height = if label_size.height > 0.0 {
        f64::from(label_size.height) + LABEL_SPACING
    } else {
        0.0
    };
    if label_height > 0.0 {
        let label_rect = Rect::new(
            bounds.x0,
            bounds.y0,
            bounds.x1,
            (bounds.y0 + f64::from(label_size.height)).min(bounds.y1),
        );
        label.render(renderer, ctx, label_rect, env);
    }

    let box_rect = Rect::new(bounds.x0, bounds.y0 + label_height, bounds.x1, bounds.y1);
    let surface = renderer.theme().surface();
    let border = renderer.theme().border();
    let muted_foreground = renderer.theme().muted_foreground();
    let foreground = renderer.theme().foreground();
    renderer.list_mut().fill(
        &RoundedRect::from_rect(box_rect, CORNER_RADIUS),
        ctx.transform,
        surface,
    );
    renderer.list_mut().stroke(
        &RoundedRect::from_rect(box_rect, CORNER_RADIUS),
        ctx.transform,
        Stroke::new(BOX_BORDER),
        border,
    );

    let content_rect = Rect::new(
        box_rect.x0 + HORIZONTAL_INSET,
        box_rect.y0 + VERTICAL_INSET,
        box_rect.x1 - HORIZONTAL_INSET,
        box_rect.y1 - VERTICAL_INSET,
    );
    if content_rect.width() <= 0.0 || content_rect.height() <= 0.0 {
        return;
    }
    if value.to_plain().is_empty() {
        emit_styled_text(renderer, ctx, content_rect, prompt, env, muted_foreground);
    } else {
        emit_styled_text(renderer, ctx, content_rect, value, env, foreground);
    }
}

/// Intrinsic size: the label row above a box sized for the larger of the
/// value and prompt texts, floored at the minimum box footprint.
fn measure(
    state: &RefCell<DewState>,
    config: &ResolvedTextFieldConfig,
    label: &LabelText,
    env: &Environment,
) -> Size {
    let label = label.measure(state, env);
    let value = config.value.get();
    let prompt = config.prompt.content.get();
    let (value_width, value_height) = state.borrow_mut().measure_styled(&value, env, None);
    let (prompt_width, prompt_height) = state.borrow_mut().measure_styled(&prompt, env, None);

    let text_height = f64::from(value_height.max(prompt_height));
    let box_width = HORIZONTAL_INSET
        .mul_add(2.0, f64::from(value_width.max(prompt_width)))
        .max(MIN_WIDTH);
    let box_height = VERTICAL_INSET.mul_add(2.0, text_height).max(MIN_HEIGHT);
    let label_height = if label.height > 0.0 {
        f64::from(label.height) + LABEL_SPACING
    } else {
        0.0
    };
    Size::new(
        to_f32(f64::from(label.width).max(box_width)),
        to_f32(label_height + box_height),
    )
}
