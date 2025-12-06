//! Control render nodes (slider, toggle, etc.).

use nami::{Binding, SignalExt};
use waterui_controls::{
    slider::SliderConfig, stepper::StepperConfig, text_field::TextFieldConfig, toggle::ToggleConfig,
};
use waterui_core::Str;

use crate::{DrawCommand, LayoutCtx, LayoutResult, NodeSignal, RenderCtx, RenderNode, Size};

/// Simplified slider node (placeholder visuals until a real skin exists).
#[derive(Debug)]

pub struct SliderNode {
    range: (f64, f64),
    binding: Binding<f64>,
    value: NodeSignal<f64>,
}

impl SliderNode {
    /// Creates a new slider node from the provided configuration.
    #[must_use]
    pub fn new(config: SliderConfig) -> Self {
        let range = (*config.range.start(), *config.range.end());
        let binding = config.value;
        let value = NodeSignal::new(binding.clone().computed());
        Self {
            range,
            binding,
            value,
        }
    }
}

impl RenderNode for SliderNode {
    fn layout(&mut self, _ctx: LayoutCtx<'_>) -> LayoutResult {
        LayoutResult {
            size: Size::new(200.0, 32.0),
        }
    }

    fn paint(&mut self, ctx: &mut RenderCtx<'_>) {
        // TODO(slider): draw track/thumb and encode the binding + range.
        ctx.push(DrawCommand::Placeholder("Slider track"));
        ctx.push(DrawCommand::Placeholder("Slider thumb"));
    }
}

/// Placeholder toggle node (draws checkboxes until skins are ready).
#[derive(Debug)]

pub struct ToggleNode {
    binding: Binding<bool>,
    value: NodeSignal<bool>,
}

impl ToggleNode {
    #[must_use]
    /// Creates a toggle node from its config binding.
    pub fn new(config: ToggleConfig) -> Self {
        let binding = config.toggle;
        let value = NodeSignal::new(binding.clone().computed());
        Self { binding, value }
    }
}

impl RenderNode for ToggleNode {
    fn layout(&mut self, _ctx: LayoutCtx<'_>) -> LayoutResult {
        self.value.refresh();
        LayoutResult {
            size: Size::new(32.0, 32.0),
        }
    }

    fn paint(&mut self, ctx: &mut RenderCtx<'_>) {
        ctx.push(DrawCommand::Placeholder("Toggle body"));
    }
}

/// Placeholder node for numeric steppers.
#[derive(Debug)]

pub struct StepperNode {
    binding: Binding<i32>,
    step: NodeSignal<i32>,
    range: (i32, i32),
}

impl StepperNode {
    #[must_use]
    /// Creates a stepper node from the provided configuration.
    pub fn new(config: StepperConfig) -> Self {
        let binding = config.value.clone();
        let step = NodeSignal::new(config.step);
        let range = (*config.range.start(), *config.range.end());
        Self {
            binding,
            step,
            range,
        }
    }
}

impl RenderNode for StepperNode {
    fn layout(&mut self, _ctx: LayoutCtx<'_>) -> LayoutResult {
        self.step.refresh();
        LayoutResult {
            size: Size::new(160.0, 32.0),
        }
    }

    fn paint(&mut self, ctx: &mut RenderCtx<'_>) {
        ctx.push(DrawCommand::Placeholder("Stepper control"));
    }
}

/// Placeholder node for text fields.
#[derive(Debug)]

pub struct TextFieldNode {
    binding: Binding<Str>,
    value: NodeSignal<Str>,
}

impl TextFieldNode {
    #[must_use]
    /// Creates a text field node from the provided configuration.
    pub fn new(config: TextFieldConfig) -> Self {
        let binding = config.value;
        let value = NodeSignal::new(binding.clone().computed());
        Self { binding, value }
    }
}

impl RenderNode for TextFieldNode {
    fn layout(&mut self, _ctx: LayoutCtx<'_>) -> LayoutResult {
        self.value.refresh();
        LayoutResult {
            size: Size::new(220.0, 32.0),
        }
    }

    fn paint(&mut self, ctx: &mut RenderCtx<'_>) {
        ctx.push(DrawCommand::Placeholder("Text field"));
    }
}
