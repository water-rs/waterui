use crate::IntoFFI;
use crate::{color::WuiColor, ffi_struct, ffi_view};
use waterui::component::Native;
use waterui::{Computed, Str, view::ConfigurableView};
use waterui_text::Text;
use waterui_text::{TextConfig, font::Font};

/// C representation of Font
#[repr(C)]
pub struct WuiFont {
    pub size: f64,
    pub italic: bool,
    pub strikethrough: *mut WuiColor,
    pub underlined: *mut WuiColor,
    pub bold: bool,
}

/// C representation of Text configuration
#[repr(C)]
pub struct WuiText {
    /// Pointer to the text content computed value
    pub content: *mut Computed<Str>,
    /// Pointer to the font computed value
    pub font: *mut Computed<Font>,
}

impl IntoFFI for Text {
    type FFI = WuiText;
    fn into_ffi(self) -> Self::FFI {
        self.config().into_ffi()
    }
}

// Implement struct conversions
ffi_struct!(Font, WuiFont, size, italic, strikethrough, underlined, bold);
ffi_struct!(TextConfig, WuiText, content, font);

// FFI view bindings for text components
ffi_view!(
    Native<TextConfig>,
    WuiText,
    waterui_text_id,
    waterui_force_as_text
);
