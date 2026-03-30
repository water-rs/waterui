#![no_std]

//! Navigation module for `WaterUI` framework.
//!
//! This module provides navigation components and utilities for building
//! hierarchical user interfaces with navigation bars and links.
extern crate alloc;

/// Provides search functionality for navigation.
pub mod search;
pub mod split;
pub mod tab;

use alloc::{rc::Rc, vec::Vec};
use core::{cell::RefCell, fmt::Debug};

use nami::{
    Binding, Computed,
    collection::{Collection, List},
};
use waterui_controls::{IntoLabel, button};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{
    AnyView, Environment, Error, Metadata, Retain, Str, View, env::use_env, extract::Extractor,
    extract::Use, handler::ViewBuilder, impl_extractor, layout::StretchAxis, raw_view,
};
use waterui_graphics::color::Color;
use waterui_text::IntoText;

pub use search::NavigationSearch;
pub use split::{NavigationSplitLayout, NavigationSplitView};

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

/// A trait for handling custom navigation actions.
/// For renderers to implement navigation handling.
pub trait CustomNavigationController: 'static {
    /// Pushes a destination builder onto the stack.
    /// Renderers that need persistent rebuild capability should override this.
    fn push_builder(&mut self, content: AnyViewBuilder<NavigationView>) {
        self.push(content.build());
    }

    /// Pushes a new navigation view onto the stack.
    /// # Arguments
    /// * `content` - The navigation view to push
    fn push(&mut self, content: NavigationView);
    /// Pops the top navigation view off the stack.
    fn pop(&mut self);
}

/// A receiver that handles navigation actions.
/// For renderers to implement navigation handling.
#[derive(Clone)]
pub struct NavigationController(Rc<RefCell<dyn CustomNavigationController>>);

impl_extractor!(NavigationController);

impl Debug for NavigationController {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NavigationController").finish()
    }
}

impl NavigationController {
    /// Creates a new navigation receiver.
    ///
    /// # Arguments
    ///
    /// * `receiver` - An implementation of `CustomNavigationController`
    pub fn new(receiver: impl CustomNavigationController) -> Self {
        Self(Rc::new(RefCell::new(receiver)))
    }

    /// Pushes a new navigation view onto the stack.
    ///
    /// # Arguments
    ///
    /// * `content` - The navigation view to push
    pub fn push(&self, content: NavigationView) {
        self.0.borrow_mut().push(content);
    }

    /// Pushes a destination builder onto the stack.
    pub fn push_builder(&self, content: AnyViewBuilder<NavigationView>) {
        self.0.borrow_mut().push_builder(content);
    }

    /// Pops the top navigation view off the stack.
    pub fn pop(&self) {
        self.0.borrow_mut().pop();
    }
}

/// Programmatic controller for a typed navigation path.
#[derive(Clone)]
pub struct NavigationPathController<T>(NavigationPath<T>);

impl<T> Debug for NavigationPathController<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NavigationPathController").finish()
    }
}

impl<T: 'static + Clone> NavigationPathController<T> {
    /// Pushes a new route value.
    pub fn push(&self, value: T) {
        self.0.push(value);
    }

    /// Pops the top route value.
    pub fn pop(&self) {
        self.0.pop();
    }

    /// Pops the top `n` route values.
    pub fn pop_n(&self, n: usize) {
        self.0.pop_n(n);
    }

    /// Clears the entire path.
    pub fn clear(&self) {
        self.0.clear();
    }
}

impl<T: 'static + Clone> Extractor for NavigationPathController<T> {
    fn extract(env: &Environment) -> Result<Self, Error> {
        <Use<Self> as Extractor>::extract(env).map(|value| value.0)
    }
}

raw_view!(NavigationView, StretchAxis::Both);

/// The display mode for the navigation bar title.
///
/// Controls how the navigation bar title is displayed, similar to SwiftUI's
/// `navigationBarTitleDisplayMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum NavigationTitleDisplayMode {
    /// System decides based on context (large on root, inline when pushed).
    #[default]
    Automatic = 0,
    /// Always use inline (small) title in the navigation bar.
    Inline = 1,
    /// Always use large title that collapses on scroll.
    Large = 2,
}

