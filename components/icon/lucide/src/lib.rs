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
//! - `svg` (default) - Icons as [`Svg`] components for native rendering
//! - `webfont` - Icons as styled text using Lucide font (requires font installation)
//!
//! # Usage
//!
//! ```ignore
//! use waterui_icons_lucide as lucide;
//!
//! // Get an icon as Svg component (svg feature)
//! lucide::home()
//! lucide::settings()
//! lucide::user()
//!
//! // Or access the raw SVG path data
//! lucide::HOME_PATH
//! lucide::SETTINGS_PATH
//! ```
//!
//! # Icon Naming
//!
//! Icon names use `snake_case` in Rust, converted from the original kebab-case names:
//! - `arrow-left` → `arrow_left()` (SVG) / `ARROW_LEFT_PATH`
//! - `check-circle` → `check_circle()` (SVG) / `CHECK_CIRCLE_PATH`
//! - `file-text` → `file_text()` (SVG) / `FILE_TEXT_PATH`
//!
//! All icons render at 24x24 points by default (standard Lucide size).

#![no_std]

// Re-export from waterui-icon
#[cfg(feature = "svg")]
pub use waterui_icon::Svg;

// Include the auto-generated icon definitions
// This generates:
// - `ICON_NAME_PATH` constants with SVG path data
// - `icon_name()` functions (svg feature) returning Svg
include!(concat!(env!("OUT_DIR"), "/icons.rs"));
