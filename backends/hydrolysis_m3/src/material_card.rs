//! Material Design 3 card components composed from `WaterUI` card primitives.

use core::fmt::{self, Debug};

use waterui::accessibility::AccessibilityRole;
use waterui::widget::{Card as WaterCard, CardStyle};
use waterui::{AnyView, Environment, Str, View, ViewExt as _};

/// A Material Design 3 card container.
#[derive(Clone)]
pub struct MaterialCard<Content> {
    content: Content,
    style: CardStyle,
    accessibility_label: Option<Str>,
}

impl<Content> Debug for MaterialCard<Content> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MaterialCard")
            .field("style", &self.style)
            .field("accessibility_label", &self.accessibility_label)
            .finish_non_exhaustive()
    }
}

impl<Content> MaterialCard<Content> {
    /// Creates a filled Material card.
    #[must_use]
    pub const fn new(content: Content) -> Self {
        Self {
            content,
            style: CardStyle::Filled,
            accessibility_label: None,
        }
    }

    /// Uses the elevated Material card style.
    #[must_use]
    pub const fn elevated(mut self) -> Self {
        self.style = CardStyle::Elevated;
        self
    }

    /// Uses the filled Material card style.
    #[must_use]
    pub const fn filled(mut self) -> Self {
        self.style = CardStyle::Filled;
        self
    }

    /// Uses the outlined Material card style.
    #[must_use]
    pub const fn outlined(mut self) -> Self {
        self.style = CardStyle::Outlined;
        self
    }

    /// Sets the card accessibility label.
    #[must_use]
    pub fn label(mut self, label: impl Into<Str>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }
}

impl<Content> View for MaterialCard<Content>
where
    Content: View,
{
    fn body(self, _env: &Environment) -> impl View {
        let card = WaterCard::new(self.content)
            .style(self.style)
            .a11y_role(AccessibilityRole::Group);

        match self.accessibility_label {
            Some(label) => AnyView::new(card.a11y_label(label)),
            None => AnyView::new(card),
        }
    }
}

/// Creates a filled Material Design 3 card.
#[must_use]
pub const fn material_card<Content>(content: Content) -> MaterialCard<Content> {
    MaterialCard::new(content)
}

#[cfg(test)]
mod tests {
    use crate::MaterialTheme;
    use waterui::widget::CardStyle;

    #[test]
    fn material_card_tokens_match_material_web_v0_192_cards() {
        let theme = crate::layout::card::theme(&MaterialTheme::new().colors());

        assert_eq!(theme.default_style, CardStyle::Filled);
        assert_eq!(theme.elevated.corner_radius, 12.0);
        assert_eq!(theme.filled.corner_radius, 12.0);
        assert_eq!(theme.outlined.corner_radius, 12.0);
        assert_eq!(theme.outlined.outline_width, 1.0);
        assert_eq!(theme.elevated.shadow_radius, 1.0);
        assert_eq!(theme.filled.shadow_radius, 0.0);
        assert_eq!(theme.outlined.shadow_radius, 0.0);
    }
}
