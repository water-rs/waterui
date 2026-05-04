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
//! ## [`Svg`] - SVG Icons (requires `svg` feature)
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
/// On Apple platforms, this renders SF Symbols.
/// Other backends may choose not to implement `SystemIcon`; for cross-platform usage, prefer icon-pack crates.
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
/// // Or create dynamically
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
                pub fn $name() -> SystemIcon {
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
