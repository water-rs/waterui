use crate::array::WuiArray;
use crate::color::WuiColor;
use crate::reactive::WuiComputed;
use crate::{IntoFFI, IntoRust, WuiEnv, WuiStr, ffi_computed, ffi_computed_ctor};
use alloc::vec::Vec;
use waterui::Str;
use waterui::layout::HorizontalAlignment;
pub use waterui_text::font::ResolvedFont;
use waterui_text::font::{Body, Font, FontWeight};
use waterui_text::styled::{Style, StyledStr};
use waterui_text::{Text, TextConfig};

/// FFI representation of a resolved font.
#[repr(C)]
#[derive(Debug)]
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
        // SAFETY: the caller contract makes `weight` an owning handle from the
        // matching FFI constructor; it is consumed here and not observed
        // again.
        let weight = unsafe { self.weight.into_rust() };
        // SAFETY: the caller contract makes `family` an owning handle from the
        // matching FFI constructor; it is consumed here and not observed
        // again.
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

/// FFI representation of a single run of text sharing one `WuiTextStyle`.
#[repr(C)]
#[derive(Debug)]
pub struct WuiStyledChunk {
    /// The text content of this run.
    pub text: WuiStr,
    /// The style applied to this run.
    pub style: WuiTextStyle,
}

/// FFI representation of a string composed of independently styled runs.
#[repr(C)]
#[derive(Debug)]
pub struct WuiStyledStr {
    /// The styled runs that make up the string, in order.
    pub chunks: WuiArray<WuiStyledChunk>,
}

impl IntoFFI for WuiStyledChunk {
    type FFI = Self;

    fn into_ffi(self) -> Self::FFI {
        self
    }
}

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

impl IntoRust for WuiTextStyle {
    type Rust = Style;

    unsafe fn into_rust(self) -> Self::Rust {
        // SAFETY: the caller contract makes `font` an owning handle from the
        // matching FFI constructor; it is consumed here and not observed
        // again.
        let font = unsafe { self.font.into_rust() };

        let foreground = if self.foreground.is_null() {
            None
        } else {
            // SAFETY: the caller contract makes `foreground` an owning handle from
            // the matching FFI constructor; it is consumed here and not
            // observed again.
            Some(unsafe { self.foreground.into_rust() })
        };

        let background = if self.background.is_null() {
            None
        } else if self.background == self.foreground && foreground.is_some() {
            foreground.clone()
        } else {
            // SAFETY: the caller contract makes `background` an owning handle from
            // the matching FFI constructor; it is consumed here and not
            // observed again.
            Some(unsafe { self.background.into_rust() })
        };

        Style {
            font,
            foreground,
            background,
            italic: self.italic,
            underline: self.underline,
            strikethrough: self.strikethrough,
        }
    }
}

impl IntoRust for WuiStyledChunk {
    type Rust = (Str, Style);

    unsafe fn into_rust(self) -> Self::Rust {
        // SAFETY: the caller contract makes `text` an owning handle from the
        // matching FFI constructor; it is consumed here and not observed
        // again.
        let text = unsafe { self.text.into_rust() };
        // SAFETY: the caller contract makes `style` an owning handle from the
        // matching FFI constructor; it is consumed here and not observed
        // again.
        let style = unsafe { self.style.into_rust() };
        (text, style)
    }
}

impl IntoRust for WuiStyledStr {
    type Rust = StyledStr;

    unsafe fn into_rust(mut self) -> Self::Rust {
        let mut styled = StyledStr::empty();
        let chunks = self.chunks.as_mut_slice();
        let len = chunks.len();
        let ptr = chunks.as_mut_ptr();

        for index in 0..len {
            // SAFETY: `index` is below the length the caller supplied alongside `ptr`,
            // so the element is in bounds and initialized; each index is read once, so
            // ownership is taken exactly once.
            let chunk = unsafe { core::ptr::read(ptr.add(index)) };
            // SAFETY: the caller contract makes `chunk` an owning handle from the
            // matching FFI constructor; it is consumed here and not observed
            // again.
            let (text, style) = unsafe { chunk.into_rust() };
            styled.push(text, style);
        }

        self.chunks.consume();
        styled
    }
}

