//! # Theme FFI
//!
//! This module provides the FFI bindings for the `WaterUI` theme system, allowing
//! native backends (iOS, Android) to inject reactive color and font signals.
//!
//! ## Overview
//!
//! The theme FFI uses a **slot-based approach**:
//! 1. Native code creates reactive signals (`WuiComputed<ResolvedColor>`, etc.)
//! 2. Native installs signals for specific slots using enum-based APIs
//! 3. `WaterUI` views resolve these slots to get reactive theme values
//!
//! ## Color Scheme
//!
//! Native backends should track system appearance and update the color scheme:
//!
//! ```c
//! // Create a native-owned reactive color scheme signal
//! WuiComputed_ColorScheme* scheme =
//!     waterui_new_computed_color_scheme(data, get, watch, drop);
//!
//! // Install it
//! waterui_theme_install_color_scheme(env, scheme);
//! ```
//!
//! ## Installing Theme Slots
//!
//! Use the slot enums to install colors and fonts:
//!
//! ```c
//! // Install foreground color
//! WuiComputed_ResolvedColor* fg = create_foreground_signal();
//! waterui_theme_install_color(env, WuiColorSlot_Foreground, fg);
//!
//! // Install body font
//! WuiComputed_ResolvedFont* body = create_body_font_signal();
//! waterui_theme_install_font(env, WuiFontSlot_Body, body);
//! ```
//!
//! ## Querying Theme Values
//!
//! Native components can query current theme values:
//!
//! ```c
//! WuiComputed_ResolvedColor* accent = waterui_theme_color(env, WuiColorSlot_Accent);
//! // Use the signal, then drop when done
//! waterui_drop_computed_resolved_color(accent);
//! ```

use alloc::boxed::Box;

use crate::color::WuiResolvedColor;
use crate::components::text::WuiResolvedFont;
use crate::{IntoFFI, IntoRust, WuiEnv, ffi_computed, ffi_computed_ctor, reactive::WuiComputed};
use nami::SignalExt;
use waterui::theme::{
    self, color, install_color_scheme, install_color_signal, install_font_signal,
};
use waterui_core::resolve::Resolvable;
use waterui_graphics::color::ResolvedColor;
use waterui_text::font::{Body, Caption, Footnote, Headline, ResolvedFont, Subheadline, Title};

// ============================================================================
// ColorScheme FFI
// ============================================================================

/// Color scheme enum for FFI.
///
/// Maps directly to `waterui::theme::ColorScheme`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiColorScheme {
    /// Light appearance.
    Light = 0,
    /// Dark appearance.
    Dark = 1,
}

impl From<WuiColorScheme> for theme::ColorScheme {
    fn from(value: WuiColorScheme) -> Self {
        match value {
            WuiColorScheme::Light => Self::Light,
            WuiColorScheme::Dark => Self::Dark,
        }
    }
}

impl From<theme::ColorScheme> for WuiColorScheme {
    fn from(value: theme::ColorScheme) -> Self {
        match value {
            theme::ColorScheme::Light => Self::Light,
            theme::ColorScheme::Dark => Self::Dark,
        }
    }
}

impl IntoFFI for theme::ColorScheme {
    type FFI = WuiColorScheme;
    fn into_ffi(self) -> Self::FFI {
        self.into()
    }
}

impl IntoRust for WuiColorScheme {
    type Rust = theme::ColorScheme;
    unsafe fn into_rust(self) -> Self::Rust {
        self.into()
    }
}

// Generate FFI support for ColorScheme computed signals
// This enables native backends to create reactive ColorScheme signals with callbacks
ffi_computed!(theme::ColorScheme, WuiColorScheme, color_scheme);
ffi_computed_ctor!(theme::ColorScheme, WuiColorScheme, color_scheme);

/// Installs a color scheme signal into the environment.
///
/// # Safety
/// The signal pointer must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_theme_install_color_scheme(
    env: *mut WuiEnv,
    signal: *mut WuiComputed<theme::ColorScheme>,
) {
    // SAFETY: the caller contract requires `env` to be a valid handle, alive and not
    // otherwise borrowed for this call; the exclusive borrow ends here.
    let env = unsafe { crate::borrow_ffi_mut(env) };
    // SAFETY: the caller contract makes `signal` an owning pointer from the matching
    // FFI constructor, so reclaiming the box frees it exactly once.
    let computed = unsafe { Box::from_raw(signal) }.0;
    install_color_scheme(env, computed);
}

