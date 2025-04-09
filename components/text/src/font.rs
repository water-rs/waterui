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

pub(crate) mod ffi {
    use waterui_core::ffi::{WuiColor, ffi_struct};

    use super::Font;

    #[repr(C)]
    pub struct WuiFont {
        pub size: f64,
        pub italic: bool,
        pub strikethrough: WuiColor,
        pub underlined: WuiColor,
        pub bold: bool,
    }

    ffi_struct!(Font, WuiFont, size, italic, strikethrough, underlined, bold);
}
