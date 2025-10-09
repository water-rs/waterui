//! A card is a styled container that groups related content.
use crate::{
    background::Background,
    component::style::{Shadow, Vector},
    prelude::*,
    view::ViewExt,
};
use waterui_color::Color;

/// A card is a styled container that groups related content.
///
/// It typically has a distinct background, rounded corners, and a shadow
/// to appear elevated from the surface behind it.
#[derive(Debug)]
pub struct Card<Content: View> {
    content: Content,
}

impl<Content: View> Card<Content> {
    /// Creates a new card with the given content.
    pub const fn new(content: Content) -> Self {
        Self { content }
    }
}

impl<Content: View> View for Card<Content> {
    fn body(self, _env: &Environment) -> impl View {
        // Compose existing view extensions to create the card style.
        // A pure widget is built by composing primitives and other widgets.
        self.content
            .padding_with(16)
            .background(Background::color(Color::srgb(255, 255, 255)))
            .metadata(Shadow {
                // TODO: Find out how to create a color with alpha.
                // Using a solid grey for now to allow compilation.
                color: Color::srgb(200, 200, 200),
                offset: Vector { x: 0.0, y: 2.0 },
                radius: 4.0,
            })
        // TODO: Add support for corner radius (border-radius).
        // This might require a new primitive metadata type.
    }
}

/// Convenience function to create a new `Card`.
pub const fn card<Content: View>(content: Content) -> Card<Content> {
    Card::new(content)
}
