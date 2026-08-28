//! Material Design 3 badge components composed from `WaterUI` badge primitives.

use core::fmt::{self, Debug};

use waterui::component::badge::Badge as WaterBadge;
use waterui::reactive::signal::IntoComputed;
use waterui::{AnyView, Environment, Str, View, ViewExt as _};
use waterui_core::Computed;

/// A Material Design 3 badge attached to another view.
#[derive(Clone)]
pub struct MaterialBadge<Content> {
    value: Computed<i32>,
    content: Content,
    accessibility_label: Option<Str>,
}

impl<Content> Debug for MaterialBadge<Content> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MaterialBadge")
            .field("accessibility_label", &self.accessibility_label)
            .finish_non_exhaustive()
    }
}

impl<Content> MaterialBadge<Content> {
    /// Creates a Material badge. A value of `0` renders the small dot variant.
    #[must_use]
    pub fn new(value: impl IntoComputed<i32>, content: Content) -> Self {
        Self {
            value: value.into_computed(),
            content,
            accessibility_label: None,
        }
    }

    /// Sets the combined accessibility label for the badged content.
    #[must_use]
    pub fn label(mut self, label: impl Into<Str>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }
}

impl<Content> View for MaterialBadge<Content>
where
    Content: Clone + View,
{
    fn body(self, _env: &Environment) -> impl View {
        let badge = WaterBadge::new(self.value, self.content);
        match self.accessibility_label {
            Some(label) => AnyView::new(badge.a11y_label(label)),
            None => AnyView::new(badge),
        }
    }
}

/// Creates a Material Design 3 badge.
#[must_use]
pub fn material_badge<Content>(
    value: impl IntoComputed<i32>,
    content: Content,
) -> MaterialBadge<Content> {
    MaterialBadge::new(value, content)
}

#[cfg(test)]
mod tests {
    use crate::layout::badge::metrics;

    #[test]
    fn material_badge_tokens_match_compose_badge_tokens() {
        let metrics = metrics();

        assert_eq!(metrics.small_size, 6.0);
        assert_eq!(metrics.large_size, 16.0);
        assert_eq!(metrics.large_horizontal_padding, 4.0);
    }
}
