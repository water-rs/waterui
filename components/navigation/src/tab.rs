//! Tabs module provides UI elements for building tabbed interfaces.
//!
//! This module includes the components needed to create and manage tabs,
//! with support for selection binding and navigation views.

use core::num::NonZeroI32;

use alloc::{boxed::Box, vec::Vec};

use waterui_core::{
    AnyView, Environment, configurable,
    handler::{BoxHandler, HandlerFn, into_handler},
    id::{Id, Mapping},
    impl_debug,
};
use waterui_reactive::Binding;

use super::NavigationView;
use waterui_core::id::TaggedView;

/// Represents a single tab with a label and content.
///
/// The generic parameter `T` is used for tag identification.
///
pub struct Tab<T> {
    /// The visual label for the tab, wrapped in a tagged view.
    pub label: TaggedView<T, AnyView>,

    /// The content to display when this tab is selected.
    /// Returns a NavigationView when given an Environment.
    pub content: BoxHandler<NavigationView>,
}

impl_debug!(Tab<Id>);

impl<T> Tab<T> {
    /// Creates a new tab with the given label and content.
    ///
    /// # Arguments
    ///
    /// * `label` - The visual representation of the tab
    /// * `content` - A function that returns the tab's content as a NavigationView
    pub fn new<H: 'static>(
        label: TaggedView<T, AnyView>,
        content: impl HandlerFn<H, NavigationView>,
    ) -> Self {
        Self {
            label,
            content: Box::new(into_handler(content)),
        }
    }
}

/// Configuration for the Tabs component.
///
/// This struct holds the current tab selection and the collection of tabs.
#[derive(Debug)]
#[non_exhaustive]
pub struct TabsConfig {
    /// The currently selected tab identifier.
    pub selection: Binding<Id>,

    /// The collection of tabs to display.
    pub tabs: Vec<Tab<Id>>,
}

configurable!(Tabs, TabsConfig);

impl TabsConfig {
    /// Creates a new tabs configuration with the given selection and tabs.
    ///
    /// # Arguments
    ///
    /// * `selection` - A binding to the currently selected tab ID
    /// * `tabs` - The collection of tabs to display
    pub const fn new(selection: Binding<Id>, tabs: Vec<Tab<Id>>) -> Self {
        Self { selection, tabs }
    }
}