ffi_computed!(StyledStr, WuiStyledStr);

/// FFI-safe horizontal paragraph alignment.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WuiHorizontalAlignment {
    /// Align to the leading edge (left in left-to-right locales).
    Leading = 0,
    /// Center within the available width.
    #[default]
    Center = 1,
    /// Align to the trailing edge (right in left-to-right locales).
    Trailing = 2,
}

impl IntoFFI for HorizontalAlignment {
    type FFI = WuiHorizontalAlignment;

    fn into_ffi(self) -> Self::FFI {
        if self == Self::Leading {
            WuiHorizontalAlignment::Leading
        } else if self == Self::Trailing {
            WuiHorizontalAlignment::Trailing
        } else {
            WuiHorizontalAlignment::Center
        }
    }
}

impl IntoRust for WuiHorizontalAlignment {
    type Rust = HorizontalAlignment;

    unsafe fn into_rust(self) -> Self::Rust {
        match self {
            Self::Leading => HorizontalAlignment::Leading,
            Self::Center => HorizontalAlignment::Center,
            Self::Trailing => HorizontalAlignment::Trailing,
        }
    }
}

ffi_computed!(
    HorizontalAlignment,
    WuiHorizontalAlignment,
    horizontal_alignment
);

into_ffi! {
    TextConfig,
    pub struct WuiText {
        content: *mut WuiComputed<StyledStr>,
        paragraph_alignment: *mut WuiComputed<HorizontalAlignment>,
    }
}

impl IntoFFI for Text {
    type FFI = WuiText;
    fn into_ffi(self) -> Self::FFI {
        self.into_config_without_env().into_ffi()
    }
}

// FFI view bindings for text components
ffi_view!(TextConfig, WuiText, text);

ffi_computed!(ResolvedFont, WuiResolvedFont);
ffi_computed_ctor!(ResolvedFont, WuiResolvedFont);

/// Creates a new `WuiResolvedFont` with a properly initialized empty family string.
///
/// This function is needed for native code (Android JNI) to create `WuiResolvedFont`
/// structs with valid vtables for the family field.
#[unsafe(no_mangle)]
pub extern "C" fn waterui_resolved_font_new(size: f32, weight: WuiFontWeight) -> WuiResolvedFont {
    WuiResolvedFont {
        size,
        weight,
        family: waterui::Str::from("").into_ffi(),
    }
}

/// Creates a concrete `Font` from resolved font properties.
///
/// `family` can be an empty string to indicate system font.
///
/// # Safety
/// `family` must contain valid UTF-8 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_font_from_resolved(
    size: f32,
    weight: WuiFontWeight,
    family: WuiStr,
) -> *mut WuiFont {
    // SAFETY: the caller contract makes `weight` an owning handle from the matching
    // FFI constructor; it is consumed here and not observed again.
    let weight = unsafe { weight.into_rust() };
    // SAFETY: the caller contract makes `family` an owning handle from the matching
    // FFI constructor; it is consumed here and not observed again.
    let family: Str = unsafe { family.into_rust() };

    let mut font = Font::from(Body).size(size).weight(weight);
    if !family.is_empty() {
        font = font.family(family);
    }
    font.into_ffi()
}

/// Resolves a font in the given environment.
///
/// # Safety
/// Both `font` and `env` must be valid, non-null pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_resolve_font(
    font: *const WuiFont,
    env: *const WuiEnv,
) -> *mut WuiComputed<ResolvedFont> {
    // SAFETY: the caller contract requires `font` to be a valid handle alive for this
    // call; it is only borrowed.
    let font = unsafe { &*font };
    // SAFETY: likewise for `env`.
    let env = unsafe { &*env };
    let resolved = font.resolve(env);
    resolved.into_ffi()
}
