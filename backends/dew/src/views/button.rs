//! Persistent [`ButtonConfig`] node with retained pointer activation.

use core::cell::RefCell;

use accesskit::{Action as AccessibilityAction, Node as AccessibilityNode, NodeId, Role};
use kurbo::{Rect, RoundedRect, Stroke};
use nami::{Computed, Signal};
use waterui_controls::button::{ButtonConfig, ButtonStyle};
use waterui_core::Environment;
use waterui_core::handler::BoxedAction;
use waterui_core::layout::{ProposalSize, Size, ViewDimensions};

use crate::accessibility::ActionTarget;
use crate::dispatch::{DewNode, DewRenderer, RenderContext, WatchedSignal};
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
    /// The environment's disabled scope, subscribed once at build.
    disabled: WatchedSignal<Computed<bool>>,
    env: Environment,
    pointer: PointerTargetHandle,
    accessibility_id: NodeId,
}

struct ButtonPointer {
    action: BoxedAction<()>,
    disabled: Computed<bool>,
    armed: bool,
    /// The environment this button was built in — the one its action's
    /// extractors have to resolve against.
    env: Environment,
}

impl PointerHandler for ButtonPointer {
    fn pointer_down(&mut self, _point: kurbo::Point, _bounds: Rect) -> bool {
        self.armed = !self.disabled.get();
        false
    }

    fn pointer_up(&mut self, point: kurbo::Point, bounds: Rect) -> bool {
        let activate =
            core::mem::take(&mut self.armed) && bounds.contains(point) && !self.disabled.get();
        if activate {
            (self.action)(&self.env);
        }
        activate
    }

    fn pointer_cancel(&mut self) -> bool {
        self.armed = false;
        false
    }
}

pub fn build(
    renderer: &mut DewRenderer,
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
        env: env.clone(),
    });
    let label = LabelText::new(&label, env, renderer.signals());
    Box::new(ButtonNode {
        label,
        style,
        disabled: WatchedSignal::new(disabled, renderer.signals()),
        env: env.clone(),
        pointer,
        accessibility_id: renderer.allocate_accessibility_id(),
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
        let disabled = self.disabled.get();
        let (background, border, foreground) = palette(renderer.theme(), self.style, disabled);
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
        if renderer.accessibility_enabled() {
            renderer.register_built_accessibility_node(
                self.accessibility_id,
                ctx.window_bounds(),
                || {
                    let mut node = AccessibilityNode::new(match self.style {
                        ButtonStyle::Link => Role::Link,
                        _ => Role::Button,
                    });
                    node.set_label(self.label.semantic_text());
                    node.add_action(AccessibilityAction::Focus);
                    let target = if disabled {
                        node.set_disabled();
                        None
                    } else {
                        node.add_action(AccessibilityAction::Click);
                        Some(ActionTarget::Pointer {
                            handler: self.pointer.clone(),
                            bounds: ctx.window_bounds(),
                        })
                    };
                    (node, target)
                },
            );
        }
    }
}

fn palette(
    theme: &theme::ThemePalette,
    style: ButtonStyle,
    disabled: bool,
) -> (Option<peniko::Color>, Option<peniko::Color>, peniko::Color) {
    if disabled {
        return (
            matches!(
                style,
                ButtonStyle::Automatic | ButtonStyle::Bordered | ButtonStyle::BorderedProminent
            )
            .then_some(theme.surface()),
            matches!(style, ButtonStyle::Automatic | ButtonStyle::Bordered)
                .then_some(theme.border()),
            theme.muted_foreground(),
        );
    }
    match style {
        ButtonStyle::Automatic | ButtonStyle::Bordered => (
            Some(theme.surface()),
            Some(theme.border()),
            theme.foreground(),
        ),
        ButtonStyle::BorderedProminent => (Some(theme.accent()), None, theme.accent_foreground()),
        ButtonStyle::Plain | ButtonStyle::Borderless => (None, None, theme.foreground()),
        ButtonStyle::Link => (None, None, theme.accent()),
        _ => panic!("dew does not implement ButtonStyle::{style:?}"),
    }
}
