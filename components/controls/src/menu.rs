//! A menu component that displays a dropdown menu when tapped.
//!

use alloc::vec::Vec;
use nami::{Computed, impl_constant, signal::IntoComputed};
use waterui_core::{
    AnyView, View,
    handler::{SharedAction, shared_action},
    layout::StretchAxis,
    raw_view,
};
use waterui_text::Text;

/// A menu component that displays a dropdown menu when tapped.
///
/// On iOS, this renders as a button with a `UIMenu` attached.
/// On macOS, this renders as an `NSPopUpButton`.
/// On Android, this renders as a button with a `PopupMenu`.
///
/// # Example
///
/// ```rust,ignore
/// use waterui::prelude::*;
///
/// Menu::new(
///     text!("Options"),
///     [
///         MenuItem::new("Copy", || println!("Copy")),
///         MenuItem::new("Paste", || println!("Paste")),
///         MenuItem::new("Delete", || println!("Delete")),
///     ],
/// )
/// ```
#[derive(Debug)]
pub struct Menu {
    /// The label view displayed on the menu button.
    pub label: AnyView,
    /// The menu items to display when the menu is opened.
    pub items: Computed<Vec<MenuItem>>,
}

raw_view!(Menu, StretchAxis::None);

impl Menu {
    /// Creates a new menu with the given label and items.
    ///
    /// # Arguments
    /// * `label` - The view to display as the menu button label
    /// * `items` - The menu items to display when tapped
    pub fn new(label: impl View, items: impl IntoComputed<Vec<MenuItem>>) -> Self {
        Self {
            label: AnyView::new(label),
            items: items.into_computed(),
        }
    }
}

/// Configuration for a menu component.
#[derive(Debug)]
#[non_exhaustive]
pub struct MenuConfig {
    /// The label for the menu.
    pub label: AnyView,
    /// The items in the menu.
    pub items: Computed<Vec<MenuItem>>,
}

/// A single item in a menu.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MenuItem {
    /// The label for the menu item.
    pub label: Text,
    /// The action to perform when the menu item is selected.
    pub action: SharedAction<()>,
}

impl_constant!(MenuItem);

impl MenuItem {
    /// Creates a new menu item with the given label and action.
    pub fn new(label: impl Into<Text>, action: impl FnMut() + 'static) -> Self {
        Self {
            label: label.into(),
            action: shared_action(action),
        }
    }
}
