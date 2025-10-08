use crate::array::WuiArray;
use crate::color::WuiColor;
use crate::{IntoFFI, WuiEnv, WuiStr, ffi_struct, ffi_view, impl_computed};
use alloc::vec::Vec;
use waterui::component::Native;
use waterui::{Computed, view::ConfigurableView};
use waterui_text::font::{Font, FontWeight, ResolvedFont};
use waterui_text::styled::{Style, StyledStr};
use waterui_text::{Text, TextConfig};

/// C representation of Font
#[repr(C)]
pub struct WuiResolvedFont {
    size: f32,
    weight: WuiFontWeight,
}

ffi_struct!(ResolvedFont, WuiResolvedFont, size, weight);

ffi_type!(WuiFont, Font, waterui_drop_font);

ffi_enum!(
    FontWeight,
    WuiFontWeight,
    Thin,
    UltraLight,
    Light,
    Normal,
    Medium,
    SemiBold,
    Bold,
    UltraBold,
    Black
);

#[repr(C)]
pub struct WuiTextStyle {
    pub font: *mut WuiFont,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub foreground: *mut WuiColor,
    pub background: *mut WuiColor,
}

ffi_struct!(
    Style,
    WuiTextStyle,
    font,
    italic,
    underline,
    strikethrough,
    foreground,
    background
);

#[repr(C)]
pub struct WuiStyledChunk {
    pub text: WuiStr,
    pub style: WuiTextStyle,
}

#[repr(C)]
pub struct WuiStyledStr {
    pub chunks: WuiArray<WuiStyledChunk>,
}

ffi_safe!(WuiStyledChunk);

impl IntoFFI for StyledStr {
    type FFI = WuiStyledStr;
    fn into_ffi(self) -> Self::FFI {
        WuiStyledStr {
            chunks: self
                .into_chunks()
                .into_iter()
                .map(|(text, style)| WuiStyledChunk {
                    text: text.into_ffi(),
                    style: style.into_ffi(),
                })
                .collect::<Vec<WuiStyledChunk>>()
                .into_ffi(),
        }
    }
}

impl_computed!(
    StyledStr,
    WuiStyledStr,
    waterui_read_computed_styled_str,
    waterui_watch_computed_styled_str,
    waterui_drop_computed_styled_str
);

/// C representation of Text configuration
#[repr(C)]
pub struct WuiText {
    /// Pointer to the text content computed value
    pub content: *mut Computed<StyledStr>,
}

impl_computed!(
    Font,
    *mut WuiFont,
    waterui_read_computed_font,
    waterui_watch_computed_font,
    waterui_drop_computed_font
);

impl IntoFFI for Text {
    type FFI = WuiText;
    fn into_ffi(self) -> Self::FFI {
        self.config().into_ffi()
    }
}

ffi_struct!(TextConfig, WuiText, content);

// FFI view bindings for text components
ffi_view!(
    Native<TextConfig>,
    WuiText,
    waterui_text_id,
    waterui_force_as_text
);

impl_computed!(
    ResolvedFont,
    WuiResolvedFont,
    waterui_read_computed_resolved_font,
    waterui_watch_computed_resolved_font,
    waterui_drop_computed_resolved_font
);

#[unsafe(no_mangle)]
unsafe extern "C" fn waterui_resolve_font(
    font: *const WuiFont,
    env: *const WuiEnv,
) -> *mut Computed<ResolvedFont> {
    let font = unsafe { &*font };
    let env = unsafe { &*env };
    let resolved = font.resolve(env);
    resolved.into_ffi()
}
