use crate::array::WuiArray;
use crate::color::WuiColor;
use crate::reactive::WuiComputed;
use crate::{IntoFFI, IntoRust, WuiEnv, WuiStr, ffi_computed, ffi_computed_ctor, ffi_reactive};
use alloc::vec::Vec;
use waterui::view::ConfigurableView;
use waterui_text::font::{Font, FontWeight, ResolvedFont};
use waterui_text::styled::{Style, StyledStr};
use waterui_text::{Text, TextConfig};

/// FFI representation of a resolved font.
#[repr(C)]
pub struct WuiResolvedFont {
    /// Font size in points.
    pub size: f32,
    /// Font weight.
    pub weight: WuiFontWeight,
    /// Font family name (empty string means system default).
    pub family: WuiStr,
}

impl IntoFFI for ResolvedFont {
    type FFI = WuiResolvedFont;
    fn into_ffi(self) -> Self::FFI {
        WuiResolvedFont {
            size: self.size,
            weight: self.weight.into_ffi(),
            family: self
                .family
                .map_or_else(|| waterui::Str::from("").into_ffi(), IntoFFI::into_ffi),
        }
    }
}

impl IntoRust for WuiResolvedFont {
    type Rust = ResolvedFont;
    unsafe fn into_rust(self) -> Self::Rust {
        let weight = unsafe { self.weight.into_rust() };
        let family_str: waterui::Str = unsafe { self.family.into_rust() };
        if family_str.is_empty() {
            ResolvedFont::new(self.size, weight)
        } else {
            ResolvedFont::with_family(self.size, weight, family_str)
        }
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
ffi_view!(TextConfig, WuiText, text);

ffi_computed!(ResolvedFont, WuiResolvedFont);
ffi_computed_ctor!(ResolvedFont, WuiResolvedFont);

/// Creates a new WuiResolvedFont with a properly initialized empty family string.
///
/// This function is needed for native code (Android JNI) to create WuiResolvedFont
/// structs with valid vtables for the family field.
#[unsafe(no_mangle)]
pub extern "C" fn waterui_resolved_font_new(size: f32, weight: WuiFontWeight) -> WuiResolvedFont {
    WuiResolvedFont {
        size,
        weight,
        family: waterui::Str::from("").into_ffi(),
    }
}

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
