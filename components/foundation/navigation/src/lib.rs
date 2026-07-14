#![no_std]

//! Navigation module for `WaterUI` framework.
//!
//! This module provides navigation components and utilities for building
//! hierarchical user interfaces with navigation bars and links.
extern crate alloc;

/// Provides search functionality for navigation.
pub mod search;
/// Split-view navigation containers.
pub mod split;
/// Tab navigation containers.
pub mod tab;

use alloc::{rc::Rc, vec::Vec};
use core::{cell::RefCell, fmt::Debug};

use nami::{
    Binding, Computed, SignalExt as _,
    collection::{Collection, List},
};
use waterui_controls::{IntoLabel, button};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{
    AnyView, Environment, Error, IntoSignal, Metadata, Native, NativeView, Retain, Str, View,
    env::use_env, extract::Extractor, extract::Use, flatten_signal, handler::ViewBuilder,
    impl_extractor, layout::StretchAxis, raw_view,
};
use waterui_graphics::color::{Color, ResolvedColor};
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
pub struct NavigationController {
    receiver: Rc<RefCell<dyn CustomNavigationController>>,
    retained: Rc<RefCell<Option<Retain>>>,
    retained_environment: Rc<RefCell<Option<Environment>>>,
}

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
        Self {
            receiver: Rc::new(RefCell::new(receiver)),
            retained: Rc::new(RefCell::new(None)),
            retained_environment: Rc::new(RefCell::new(None)),
        }
    }

    /// Pushes a new navigation view onto the stack.
    ///
    /// # Arguments
    ///
    /// * `content` - The navigation view to push
    pub fn push(&self, content: NavigationView) {
        let content = self.with_retained_environment(content);
        self.receiver.borrow_mut().push(content);
    }

    /// Pushes a destination builder onto the stack.
    pub fn push_builder(&self, content: AnyViewBuilder<NavigationView>) {
        let content = self.with_retained_environment_builder(content);
        self.receiver.borrow_mut().push_builder(content);
    }

    /// Pops the top navigation view off the stack.
    pub fn pop(&self) {
        self.receiver.borrow_mut().pop();
    }

    /// Replaces the controller-scoped retained value.
    ///
    /// Path-backed navigation stacks use this to keep their path watcher alive
    /// while destinations are active and the root view is no longer rendered.
    pub fn retain(&self, retained: Retain) {
        *self.retained.borrow_mut() = Some(retained);
    }

    /// Replaces the controller-scoped environment.
    pub fn retain_environment(&self, env: Environment) {
        *self.retained_environment.borrow_mut() = Some(env);
    }

    /// Returns the controller-scoped environment.
    #[must_use]
    pub fn retained_environment(&self) -> Option<Environment> {
        self.retained_environment.borrow().clone()
    }

    fn with_retained_environment(&self, content: NavigationView) -> NavigationView {
        if let Some(env) = self.retained_environment() {
            navigation_view_with_environment(content, &env)
        } else {
            content
        }
    }

    fn with_retained_environment_builder(
        &self,
        content: AnyViewBuilder<NavigationView>,
    ) -> AnyViewBuilder<NavigationView> {
        if let Some(env) = self.retained_environment() {
            AnyViewBuilder::new(move || navigation_view_with_environment(content.build(), &env))
        } else {
            content
        }
    }
}

fn navigation_slot_with_environment(content: AnyView, env: &Environment) -> AnyView {
    AnyView::new(Metadata::new(content, env.clone()))
}

