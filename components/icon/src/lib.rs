//! Icon components for `WaterUI`.
//!
//! This module provides icon components for displaying icons in `WaterUI` apps.
//!
//! # Icon Types
//!
//! ## [`IconGlyph`] - Webfont Icons (Recommended)
//!
//! Renders icons using icon fonts (webfonts). Simpler and more reliable than SVG.
//! Requires the icon font to be bundled with your app.
//!
//! ```ignore
//! use waterui_icon::IconGlyph;
//!
//! // Material Icons
//! const HOME: IconGlyph = IconGlyph::new('\u{e88a}', "MaterialIcons-Regular");
//!
//! // Font Awesome
//! const HOUSE: IconGlyph = IconGlyph::new('\u{f015}', "FontAwesome7Free-Solid");
//! ```
//!
//! ## [`SystemIcon`] - Platform-Native Icons
//!
//! Renders platform-native icons where the backend provides a native system icon catalog.
//! - Apple: SF Symbols
//!
//! For cross-platform icons, prefer icon-pack crates such as Material, Lucide, Font Awesome, or Native packs.
//!
//! ```ignore
//! use waterui_icon::{SystemIcon, system_icon};
//!
//! // Common icons via the function-form module
//! system_icon::home()
//! system_icon::settings()
//!
//! // Or create from a name
//! SystemIcon::new("custom.icon.name")
//! ```
//!
//! ## `Svg` - SVG Icons (requires `svg` feature)
//!
//! Re-exported from `waterui-svg` for convenience.
//! Icon packs can use this for SVG icon rendering.

#![no_std]
extern crate alloc;

// Re-export Svg for icon packs (requires svg feature)
#[cfg(feature = "svg")]
pub use waterui_svg::Svg;

mod glyph;
pub use glyph::IconGlyph;

use waterui_core::{impl_constant, raw_view};
use waterui_str::Str;

/// `SystemIcon` component representing a platform system icon by name.
///
/// # Platform support
///
/// `SystemIcon` is **intentionally asymmetric** across backends, because the
/// underlying primitive is asymmetric:
///
/// - **Apple platforms** (iOS / iPadOS / macOS / tvOS / visionOS): renders the
///   named glyph from Apple's SF Symbols catalog. Names follow SF Symbols
///   conventions (e.g. `"house"`, `"chevron.right"`, `"wifi"`).
/// - **Android, Linux, Web, terminal**: **not supported.** Android has no
///   OS-supplied icon catalog — `SystemIcon` does not silently substitute a
///   bundled Material font, because that would make the asymmetry invisible
///   to view code that should know about it (see `WaterUI`'s "Asymmetric
///   primitives are documented, not faked" design principle in `AGENTS.md`).
///
/// For **portable, cross-platform** icons, depend on a packaged icon-pack
/// crate instead — `waterui-icons-lucide`, `waterui-icons-material-icon`,
/// `waterui-icons-fontawesome7`, etc. Those crates ship the icon font and
/// render via [`IconGlyph`], which is supported on every backend.
///
/// # Example
///
/// ```ignore
/// use waterui_icon::{SystemIcon, system_icon};
///
/// // Common icons via the function-form module
/// system_icon::home()
/// system_icon::settings()
///
/// // Or create dynamically — note: only meaningful on Apple platforms.
/// SystemIcon::new("house")
/// ```
#[derive(Debug, Clone)]
pub struct SystemIcon {
    /// The name of the system icon.
    pub name: Str,
}

impl SystemIcon {
    /// Creates a new system icon with the given name.
    #[must_use]
    pub fn new(name: impl Into<Str>) -> Self {
        Self { name: name.into() }
    }

    /// Creates a system icon from a static string (const-compatible).
    #[must_use]
    pub const fn from_static(name: &'static str) -> Self {
        Self {
            name: Str::from_static(name),
        }
    }
}

raw_view!(SystemIcon);
impl_constant!(SystemIcon);

/// Common cross-platform system icon constructors.
///
/// Function-form entry points that match the shape of icon-pack crates
/// (`lucide`, `material-icon`, `sf-symbol`).
pub mod system_icon {
    use super::SystemIcon;

    macro_rules! system_icons {
        ($($(#[$meta:meta])* $name:ident => $sf:literal,)*) => {
            $(
                $(#[$meta])*
                #[must_use]
                pub const fn $name() -> SystemIcon {
                    SystemIcon::from_static($sf)
                }
            )*
        };
    }

    system_icons! {
        /// Home icon.
        home => "house",
        /// Settings/gear icon.
        settings => "gear",
        /// Search/magnifying glass icon.
        search => "magnifyingglass",
        /// Person icon.
        person => "person",
        /// Plus/add icon.
        plus => "plus",
        /// Trash/delete icon.
        trash => "trash",
        /// Chevron right icon.
        chevron_right => "chevron.right",
        /// Chevron left icon.
        chevron_left => "chevron.left",
        /// Close/X icon.
        close => "xmark",
        /// Checkmark icon.
        checkmark => "checkmark",
        /// Star icon.
        star => "star",
        /// Heart icon.
        heart => "heart",
    }
}
