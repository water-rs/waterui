//! Icon glyph support for webfont rendering.
//!
//! This module provides [`IconGlyph`] for rendering icons using webfonts
//! (icon fonts) instead of SVG. Icons are rendered as styled text with
//! a custom font family.
//!
//! # Usage
//!
//! ```ignore
//! use waterui_icon::IconGlyph;
//!
//! // Material Icons home icon
//! const HOME: IconGlyph = IconGlyph::new('\u{e88a}', "MaterialIcons-Regular");
//!
//! // Font Awesome house icon (solid)
//! const HOUSE: IconGlyph = IconGlyph::new('\u{f015}', "FontAwesome7Free-Solid");
//!
//! // With custom size
//! HOME.with_size(32.0)
//! ```

use waterui_core::{Environment, View};
use waterui_text::font::Font;
use waterui_text::styled::StyledStr;

/// A const-constructible icon glyph that renders as styled text with an icon font.
///
/// This type uses webfonts (icon fonts) to render icons as text glyphs.
/// Each icon is a Unicode codepoint that maps to a glyph in the icon font.
///
/// # Font Requirements
///
/// The icon font must be bundled with the app:
/// - **iOS/macOS**: Add font file to bundle and declare in Info.plist
/// - **Android**: Add font file to assets/fonts/
///
/// # Example
///
/// ```ignore
/// use waterui_icon::IconGlyph;
///
/// // Define icon constants
/// const SETTINGS: IconGlyph = IconGlyph::new('\u{e8b8}', "MaterialIcons-Regular");
/// const GEAR: IconGlyph = IconGlyph::new('\u{f013}', "FontAwesome7Free-Solid");
///
/// // Use in views
/// fn settings_view() -> impl View {
///     SETTINGS.with_size(24.0)
/// }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct IconGlyph {
    /// The Unicode codepoint for this icon in the font.
    pub codepoint: char,
    /// The font family name (e.g., "MaterialIcons-Regular", "FontAwesome7Free-Solid").
    pub font_family: &'static str,
    /// The font size in points. Default is 24.0.
    pub size: f32,
}

impl IconGlyph {
    /// Creates a new icon glyph with the default size (24.0).
    ///
    /// # Arguments
    ///
    /// * `codepoint` - The Unicode codepoint for the icon
    /// * `font_family` - The font family name
    #[must_use]
    pub const fn new(codepoint: char, font_family: &'static str) -> Self {
        Self {
            codepoint,
            font_family,
            size: 24.0,
        }
    }

    /// Returns a new glyph with the specified size.
    #[must_use]
    pub const fn with_size(self, size: f32) -> Self {
        Self { size, ..self }
    }
}

impl View for IconGlyph {
    fn body(self, _env: &Environment) -> impl View {
        // Encode the codepoint to UTF-8 and convert to owned String
        let text = alloc::string::String::from(self.codepoint);
        StyledStr::plain(text).font(
            Font::default()
                .size(self.size)
                .family(self.font_family),
        )
    }
}
