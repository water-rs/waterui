//! Lucide Icons for `WaterUI`.
//!
//! This crate provides 1500+ Lucide icons as SVG components for native rendering.
//!
//! # License
//!
//! Icons are licensed under **ISC** by Lucide Contributors.
//! See [LICENSE](https://github.com/lucide-icons/lucide/blob/main/LICENSE)
//! for full license text.
//!
//! # Features
//!
//! - `svg` (default) - Icons backed by SVG paths
//! - `webfont` - Icons as styled text using Lucide font (requires font installation)
//!
//! # Usage
//!
//! ```
//! use waterui_icons_lucide as lucide;
//!
//! // Every icon constructor returns `waterui_icon::Icon`
//! let _house = lucide::house();
//! let _settings = lucide::settings();
//! let _user = lucide::user();
//!
//! // Or access the raw SVG path data
//! let _paths = [lucide::HOUSE_PATH, lucide::SETTINGS_PATH];
//! ```
//!
//! # Icon Naming
//!
//! Icon names use `snake_case` in Rust, converted from the original kebab-case names:
//! - `arrow-left` → `arrow_left()` / `ARROW_LEFT_PATH`
//! - `circle-check` → `circle_check()` / `CIRCLE_CHECK_PATH`
//! - `file-text` → `file_text()` / `FILE_TEXT_PATH`
//!
//! All icons render at 24x24 points by default (standard Lucide size).

#![no_std]

pub use waterui_icon::Icon;

#[cfg(feature = "svg")]
pub use waterui_icon::Svg;

#[cfg(feature = "webfont")]
pub use waterui_icon::IconGlyph;

// Include the auto-generated icon definitions
// This generates:
// - `ICON_NAME_PATH` constants with SVG path data
// - `icon_name()` functions returning `Icon`
include!(concat!(env!("OUT_DIR"), "/icons.rs"));
