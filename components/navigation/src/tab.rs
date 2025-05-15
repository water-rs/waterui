//! Tabs module provides UI elements for building tabbed interfaces.
//!
//! This module includes the components needed to create and manage tabs,
//! with support for selection binding and navigation views.

use core::num::NonZeroI32;

use alloc::{boxed::Box, vec::Vec};
use uniffi::custom_type;
use waterui_core::{
    AnyView, Environment, configurable,
    handler::{BoxHandler, HandlerFn, into_handler},
    id::{Id, Mapping},
    impl_debug,
};
use waterui_reactive::{Binding, ffi_binding};

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

mod ffi {
    use uniffi::custom_type;
    use waterui_core::{AnyView, ffi_handler, handler::BoxHandler, id::TaggedView};

    use crate::NavigationView;

    use super::{Id, Tab};
    #[derive(uniffi::Record)]
    pub struct FFIRawTab {
        label: TaggedView<Id, AnyView>,
        content: BoxHandler<NavigationView>,
    }
    ffi_handler!(NavigationView);
    type RawTab = Tab<Id>;
    custom_type!(RawTab,FFIRawTab,{
        lower:|value|{
            FFIRawTab {
                label: value.label,
                content: value.content,
            }
        },
        try_lift:|value|{
            Ok(Tab {
                label: value.label,
                content: value.content,
            })
        }
    });
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
uniffi::use_remote_type!(waterui_core::Binding<Id>);

/// Configuration for the Tabs component.
///
/// This struct holds the current tab selection and the collection of tabs.
#[derive(Debug, uniffi::Record)]
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
    pub fn new(selection: Binding<Id>, tabs: Vec<Tab<Id>>) -> Self {
        Self { selection, tabs }
    }
}

impl Tabs {
    /// Creates a new Tabs component with the provided tabs.
    ///
    /// The first tab will be selected by default.
    ///
    /// # Arguments
    ///
    /// * `tabs` - Collection of tabs to include in the component
    ///
    /// # Panics
    ///
    /// Panics if the tabs collection is empty.
    pub fn new<T: Ord + Clone + 'static>(tabs: impl IntoIterator<Item = Tab<T>>) -> Self {
        let mut tabs = tabs.into_iter().peekable();
        let tab = tabs
            .peek()
            .expect("Tabs can not be empty")
            .label
            .tag
            .clone();
        let selection = Binding::container(tab);
        Self::with_selection(&selection, tabs)
    }

    /// Creates a new Tabs component with a specific tab selection binding.
    ///
    /// # Arguments
    ///
    /// * `selection` - A binding to control which tab is selected
    /// * `tabs` - Collection of tabs to include in the component
    pub fn with_selection<T: Ord + Clone + 'static>(
        selection: &Binding<T>,
        tabs: impl IntoIterator<Item = Tab<T>>,
    ) -> Self {
        let mapping = Mapping::new();
        let selection = mapping.binding(selection.clone());

        let tabs = tabs
            .into_iter()
            .map(|value| Tab {
                label: value.label.map(|value| mapping.to_id(value)),
                content: value.content,
            })
            .collect::<Vec<_>>();

        Self(TabsConfig { selection, tabs })
    }
}
