use crate::array::WuiArray;
use crate::color::WuiColor;
use crate::reactive::WuiComputed;
use crate::{IntoFFI, WuiEnv, WuiStr, ffi_computed, ffi_reactive};
use alloc::vec::Vec;
use waterui::view::ConfigurableView;
use waterui_text::font::{Font, FontWeight, ResolvedFont};
use waterui_text::styled::{Style, StyledStr};
use waterui_text::{Text, TextConfig};

into_ffi! {
    ResolvedFont,
    pub struct WuiResolvedFont {
        size: f32,
        weight: WuiFontWeight,
    }
}

opaque!(WuiFont, Font);

into_ffi!(
    FontWeight,
    pub enum WuiFontWeight {
        Thin,
        UltraLight,
        Light,
        Normal,
        Medium,
        SemiBold,
        Bold,
        UltraBold,
        Black,
    }
);

into_ffi! {
    Style,
    pub struct WuiTextStyle {
        font: *mut WuiFont,
        italic: bool,
        underline: bool,
        strikethrough: bool,
        foreground: *mut WuiColor,
        background: *mut WuiColor,
    }
}

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

ffi_computed!(StyledStr, WuiStyledStr);

into_ffi! {
    TextConfig,
    pub struct WuiText {
        content: *mut WuiComputed<StyledStr>,
    }
}

ffi_reactive!(Font, *mut WuiFont);

impl IntoFFI for Text {
    type FFI = WuiText;
    fn into_ffi(self) -> Self::FFI {
        self.config().into_ffi()
    }
}

// FFI view bindings for text components
native_view!(Text, WuiText);

ffi_computed!(ResolvedFont, WuiResolvedFont);

#[unsafe(no_mangle)]
unsafe extern "C" fn waterui_resolve_font(
    font: *const WuiFont,
    env: *const WuiEnv,
) -> *mut WuiComputed<ResolvedFont> {
    let font = unsafe { &*font };
    let env = unsafe { &*env };
    let resolved = font.resolve(env);
    resolved.into_ffi()
}
