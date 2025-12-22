//! Font Awesome 7 Free icons for WaterUI.
//!
//! This crate provides 2800+ Font Awesome 7 Free icons as SVG components.
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
//! # Usage
//!
//! ```ignore
//! use waterui_icons_fontawesome7 as fa;
//!
//! // Solid icons (most common)
//! fa::solid::house()
//! fa::solid::user()
//!
//! // Regular (outlined) icons
//! fa::regular::heart()
//!
//! // Brand icons
//! fa::brands::github()
//! fa::brands::twitter()
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

#[cfg(feature = "svg")]
pub use waterui_graphics::Svg;

/// Brand logos.
pub mod brands {
    #[cfg(feature = "svg")]
    #[allow(unused_imports)]
    use crate::Svg;

    include!(concat!(env!("OUT_DIR"), "/brands.rs"));
}

/// Regular weight (outlined) icons.
pub mod regular {
    #[cfg(feature = "svg")]
    #[allow(unused_imports)]
    use crate::Svg;

    include!(concat!(env!("OUT_DIR"), "/regular.rs"));
}

/// Solid (filled) icons.
pub mod solid {
    #[cfg(feature = "svg")]
    #[allow(unused_imports)]
    use crate::Svg;

    include!(concat!(env!("OUT_DIR"), "/solid.rs"));
}
