//! Material Design Icons for `WaterUI`.
//!
//! This crate provides 7400+ Material Design icons from [Pictogrammers](https://pictogrammers.com/)
//! as SVG components for native rendering.
//!
//! # License
//!
//! Icons are licensed under **Apache 2.0** by Pictogrammers.
//! See [LICENSE-ICONS](https://github.com/Templarian/MaterialDesign-SVG/blob/master/LICENSE)
//! for full license text.
//!
//! # Features
//!
//! - `svg` (default) - Icons as [`Svg`] components for native rendering
//! - `webfont` - Icons as styled text using Material Icons font (requires font installation)
//!
//! # Usage
//!
//! ```ignore
//! use waterui_icons_material_icon as mdi;
//!
//! // Get an icon as Svg component
//! mdi::home()
//! mdi::settings()
//! mdi::account()
//!
//! // Or access the raw SVG path data
//! mdi::HOME_PATH
//! mdi::SETTINGS_PATH
//! ```
//!
//! # Icon Naming
//!
//! Icon names use `snake_case` in Rust, converted from the original kebab-case names:
//! - `account-circle` → `account_circle()`
//! - `arrow-left` → `arrow_left()`
//! - `check-circle` → `check_circle()`
//!
//! All icons render at 24x24 points by default (standard Material Design size).

#![no_std]

// Re-export from waterui-icon
#[cfg(feature = "svg")]
pub use waterui_icon::Svg;

#[cfg(feature = "webfont")]
pub use waterui_icon::IconGlyph;

// Include the auto-generated icon definitions
include!(concat!(env!("OUT_DIR"), "/icons.rs"));
