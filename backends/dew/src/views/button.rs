//! Persistent [`ButtonConfig`] node with retained pointer activation.

use core::cell::RefCell;

use kurbo::{Rect, RoundedRect, Stroke};
use nami::{Computed, Signal};
use waterui_controls::button::{ButtonConfig, ButtonStyle};
use waterui_core::Environment;
use waterui_core::handler::BoxedAction;
use waterui_core::layout::{ProposalSize, Size, ViewDimensions};

use crate::dispatch::{DewNode, DewRenderer, RenderContext};
use crate::pointer::{PointerHandler, PointerTargetHandle};
use crate::text::DewState;
use crate::theme;
use crate::views::{LabelText, to_f32};

const HORIZONTAL_PADDING: f64 = 12.0;
const VERTICAL_PADDING: f64 = 7.0;
const MIN_HEIGHT: f64 = 32.0;
const CORNER_RADIUS: f64 = 7.0;
const BORDER_WIDTH: f64 = 1.0;

struct ButtonNode {
    label: LabelText,
    style: ButtonStyle,
    disabled: Computed<bool>,
    env: Environment,
    pointer: PointerTargetHandle,
}

struct ButtonPointer {
    action: BoxedAction<()>,
    disabled: Computed<bool>,
    armed: bool,
}

impl PointerHandler for ButtonPointer {
    fn pointer_down(&mut self, _point: kurbo::Point, _bounds: Rect, _env: &Environment) -> bool {
        self.armed = !self.disabled.get();
        false
    }

    fn pointer_up(&mut self, point: kurbo::Point, bounds: Rect, env: &Environment) -> bool {
        let activate =
            core::mem::take(&mut self.armed) && bounds.contains(point) && !self.disabled.get();
        if activate {
            (self.action)(env);
        }
        activate
    }

    fn pointer_cancel(&mut self, _env: &Environment) -> bool {
        self.armed = false;
        false
    }
}

pub fn build(
    renderer: &DewRenderer,
    config: ButtonConfig,
    env: &Environment,
) -> Box<dyn DewNode> {
    let ButtonConfig {
        label,
        action,
        style,
        ..
    } = config;
    // Disabled state is a scoped subtree attribute, read from the environment
    // in force at this leaf rather than carried on the control's config.
    let disabled = crate::views::view_disabled(env);
    let pointer = PointerTargetHandle::new(ButtonPointer {
        action,
        disabled: disabled.clone(),
        armed: false,
    });
    let label = LabelText::new(&label, env, renderer.signals());
    Box::new(ButtonNode {
        label,
        style,
        disabled,
        env: env.clone(),
        pointer,
    })
}

impl DewNode for ButtonNode {
    fn measure(&self, state: &RefCell<DewState>, _proposal: ProposalSize) -> ViewDimensions {
        let label = self.label.measure(state, &self.env);
        ViewDimensions::new(Size::new(
            to_f32(HORIZONTAL_PADDING.mul_add(2.0, f64::from(label.width))),
            to_f32(
                VERTICAL_PADDING
                    .mul_add(2.0, f64::from(label.height))
                    .max(MIN_HEIGHT),
            ),
        ))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        let disabled = renderer.read_signal(&self.disabled);
        let (background, border, foreground) = palette(self.style, disabled);
        if let Some(background) = background {
            renderer.list_mut().fill(
                &RoundedRect::from_rect(ctx.bounds, CORNER_RADIUS),
                ctx.transform,
                background,
            );
        }
        if let Some(border) = border {
            renderer.list_mut().stroke(
                &RoundedRect::from_rect(ctx.bounds, CORNER_RADIUS),
                ctx.transform,
                Stroke::new(BORDER_WIDTH),
                border,
            );
        }
        let label_rect = Rect::new(
            ctx.bounds.x0 + HORIZONTAL_PADDING,
            ctx.bounds.y0 + VERTICAL_PADDING,
            (ctx.bounds.x1 - HORIZONTAL_PADDING).max(ctx.bounds.x0 + HORIZONTAL_PADDING),
            (ctx.bounds.y1 - VERTICAL_PADDING).max(ctx.bounds.y0 + VERTICAL_PADDING),
        );
        self.label
            .render_with_brush(renderer, ctx, label_rect, &self.env, foreground);
        if !disabled {
            renderer.register_pointer_target(ctx.window_bounds(), self.pointer.clone());
        }
    }
}

fn palette(
    style: ButtonStyle,
    disabled: bool,
) -> (Option<peniko::Color>, Option<peniko::Color>, peniko::Color) {
    if disabled {
        return (
            matches!(
                style,
                ButtonStyle::Automatic | ButtonStyle::Bordered | ButtonStyle::BorderedProminent
            )
            .then_some(theme::SURFACE),
            matches!(style, ButtonStyle::Automatic | ButtonStyle::Bordered)
                .then_some(theme::BORDER),
            theme::MUTED_FOREGROUND,
        );
    }
    match style {
        ButtonStyle::Automatic | ButtonStyle::Bordered => {
            (Some(theme::SURFACE), Some(theme::BORDER), theme::FOREGROUND)
        }
        ButtonStyle::BorderedProminent => (Some(theme::ACCENT), None, theme::ACCENT_FOREGROUND),
        ButtonStyle::Plain | ButtonStyle::Borderless => (None, None, theme::FOREGROUND),
        ButtonStyle::Link => (None, None, theme::ACCENT),
        _ => panic!("dew does not implement ButtonStyle::{style:?}"),
    }
}
