use waterui_core::{AnyView, View};
#[derive(Debug)]
#[must_use]
/// Represents an overlay that can be displayed on top of other content.
pub struct Overlay {
    /// The content to display in the overlay.
    pub content: AnyView,
}

/// Creates a new overlay with the specified content.
pub fn overlay(content: impl View) -> Overlay {
    Overlay {
        content: AnyView::new(content),
    }
}