/// The transition style used by `NavigationStack` push/pop operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum NavigationTransition {
    /// Platform-style push/pop transition (default).
    #[default]
    PushPop = 0,
    /// Fade between screens.
    Fade = 1,
    /// Disable transition animation.
    None = 2,
}

/// Configuration for a navigation bar.
///
/// Represents the appearance and behavior of a navigation bar, including
/// its title, color, and visibility.
#[derive(Debug)]
pub struct Bar {
    /// The title view displayed in the navigation bar
    pub title: AnyView,
    /// Leading navigation bar content.
    pub leading: AnyView,
    /// Trailing navigation bar content.
    pub trailing: AnyView,
    /// Optional search field configuration displayed in navigation chrome.
    pub search: Option<NavigationSearch>,
    /// The background color of the navigation bar
    pub color: Computed<Color>,
    /// Whether the navigation bar is hidden
    pub hidden: Computed<bool>,
    /// The display mode for the title (automatic, inline, or large)
    pub display_mode: NavigationTitleDisplayMode,
}

impl Default for Bar {
    fn default() -> Self {
        Self {
            title: AnyView::default(),
            leading: AnyView::default(),
            trailing: AnyView::default(),
            search: None,
            // Sentinel: treat fully-transparent as "use platform default".
            // Backends can still choose to render this as transparent if they prefer.
            color: Computed::constant(Color::transparent()),
            hidden: Computed::constant(false),
            display_mode: NavigationTitleDisplayMode::Automatic,
        }
    }
}

/// A link that navigates to another view when activated.
///
/// The `NavigationLink` combines a label view with a function that creates
/// the destination view when the link is activated.
#[must_use]
#[derive(Debug)]
pub struct NavigationLink<Label, Content> {
    /// The label view displayed for this link
    pub label: Label,
    /// A function that creates the destination view when the link is activated
    pub content: Content,
}

/// A typed navigation link that pushes a route value onto a path-backed stack.
#[must_use]
#[derive(Debug)]
pub struct NavigationValueLink<Label, T> {
    /// The label view displayed for this link.
    pub label: Label,
    /// The route value pushed when the link is activated.
    pub value: T,
}

impl<Label, Content> NavigationLink<Label, Content>
where
    Label: IntoLabel + 'static,
    Content: ViewBuilder<Output = NavigationView>,
{
    /// Creates a new navigation link.
    ///
    /// # Arguments
    ///
    /// * `label` - The label view to display for the link
    /// * `content` - A function that creates the destination view
    pub const fn new(label: Label, content: Content) -> Self {
        Self { label, content }
    }
}

impl NavigationLink<(), ()> {
    /// Creates a typed value link for a path-backed navigation stack.
    pub fn value<Label, T>(label: Label, value: T) -> NavigationValueLink<Label, T>
    where
        Label: View,
        T: 'static + Clone,
    {
        NavigationValueLink { label, value }
    }
}

/// A stack of navigation views.
#[must_use]
#[derive(Debug)]
pub struct NavigationStack<T, F> {
    root: AnyView, // Renderer requires to inject `NavigationController` to the root view's environment
    path: T,
    destination: F,
    transition: NavigationTransition,
}

impl NavigationStack<(), ()> {
    /// Creates a new navigation stack with the specified root view.
    ///
    /// # Arguments
    /// * `root` - The root view of the navigation stack
    pub fn new(root: impl View) -> Self {
        Self {
            root: AnyView::new(root),
            path: (),
            destination: (),
            transition: NavigationTransition::PushPop,
        }
    }

    /// Consumes the navigation stack and returns its root view.
    pub fn into_inner(self) -> AnyView {
        self.root
    }
}

impl<T, F> NavigationStack<T, F> {
    /// Returns the configured transition style.
    #[must_use]
    pub const fn transition_style(&self) -> NavigationTransition {
        self.transition
    }

    /// Sets the transition style for push/pop operations.
    #[must_use]
    pub const fn transition(mut self, transition: NavigationTransition) -> Self {
        self.transition = transition;
        self
    }
}

