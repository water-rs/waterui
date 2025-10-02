use waterui_core::Color;

/// Font configuration for text rendering.
///
/// This struct defines all the visual properties that can be applied to text,
/// including size, styling, and decorations.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Font {
    /// The font size in points.
    pub size: f64,
    /// Whether the text should be rendered in italic style.
    pub italic: bool,
    /// Optional strikethrough decoration with customizable color.
    pub strikethrough: Option<Color>,
    /// Optional underline decoration with customizable color.
    pub underlined: Option<Color>,
    /// Whether the text should be rendered in bold weight.
    pub bold: bool,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            size: f64::NAN,
            italic: false,
            bold: false,
            strikethrough: None,
            underlined: None,
        }
    }
}
