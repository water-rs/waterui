use waterui_core::{AnyView, View};
#[derive(Debug, uniffi::Record)]
#[must_use]
pub struct Overlay {
    pub content: AnyView,
}

pub fn overlay(content: impl View) -> Overlay {
    Overlay {
        content: AnyView::new(content),
    }
}