/// Returns the current color scheme signal from the environment.
///
/// # Safety
/// The returned pointer must be dropped by the caller when no longer needed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_theme_color_scheme(
    env: *const WuiEnv,
) -> *mut WuiComputed<theme::ColorScheme> {
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let env = unsafe { crate::borrow_ffi(env) };
    let computed = theme::current_color_scheme(env);
    computed.into_ffi()
}

// ============================================================================
// Color Slot FFI
// ============================================================================

/// Color slot enum for FFI.
///
/// Each variant corresponds to a color token in `waterui::theme::color`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiColorSlot {
    /// Primary background color.
    Background = 0,
    /// Elevated surface color (cards, sheets).
    Surface = 1,
    /// Alternate surface color.
    SurfaceVariant = 2,
    /// Border and divider color.
    Border = 3,
    /// Primary text and icon color.
    Foreground = 4,
    /// Secondary/dimmed text color.
    MutedForeground = 5,
    /// Accent color for interactive elements.
    Accent = 6,
    /// Foreground color on accent backgrounds.
    AccentForeground = 7,
    /// Container color associated with the accent.
    AccentContainer = 8,
    /// Contrasting accent color for complementary emphasis.
    Tertiary = 9,
    /// Container color associated with the tertiary accent.
    TertiaryContainer = 10,
}

/// Installs a color signal for a specific slot.
///
/// Takes ownership of the signal pointer.
///
/// # Safety
/// The signal pointer must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_theme_install_color(
    env: *mut WuiEnv,
    slot: WuiColorSlot,
    signal: *mut WuiComputed<ResolvedColor>,
) {
    // SAFETY: the caller contract requires `env` to be a valid handle, alive and not
    // otherwise borrowed for this call; the exclusive borrow ends here.
    let env = unsafe { crate::borrow_ffi_mut(env) };
    // SAFETY: the caller contract makes `signal` an owning pointer from the matching
    // FFI constructor, so reclaiming the box frees it exactly once.
    let computed = unsafe { Box::from_raw(signal) }.0;

    match slot {
        WuiColorSlot::Background => install_color_signal::<color::Background>(env, computed),
        WuiColorSlot::Surface => install_color_signal::<color::Surface>(env, computed),
        WuiColorSlot::SurfaceVariant => {
            install_color_signal::<color::SurfaceVariant>(env, computed);
        }
        WuiColorSlot::Border => install_color_signal::<color::Border>(env, computed),
        WuiColorSlot::Foreground => install_color_signal::<color::Foreground>(env, computed),
        WuiColorSlot::MutedForeground => {
            install_color_signal::<color::MutedForeground>(env, computed);
        }
        WuiColorSlot::Accent => install_color_signal::<color::Accent>(env, computed),
        WuiColorSlot::AccentForeground => {
            install_color_signal::<color::AccentForeground>(env, computed);
        }
        WuiColorSlot::AccentContainer => {
            install_color_signal::<color::AccentContainer>(env, computed);
        }
        WuiColorSlot::Tertiary => install_color_signal::<color::Tertiary>(env, computed),
        WuiColorSlot::TertiaryContainer => {
            install_color_signal::<color::TertiaryContainer>(env, computed);
        }
    }
}

/// Returns the color signal for a specific slot.
///
/// Returns a new reference to the signal. Caller must drop it when done.
///
/// # Safety
/// The env pointer must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_theme_color(
    env: *const WuiEnv,
    slot: WuiColorSlot,
) -> *mut WuiComputed<ResolvedColor> {
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let env = unsafe { crate::borrow_ffi(env) };

    let computed = match slot {
        WuiColorSlot::Background => color::Background.resolve(env).computed(),
        WuiColorSlot::Surface => color::Surface.resolve(env).computed(),
        WuiColorSlot::SurfaceVariant => color::SurfaceVariant.resolve(env).computed(),
        WuiColorSlot::Border => color::Border.resolve(env).computed(),
        WuiColorSlot::Foreground => color::Foreground.resolve(env).computed(),
        WuiColorSlot::MutedForeground => color::MutedForeground.resolve(env).computed(),
        WuiColorSlot::Accent => color::Accent.resolve(env).computed(),
        WuiColorSlot::AccentForeground => color::AccentForeground.resolve(env).computed(),
        WuiColorSlot::AccentContainer => color::AccentContainer.resolve(env).computed(),
        WuiColorSlot::Tertiary => color::Tertiary.resolve(env).computed(),
        WuiColorSlot::TertiaryContainer => color::TertiaryContainer.resolve(env).computed(),
    };

    computed.into_ffi()
}

