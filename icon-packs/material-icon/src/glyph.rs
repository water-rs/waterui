//! Icon glyph support for webfont rendering.
//!
//! This module provides [`IconGlyph`] for rendering Material Icons
//! using the Material Icons font instead of SVG.

use waterui_core::{Environment, View};
use waterui_text::font::{Font, FontWeight, ResolvedFont};
use waterui_text::styled::StyledStr;

/// A const-constructible icon glyph that renders as styled text.
///
/// This type uses the Material Icons font to render icons as text glyphs.
/// It implements [`View`] so it can be used directly in view hierarchies.
///
/// # Usage
///
/// ```ignore
/// use waterui_icons_material_icon::IconGlyph;
///
/// // Create a custom icon glyph
/// const MY_ICON: IconGlyph = IconGlyph::new('\u{e88a}', "MaterialIcons-Regular", 24.0);
///
/// // Use with custom size
/// MY_ICON.with_size(32.0)
/// ```
#[derive(Debug, Clone, Copy)]
pub struct IconGlyph {
    /// The Unicode codepoint for this icon.
    pub codepoint: char,
    /// The font family name (e.g., "MaterialIcons-Regular").
    pub font_family: &'static str,
    /// The font size in points.
    pub size: f32,
}

impl IconGlyph {
    /// Creates a new icon glyph.
    #[must_use]
    pub const fn new(codepoint: char, font_family: &'static str, size: f32) -> Self {
        Self {
            codepoint,
            font_family,
            size,
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
        let mut buf = [0u8; 4];
        let text = self.codepoint.encode_utf8(&mut buf);
        StyledStr::plain(text).font(&Font::new(ResolvedFont::with_static_family(
            self.size,
            FontWeight::Normal,
            self.font_family,
        )))
    }
}
