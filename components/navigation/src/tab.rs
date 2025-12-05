//! Tabs module provides UI elements for building tabbed interfaces.
//!
//! This module includes the components needed to create and manage tabs,
//! with support for selection binding and navigation views.

use alloc::vec::Vec;

use nami::Binding;
use waterui_core::{
    AnyView,
    handler::{AnyViewBuilder, ViewBuilder},
    id::Id,
    impl_debug,
    layout::StretchAxis,
    raw_view,
};

use super::NavigationView;
use waterui_core::id::TaggedView;

/// Position of the tab bar within the tab container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum TabPosition {
    /// Tab bar is positioned at the top of the container.
    Top,
    /// Tab bar is positioned at the bottom of the container (default).
    #[default]
    Bottom,
}

/// Represents a single tab with a label and content.
///
/// The generic parameter `T` is used for tag identification.
///
pub struct Tab<T> {
    /// The visual label for the tab, wrapped in a tagged view.
    pub label: TaggedView<T, AnyView>,

    /// The content to display when this tab is selected.
    /// Returns a [`NavigationView`] when given an Environment.
    pub content: AnyViewBuilder<NavigationView>,
}

impl_debug!(Tab<Id>);

impl<T> Tab<T> {
    /// Creates a new tab with the given label and content.
    ///
    /// # Arguments
    ///
    /// * `label` - The visual representation of the tab
    /// * `content` - A function that returns the tab's content as a [`NavigationView`]
    pub fn new(
        label: TaggedView<T, AnyView>,
        content: impl ViewBuilder<Output = NavigationView>,
    ) -> Self {
        Self {
            label,
            content: AnyViewBuilder::new(content),
        }
    }
}

/// Configuration for the Tabs component.
///
/// This struct holds the current tab selection and the collection of tabs.
#[derive(Debug)]
#[non_exhaustive]
pub struct Tabs {
    /// The currently selected tab identifier.
    pub selection: Binding<Id>,

    /// The collection of tabs to display.
    pub tabs: Vec<Tab<Id>>,

    /// Position of the tab bar (top or bottom).
    pub position: TabPosition,
}

// Make Tabs a raw view that stretches to fill available space
raw_view!(Tabs, StretchAxis::Both);
