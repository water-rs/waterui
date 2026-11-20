//! Icon components for `WaterUI`.
//!
//! This module provides [`IconGlyph`] for bundled icon fonts and optionally
//! re-exports `Svg` for vector icon packs. Platform-specific system catalogs
//! (such as Apple SF Symbols) live in their respective backends rather than in
//! cross-platform view crates.
//!
//! # Icon Types
//!
//! ## [`IconGlyph`] - Webfont Icons
//!
//! Renders icons using icon fonts (webfonts). Simpler and more reliable than SVG.
//! Requires the icon font to be bundled with your app.
//!
//! ```
//! use waterui_icon::IconGlyph;
//!
//! // Material Icons
//! const HOME: IconGlyph = IconGlyph::new('\u{e88a}', "MaterialIcons-Regular");
//!
//! // Font Awesome
//! const HOUSE: IconGlyph = IconGlyph::new('\u{f015}', "FontAwesome7Free-Solid");
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
