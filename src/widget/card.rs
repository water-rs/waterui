//! A card is a styled container that groups related content.
use crate::prelude::*;
use waterui_layout::stack::vstack;
use waterui_text::font::Title;

/// A card widget that displays optional title, subtitle, and content.
#[derive(Debug, Clone)]
pub struct Card<Content> {
    title: Option<Text>,
    subtitle: Option<Text>,
    content: Content,
}

impl<Content> Card<Content> {
    // Creates a new card with the specified content.

    /// # Arguments
    /// * `content` - The main content of the card.
    pub const fn new(content: Content) -> Self {
        Self {
            title: None,
            subtitle: None,
            content,
        }
    }
    /// Sets the title of the card.
    #[must_use]
    pub fn title<T: Into<Text>>(mut self, title: T) -> Self {
        self.title = Some(title.into().font(Title));
        self
    }

    /// Sets the subtitle of the card.
    #[must_use]
    pub fn subtitle<T: Into<Text>>(mut self, subtitle: T) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }
}

impl<Content> View for Card<Content>
where
    Content: View,
{
    fn body(self, _env: &Environment) -> impl View {
        vstack((self.title, self.subtitle, self.content))
    }
}

/// Creates a new card widget with the specified content.
///
/// # Arguments
/// * `content` - The main content of the card.
pub const fn card<Content>(content: Content) -> Card<Content> {
    Card::new(content)
}
