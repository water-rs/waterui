//! Chart tooltip component for displaying data point information on hover.

extern crate alloc;

use alloc::vec::Vec;

use waterui_core::Str;
use waterui_core::{AnyView, View};
use waterui_graphics::color::{Color, Srgb};
use waterui_layout::frame::Frame;
use waterui_layout::stack::{HStack, HorizontalAlignment, VStack, VerticalAlignment};
use waterui_shape::{RoundedRectangle, ShapeExt};
use waterui_text::text;

/// Content to display in a tooltip.
#[derive(Debug, Clone, Default)]
pub struct TooltipContent {
    /// Optional title line.
    pub title: Option<Str>,
    /// Value lines to display.
    pub values: Vec<TooltipValue>,
}

/// A single value line in a tooltip.
#[derive(Debug, Clone)]
pub struct TooltipValue {
    /// Label for the value.
    pub label: Str,
    /// The value to display.
    pub value: Str,
    /// Optional color indicator.
    pub color: Option<Srgb>,
}

impl TooltipValue {
    /// Creates a new tooltip value.
    #[must_use]
    pub fn new(label: impl Into<Str>, value: impl Into<Str>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            color: None,
        }
    }

    /// Sets the color indicator.
    #[must_use]
    pub fn color(mut self, color: impl Into<Srgb>) -> Self {
        self.color = Some(color.into());
        self
    }
}

impl TooltipContent {
    /// Creates a new empty tooltip content.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the title.
    #[must_use]
    pub fn title(mut self, title: impl Into<Str>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Adds a value line.
    #[must_use]
    pub fn value(mut self, label: impl Into<Str>, value: impl Into<Str>) -> Self {
        self.values.push(TooltipValue::new(label, value));
        self
    }

    /// Adds a value line with a color indicator.
    #[must_use]
    pub fn colored_value(
        mut self,
        label: impl Into<Str>,
        value: impl Into<Str>,
        color: impl Into<Srgb>,
    ) -> Self {
        self.values
            .push(TooltipValue::new(label, value).color(color));
        self
    }

    /// Returns true if the tooltip has content to display.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.values.is_empty()
    }
}

/// Chart tooltip view.
///
/// Displays formatted data in a styled container.
///
/// # Example
///
/// ```ignore
/// use waterui_chart::{Tooltip, TooltipContent};
///
/// Tooltip::new(
///     TooltipContent::new()
///         .title("Point Data")
///         .value("X", "10.5")
///         .value("Y", "25.3")
/// )
/// .background(Srgb::from_hex("#1F2937"))
/// ```
pub struct Tooltip {
    content: TooltipContent,
    background: Srgb,
    text_color: Srgb,
    corner_radius: f32,
    padding: f32,
}

impl Tooltip {
    /// Creates a new tooltip with the given content.
    #[must_use]
    pub fn new(content: TooltipContent) -> Self {
        Self {
            content,
            background: Srgb::from_hex("#1F2937"),
            text_color: Srgb::from_hex("#FFFFFF"),
            corner_radius: 4.0,
            padding: 8.0,
        }
    }

    /// Sets the background color.
    #[must_use]
    pub fn background(mut self, color: impl Into<Srgb>) -> Self {
        self.background = color.into();
        self
    }

    /// Sets the text color.
    #[must_use]
    pub fn text_color(mut self, color: impl Into<Srgb>) -> Self {
        self.text_color = color.into();
        self
    }

    /// Sets the corner radius.
    #[must_use]
    pub const fn corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }

    /// Sets the padding.
    #[must_use]
    pub const fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }
}

impl View for Tooltip {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        if self.content.is_empty() {
            return AnyView::new(());
        }

        let text_color = Color::from(self.text_color);
        let mut views: Vec<AnyView> = Vec::new();

        // Add title if present
        if let Some(title) = &self.content.title {
            views.push(AnyView::new(
                text((*title).clone()).color(text_color.clone()),
            ));
        }

        // Add values with optional color indicators
        for val in &self.content.values {
            let value_view = if let Some(color) = val.color {
                // Color indicator + label: value
                let indicator = Frame::new(RoundedRectangle::new(0.5).fill(Color::from(color)))
                    .width(8.0)
                    .height(8.0);
                let line =
                    text(alloc::format!("{}: {}", val.label, val.value)).color(text_color.clone());
                AnyView::new(HStack::new(
                    VerticalAlignment::Center,
                    6.0,
                    (indicator, line),
                ))
            } else {
                AnyView::new(
                    text(alloc::format!("{}: {}", val.label, val.value)).color(text_color.clone()),
                )
            };
            views.push(value_view);
        }

        // Background with rounded corners
        let content = VStack::new(HorizontalAlignment::Leading, 4.0, views);
        let background =
            RoundedRectangle::new(self.corner_radius / 100.0).fill(Color::from(self.background));

        // Stack content over background
        AnyView::new(Frame::new(waterui_layout::stack::ZStack::new(
            waterui_layout::stack::Alignment::default(),
            (background, Frame::new(content).min_width(80.0)),
        )))
    }
}
