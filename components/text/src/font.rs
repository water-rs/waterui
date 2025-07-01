use waterui_core::Color;

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Font {
    pub size: f64,
    pub italic: bool,
    pub strikethrough: Option<Color>,
    pub underlined: Option<Color>,
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
