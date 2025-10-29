#![no_std]

//! Navigation module for `WaterUI` framework.
//!
//! This module provides navigation components and utilities for building
//! hierarchical user interfaces with navigation bars and links.
extern crate alloc;

/// Provides search functionality for navigation.
pub mod search;
pub mod tab;

use alloc::{rc::Rc, vec::Vec};
use core::{
    cell::{Cell, RefCell},
    fmt::Debug,
};

use nami::{
    Computed,
    collection::{Collection, List},
};
use waterui_color::Color;
use waterui_controls::button;
use waterui_core::{
    AnyView, Environment, IgnorableMetadata, View, env::use_env, handler::ViewBuilder,
    impl_extractor, raw_view,
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

/// A trait for handling custom navigation actions.
/// For renderers to implement navigation handling.
pub trait CustomNavigationController: 'static {
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
    /// Pops the top navigation view off the stack.
    pub fn pop(&self) {
        self.0.borrow_mut().pop();
    }
}

raw_view!(
    NavigationView,
    "Please use `NavigationView` in a proper navigation context"
);

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
impl<Label, Content> NavigationLink<Label, Content>
where
    Label: View,
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

/// A stack of navigation views.
#[must_use]
#[derive(Debug)]
pub struct NavigationStack<T, F> {
    root: AnyView, // Renderer requires to inject `NavigationController` to the root view's environment
    path: T,
    destination: F,
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
        }
    }

    /// Consumes the navigation stack and returns its root view.
    pub fn into_inner(self) -> AnyView {
        self.root
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
        }
    }
}

raw_view!(NavigationStack<(),()>);

impl<T, F> View for NavigationStack<NavigationPath<T>, F>
where
    T: 'static + Clone + View,
    F: 'static + Fn(T) -> NavigationView,
{
    fn body(self, _env: &Environment) -> impl View {
        let path: NavigationPath<T> = self.path;
        let destination = self.destination;
        let root = self.root;
        NavigationStack::new(use_env(move |receiver: NavigationController| {
            let path = path.inner;
            for component in &path {
                receiver.push(destination(component));
            }

            let old_len = Cell::new(path.len());
            #[allow(clippy::cast_possible_wrap)]
            let guard = path.watch(.., move |slice| {
                // list is a stack, only pop or push. So we only watch its length change
                let slice = slice.into_value();
                let len = slice.len();
                let change = len as isize - old_len.get() as isize;
                if change > 0 {
                    // length increase, it has been pushed
                    for item in slice.iter().skip(old_len.get()).take(len - old_len.get()) {
                        receiver.push(destination(item.clone()));
                    }
                }
                #[allow(clippy::cast_sign_loss)]
                if change < 0 {
                    //length decrease, it has been popped
                    let pop_count = (-change) as usize;
                    for _ in 0..pop_count {
                        receiver.pop();
                    }
                }
                old_len.set(len);
            });

            IgnorableMetadata::new(root, guard)
        }))
    }
}

/// A path representing the current navigation stack.
#[must_use]
#[derive(Debug)]
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
    pub fn push(&mut self, value: T) {
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

    /// Returns an iterator over the items in the navigation path.
    pub fn iter(&self) -> impl Iterator<Item = T> {
        self.inner.iter()
    }
}

impl<Label, Content> View for NavigationLink<Label, Content>
where
    Label: View,
    Content: ViewBuilder<Output = NavigationView>,
{
    fn body(self, env: &waterui_core::Environment) -> impl View {
        debug_assert!(
            env.get::<NavigationController>().is_some(),
            "NavigationLink used outside of a navigation context"
        );

        button(self.label).action(move |receiver: NavigationController| {
            let content = (self.content).build();
            receiver.push(content);
        })
    }
}

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
