use nami::{Binding, Computed, signal::IntoComputed};
use waterui_core::{AnyView, View};
use waterui_layout::{Point, Rect, Size};
use waterui_str::Str;

/// Represents a window in the UI.
#[derive(Debug)]
pub struct Window {
    /// The title of the window.
    ///
    /// Notice that it may not be displayed on all platforms.
    pub title: Computed<Str>,
    /// Whether the window is closable.
    ///
    /// Notice that it may not be supported on all platforms.
    pub closable: bool,
    /// The frame of the window.
    ///
    /// Notice that it may not be supported on all platforms.
    pub frame: Binding<Rect>,
    /// The content of the window.
    pub content: AnyView,
}

impl Window {
    /// Create a new window with the specified title and content.
    #[must_use]
    pub fn new(title: impl IntoComputed<Str>, content: impl View) -> Self {
        let default_frame = Rect::new(Point::zero(), Size::new(800.0, 600.0));
        Self {
            title: title.into_computed(),
            closable: true,
            frame: Binding::container(default_frame),
            content: AnyView::new(content),
        }
    }
}
