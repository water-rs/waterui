//! `SubView` implementation using GTK widget measurement.

use gtk4::Widget;
use gtk4::prelude::*;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView};

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
    fn size_that_fits(&self, proposal: ProposalSize) -> Size {
        // Use GTK's measurement API
        // -1 means "no constraint" in GTK's measure()

        let for_height = proposal.height.map(|h| h as i32).unwrap_or(-1);

        let for_width = proposal.width.map(|w| w as i32).unwrap_or(-1);

        // Measure horizontal (width)
        let (_min_width, natural_width, _min_baseline, _nat_baseline) = self
            .widget
            .measure(gtk4::Orientation::Horizontal, for_height);

        // Measure vertical (height)
        let (_min_height, natural_height, _min_baseline2, _nat_baseline2) =
            self.widget.measure(gtk4::Orientation::Vertical, for_width);

        // Clamp to proposal if provided
        let width = match proposal.width {
            Some(proposed) => proposed.min(natural_width as f32),
            None => natural_width as f32,
        };

        let height = match proposal.height {
            Some(proposed) => proposed.min(natural_height as f32),
            None => natural_height as f32,
        };

        Size { width, height }
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
