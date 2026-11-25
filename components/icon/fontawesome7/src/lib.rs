//! Font Awesome 7 Free icons for `WaterUI`.
//!
//! This crate provides 2800+ Font Awesome 7 Free icons with two rendering modes:
//! - **Webfont** (default) - Icons rendered as styled text using Font Awesome webfont
//! - **SVG** (optional) - Icons rendered as SVG paths
//!
//! # License
//!
//! Icons are licensed under **CC BY 4.0** by [Fonticons, Inc.](https://fontawesome.com)
//! Attribution is required. See [LICENSE-ICONS](./LICENSE-ICONS) for details.
//!
//! # Icon Styles
//!
//! Font Awesome icons come in three styles:
//! - [`brands`] - Brand logos (549 icons)
//! - [`regular`] - Regular weight icons (273 icons)
//! - [`solid`] - Solid filled icons (1984 icons)
//!
//! # Usage (Webfont - Default)
//!
//! ```ignore
//! use waterui_icons_fontawesome7 as fa;
//!
//! // Every icon constructor returns `waterui_icon::Icon`
//! fa::solid::house()
//! fa::solid::user()
//! fa::regular::heart()
//! fa::brands::github()
//! ```
//!
//! # Font Bundle Requirements
//!
//! To use webfont icons, bundle the Font Awesome webfont files with your app:
//! - `fa-solid-900.ttf` (FontAwesome7Free-Solid)
//! - `fa-regular-400.ttf` (FontAwesome7Free-Regular)
//! - `fa-brands-400.ttf` (FontAwesome7Free-Brands)
//!
//! Register fonts in your app bundle (Info.plist on Apple, assets on Android).
//!
//! # Usage (SVG - Optional)
//!
//! Enable the `svg` feature for SVG rendering with the same call shape:
//!
//! ```ignore
//! use waterui_icons_fontawesome7 as fa;
//!
//! // Same function API, backed by SVG paths when the `svg` feature is enabled
//! fa::solid::house()
//! fa::brands::github()
//!
//! // Access raw path data
//! fa::solid::HOUSE_PATH
//! fa::solid::HOUSE_VIEWBOX // (width, height)
//! ```
//!
//! # Attribution
//!
//! Font Awesome Free 7.1.0 by @fontawesome - <https://fontawesome.com>
//!
//! License: <https://fontawesome.com/license/free>
//! - Icons: CC BY 4.0
//! - Fonts: SIL OFL 1.1
//! - Code: MIT License

#![no_std]

pub use waterui_icon::Icon;

#[cfg(feature = "svg")]
pub use waterui_icon::Svg;

#[cfg(feature = "webfont")]
pub use waterui_icon::IconGlyph;

/// Brand logos.
pub mod brands {
    include!(concat!(env!("OUT_DIR"), "/brands.rs"));
}

/// Regular weight (outlined) icons.
pub mod regular {
    include!(concat!(env!("OUT_DIR"), "/regular.rs"));
}

/// Solid (filled) icons.
pub mod solid {
    include!(concat!(env!("OUT_DIR"), "/solid.rs"));
}
