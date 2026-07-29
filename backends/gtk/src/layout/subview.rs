//! `SubView` implementation using GTK widget measurement.

use gtk4::Widget;
use gtk4::prelude::*;
use waterui_core::layout::{
    ProposalSize, Size, StretchAxis, SubView, VerticalAlignment, ViewDimensions,
};

use crate::components::fixed_container_widget::WuiFixedContainer;

fn layout_debug_enabled() -> bool {
    std::env::var_os("WATERUI_GTK_LAYOUT_DEBUG").is_some()
}

/// A wrapper around a GTK widget that implements the `SubView` trait.
///
/// This allows `waterui-layout` algorithms to measure GTK widgets
/// without knowing about GTK internals.
#[derive(Debug)]
pub struct GtkSubView {
    widget: Widget,
    stretch_axis: StretchAxis,
    priority: i32,
}

impl GtkSubView {
    /// Creates a new `GtkSubView` wrapping the given widget.
    #[must_use]
    pub fn new(widget: Widget, stretch_axis: StretchAxis) -> Self {
        Self {
            widget,
            stretch_axis,
            priority: 0,
        }
    }

    /// Creates a new `GtkSubView` with custom priority.
    #[must_use]
    pub fn with_priority(widget: Widget, stretch_axis: StretchAxis, priority: i32) -> Self {
        Self {
            widget,
            stretch_axis,
            priority,
        }
    }

    /// Returns a reference to the underlying GTK widget.
    #[must_use]
    pub const fn widget(&self) -> &Widget {
        &self.widget
    }
}

impl SubView for GtkSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        if let Some(container) = self.widget.downcast_ref::<WuiFixedContainer>() {
            let margin_h = (self.widget.margin_start() + self.widget.margin_end()) as f32;
            let margin_v = (self.widget.margin_top() + self.widget.margin_bottom()) as f32;
            let inner_proposal = ProposalSize::new(
                proposal.width.map(|w| (w - margin_h).max(0.0)),
                proposal.height.map(|h| (h - margin_v).max(0.0)),
            );
            let inner_dimensions = container.layout_measure(inner_proposal);
            let size = Size::new(
                inner_dimensions.size.width + margin_h,
                inner_dimensions.size.height + margin_v,
            );
            if layout_debug_enabled() {
                tracing::debug!(
                    target: "waterui::gtk::layout",
                    widget_type = %self.widget.type_().name(),
                    proposal_width = ?proposal.width,
                    proposal_height = ?proposal.height,
                    inner_width = inner_dimensions.size.width,
                    inner_height = inner_dimensions.size.height,
                    margin_horizontal = margin_h,
                    margin_vertical = margin_v,
                    width = size.width,
                    height = size.height,
                    stretch_axis = ?self.stretch_axis,
                    "Measured GTK container subview"
                );
            }
            let mut dimensions = ViewDimensions::new(size);
            for (alignment, value) in inner_dimensions.explicit_horizontal_guides() {
                dimensions.set_horizontal(alignment, value + self.widget.margin_start() as f32);
            }
            for (alignment, value) in inner_dimensions.explicit_vertical_guides() {
                dimensions.set_vertical(alignment, value + self.widget.margin_top() as f32);
            }
            return dimensions;
        }

        // Use GTK's measurement API
        // -1 means "no constraint" in GTK's measure()

        let for_height = proposal.height.map(|h| h as i32).unwrap_or(-1);

        let for_width = proposal.width.map(|w| w as i32).unwrap_or(-1);

        // Measure horizontal (width)
        let (_min_width, natural_width, _min_baseline, _nat_baseline) = self
            .widget
            .measure(gtk4::Orientation::Horizontal, for_height);

        // Measure vertical (height)
        let (_min_height, natural_height, min_baseline, nat_baseline) =
            self.widget.measure(gtk4::Orientation::Vertical, for_width);

        // Default behavior: intrinsic size clamped by proposal.
        let mut width = match proposal.width {
            Some(proposed) => proposed.min(natural_width as f32),
            None => natural_width as f32,
        };

        let mut height = match proposal.height {
            Some(proposed) => proposed.min(natural_height as f32),
            None => natural_height as f32,
        };

        // For stretch axes, fill the proposed extent instead of shrinking to
        // intrinsic size. This prevents feedback loops where a transient narrow
        // allocation becomes the next intrinsic width (e.g. GtkGLArea -> 18px lock-in).
        if self.stretch_axis.stretches_horizontal()
            && let Some(proposed) = proposal.width
        {
            width = proposed.max(0.0);
        }
        if self.stretch_axis.stretches_vertical()
            && let Some(proposed) = proposal.height
        {
            height = proposed.max(0.0);
        }

        // Some stretch-based native views (e.g. GpuSurface/Spacer) have no intrinsic
        // size and GTK reports 0x0. Respect the proposal in that case.
        if width <= 0.0
            && self.stretch_axis.stretches_horizontal()
            && let Some(proposed) = proposal.width
        {
            width = proposed.max(0.0);
        }
        if height <= 0.0
            && self.stretch_axis.stretches_vertical()
            && let Some(proposed) = proposal.height
        {
            height = proposed.max(0.0);
        }
        if layout_debug_enabled() {
            tracing::debug!(
                target: "waterui::gtk::layout",
                widget_type = %self.widget.type_().name(),
                proposal_width = ?proposal.width,
                proposal_height = ?proposal.height,
                for_width,
                for_height,
                natural_width,
                natural_height,
                width,
                height,
                stretch_axis = ?self.stretch_axis,
                "Measured GTK subview"
            );
        }

        let mut dimensions = ViewDimensions::new(Size { width, height });
        if min_baseline >= 0 {
            dimensions.set_vertical(VerticalAlignment::FirstBaseline, min_baseline as f32);
        }
        if nat_baseline >= 0 {
            dimensions.set_vertical(VerticalAlignment::LastBaseline, nat_baseline as f32);
        }
        dimensions
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch_axis
    }

    fn priority(&self) -> i32 {
        self.priority
    }
}

/// Helper to determine the `StretchAxis` for common GTK widgets.
#[must_use]
pub fn stretch_axis_for_widget(widget: &Widget) -> StretchAxis {
    // Check widget type and return appropriate stretch behavior
    if widget.is::<gtk4::Label>() || widget.is::<gtk4::Button>() {
        StretchAxis::None // Content-sized
    } else if widget.is::<gtk4::Entry>()
        || widget.is::<gtk4::Scale>()
        || widget.is::<gtk4::ProgressBar>()
    {
        StretchAxis::Horizontal // Expands width
    } else if widget.is::<gtk4::ScrolledWindow>() {
        StretchAxis::Both // Greedy
    } else {
        StretchAxis::None // Default to content-sized
    }
}
