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
///         MenuItem::new("Copy").action(|| println!("Copy")),
///         MenuItem::new("Paste").action(|| println!("Paste")),
///         MenuItem::new("Delete").action(|| println!("Delete")),
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
    /// Creates a menu item builder with the given label.
    ///
    /// Use `.action()` to set the handler, or `.with_state()` to capture state first.
    ///
    /// # Examples
    ///
    /// Simple action:
    /// ```rust,ignore
    /// MenuItem::new("Copy").action(|| clipboard.copy())
    /// ```
    ///
    /// With state:
    /// ```rust,ignore
    /// MenuItem::new("Paste")
    ///     .with_state(&clipboard)
    ///     .action(|cb| cb.paste())
    /// ```
    #[must_use]
    pub fn new(label: impl Into<Text>) -> MenuItemBuilder {
        MenuItemBuilder {
            label: label.into(),
        }
    }
}

/// Builder for creating menu items with captured state.
#[derive(Debug, Clone)]
pub struct MenuItemBuilder {
    label: Text,
}

impl MenuItemBuilder {
    /// Sets the action for this menu item (no state).
    #[must_use]
    pub fn action(self, action: impl FnMut() + 'static) -> MenuItem {
        MenuItem {
            label: self.label,
            action: shared_action(action),
        }
    }

    /// Adds state to capture for the action.
    #[must_use]
    pub fn with_state<T: Clone + 'static>(self, state: &T) -> MenuItemStatefulBuilder<T> {
        MenuItemStatefulBuilder {
            label: self.label,
            state: state.clone(),
        }
    }
}

/// Builder for menu items with captured state.
#[derive(Debug, Clone)]
pub struct MenuItemStatefulBuilder<State> {
    label: Text,
    state: State,
}

// Generate with_state for state chaining: S -> (S, T)
waterui_core::impl_stateful_builder!(MenuItemStatefulBuilder; state; label);

impl<S: Clone + 'static> MenuItemStatefulBuilder<S> {
    /// Sets the action for this menu item with captured state.
    #[must_use]
    pub fn action(self, mut action: impl FnMut(S) + 'static) -> MenuItem {
        let state = self.state;
        MenuItem {
            label: self.label,
            action: shared_action(move || action(state.clone())),
        }
    }
}
