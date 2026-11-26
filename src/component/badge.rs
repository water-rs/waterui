//! Badge component for displaying indicators attached to content.
//!
//! The Badge component displays a small indicator attached to another view,
//! commonly used to show unread counts, status dots, or short labels that
//! require attention. Zero counts and [`BadgeValue::Hidden`] hide the badge.
//!
//! # Example
//!
//! ```
//! use waterui::prelude::*;
//!
//! let badge = badge::Badge::new(5_u32, button("Messages").action(|| {}));
//! ```

pub use waterui_controls::badge::{Badge, BadgeConfig, BadgeValue};

#[cfg(test)]
mod tests {
    use super::Badge;
    use crate::view::ViewExt as _;
    use waterui_text::text;

    #[test]
    fn badge_accepts_non_clone_modified_views() {
        let _by_modifier = text("Inbox").padding().badge(3_u32);
        let _by_constructor = Badge::new(3_u32, text("Inbox").padding().size(48.0, 48.0));
    }
}
