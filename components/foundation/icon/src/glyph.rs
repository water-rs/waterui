//! Icon glyph support for webfont rendering.
//!
//! This module provides [`IconGlyph`] for rendering icons using webfonts
//! (icon fonts) instead of SVG. Icons are rendered as styled text with
//! a custom font family.

use waterui_core::{Computed, Environment, Str, View};
use waterui_text::font::Font;
use waterui_text::styled::StyledStr;

use crate::IconSize;

fn codepoint_to_string(codepoint: char) -> Str {
    let mut text = alloc::string::String::with_capacity(codepoint.len_utf8());
    text.push(codepoint);
    Str::from(text)
}

/// A const-constructible icon glyph that renders as styled text with an icon font.
///
/// This type uses webfonts (icon fonts) to render icons as text glyphs.
/// Each icon is a Unicode codepoint that maps to a glyph in the icon font.
///
/// Colour follows the environment foreground (`.foreground(...)`), and size
/// follows [`IconSize`] when installed in the environment (otherwise
/// [`Self::font_size()`], defaulting to `24.0`).
#[derive(Debug, Clone, Copy)]
#[must_use = "an `IconGlyph` does nothing unless it is rendered as part of a view"]
#[non_exhaustive]
pub struct IconGlyph {
    /// The Unicode codepoint for this icon in the font.
    codepoint: char,
    /// The font family name (e.g., "MaterialIcons-Regular", "FontAwesome7Free-Solid").
    font_family: &'static str,
    /// The fallback font size in points when no [`IconSize`] is installed. Default is 24.0.
    size: f32,
}

impl IconGlyph {
    /// The Unicode codepoint for this icon in the font.
    #[must_use]
    pub const fn codepoint(&self) -> char {
        self.codepoint
    }

    /// The font family name (e.g., "MaterialIcons-Regular", "FontAwesome7Free-Solid").
    #[must_use]
    pub const fn font_family(&self) -> &'static str {
        self.font_family
    }

    /// The fallback font size in points. Default is 24.0.
    #[must_use]
    pub const fn font_size(&self) -> f32 {
        self.size
    }

    /// Creates a new icon glyph with the default size (24.0).
    pub const fn new(codepoint: char, font_family: &'static str) -> Self {
        Self {
            codepoint,
            font_family,
            size: crate::DEFAULT_ICON_SIZE,
        }
    }

    /// Returns a new glyph with the specified fallback font size.
    pub const fn with_font_size(self, size: f32) -> Self {
        Self { size, ..self }
    }
}

impl View for IconGlyph {
    fn body(self, env: &Environment) -> impl View {
        let size = env.get::<IconSize>().map_or_else(
            || Computed::constant(self.size),
            |icon_size| icon_size.size().clone(),
        );
        let font = Font::default().font_size(size).family(self.font_family);
        let glyph = StyledStr::plain(codepoint_to_string(self.codepoint)).font(font);
        crate::icon::wrap_icon_accessibility(glyph, env)
    }
}
