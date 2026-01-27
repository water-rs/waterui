//! Label component combining optional icon and text.
//!
//! Labels are commonly used in buttons, menu items, snackbars, and navigation.
//!
//! # Examples
//!
//! ```ignore
//! use waterui::prelude::*;
//! use waterui::component::label::{Label, label};
//!
//! // Text only
//! label("Hello")
//!
//! // With leading icon (default)
//! Label::new("Home").icon(SystemIcon::HOME)
//!
//! // With trailing icon
//! Label::new("Next").icon(SystemIcon::CHEVRON_RIGHT).trailing()
//! ```

use waterui_core::{AnyView, View};
use waterui_icon::SystemIcon;
use waterui_layout::stack::hstack;
use waterui_str::Str;
use waterui_text::text::text;

/// Position of the icon relative to the text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IconPosition {
    /// Icon appears before the text (left in LTR).
    #[default]
    Leading,
    /// Icon appears after the text (right in LTR).
    Trailing,
}

/// A label combining optional icon and text in a horizontal layout.
///
/// Labels provide a consistent way to display icon + text combinations
/// across the UI. They're used internally by Snackbar and can be used
/// in buttons, menu items, and other interactive elements.
///
/// # Examples
///
/// ```ignore
/// // Simple text label
/// Label::new("Settings")
///
/// // With icon
/// Label::new("Home").icon(SystemIcon::HOME)
///
/// // Icon on trailing side
/// Label::new("More").icon(SystemIcon::CHEVRON_RIGHT).trailing()
///
/// // Custom spacing
/// Label::new("Save").icon(SystemIcon::CHECKMARK).spacing(8.0)
/// ```
#[derive(Debug, Clone)]
pub struct Label {
    text: Str,
    icon: Option<SystemIcon>,
    icon_position: IconPosition,
    spacing: f32,
}

impl Label {
    /// Creates a new label with the specified text.
    #[must_use]
    pub fn new(text: impl Into<Str>) -> Self {
        Self {
            text: text.into(),
            icon: None,
            icon_position: IconPosition::Leading,
            spacing: 6.0,
        }
    }

    /// Adds an icon to the label.
    #[must_use]
    pub fn icon(mut self, icon: SystemIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Places the icon on the trailing side (after text).
    #[must_use]
    pub const fn trailing(mut self) -> Self {
        self.icon_position = IconPosition::Trailing;
        self
    }

    /// Places the icon on the leading side (before text). This is the default.
    #[must_use]
    pub const fn leading(mut self) -> Self {
        self.icon_position = IconPosition::Leading;
        self
    }

    /// Sets the spacing between icon and text.
    #[must_use]
    pub const fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }
}

impl View for Label {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        match (self.icon, self.icon_position) {
            (None, _) => AnyView::new(text(self.text)),
            (Some(icon), IconPosition::Leading) => {
                AnyView::new(hstack((icon, text(self.text))).spacing(self.spacing))
            }
            (Some(icon), IconPosition::Trailing) => {
                AnyView::new(hstack((text(self.text), icon)).spacing(self.spacing))
            }
        }
    }
}

/// Convenience function to create a label.
///
/// # Examples
///
/// ```ignore
/// label("Click me")
/// ```
#[must_use]
pub fn label(text: impl Into<Str>) -> Label {
    Label::new(text)
}