// ============================================================================
// Font Slot FFI
// ============================================================================

/// Font slot enum for FFI.
///
/// Each variant corresponds to a font token in `waterui::text::font`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiFontSlot {
    /// Body text font.
    Body = 0,
    /// Title font.
    Title = 1,
    /// Headline font.
    Headline = 2,
    /// Subheadline font.
    Subheadline = 3,
    /// Caption font.
    Caption = 4,
    /// Footnote font.
    Footnote = 5,
}

/// Installs a font signal for a specific slot.
///
/// Takes ownership of the signal pointer.
///
/// # Safety
/// The env pointer must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_theme_install_font(
    env: *mut WuiEnv,
    slot: WuiFontSlot,
    signal: *mut WuiComputed<ResolvedFont>,
) {
    // SAFETY: the caller contract requires `env` to be a valid handle, alive and not
    // otherwise borrowed for this call; the exclusive borrow ends here.
    let env = unsafe { crate::borrow_ffi_mut(env) };
    // SAFETY: the caller contract makes `signal` an owning pointer from the matching
    // FFI constructor, so reclaiming the box frees it exactly once.
    let computed = unsafe { Box::from_raw(signal) }.0;

    match slot {
        WuiFontSlot::Body => install_font_signal::<Body>(env, computed),
        WuiFontSlot::Title => install_font_signal::<Title>(env, computed),
        WuiFontSlot::Headline => install_font_signal::<Headline>(env, computed),
        WuiFontSlot::Subheadline => install_font_signal::<Subheadline>(env, computed),
        WuiFontSlot::Caption => install_font_signal::<Caption>(env, computed),
        WuiFontSlot::Footnote => install_font_signal::<Footnote>(env, computed),
    }
}

/// Returns the font signal for a specific slot.
///
/// Returns a new reference to the signal. Caller must drop it when done.
///
/// # Safety
/// The env pointer must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_theme_font(
    env: *const WuiEnv,
    slot: WuiFontSlot,
) -> *mut WuiComputed<ResolvedFont> {
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let env = unsafe { crate::borrow_ffi(env) };

    let computed = match slot {
        WuiFontSlot::Body => Body.resolve(env).computed(),
        WuiFontSlot::Title => Title.resolve(env).computed(),
        WuiFontSlot::Headline => Headline.resolve(env).computed(),
        WuiFontSlot::Subheadline => Subheadline.resolve(env).computed(),
        WuiFontSlot::Caption => Caption.resolve(env).computed(),
        WuiFontSlot::Footnote => Footnote.resolve(env).computed(),
    };

    computed.into_ffi()
}

// ============================================================================
// Watcher call functions for native-controlled reactive signals
// ============================================================================

use crate::reactive::WuiWatcher;

/// Calls a `ColorScheme` watcher with the given value.
/// Used by native code to notify Rust when color scheme changes.
/// # Safety
/// The watcher pointer must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_watcher_color_scheme(
    watcher: *const WuiWatcher<theme::ColorScheme>,
    value: WuiColorScheme,
) {
    // SAFETY: the caller contract requires `watcher` to be a valid handle alive for
    // this call; it is only borrowed.
    unsafe {
        let watcher = crate::borrow_ffi(watcher);
        let rust_value: theme::ColorScheme = value.into();
        let metadata = waterui::reactive::watcher::Metadata::default();
        watcher.call(rust_value, metadata);
    }
}

