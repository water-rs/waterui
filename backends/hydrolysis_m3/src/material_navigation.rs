//! Material Design 3 top app bar navigation API.

use crate::semantics::label_plain_text;
use waterui::View;
use waterui::component::IntoLabel;
use waterui::navigation::{NavigationTitleDisplayMode, NavigationView};
use waterui::{Environment, Str, ViewExt as _};

/// A Material Design 3 navigation view with top app bar chrome.
#[derive(Debug)]
pub struct MaterialNavigationView {
    view: NavigationView,
    accessibility_label: Str,
}

/// Creates a Material Design 3 navigation view.
pub fn material_navigation_view(
    title: impl IntoLabel,
    content: impl View,
) -> MaterialNavigationView {
    let title = title.into_label();
    let accessibility_label = label_plain_text(&title);
    MaterialNavigationView {
        view: NavigationView::new(title.semantic_text().clone(), content),
        accessibility_label,
    }
}

/// Uses the small top app bar presentation.
#[must_use]
pub fn material_small_top_app_bar(mut view: MaterialNavigationView) -> MaterialNavigationView {
    view.view = view
        .view
        .navigation_title_display_mode(NavigationTitleDisplayMode::Inline);
    view
}

/// Uses the large top app bar presentation.
#[must_use]
pub fn material_large_top_app_bar(mut view: MaterialNavigationView) -> MaterialNavigationView {
    view.view = view
        .view
        .navigation_title_display_mode(NavigationTitleDisplayMode::Large);
    view
}

impl View for MaterialNavigationView {
    fn body(self, _env: &Environment) -> impl View {
        self.view.a11y_label(self.accessibility_label)
    }
}

#[cfg(test)]
mod tests {
    use crate::navigation_chrome::metrics;

    #[test]
    fn material_top_app_bar_tokens_match_compose_app_bar_tokens() {
        let metrics = metrics();

        assert_eq!(metrics.inline_bar_height, 64.0);
        assert_eq!(metrics.automatic_bar_height, 64.0);
        assert_eq!(metrics.large_bar_height, 152.0);
        assert_eq!(metrics.title_leading_inset, 16.0);
        assert_eq!(metrics.title_trailing_inset, 16.0);
        assert_eq!(metrics.horizontal_inset, 4.0);
    }
}