impl<T> NavigationStack<NavigationPath<T>, ()> {
    /// Creates a new navigation stack with the specified navigation path and root view.
    ///
    /// # Arguments
    /// * `path` - The navigation path representing the current stack
    /// * `root` - The root view of the navigation stack
    pub fn with(path: NavigationPath<T>, root: impl View) -> Self {
        Self {
            root: AnyView::new(root),
            path,
            destination: (),
            transition: NavigationTransition::PushPop,
        }
    }

    /// Sets the destination builder for the navigation stack.
    ///
    /// # Arguments
    /// * `destination` - A function that creates a `NavigationView` from a path component
    pub fn destination<F>(self, destination: F) -> NavigationStack<NavigationPath<T>, F>
    where
        F: 'static + Fn(T) -> NavigationView,
    {
        NavigationStack {
            root: self.root,
            path: self.path,
            destination,
            transition: self.transition,
        }
    }
}

raw_view!(NavigationStack<(),()>, StretchAxis::Both);

impl<T, F> View for NavigationStack<NavigationPath<T>, F>
where
    T: 'static + Clone + PartialEq,
    F: 'static + Fn(T) -> NavigationView,
{
    fn body(self, _env: &Environment) -> impl View {
        let path: NavigationPath<T> = self.path;
        let path_controller = NavigationPathController(path.clone());
        let destination = self.destination;
        let root = self.root;
        let transition = self.transition;
        NavigationStack::new(use_env(
            move |(receiver, mut local_env): (NavigationController, Environment)| {
                let path = path.inner;
                let current_path = RefCell::new(path.snapshot());
                for component in current_path.borrow().iter().cloned() {
                    receiver.push(destination(component));
                }

                let guard = path.watch(.., move |slice| {
                    let next_path = slice.into_value().to_vec();
                    let mut current_path = current_path.borrow_mut();
                    let shared_prefix = shared_prefix_len(&current_path, &next_path);

                    for _ in shared_prefix..current_path.len() {
                        receiver.pop();
                    }

                    for item in next_path.iter().skip(shared_prefix) {
                        receiver.push(destination(item.clone()));
                    }

                    *current_path = next_path;
                });

                local_env.insert(path_controller.clone());
                Metadata::new(Metadata::new(root, Retain::new(guard)), local_env)
            },
        ))
        .transition(transition)
    }
}

/// A path representing the current navigation stack.
#[must_use]
#[derive(Debug, Clone)]
pub struct NavigationPath<T> {
    inner: List<T>,
}

impl<T: 'static> From<Vec<T>> for NavigationPath<T> {
    fn from(value: Vec<T>) -> Self {
        Self {
            inner: value.into(),
        }
    }
}

impl<T: 'static> FromIterator<T> for NavigationPath<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self {
            inner: List::from_iter(iter),
        }
    }
}

impl<T: 'static + Clone> Default for NavigationPath<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: 'static + Clone> NavigationPath<T> {
    /// Creates a new, empty navigation path.
    pub fn new() -> Self {
        Self { inner: List::new() }
    }

    /// Pushes a new item onto the navigation path.
    pub fn push(&self, value: T) {
        self.inner.push(value);
    }

    /// Pops the top item from the navigation path.
    pub fn pop(&self) {
        let _ = self.inner.pop();
    }

    /// Pops `n` items from the navigation path.
    pub fn pop_n(&self, n: usize) {
        for _ in 0..n {
            self.pop();
        }
    }

    /// Clears the entire path.
    pub fn clear(&self) {
        self.inner.clear();
    }

    /// Returns the current path length.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns whether the path is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns a cloned snapshot of the path.
    #[must_use]
    pub fn snapshot(&self) -> Vec<T> {
        self.inner.snapshot()
    }

    /// Returns an iterator over the items in the navigation path.
    pub fn iter(&self) -> impl Iterator<Item = T> {
        self.inner.iter()
    }
}

