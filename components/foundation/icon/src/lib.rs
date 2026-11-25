//! Icon components for `WaterUI`.
//!
//! [`Icon`] is the unified icon view returned by every icon pack (`waterui-icons-lucide`,
//! `waterui-icons-material-icon`, `waterui-icons-fontawesome7`). Whether an icon is drawn
//! from an SVG path or a webfont [`IconGlyph`] is selected by pack features rather than by
//! changing the view type. Colour comes from `.foreground(...)` and size from [`IconSize`]
//! (installed via `.icon_size(...)`).
//!
//! Platform-specific system catalogs (such as `waterui_apple::SfSymbol`) live in their
//! respective backends rather than in cross-platform view crates.

#![no_std]
extern crate alloc;

#[cfg(feature = "svg")]
pub use waterui_svg::Svg;

mod glyph;
mod icon;

pub use glyph::IconGlyph;
pub use icon::{DEFAULT_ICON_SIZE, Icon, IconSize};