/// Drops a `ColorScheme` watcher.
/// # Safety
/// The watcher pointer must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_watcher_color_scheme(
    watcher: *mut WuiWatcher<theme::ColorScheme>,
) {
    // SAFETY: the caller contract makes `watcher` an owning pointer from the matching
    // constructor that has not been dropped.
    unsafe {
        drop(Box::from_raw(watcher));
    }
}

/// Calls a `ResolvedColor` watcher with the given value.
/// Used by native code to notify Rust when a color value changes.
/// # Safety
/// The watcher pointer must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_watcher_resolved_color(
    watcher: *const WuiWatcher<ResolvedColor>,
    value: WuiResolvedColor,
) {
    // SAFETY: the caller contract requires `watcher` to be a valid handle alive for
    // this call; it is only borrowed.
    unsafe {
        let watcher = crate::borrow_ffi(watcher);
        let rust_value = value.into_rust();
        let metadata = waterui::reactive::watcher::Metadata::default();
        watcher.call(rust_value, metadata);
    }
}

/// Drops a `ResolvedColor` watcher.
/// # Safety
/// The watcher pointer must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_watcher_resolved_color(
    watcher: *mut WuiWatcher<ResolvedColor>,
) {
    // SAFETY: the caller contract makes `watcher` an owning pointer from the matching
    // constructor that has not been dropped.
    unsafe {
        drop(Box::from_raw(watcher));
    }
}

/// Calls a `ResolvedFont` watcher with the given value.
/// Used by native code to notify Rust when a font value changes.
/// # Safety
/// The watcher pointer must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_watcher_resolved_font(
    watcher: *const WuiWatcher<ResolvedFont>,
    value: WuiResolvedFont,
) {
    // SAFETY: the caller contract requires `watcher` to be a valid handle alive for
    // this call; it is only borrowed.
    unsafe {
        let watcher = crate::borrow_ffi(watcher);
        let rust_value = value.into_rust();
        let metadata = waterui::reactive::watcher::Metadata::default();
        watcher.call(rust_value, metadata);
    }
}

/// Drops a `ResolvedFont` watcher.
/// # Safety
/// The watcher pointer must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_watcher_resolved_font(
    watcher: *mut WuiWatcher<ResolvedFont>,
) {
    // SAFETY: the caller contract makes `watcher` an owning pointer from the matching
    // constructor that has not been dropped.
    unsafe {
        drop(Box::from_raw(watcher));
    }
}

#[cfg(all(test, feature = "c-api"))]
mod tests {
    use super::*;
    use crate::WuiEnv;

    #[test]
    fn color_scheme_roundtrip() {
        let ptr = waterui::Computed::constant(theme::ColorScheme::Dark).into_ffi();
        assert!(!ptr.is_null());
        // SAFETY: `ptr` is the owning handle created just above in this test.
        let value = unsafe { waterui_read_computed_color_scheme(ptr) };
        assert_eq!(value, WuiColorScheme::Dark);
        // SAFETY: `ptr` is that same handle, dropped once here at end of test.
        unsafe {
            waterui_drop_computed_color_scheme(ptr);
        }
    }

    #[test]
    fn slot_based_color_install_and_query() {
        let mut env = WuiEnv(waterui::Environment::new());

        // Create and install a foreground color
        let fg_signal = waterui::Computed::constant(ResolvedColor {
            red: 1.0,
            green: 0.0,
            blue: 0.0,
            headroom: 0.0,
            opacity: 1.0,
        });
        let fg_ptr = fg_signal.into_ffi();

        // SAFETY: `env` is a live local and `fg_ptr` an owning color handle the
        // install consumes.
        unsafe {
            waterui_theme_install_color(&raw mut env, WuiColorSlot::Foreground, fg_ptr);
        }

        // Query it back
        // SAFETY: `env` is a live local for the duration of the call.
        let queried = unsafe { waterui_theme_color(&raw const env, WuiColorSlot::Foreground) };
        assert!(!queried.is_null());

        // SAFETY: `queried` is the non-null owning handle just returned.
        let value = unsafe { crate::color::waterui_read_computed_resolved_color(queried) };
        assert!((value.red - 1.0).abs() < 0.001);

        // SAFETY: `queried` is that same handle, dropped once here.
        unsafe {
            crate::color::waterui_drop_computed_resolved_color(queried);
        }
    }
}