impl<Label, Content> View for NavigationLink<Label, Content>
where
    Label: IntoLabel + 'static,
    Content: ViewBuilder<Output = NavigationView>,
{
    fn body(self, env: &waterui_core::Environment) -> impl View {
        debug_assert!(
            env.get::<NavigationController>().is_some(),
            "NavigationLink used outside of a navigation context"
        );

        let destination = AnyViewBuilder::new(self.content);
        button(self.label)
            .extract::<NavigationController>()
            .action(move |receiver| receiver.push_builder(destination.clone()))
    }
}

impl<Label, T> View for NavigationValueLink<Label, T>
where
    Label: IntoLabel + 'static,
    T: 'static + Clone,
{
    fn body(self, env: &waterui_core::Environment) -> impl View {
        debug_assert!(
            env.get::<NavigationPathController<T>>().is_some(),
            "NavigationLink::value used outside of a path-backed navigation stack"
        );

        let value = self.value;
        button(self.label)
            .extract::<NavigationPathController<T>>()
            .action(move |controller| controller.push(value.clone()))
    }
}

impl NavigationView {
    /// Creates a new navigation view.
    ///
    /// # Arguments
    ///
    /// * `title` - The semantic title to display in the navigation bar
    /// * `content` - The content view to display
    pub fn new(title: impl IntoText, content: impl View) -> Self {
        let bar = Bar {
            title: AnyView::new(title.into_text()),
            ..Default::default()
        };

        Self {
            bar,
            content: AnyView::new(content),
        }
    }

    /// Sets the display mode for the navigation bar title.
    ///
    /// # Arguments
    ///
    /// * `mode` - The display mode to use
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// some_view
    ///     .title("Settings")
    ///     .navigation_bar_title_display_mode(NavigationTitleDisplayMode::Large)
    /// ```
    #[must_use]
    pub fn navigation_bar_title_display_mode(mut self, mode: NavigationTitleDisplayMode) -> Self {
        self.bar.display_mode = mode;
        self
    }

    /// Sets the display mode to inline (small title).
    #[must_use]
    pub fn inline_title(self) -> Self {
        self.navigation_bar_title_display_mode(NavigationTitleDisplayMode::Inline)
    }

    /// Sets the display mode to large title.
    #[must_use]
    pub fn large_title(self) -> Self {
        self.navigation_bar_title_display_mode(NavigationTitleDisplayMode::Large)
    }

    /// Sets leading navigation bar content.
    #[must_use]
    pub fn navigation_bar_leading(mut self, leading: impl View) -> Self {
        self.bar.leading = AnyView::new(leading);
        self
    }

    /// Sets trailing navigation bar content.
    #[must_use]
    pub fn navigation_bar_trailing(mut self, trailing: impl View) -> Self {
        self.bar.trailing = AnyView::new(trailing);
        self
    }

    /// Installs a navigation-scoped search field in the bar chrome.
    #[must_use]
    pub fn searchable(mut self, text: &Binding<Str>, prompt: impl IntoText) -> Self {
        self.bar.search = Some(NavigationSearch::new(text, prompt));
        self
    }
}

/// Convenience function to create a navigation view.
///
/// # Arguments
///
/// * `title` - The semantic title to display in the navigation bar
/// * `view` - The content view to display
pub fn navigation(title: impl IntoText, view: impl View) -> NavigationView {
    NavigationView::new(title, view)
}

fn shared_prefix_len<T: PartialEq>(left: &[T], right: &[T]) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(left, right)| left == right)
        .count()
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::shared_prefix_len;

    #[test]
    fn shared_prefix_is_entire_prefix() {
        let left = vec![1, 2, 3];
        let right = vec![1, 2, 4];
        assert_eq!(shared_prefix_len(&left, &right), 2);
    }

    #[test]
    fn shared_prefix_is_zero_when_root_changes() {
        let left = vec![1, 2];
        let right = vec![3, 4];
        assert_eq!(shared_prefix_len(&left, &right), 0);
    }

    #[test]
    fn shared_prefix_matches_complete_path() {
        let left = vec![1, 2, 3];
        let right = vec![1, 2, 3];
        assert_eq!(shared_prefix_len(&left, &right), 3);
    }
}
