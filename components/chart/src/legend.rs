//! Chart legend component for displaying series labels with color markers.

extern crate alloc;

use alloc::vec::Vec;

use waterui_core::{AnyView, View};
use waterui_graphics::color::Color;
use waterui_layout::frame::Frame;
use waterui_layout::stack::{HStack, HorizontalAlignment, VStack, VerticalAlignment};
use waterui_layout::{PositionExt, UnitPoint, absolute};
use waterui_shape::{Rectangle, ShapeExt};
use waterui_text::{IntoText, Text};

/// A single item in the legend.
#[derive(Debug, Clone)]
pub struct LegendItem {
    /// Label text.
    pub label: Text,
    /// Color marker.
    pub color: Color,
}

impl LegendItem {
    /// Creates a new legend item.
    #[must_use]
    pub fn new(label: impl IntoText, color: impl Into<Color>) -> Self {
        Self {
            label: label.into_text(),
            color: color.into(),
        }
    }
}

/// Position of the legend relative to the chart.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LegendPosition {
    /// Top-right corner (default).
    #[default]
    TopRight,
    /// Top-left corner.
    TopLeft,
    /// Bottom-right corner.
    BottomRight,
    /// Bottom-left corner.
    BottomLeft,
    /// Top center.
    Top,
    /// Bottom center.
    Bottom,
}

/// Orientation of legend items.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LegendOrientation {
    /// Items stacked vertically (default).
    #[default]
    Vertical,
    /// Items arranged horizontally.
    Horizontal,
}

/// Chart legend component.
///
/// Displays color-coded labels for chart series.
///
/// # Example
///
/// ```ignore
/// use waterui_chart::{Legend, LegendItem, LegendPosition};
///
/// Legend::new(vec![
///     LegendItem::new("Sales", Srgb::from_hex("#3B82F6")),
///     LegendItem::new("Revenue", Srgb::from_hex("#EF4444")),
/// ])
/// .position(LegendPosition::TopRight)
/// .marker_size(12.0)
/// ```
#[derive(Debug)]
pub struct Legend {
    items: Vec<LegendItem>,
    position: LegendPosition,
    orientation: LegendOrientation,
    marker_size: f32,
    spacing: f32,
}

impl Legend {
    /// Creates a new legend with the given items.
    #[must_use]
    pub fn new(items: Vec<LegendItem>) -> Self {
        Self {
            items,
            position: LegendPosition::default(),
            orientation: LegendOrientation::default(),
            marker_size: 12.0,
            spacing: 8.0,
        }
    }

    /// Sets the legend position.
    #[must_use]
    pub const fn position(mut self, position: LegendPosition) -> Self {
        self.position = position;
        self
    }

    /// Sets the legend orientation.
    #[must_use]
    pub const fn orientation(mut self, orientation: LegendOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Sets the color marker size.
    #[must_use]
    pub const fn marker_size(mut self, size: f32) -> Self {
        self.marker_size = size;
        self
    }

    /// Sets the spacing between items.
    #[must_use]
    pub const fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }
}

impl View for Legend {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        let marker_size = self.marker_size;
        let spacing = self.spacing;

        // Convert items to entry views
        let entries: Vec<_> = self
            .items
            .into_iter()
            .map(|item| {
                let marker = Frame::new(Rectangle.fill(item.color))
                    .width(marker_size)
                    .height(marker_size);
                let label = item.label;
                HStack::new(VerticalAlignment::Center, 6.0, (marker, label))
            })
            .collect();

        // Render based on orientation
        let content = if self.orientation == LegendOrientation::Horizontal {
            AnyView::new(HStack::new(VerticalAlignment::Center, spacing, entries))
        } else {
            AnyView::new(VStack::new(HorizontalAlignment::Leading, spacing, entries))
        };

        // Position legend within the available bounds
        let inset: f32 = 8.0;
        let (anchor, position, offset_x, offset_y) = match self.position {
            LegendPosition::TopRight => (
                UnitPoint::TOP_TRAILING,
                UnitPoint::TOP_TRAILING,
                -inset,
                inset,
            ),
            LegendPosition::TopLeft => {
                (UnitPoint::TOP_LEADING, UnitPoint::TOP_LEADING, inset, inset)
            }
            LegendPosition::BottomRight => (
                UnitPoint::BOTTOM_TRAILING,
                UnitPoint::BOTTOM_TRAILING,
                -inset,
                -inset,
            ),
            LegendPosition::BottomLeft => (
                UnitPoint::BOTTOM_LEADING,
                UnitPoint::BOTTOM_LEADING,
                inset,
                -inset,
            ),
            LegendPosition::Top => (UnitPoint::TOP, UnitPoint::TOP, 0.0, inset),
            LegendPosition::Bottom => (UnitPoint::BOTTOM, UnitPoint::BOTTOM, 0.0, -inset),
        };

        absolute((content.position_in_offset(anchor, position, offset_x, offset_y),))
    }
}