fn navigation_view_with_environment(
    mut content: NavigationView,
    env: &Environment,
) -> NavigationView {
    content.resolve_native_fields(env);
    content.bar.title = navigation_slot_with_environment(content.bar.title, env);
    content.bar.leading = navigation_slot_with_environment(content.bar.leading, env);
    content.bar.trailing = navigation_slot_with_environment(content.bar.trailing, env);
    content.content = navigation_slot_with_environment(content.content, env);
    content
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

impl NativeView for NavigationView {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

impl View for NavigationView {
    fn body(mut self, env: &Environment) -> impl View {
        self.resolve_native_fields(env);
        Native::new(self)
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// The display mode for the navigation bar title.
///
/// Controls how the navigation bar title is displayed, similar to `SwiftUI`'s
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
    pub color: Option<Computed<Color>>,
    /// Bar color resolved against the effective environment for native backends.
    #[doc(hidden)]
    pub resolved_color: Option<Computed<ResolvedColor>>,
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
            color: None,
            resolved_color: None,
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
    pub const fn value<Label, T>(label: Label, value: T) -> NavigationValueLink<Label, T>
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

struct NavigationPathSubscriptionState<T> {
    current: Option<Vec<T>>,
    pending: Vec<Vec<T>>,
}

fn reconcile_navigation_path<T, F>(
    receiver: &NavigationController,
    destination: &Rc<F>,
    current_path: &mut Vec<T>,
    next_path: Vec<T>,
) where
    T: 'static + Clone + PartialEq,
    F: 'static + Fn(T) -> NavigationView,
{
    let shared_prefix = shared_prefix_len(current_path, &next_path);

    for _ in shared_prefix..current_path.len() {
        receiver.pop();
    }

    for item in next_path.iter().skip(shared_prefix) {
        receiver.push_builder(path_destination_builder(
            Rc::clone(destination),
            item.clone(),
        ));
    }

    *current_path = next_path;
}

fn navigation_path_snapshot<C>(path: &C) -> Vec<C::Item>
where
    C: Collection,
{
    (0..path.len())
        .map(|index| {
            path.get(index)
                .expect("navigation path must contain every item within its reported length")
        })
        .collect()
}

fn subscribe_navigation_path<C, T, F>(
    path: &C,
    receiver: &NavigationController,
    destination: &Rc<F>,
) -> C::Guard
where
    C: Collection<Item = T>,
    T: 'static + Clone + PartialEq,
    F: 'static + Fn(T) -> NavigationView,
{
    let state = Rc::new(RefCell::new(NavigationPathSubscriptionState {
        current: None,
        pending: Vec::new(),
    }));
    let guard = path.watch(.., {
        let state = Rc::clone(&state);
        let receiver = receiver.clone();
        let destination = Rc::clone(destination);
        move |slice| {
            let next_path = slice.into_value().to_vec();
            let mut state = state.borrow_mut();
            if let Some(current_path) = state.current.as_mut() {
                reconcile_navigation_path(&receiver, &destination, current_path, next_path);
            } else {
                state.pending.push(next_path);
            }
        }
    });

    let snapshot = navigation_path_snapshot(path);
    let pending = core::mem::take(&mut state.borrow_mut().pending);
    let mut pending = pending.into_iter();
    let mut current_path = pending.next().unwrap_or_else(|| snapshot.clone());

    for component in current_path.iter().cloned() {
        receiver.push_builder(path_destination_builder(Rc::clone(destination), component));
    }
    for next_path in pending {
        reconcile_navigation_path(receiver, destination, &mut current_path, next_path);
    }
    reconcile_navigation_path(receiver, destination, &mut current_path, snapshot);
    state.borrow_mut().current = Some(current_path);

    guard
}

impl<T, F> View for NavigationStack<NavigationPath<T>, F>
where
    T: 'static + Clone + PartialEq,
    F: 'static + Fn(T) -> NavigationView,
{
    fn body(self, _env: &Environment) -> impl View {
        let path: NavigationPath<T> = self.path;
        let path_controller = NavigationPathController(path.clone());
        let destination = Rc::new(self.destination);
        let root = self.root;
        let transition = self.transition;
        NavigationStack::new(use_env(
            move |(receiver, mut local_env): (NavigationController, Environment)| {
                let path = path.inner;
                local_env.insert(path_controller.clone());
                receiver.retain_environment(local_env.clone());
                let guard = subscribe_navigation_path(&path, &receiver, &destination);

                receiver.retain(Retain::new(guard));
                Metadata::new(root, local_env)
            },
        ))
        .transition(transition)
    }
}

fn path_destination_builder<T, F>(
    destination: Rc<F>,
    component: T,
) -> AnyViewBuilder<NavigationView>
where
    T: 'static + Clone,
    F: 'static + Fn(T) -> NavigationView,
{
    AnyViewBuilder::new(move || destination(component.clone()))
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
        button(self.label).action(move |receiver: NavigationController| {
            receiver.push_builder(destination.clone());
        })
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
            .action(move |controller: NavigationPathController<T>| controller.push(value.clone()))
    }
}

impl NavigationView {
    /// Resolves environment-dependent native fields without snapshotting signals.
    #[doc(hidden)]
    pub fn resolve_native_fields(&mut self, env: &Environment) {
        if let Some(search) = &mut self.bar.search {
            search.prompt = waterui_text::Text::computed(search.prompt.resolve(env).content);
        }

        self.bar.resolved_color = self.bar.color.as_ref().map(|color| {
            let env = env.clone();
            flatten_signal(color.clone().map(move |color| color.resolve(&env)))
        });
    }

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
    pub const fn navigation_bar_title_display_mode(
        mut self,
        mode: NavigationTitleDisplayMode,
    ) -> Self {
        self.bar.display_mode = mode;
        self
    }

    /// Sets the display mode to inline (small title).
    pub const fn inline_title(self) -> Self {
        self.navigation_bar_title_display_mode(NavigationTitleDisplayMode::Inline)
    }

    /// Sets the display mode to large title.
    pub const fn large_title(self) -> Self {
        self.navigation_bar_title_display_mode(NavigationTitleDisplayMode::Large)
    }

    /// Sets leading navigation bar content.
    pub fn navigation_bar_leading(mut self, leading: impl View) -> Self {
        self.bar.leading = AnyView::new(leading);
        self
    }

    /// Sets trailing navigation bar content.
    pub fn navigation_bar_trailing(mut self, trailing: impl View) -> Self {
        self.bar.trailing = AnyView::new(trailing);
        self
    }

    /// Installs a navigation-scoped search field in the bar chrome.
    pub fn searchable(mut self, text: &Binding<Str>, prompt: impl IntoText) -> Self {
        self.bar.search = Some(NavigationSearch::new(text, prompt));
        self
    }

    /// Overrides the platform navigation bar surface color.
    ///
    /// Without this modifier, each backend uses the surrounding `Surface`
    /// theme token and its native material treatment.
    pub fn navigation_bar_color(mut self, color: impl IntoSignal<Color> + 'static) -> Self {
        self.bar.color = Some(color.into_signal().computed());
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
    use alloc::rc::Rc;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::{
        cell::{Cell, RefCell},
        ops::RangeBounds,
    };

    use nami::{collection::Collection, watcher::Context};

    use super::{
        CustomNavigationController, NavigationController, NavigationLink, NavigationPath,
        NavigationPathController, NavigationView, shared_prefix_len, subscribe_navigation_path,
    };
    use waterui_core::{Environment, Metadata, handler::AnyViewBuilder};

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

    struct EmitsDuringSubscription {
        subscribed: Rc<Cell<bool>>,
        snapshot: Vec<u8>,
        updates: Vec<Vec<u8>>,
    }

    impl Collection for EmitsDuringSubscription {
        type Item = u8;
        type Guard = ();

        fn get(&self, index: usize) -> Option<Self::Item> {
            assert!(
                self.subscribed.get(),
                "navigation path must subscribe before reading its snapshot"
            );
            self.snapshot.as_slice().get(index).copied()
        }

        fn len(&self) -> usize {
            assert!(
                self.subscribed.get(),
                "navigation path must subscribe before reading its snapshot"
            );
            self.snapshot.len()
        }

        fn watch(
            &self,
            _range: impl RangeBounds<usize>,
            watcher: impl for<'a> Fn(Context<&'a [Self::Item]>) + 'static,
        ) -> Self::Guard {
            self.subscribed.set(true);
            for update in &self.updates {
                watcher(Context::from(update.as_slice()));
            }
        }
    }

    struct CountingNavigationController {
        pushes: Rc<Cell<usize>>,
        pops: Rc<Cell<usize>>,
    }

    impl CustomNavigationController for CountingNavigationController {
        fn push(&mut self, _content: NavigationView) {
            self.pushes.set(self.pushes.get() + 1);
        }

        fn pop(&mut self) {
            self.pops.set(self.pops.get() + 1);
        }
    }

    #[test]
    fn path_subscribes_before_snapshot_and_replays_registration_updates() {
        let subscribed = Rc::new(Cell::new(false));
        let path = EmitsDuringSubscription {
            subscribed: Rc::clone(&subscribed),
            snapshot: vec![1, 2],
            updates: vec![vec![1], vec![1, 2]],
        };
        let pushes = Rc::new(Cell::new(0));
        let pops = Rc::new(Cell::new(0));
        let controller = NavigationController::new(CountingNavigationController {
            pushes: Rc::clone(&pushes),
            pops: Rc::clone(&pops),
        });
        let destination = Rc::new(|_: u8| NavigationView::new("Route", ()));

        subscribe_navigation_path(&path, &controller, &destination);

        assert!(subscribed.get());
        assert_eq!(pushes.get(), 2);
        assert_eq!(pops.get(), 0);
    }

    #[derive(Clone, PartialEq, Eq)]
    enum TestRoute {
        Second,
    }

    struct BaseMarker;

    struct RecordingNavigationController {
        pushed: Rc<RefCell<Option<NavigationView>>>,
    }

    impl CustomNavigationController for RecordingNavigationController {
        fn push(&mut self, content: NavigationView) {
            *self.pushed.borrow_mut() = Some(content);
        }

        fn pop(&mut self) {}
    }

    #[test]
    fn retained_navigation_environment_is_layered_on_pushed_content() {
        let pushed = Rc::new(RefCell::new(None));
        let controller = NavigationController::new(RecordingNavigationController {
            pushed: Rc::clone(&pushed),
        });

        let mut base_env = Environment::new();
        base_env.insert(BaseMarker);
        let mut retained_env = base_env.clone();
        retained_env.insert(NavigationPathController(NavigationPath::<TestRoute>::new()));
        controller.retain_environment(retained_env);

        controller.push_builder(AnyViewBuilder::new(|| {
            NavigationView::new(
                "First",
                NavigationLink::value("Open Second", TestRoute::Second),
            )
        }));

        let nav_view = pushed
            .borrow_mut()
            .take()
            .expect("pushed navigation view should be recorded");
        let metadata = nav_view
            .content
            .downcast::<Metadata<Environment>>()
            .expect("pushed content should carry retained navigation environment");

        assert!(metadata.value.get::<BaseMarker>().is_some());
        assert!(
            metadata
                .value
                .get::<NavigationPathController<TestRoute>>()
                .is_some()
        );
    }
}
