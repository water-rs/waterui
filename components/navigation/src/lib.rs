//! Navigation module for `WaterUI` framework.
//!
//! This module provides navigation components and utilities for building
//! hierarchical user interfaces with navigation bars and links.
extern crate alloc;

/// Provides search functionality for navigation.
pub mod search;
pub mod tab;

use alloc::{boxed::Box, vec::Vec};
use nami::Computed;
use waterui_color::Color;
use waterui_core::{
    AnyView, View,
    handler::{BoxHandler, HandlerFn, into_handler},
    impl_debug, raw_view,
};
use waterui_text::Text;

/// A view that combines a navigation bar with content.
///
/// The `NavigationView` contains a navigation bar with a title and other
/// configuration options, along with the actual content to display.
#[derive(Debug)]
#[must_use]
pub struct NavigationView {
    /// The navigation bar for this view
    pub bar: Bar,
    /// The content to display in this view
    pub content: AnyView,
}

/// Configuration for a navigation bar.
///
/// Represents the appearance and behavior of a navigation bar, including
/// its title, color, and visibility.
#[derive(Debug, Default)]
pub struct Bar {
    /// The title text displayed in the navigation bar
    pub title: Text,
    /// The background color of the navigation bar
    pub color: Computed<Color>,
    /// Whether the navigation bar is hidden
    pub hidden: Computed<bool>,
}

/// A path of navigation views forming a navigation hierarchy.
///
/// Used to track the navigation stack in a hierarchical UI.
pub type NavigationPath = Vec<NavigationView>;

/// A link that navigates to another view when activated.
///
/// The `NavigationLink` combines a label view with a function that creates
/// the destination view when the link is activated.
#[must_use]
pub struct NavigationLink {
    /// The label view displayed for this link
    pub label: AnyView,
    /// A function that creates the destination view when the link is activated
    pub content: BoxHandler<NavigationView>,
}

impl_debug!(NavigationLink);

impl NavigationLink {
    /// Creates a new navigation link.
    ///
    /// # Arguments
    ///
    /// * `label` - The view to display as the link
    /// * `destination` - A function that creates the destination view
    pub fn new<P: 'static>(
        label: impl View,
        destination: impl HandlerFn<P, NavigationView>,
    ) -> Self {
        Self {
            label: AnyView::new(label),
            content: Box::new(into_handler(destination)),
        }
    }
}

raw_view!(NavigationLink);

impl NavigationView {
    /// Creates a new navigation view.
    ///
    /// # Arguments
    ///
    /// * `title` - The title to display in the navigation bar
    /// * `content` - The content view to display
    pub fn new(title: impl Into<Text>, content: impl View) -> Self {
        let bar = Bar {
            title: title.into(),
            ..Default::default()
        };

        Self {
            bar,
            content: AnyView::new(content),
        }
    }
}

/// Convenience function to create a navigation view.
///
/// # Arguments
///
/// * `title` - The title to display in the navigation bar
/// * `view` - The content view to display
pub fn navigation(title: impl Into<Text>, view: impl View) -> NavigationView {
    NavigationView::new(title, view)
}

/// Convenience function to create a navigation link.
///
/// # Arguments
///
/// * `label` - The view to display as the link
/// * `destination` - A function that creates the destination view
pub fn navigate(
    label: impl View,
    destination: impl 'static + Fn() -> NavigationView,
) -> NavigationLink {
    NavigationLink::new(label, destination)
}
