#![no_std]

//! Navigation module for `WaterUI` framework.
//!
//! This module provides navigation components and utilities for building
//! hierarchical user interfaces with navigation bars and links.
extern crate alloc;

mod path;
mod router;
/// Provides search functionality for navigation.
pub mod search;
/// Split-view navigation containers.
pub mod split;
/// Tab navigation containers.
pub mod tab;
mod transition;

use alloc::{collections::BTreeMap, rc::Rc, vec, vec::Vec};
use core::{
    any::TypeId,
    cell::{Cell, RefCell},
    fmt::Debug,
};

use nami::{Binding, Computed, Signal as _, SignalExt as _};
use waterui_controls::{ButtonStyle, IntoLabel, button};
use waterui_core::handler::{AnyViewBuilder, BoxedAction, Handler, boxed_action};
use waterui_core::{
    AnyView, Environment, Error, IgnorableMetadata, IntoSignal, Metadata, Native, NativeView,
    Retain, Str, View, env::use_env, extract::Extractor, extract::Use, flatten_signal,
    handler::ViewBuilder, impl_extractor, layout::StretchAxis, metadata::MetadataKey, raw_view,
};
use waterui_graphics::color::{Color, ResolvedColor};
use waterui_icon::SystemIcon;
use waterui_text::IntoText;
use waterui_text::Text;

pub use path::{
    ErasedNavigationRoute, NavigationPath, NavigationPathReplacement, RestorableNavigationRoute,
};
pub use router::NavigationRouter;
pub use search::NavigationSearch;
pub use split::{
    ColumnWidth, NativeNavigationSplitStyle, NavigationSplitColumnVisibility,
    NavigationSplitLayout, NavigationSplitStyle, NavigationSplitView, split_style,
};
pub use tab::{Tab, Tabs, TabsLayout, tab_style};
pub use transition::{
    AnyNavigationTransition, NativeNavigationTransition, NavigationTransition,
    NavigationTransitionDestination, NavigationTransitionDirection, NavigationTransitionFrame,
    NavigationTransitionLayer, NavigationTransitionSource, NavigationTransitionViewExt,
    RetainedNavigationTransition, navigation_transition,
};

/// A view that combines a navigation bar with content.
///
/// The `NavigationView` contains a navigation bar with a title and other
/// configuration options, along with the actual content to display.
#[must_use]
pub struct NavigationView {
    /// The navigation bar for this view
    pub bar: Bar,
    /// The content to display in this view
    pub content: AnyView,
    /// Reactive and callback state associated with this destination.
    #[doc(hidden)]
    pub state: NavigationDestinationState,
}

impl Debug for NavigationView {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NavigationView")
            .field("bar", &self.bar)
            .field("content", &self.content)
            .field("state", &self.state)
            .finish()
    }
}

/// Reactive policy and lifecycle handlers attached to one destination.
#[doc(hidden)]
pub struct NavigationDestinationState {
    /// Whether a user or system initiated pop may begin.
    pub pop_enabled: Computed<bool>,
    /// Called for a real user/system pop request, including a denied request.
    pub pop_attempted: Option<BoxedAction>,
    /// Called after the destination becomes active.
    pub appear: Option<BoxedAction>,
    /// Called after the destination ceases to be active.
    pub disappear: Option<BoxedAction>,
    /// Called only after a completed pop removes the destination.
    pub pop: Option<BoxedAction>,
}

impl Debug for NavigationDestinationState {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NavigationDestinationState")
            .field("pop_enabled", &self.pop_enabled)
            .field("has_pop_attempted", &self.pop_attempted.is_some())
            .field("has_appear", &self.appear.is_some())
            .field("has_disappear", &self.disappear.is_some())
            .field("has_pop", &self.pop.is_some())
            .finish()
    }
}

impl Default for NavigationDestinationState {
    fn default() -> Self {
        Self {
            pop_enabled: Computed::constant(true),
            pop_attempted: None,
            appear: None,
            disappear: None,
            pop: None,
        }
    }
}

impl NavigationDestinationState {
    /// Reports one user/system pop attempt and returns whether it may start.
    #[doc(hidden)]
    pub fn attempt_pop(&mut self, env: &Environment) -> bool {
        if let Some(handler) = &mut self.pop_attempted {
            handler(env);
        }
        self.pop_enabled.get()
    }

    /// Reports that the destination became active.
    #[doc(hidden)]
    pub fn appeared(&mut self, env: &Environment) {
        if let Some(handler) = &mut self.appear {
            handler(env);
        }
    }

    /// Reports that the destination ceased to be active.
    #[doc(hidden)]
    pub fn disappeared(&mut self, env: &Environment) {
        if let Some(handler) = &mut self.disappear {
            handler(env);
        }
    }

    /// Reports that a completed pop removed the destination.
    #[doc(hidden)]
    pub fn popped(&mut self, env: &Environment) {
        if let Some(handler) = &mut self.pop {
            handler(env);
        }
    }
}

/// Monotonically increasing identifier scoped to one navigation stack.
pub type NavigationTransactionId = u64;

type PathPopHandler = Rc<dyn Fn(usize)>;

/// One atomic change projected from navigation state into a backend stack.
///
/// A transaction always replaces a suffix: the first [`Self::retained_prefix`]
/// entries stay untouched, the [`Self::removed`] entries above them are
/// discarded, and [`Self::inserted`] is appended in their place.
pub struct NavigationTransaction {
    /// Stack-scoped identifier acknowledged by
    /// [`NavigationController::transition_completed`] or
    /// [`NavigationController::transition_cancelled`].
    pub id: NavigationTransactionId,
    /// Number of destination entries retained from the root.
    pub retained_prefix: usize,
    /// Number of entries removed after the retained prefix.
    pub removed: usize,
    /// Destination builders inserted after the retained prefix.
    pub inserted: Vec<AnyViewBuilder<NavigationView>>,
}

impl Debug for NavigationTransaction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NavigationTransaction")
            .field("id", &self.id)
            .field("retained_prefix", &self.retained_prefix)
            .field("removed", &self.removed)
            .field("inserted", &self.inserted.len())
            .finish()
    }
}

/// A trait for handling custom navigation actions.
/// For renderers to implement navigation handling.
pub trait CustomNavigationController: 'static {
    /// Applies one complete stack mutation.
    fn apply(&mut self, transaction: NavigationTransaction);
}

/// Shared interior state of one [`NavigationController`].
///
/// The projected stack is split at [`Self::path_depth`]: entries below it are
/// owned by an explicit [`NavigationPath`] and may only be removed by mutating
/// that path, while entries above it were pushed by a destination-building
/// [`NavigationLink`] and are owned by the controller alone. Keeping the split
/// explicit is what lets both link kinds coexist in one stack.
struct NavigationControllerState {
    receiver: RefCell<alloc::boxed::Box<dyn CustomNavigationController>>,
    retained: RefCell<Option<Retain>>,
    retained_environment: RefCell<Option<Environment>>,
    depth: Cell<usize>,
    path_depth: Cell<usize>,
    next_transaction_id: Cell<NavigationTransactionId>,
    pending_transaction_id: Cell<Option<NavigationTransactionId>>,
    path_pop_handler: RefCell<Option<PathPopHandler>>,
    native_applied_pops: Cell<usize>,
}

/// A receiver that handles navigation actions.
/// For renderers to implement navigation handling.
#[derive(Clone)]
pub struct NavigationController {
    state: Rc<NavigationControllerState>,
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
            state: Rc::new(NavigationControllerState {
                receiver: RefCell::new(alloc::boxed::Box::new(receiver)),
                retained: RefCell::new(None),
                retained_environment: RefCell::new(None),
                depth: Cell::new(0),
                path_depth: Cell::new(0),
                next_transaction_id: Cell::new(1),
                pending_transaction_id: Cell::new(None),
                path_pop_handler: RefCell::new(None),
                native_applied_pops: Cell::new(0),
            }),
        }
    }

    /// Number of entries above the explicit path, pushed by destination links.
    fn detached_depth(&self) -> usize {
        self.state
            .depth
            .get()
            .checked_sub(self.state.path_depth.get())
            .expect("navigation path depth cannot exceed the projected stack depth")
    }

    /// Pushes a destination builder onto the stack.
    ///
    /// The entry is owned by the controller rather than by any explicit path, so
    /// it pops without disturbing route state and is discarded when the path
    /// underneath it changes.
    pub fn push_builder(&self, content: AnyViewBuilder<NavigationView>) {
        let content = self.with_retained_environment_builder(content);
        let depth = self.state.depth.get();
        self.state.depth.set(depth + 1);
        self.commit(depth, 0, vec![content]);
    }

    /// Pops the top navigation view off the stack.
    pub fn pop(&self) {
        if self.state.depth.get() != 0 {
            self.request_pop(1);
        }
    }

    /// Projects a diffed explicit-path mutation as one backend transaction.
    ///
    /// Every entry above `retained_prefix` is removed, including link-pushed
    /// entries stacked on top of the path: a route change invalidates whatever
    /// was opened from the routes it replaced.
    ///
    /// # Panics
    ///
    /// Panics if `retained_prefix` exceeds the current path depth, or if the
    /// transaction identifier overflows.
    fn apply_path(&self, retained_prefix: usize, inserted: Vec<AnyViewBuilder<NavigationView>>) {
        let path_depth = self.state.path_depth.get();
        assert!(
            retained_prefix <= path_depth,
            "navigation path transaction retains {retained_prefix} routes but the path holds {path_depth}"
        );
        let removed = self
            .state
            .depth
            .get()
            .checked_sub(retained_prefix)
            .expect("navigation path prefix cannot exceed the projected stack depth");
        let next_depth = retained_prefix + inserted.len();
        self.state.path_depth.set(next_depth);
        self.state.depth.set(next_depth);
        self.commit(retained_prefix, removed, inserted);
    }

    /// Emits one suffix-replacing transaction, skipping work the native
    /// container already performed for an interactive pop.
    fn commit(
        &self,
        retained_prefix: usize,
        removed: usize,
        inserted: Vec<AnyViewBuilder<NavigationView>>,
    ) {
        // An interactive native pop already removed entries from the platform
        // stack; this transaction only has to cover what is left.
        let native_applied = self.state.native_applied_pops.get();
        let settled = native_applied.min(removed);
        self.state.native_applied_pops.set(native_applied - settled);
        let removed = removed - settled;

        if removed == 0 && inserted.is_empty() {
            return;
        }

        let id = self.state.next_transaction_id.get();
        self.state.next_transaction_id.set(
            id.checked_add(1)
                .expect("navigation transaction identifier overflowed"),
        );
        self.state.pending_transaction_id.set(Some(id));
        self.state
            .receiver
            .borrow_mut()
            .apply(NavigationTransaction {
                id,
                retained_prefix,
                removed,
                inserted,
            });
    }

    /// Records successful completion of the current backend transaction.
    ///
    /// Returns `false` for a stale completion superseded by a newer transaction.
    #[must_use]
    pub fn transition_completed(&self, id: NavigationTransactionId) -> bool {
        if self.state.pending_transaction_id.get() != Some(id) {
            return false;
        }
        self.state.pending_transaction_id.set(None);
        true
    }

    /// Records cancellation of the current backend transaction.
    ///
    /// Returns `false` for a stale cancellation superseded by a newer transaction.
    #[must_use]
    pub fn transition_cancelled(&self, id: NavigationTransactionId) -> bool {
        if self.state.pending_transaction_id.get() != Some(id) {
            return false;
        }
        self.state.pending_transaction_id.set(None);
        true
    }

    /// Installs the mutation used to commit a native pop into an explicit path.
    ///
    /// # Panics
    ///
    /// Panics if a path-pop handler is already installed on this controller.
    pub fn install_path_pop_handler(&self, handler: impl Fn(usize) + 'static) {
        let previous = self.state.path_pop_handler.replace(Some(Rc::new(handler)));
        assert!(
            previous.is_none(),
            "a navigation controller cannot drive more than one explicit path"
        );
    }

    /// Splits `count` pops into the link-pushed entries the controller owns and
    /// the remainder that belongs to the explicit path.
    ///
    /// A request, not an instruction: asking to pop more than the path holds
    /// pops what there is. A backend's idea of the depth can lag the path's —
    /// its back button is drawn from a stack it maintains itself — and a user
    /// pressing back is not a reason to take the process down. The invariants
    /// on [`NavigationPath::pop_n`] still hold for callers that mutate the path
    /// directly, where an over-long pop really is a programming error.
    ///
    /// # Panics
    ///
    /// Panics when `count` is zero.
    fn split_pop(&self, count: usize) -> (usize, usize) {
        assert!(count != 0, "navigation pop count must be non-zero");
        let count = count.min(self.state.depth.get());
        let detached = self.detached_depth().min(count);
        (detached, count - detached)
    }

    /// Returns the installed path-pop handler, which must exist whenever the
    /// projected stack still holds path-owned entries.
    fn path_pop_handler(&self) -> PathPopHandler {
        self.state
            .path_pop_handler
            .borrow()
            .clone()
            .expect("path-owned navigation entries require an installed path handler")
    }

    /// Requests a user-initiated pop before a native transition begins.
    ///
    /// Link-pushed entries pop directly; the rest is delegated to the explicit
    /// path so route state stays authoritative. Asking to pop more than the
    /// projected stack holds pops what there is.
    ///
    /// # Panics
    ///
    /// Panics when `count` is zero.
    pub fn request_pop(&self, count: usize) {
        let (detached, from_path) = self.split_pop(count);
        if detached != 0 {
            let retained = self.state.depth.get() - detached;
            self.state.depth.set(retained);
            self.commit(retained, detached, Vec::new());
        }
        if from_path != 0 {
            self.path_pop_handler()(from_path);
        }
    }

    /// Commits a pop that the native container has already completed.
    ///
    /// # Panics
    ///
    /// Panics when `count` is zero, exceeds the projected stack depth, or when
    /// the explicit path fails to absorb the pop in the same update.
    pub fn complete_native_pop(&self, count: usize) {
        let (detached, from_path) = self.split_pop(count);
        if detached != 0 {
            // The native container already removed these; only the projection
            // has to catch up, with no transaction sent back.
            self.state.depth.set(self.state.depth.get() - detached);
        }
        if from_path != 0 {
            let handler = self.path_pop_handler();
            self.state.native_applied_pops.set(from_path);
            handler(from_path);
            assert_eq!(
                self.state.native_applied_pops.get(),
                0,
                "a native navigation pop must be absorbed by the explicit path in the same update, \
                 but the path did not shrink by {from_path}"
            );
        }
    }

    /// Replaces the controller-scoped retained value.
    ///
    /// Path-backed navigation stacks use this to keep their path watcher alive
    /// while destinations are active and the root view is no longer rendered.
    pub fn retain(&self, retained: Retain) {
        *self.state.retained.borrow_mut() = Some(retained);
    }

    /// Replaces the controller-scoped environment.
    pub fn retain_environment(&self, env: Environment) {
        *self.state.retained_environment.borrow_mut() = Some(env);
    }

    /// Returns the controller-scoped environment.
    #[must_use]
    pub fn retained_environment(&self) -> Option<Environment> {
        self.state.retained_environment.borrow().clone()
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
    content.bar.subtitle = navigation_slot_with_environment(content.bar.subtitle, env);
    for item in &mut content.bar.toolbar.items {
        item.content = navigation_slot_with_environment(core::mem::take(&mut item.content), env);
    }
    content.content = navigation_slot_with_environment(content.content, env);
    content
}

/// Resolves a stack's deferred root after its backend controller is installed.
///
/// Path-backed stacks defer their root so the `Navigator` and retained path
/// subscription can be installed in the controller-scoped environment. Backend
/// implementations must use this function before projecting the root into their
/// native navigation container.
#[doc(hidden)]
pub fn resolve_navigation_root(mut root: AnyView, env: &Environment) -> NavigationView {
    let mut resolved_env = env.clone();
    loop {
        if root.is::<NavigationView>() {
            let root = *root
                .downcast::<NavigationView>()
                .expect("navigation root type must remain stable during downcast");
            return navigation_view_with_environment(root, &resolved_env);
        }
        if root.is::<Native<NavigationView>>() {
            let root = root
                .downcast::<Native<NavigationView>>()
                .expect("native navigation root type must remain stable during downcast")
                .into_inner();
            return navigation_view_with_environment(root, &resolved_env);
        }
        if root.is::<Metadata<Environment>>() {
            let metadata = *root
                .downcast::<Metadata<Environment>>()
                .expect("navigation environment metadata type must remain stable during downcast");
            resolved_env = metadata.value;
            root = metadata.content;
            continue;
        }
        root = AnyView::new(View::body(root, &resolved_env));
    }
}

/// Programmatic controller for the nearest typed navigation stack.
#[derive(Clone)]
pub struct Navigator<T: 'static>(NavigationPath<T>);

impl<T: 'static> Debug for Navigator<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Navigator").finish_non_exhaustive()
    }
}

impl<T: 'static + Clone + PartialEq> Navigator<T> {
    /// Pushes a new route value.
    pub fn push(&self, value: T) {
        self.0.push(value);
    }

    /// Pops the top route value and returns it, if the path was not already at
    /// its root.
    #[expect(
        clippy::must_use_candidate,
        reason = "pop is called for its effect; the popped route is a convenience, and requiring `let _ =` at every back action is the wart this signature exists to avoid"
    )]
    pub fn pop(&self) -> Option<T> {
        self.0.pop()
    }

    /// Pops the top `n` route values.
    pub fn pop_n(&self, n: usize) {
        self.0.pop_n(n);
    }

    /// Clears the entire path.
    pub fn clear(&self) {
        self.0.clear();
    }

    /// Pops every route and returns to the stack root.
    pub fn pop_to_root(&self) {
        self.clear();
    }

    /// Replaces the complete path in one navigation transaction.
    pub fn replace(&self, routes: impl IntoIterator<Item = T>) {
        self.0.replace(routes);
    }
}

impl<T: 'static + Clone + PartialEq> Extractor for Navigator<T> {
    fn extract(env: &Environment) -> Result<Self, Error> {
        <Use<Self> as Extractor>::extract(env).map(|value| value.0)
    }
}

impl Navigator<ErasedNavigationRoute> {
    /// Pushes a concrete route onto a heterogeneous path.
    pub fn push<R>(&self, route: R)
    where
        R: Clone + PartialEq + 'static,
    {
        self.0.push(route);
    }

    /// Pops the top heterogeneous route, reporting whether one was present.
    #[expect(
        clippy::must_use_candidate,
        reason = "pop is called for its effect; the popped route is a convenience, and requiring `let _ =` at every back action is the wart this signature exists to avoid"
    )]
    pub fn pop(&self) -> bool {
        self.0.pop()
    }

    /// Pops the top `count` heterogeneous routes.
    pub fn pop_n(&self, count: usize) {
        self.0.pop_n(count);
    }

    /// Clears the heterogeneous path.
    pub fn clear(&self) {
        self.0.clear();
    }

    /// Pops every route and returns to the stack root.
    pub fn pop_to_root(&self) {
        self.clear();
    }

    /// Atomically replaces a heterogeneous path.
    pub fn replace(&self, replacement: impl FnOnce(&mut NavigationPathReplacement)) {
        self.0.replace(replacement);
    }
}

impl Extractor for Navigator<ErasedNavigationRoute> {
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
    /// Always use a medium title, between inline and large.
    ///
    /// Material's flexible app bars come in two heights above the small one;
    /// this is the shorter, where [`Large`](Self::Large) is the taller.
    Medium = 2,
    /// Always use large title that collapses on scroll.
    Large = 3,
}

/// Semantic placement for one native navigation toolbar item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NavigationToolbarPlacement {
    /// Principal title-area content.
    Principal = 0,
    /// Primary destination action.
    PrimaryAction = 1,
    /// Secondary destination action.
    SecondaryAction = 2,
    /// Confirmation action in an editing flow.
    Confirmation = 3,
    /// Cancellation action in an editing flow.
    Cancellation = 4,
    /// Bottom toolbar content.
    BottomBar = 5,
    /// Read-only status content.
    Status = 6,
    /// Explicit leading content in the top navigation bar.
    TopBarLeading = 7,
    /// Explicit trailing content in the top navigation bar.
    TopBarTrailing = 8,
}

/// One semantic native toolbar item.
#[must_use]
pub struct NavigationToolbarItem {
    /// Semantic placement resolved by each platform backend.
    pub placement: NavigationToolbarPlacement,
    /// Item content.
    pub content: AnyView,
    /// The item's name, when it was declared as a semantic label.
    ///
    /// Kept beside the content rather than only inside it, because the
    /// platforms disagree about how much of a toolbar action to draw: a Mac
    /// shows the icon alone and keeps the name for the overflow menu, its
    /// tooltip and assistive technology, while a phone's navigation bar
    /// commonly shows the text. Once the label is erased into a view, no
    /// backend can tell the name from the icon, so both travel separately and
    /// each platform draws what its own chrome calls for.
    pub title: Option<Text>,
    /// The item's icon, when it was declared as a semantic label.
    pub icon: Option<ToolbarItemIcon>,
}

/// A toolbar item's icon, kept apart from its name.
///
/// The two forms are not interchangeable: a platform that recognizes the symbol
/// draws it at whatever size and weight its chrome calls for, while a view has
/// to be rendered into an image at a chosen size.
pub enum ToolbarItemIcon {
    /// A symbol the platform knows by name.
    System(SystemIcon),
    /// A view the backend renders itself.
    View(AnyViewBuilder<AnyView>),
}

impl Debug for ToolbarItemIcon {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::System(icon) => formatter.debug_tuple("System").field(icon).finish(),
            Self::View(_) => formatter.write_str("View"),
        }
    }
}

impl Debug for NavigationToolbarItem {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NavigationToolbarItem")
            .field("placement", &self.placement)
            .field("content", &self.content)
            .field("title", &self.title)
            .field("icon", &self.icon)
            .finish()
    }
}

impl NavigationToolbarItem {
    /// Creates a toolbar item with arbitrary content.
    ///
    /// Arbitrary content carries no name or icon of its own, so a backend can
    /// only place the view as given. [`Self::action`] is what lets each platform
    /// draw the item its own way.
    pub fn new(placement: NavigationToolbarPlacement, content: impl View) -> Self {
        Self {
            placement,
            content: AnyView::new(content),
            title: None,
            icon: None,
        }
    }

    /// Creates a semantic toolbar action.
    ///
    /// The label's name and icon are lifted out before the content is erased, so
    /// every backend can draw the item the way its own chrome does — see
    /// [`NavigationToolbarItem::title`].
    pub fn action<Args: 'static>(
        placement: NavigationToolbarPlacement,
        label: impl IntoLabel + 'static,
        handler: impl Handler<Args>,
    ) -> Self {
        let label = label.into_label();
        let title = Some(label.semantic_text().clone());
        let icon = label.semantic_icon().map_or_else(
            || label.icon_view().map(ToolbarItemIcon::View),
            |icon| Some(ToolbarItemIcon::System(icon)),
        );
        Self {
            placement,
            content: AnyView::new(button(label).style(ButtonStyle::Plain).action(handler)),
            title,
            icon,
        }
    }
}

/// Native toolbar contents attached to one navigation destination.
#[derive(Debug, Default)]
pub struct NavigationToolbar {
    /// Semantic toolbar items in declaration order.
    pub items: Vec<NavigationToolbarItem>,
}

impl NavigationToolbar {
    /// Creates a toolbar from semantic items.
    #[must_use]
    pub const fn new(items: Vec<NavigationToolbarItem>) -> Self {
        Self { items }
    }

    /// Appends one semantic toolbar item.
    #[must_use]
    pub fn item(mut self, item: NavigationToolbarItem) -> Self {
        self.items.push(item);
        self
    }
}

/// Configuration for a navigation bar.
///
/// Represents the appearance and behavior of a navigation bar, including
/// its title, color, and visibility.
#[derive(Debug)]
pub struct Bar {
    /// The title view displayed in the navigation bar
    pub title: AnyView,
    /// Optional semantic subtitle rendered by native chrome.
    pub subtitle: AnyView,
    /// Semantic native toolbar content.
    pub toolbar: NavigationToolbar,
    /// Optional search field configuration displayed in navigation chrome.
    pub search: Option<NavigationSearch>,
    /// Background color of the navigation bar, or `None` for the surrounding
    /// `Surface` theme token.
    pub color: Option<BarColor>,
    /// Whether the navigation bar is hidden
    pub hidden: Computed<bool>,
    /// The display mode for the title (automatic, inline, or large)
    pub display_mode: NavigationTitleDisplayMode,
}

impl Default for Bar {
    fn default() -> Self {
        Self {
            title: AnyView::default(),
            subtitle: AnyView::default(),
            toolbar: NavigationToolbar::default(),
            search: None,
            color: None,
            hidden: Computed::constant(false),
            display_mode: NavigationTitleDisplayMode::Automatic,
        }
    }
}

/// Marks a subtree as a navigation link: activating it follows to a
/// destination.
///
/// The link itself renders and behaves as a plain button; this marker is how
/// a platform that draws a destination-following affordance around the row a
/// link sits in — the iOS disclosure chevron — recognizes one. It carries no
/// behavior, and a renderer with no such affordance (a Mac list, say) ignores
/// it, which is why it wraps as [`IgnorableMetadata`].
#[derive(Debug, Clone, Copy, Default)]
pub struct NavigationLinkHint;

impl MetadataKey for NavigationLinkHint {}

/// An explicit navigation bar color, before or after environment resolution.
///
/// Authoring produces [`Self::Unresolved`]; [`NavigationView::resolve_native_fields`]
/// turns it into [`Self::Resolved`] once an effective environment exists. Carrying
/// both states in one value is what keeps "colored but unresolved" from being
/// representable, which is the state backends used to have to panic on.
#[derive(Debug, Clone)]
pub enum BarColor {
    /// An author-specified color that has not been resolved yet.
    Unresolved(Computed<Color>),
    /// A color resolved against the effective environment.
    Resolved(Computed<ResolvedColor>),
}

impl BarColor {
    /// Returns the resolved color, or `None` while still unresolved.
    #[must_use]
    pub const fn resolved(&self) -> Option<&Computed<ResolvedColor>> {
        match self {
            Self::Unresolved(_) => None,
            Self::Resolved(color) => Some(color),
        }
    }

    /// Returns the resolved color, panicking if resolution never ran.
    ///
    /// # Panics
    ///
    /// Panics when the bar reached a native backend without passing through
    /// [`NavigationView::resolve_native_fields`].
    #[doc(hidden)]
    #[must_use]
    pub const fn expect_resolved(&self) -> &Computed<ResolvedColor> {
        self.resolved().expect(
            "NavigationView bar color reached a native backend before it was resolved against an environment",
        )
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
    transition: AnyNavigationTransition,
}

struct HeterogeneousDestination {
    build: Rc<dyn Fn(&ErasedNavigationRoute) -> NavigationView>,
}

#[doc(hidden)]
pub struct HeterogeneousDestinations {
    entries: BTreeMap<TypeId, HeterogeneousDestination>,
    #[cfg(feature = "serde")]
    path: NavigationPath<ErasedNavigationRoute>,
}

impl Debug for HeterogeneousDestinations {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HeterogeneousDestinations")
            .field("len", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl HeterogeneousDestinations {
    fn new<R, F>(path: NavigationPath<ErasedNavigationRoute>, destination: F) -> Self
    where
        R: RestorableNavigationRoute,
        F: 'static + Fn(R) -> NavigationView,
    {
        let mut destinations = Self {
            entries: BTreeMap::new(),
            #[cfg(feature = "serde")]
            path,
        };
        #[cfg(not(feature = "serde"))]
        drop(path);
        destinations.insert(destination);
        destinations
    }

    fn insert<R, F>(&mut self, destination: F)
    where
        R: RestorableNavigationRoute,
        F: 'static + Fn(R) -> NavigationView,
    {
        #[cfg(feature = "serde")]
        self.path.register_restoration::<R>();
        let type_id = TypeId::of::<R>();
        let type_name = core::any::type_name::<R>();
        let previous = self.entries.insert(
            type_id,
            HeterogeneousDestination {
                build: Rc::new(move |route| {
                    let route = route
                        .downcast_ref::<R>()
                        .expect("registered navigation destination type must match route type");
                    destination(route.clone())
                }),
            },
        );
        assert!(
            previous.is_none(),
            "navigation destination for `{type_name}` was registered more than once"
        );
    }

    fn build(&self, route: &ErasedNavigationRoute) -> NavigationView {
        let destination = self.entries.get(&route.type_id()).unwrap_or_else(|| {
            panic!(
                "heterogeneous navigation route `{}` has no registered destination",
                route.type_name()
            )
        });
        (destination.build)(route)
    }
}

impl NavigationStack<(), ()> {
    /// Creates a new navigation stack with the specified root view.
    ///
    /// # Arguments
    /// * `root` - The semantic root destination of the navigation stack
    pub fn new(root: NavigationView) -> Self {
        Self::new_deferred(root)
    }

    fn new_deferred(root: impl View) -> Self {
        Self {
            root: AnyView::new(root),
            path: (),
            destination: (),
            transition: AnyNavigationTransition::new(navigation_transition::automatic()),
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
    pub const fn transition_style(&self) -> &AnyNavigationTransition {
        &self.transition
    }

    /// Sets the transition style for push/pop operations.
    pub fn transition(mut self, transition: impl NavigationTransition) -> Self {
        self.transition = AnyNavigationTransition::new(transition);
        self
    }
}

impl<T: 'static + PartialEq> NavigationStack<NavigationPath<T>, ()> {
    /// Creates a new navigation stack with the specified navigation path and root view.
    ///
    /// # Arguments
    /// * `path` - The navigation path representing the current stack
    /// * `root` - The root view of the navigation stack
    pub fn with_path(path: NavigationPath<T>, root: NavigationView) -> Self {
        Self {
            root: AnyView::new(root),
            path,
            destination: (),
            transition: AnyNavigationTransition::new(navigation_transition::automatic()),
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

impl NavigationStack<NavigationPath<ErasedNavigationRoute>, ()> {
    /// Registers the first concrete destination type for a heterogeneous path.
    pub fn destination<R, F>(
        self,
        destination: F,
    ) -> NavigationStack<NavigationPath<ErasedNavigationRoute>, HeterogeneousDestinations>
    where
        R: RestorableNavigationRoute,
        F: 'static + Fn(R) -> NavigationView,
    {
        let path = self.path;
        NavigationStack {
            root: self.root,
            destination: HeterogeneousDestinations::new(path.clone(), destination),
            path,
            transition: self.transition,
        }
    }
}

impl NavigationStack<NavigationPath<ErasedNavigationRoute>, HeterogeneousDestinations> {
    /// Registers another concrete destination type for a heterogeneous path.
    pub fn destination<R, F>(mut self, destination: F) -> Self
    where
        R: RestorableNavigationRoute,
        F: 'static + Fn(R) -> NavigationView,
    {
        self.destination.insert(destination);
        self
    }
}

raw_view!(NavigationStack<(),()>, StretchAxis::Both);

/// Projects a new path snapshot onto the backend stack as one transaction.
///
/// `same` decides route identity — plain equality for a typed path, type-and-value
/// equality for a heterogeneous one — and `build` turns a surviving route into its
/// destination builder.
fn reconcile_navigation_path<R: Clone + 'static>(
    receiver: &NavigationController,
    current_path: &mut Vec<R>,
    next_path: Vec<R>,
    same: &impl Fn(&R, &R) -> bool,
    build: &impl Fn(&R) -> AnyViewBuilder<NavigationView>,
) {
    let shared_prefix = shared_prefix_len(current_path, &next_path, same);
    let inserted = next_path.iter().skip(shared_prefix).map(build).collect();
    receiver.apply_path(shared_prefix, inserted);

    *current_path = next_path;
}

/// Subscribes a controller to a path, reconciling every snapshot into one
/// transaction.
fn subscribe_navigation_path<R: Clone + 'static>(
    path: &NavigationPath<R>,
    receiver: &NavigationController,
    same: impl Fn(&R, &R) -> bool + 'static,
    build: impl Fn(&R) -> AnyViewBuilder<NavigationView> + 'static,
) -> impl nami::watcher::WatcherGuard {
    let current_path = Rc::new(RefCell::new(Vec::new()));
    let receiver = receiver.clone();
    path.watch(move |next_path| {
        reconcile_navigation_path(
            &receiver,
            &mut current_path.borrow_mut(),
            next_path,
            &same,
            &build,
        );
    })
}

/// Shared body of every path-backed stack: publish the `Navigator`, install the
/// pop handler, and keep the path subscription alive on the controller so it
/// outlives the root view.
fn path_stack_body<R: Clone + 'static>(
    path: NavigationPath<R>,
    root: AnyView,
    transition: AnyNavigationTransition,
    same: impl Fn(&R, &R) -> bool + 'static,
    build: impl Fn(&R) -> AnyViewBuilder<NavigationView> + 'static,
) -> NavigationStack<(), ()> {
    let navigator = Navigator(path.clone());
    NavigationStack::new_deferred(use_env(
        move |(receiver, mut local_env): (NavigationController, Environment)| {
            local_env.insert(navigator.clone());
            receiver.retain_environment(local_env.clone());
            receiver.install_path_pop_handler({
                let path = path.clone();
                move |count| path.pop_n(count)
            });
            let guard = subscribe_navigation_path(&path, &receiver, same, build);

            receiver.retain(Retain::new(guard));
            Metadata::new(root, local_env)
        },
    ))
    .transition(transition)
}

impl<T, F> View for NavigationStack<NavigationPath<T>, F>
where
    T: 'static + Clone + PartialEq,
    F: 'static + Fn(T) -> NavigationView,
{
    fn body(self, _env: &Environment) -> impl View {
        let destination = Rc::new(self.destination);
        path_stack_body(
            self.path,
            self.root,
            self.transition,
            |left, right| left == right,
            move |route| {
                let destination = Rc::clone(&destination);
                let route = route.clone();
                AnyViewBuilder::new(move || destination(route.clone()))
            },
        )
    }
}

impl View for NavigationStack<NavigationPath<ErasedNavigationRoute>, HeterogeneousDestinations> {
    fn body(self, _env: &Environment) -> impl View {
        let destinations = Rc::new(self.destination);
        path_stack_body(
            self.path,
            self.root,
            self.transition,
            ErasedNavigationRoute::same_route,
            move |route| {
                let destinations = Rc::clone(&destinations);
                let route = route.clone();
                AnyViewBuilder::new(move || destinations.build(&route))
            },
        )
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
        IgnorableMetadata::new(
            button(self.label)
                // A navigation link carries no chrome of its own. It is a button
                // semantically — it is activated, and reads as one to assistive
                // technology — but the platforms draw the destination-following
                // affordance around the row it sits in, not as a filled capsule
                // behind its label. The default style would give every list row a
                // button's background, which is the grey card a Mac list does not
                // have.
                .style(ButtonStyle::Plain)
                .action(move |receiver: NavigationController| {
                    receiver.push_builder(destination.clone());
                }),
            NavigationLinkHint,
        )
    }
}

impl<Label, T> View for NavigationValueLink<Label, T>
where
    Label: IntoLabel + 'static,
    T: 'static + Clone + PartialEq,
{
    fn body(self, env: &waterui_core::Environment) -> impl View {
        let value = self.value;
        // Plain for the same reason as `NavigationLink` above: the link is a
        // button in behaviour, not in chrome.
        let link = if let Some(navigator) = env.get::<Navigator<T>>().cloned() {
            AnyView::new(
                button(self.label)
                    .style(ButtonStyle::Plain)
                    .action(move || navigator.push(value.clone())),
            )
        } else if let Some(navigator) = env.get::<Navigator<ErasedNavigationRoute>>().cloned() {
            AnyView::new(
                button(self.label)
                    .style(ButtonStyle::Plain)
                    .action(move || navigator.push(value.clone())),
            )
        } else {
            panic!("NavigationLink::value used outside of a compatible path-backed stack")
        };
        IgnorableMetadata::new(link, NavigationLinkHint)
    }
}

impl NavigationView {
    /// Resolves environment-dependent native fields without snapshotting signals.
    #[doc(hidden)]
    pub fn resolve_native_fields(&mut self, env: &Environment) {
        if let Some(search) = &mut self.bar.search {
            search.prompt = waterui_text::Text::computed(search.prompt.resolve(env).content);
        }

        // A toolbar item's name crosses the boundary as resolved content, so it
        // has to stop depending on the environment here — `Text::content` panics
        // on an environment-dependent text, and the FFI layer has no environment
        // to resolve one against.
        for item in &mut self.bar.toolbar.items {
            if let Some(title) = item.title.take() {
                item.title = Some(Text::computed(title.resolve(env).content));
            }
        }

        if let Some(BarColor::Unresolved(color)) = &self.bar.color {
            let resolved_env = env.clone();
            self.bar.color = Some(BarColor::Resolved(flatten_signal(
                color
                    .clone()
                    .map(move |color| color.resolve(&resolved_env)),
            )));
        }
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
            state: NavigationDestinationState::default(),
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
    ///     .navigation_title_display_mode(NavigationTitleDisplayMode::Large)
    /// ```
    pub const fn navigation_title_display_mode(mut self, mode: NavigationTitleDisplayMode) -> Self {
        self.bar.display_mode = mode;
        self
    }

    /// Sets the display mode to inline (small title).
    pub const fn inline_title(self) -> Self {
        self.navigation_title_display_mode(NavigationTitleDisplayMode::Inline)
    }

    /// Sets the display mode to large title.
    pub const fn large_title(self) -> Self {
        self.navigation_title_display_mode(NavigationTitleDisplayMode::Large)
    }

    /// Sets the semantic navigation subtitle.
    pub fn navigation_subtitle(mut self, subtitle: impl IntoText) -> Self {
        self.bar.subtitle = AnyView::new(subtitle.into_text());
        self
    }

    /// Installs semantic native toolbar content.
    pub fn navigation_toolbar(mut self, toolbar: NavigationToolbar) -> Self {
        self.bar.toolbar = toolbar;
        self
    }

    /// Controls native navigation bar visibility (`true` means visible).
    pub fn navigation_bar_visibility(mut self, visible: impl IntoSignal<bool> + 'static) -> Self {
        self.bar.hidden = visible.into_signal().map(|visible| !visible).computed();
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
        self.bar.color = Some(BarColor::Unresolved(color.into_signal().computed()));
        self
    }

    /// Controls whether user and system initiated pop may begin.
    pub fn navigation_pop_enabled(mut self, enabled: impl IntoSignal<bool> + 'static) -> Self {
        self.state.pop_enabled = enabled.into_signal().computed();
        self
    }

    /// Handles real user or system pop attempts, including denied attempts.
    pub fn on_navigation_pop_attempted<Args>(mut self, handler: impl Handler<Args>) -> Self {
        self.state.pop_attempted = Some(boxed_action(handler));
        self
    }

    /// Handles completion of the destination's transition to active.
    pub fn on_navigation_appear<Args>(mut self, handler: impl Handler<Args>) -> Self {
        self.state.appear = Some(boxed_action(handler));
        self
    }

    /// Handles completion of the destination's transition away from active.
    pub fn on_navigation_disappear<Args>(mut self, handler: impl Handler<Args>) -> Self {
        self.state.disappear = Some(boxed_action(handler));
        self
    }

    /// Handles a completed pop that removes this destination.
    pub fn on_navigation_pop<Args>(mut self, handler: impl Handler<Args>) -> Self {
        self.state.pop = Some(boxed_action(handler));
        self
    }
}

/// Translation key for the back affordance every navigation backend draws.
pub const NAVIGATION_BACK_LABEL_KEY: &str = "waterui.navigation.back";

/// Semantic label for the back affordance.
///
/// Backends must render and speak this rather than a string of their own, so a
/// single catalog entry translates the back button on every platform at once.
#[must_use]
pub fn navigation_back_label() -> waterui_controls::label::Label {
    waterui_text::Text::localized_or(NAVIGATION_BACK_LABEL_KEY, "Back").into_label()
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

fn shared_prefix_len<R>(left: &[R], right: &[R], same: &impl Fn(&R, &R) -> bool) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(left, right)| same(left, right))
        .count()
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use alloc::vec;
    use core::cell::{Cell, RefCell};

    use super::{
        CustomNavigationController, NavigationController, NavigationLink, NavigationPath,
        NavigationTransaction, NavigationView, Navigator, shared_prefix_len,
        subscribe_navigation_path,
    };
    use waterui_core::{Environment, Metadata, handler::AnyViewBuilder};

    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "the signature is fixed by the destination-builder closure the reconciler takes"
    )]
    fn route_destination(_route: &u8) -> AnyViewBuilder<NavigationView> {
        AnyViewBuilder::new(|| NavigationView::new("Route", ()))
    }

    fn equal<T: PartialEq>(left: &T, right: &T) -> bool {
        left == right
    }

    #[test]
    fn shared_prefix_is_entire_prefix() {
        let left = vec![1, 2, 3];
        let right = vec![1, 2, 4];
        assert_eq!(shared_prefix_len(&left, &right, &equal), 2);
    }

    #[test]
    fn shared_prefix_is_zero_when_root_changes() {
        let left = vec![1, 2];
        let right = vec![3, 4];
        assert_eq!(shared_prefix_len(&left, &right, &equal), 0);
    }

    #[test]
    fn shared_prefix_matches_complete_path() {
        let left = vec![1, 2, 3];
        let right = vec![1, 2, 3];
        assert_eq!(shared_prefix_len(&left, &right, &equal), 3);
    }

    struct CountingNavigationController {
        pushes: Rc<Cell<usize>>,
        pops: Rc<Cell<usize>>,
    }

    impl CustomNavigationController for CountingNavigationController {
        fn apply(&mut self, transaction: NavigationTransaction) {
            self.pushes
                .set(self.pushes.get() + transaction.inserted.len());
            self.pops.set(self.pops.get() + transaction.removed);
        }
    }

    #[test]
    fn path_subscription_reconciles_initial_and_replaced_routes() {
        let path = NavigationPath::from(vec![1_u8, 2]);
        let pushes = Rc::new(Cell::new(0));
        let pops = Rc::new(Cell::new(0));
        let controller = NavigationController::new(CountingNavigationController {
            pushes: Rc::clone(&pushes),
            pops: Rc::clone(&pops),
        });

        let _guard = subscribe_navigation_path(&path, &controller, equal, route_destination);
        path.replace([1, 3, 4]);

        assert_eq!(pushes.get(), 4);
        assert_eq!(pops.get(), 1);
    }

    /// A destination-building link and a route-driven link can share one stack.
    /// The link-pushed entry sits above the path, so popping it must leave the
    /// routes untouched, and only the pop past it may reach the path.
    #[test]
    fn link_pushed_entries_pop_before_the_explicit_path_does() {
        let path = NavigationPath::from(vec![1_u8]);
        let controller = NavigationController::new(CountingNavigationController {
            pushes: Rc::new(Cell::new(0)),
            pops: Rc::new(Cell::new(0)),
        });
        controller.install_path_pop_handler({
            let path = path.clone();
            move |count| path.pop_n(count)
        });
        let _guard = subscribe_navigation_path(&path, &controller, equal, route_destination);

        controller.push_builder(AnyViewBuilder::new(|| NavigationView::new("Link", ())));
        controller.request_pop(1);
        assert_eq!(
            path.snapshot(),
            vec![1],
            "popping a link-pushed entry must not consume a route"
        );

        controller.request_pop(1);
        assert!(
            path.snapshot().is_empty(),
            "popping past the link entries must reach the explicit path"
        );
    }

    /// A route change invalidates whatever was opened from the routes it
    /// replaced, so link-pushed entries above the path are dropped with it.
    #[test]
    fn a_route_change_discards_link_pushed_entries_above_it() {
        let path = NavigationPath::from(vec![1_u8]);
        let pops = Rc::new(Cell::new(0));
        let controller = NavigationController::new(CountingNavigationController {
            pushes: Rc::new(Cell::new(0)),
            pops: Rc::clone(&pops),
        });
        let _guard = subscribe_navigation_path(&path, &controller, equal, route_destination);

        controller.push_builder(AnyViewBuilder::new(|| NavigationView::new("Link", ())));
        pops.set(0);
        path.replace([2_u8]);

        assert_eq!(
            pops.get(),
            2,
            "replacing the route must remove both the route entry and the link entry above it"
        );
    }

    /// A native interactive pop is reported after the platform already removed
    /// the page, so the explicit path must absorb it without sending a second
    /// removal back to the backend.
    #[test]
    fn a_native_pop_is_absorbed_by_the_path_without_a_second_transaction() {
        let path = NavigationPath::from(vec![1_u8, 2]);
        let pops = Rc::new(Cell::new(0));
        let controller = NavigationController::new(CountingNavigationController {
            pushes: Rc::new(Cell::new(0)),
            pops: Rc::clone(&pops),
        });
        controller.install_path_pop_handler({
            let path = path.clone();
            move |count| path.pop_n(count)
        });
        let _guard = subscribe_navigation_path(&path, &controller, equal, route_destination);

        pops.set(0);
        controller.complete_native_pop(1);

        assert_eq!(path.snapshot(), vec![1]);
        assert_eq!(
            pops.get(),
            0,
            "the backend already removed the page, so no removal may be sent back"
        );
    }

    #[test]
    fn stale_transaction_acknowledgements_do_not_settle_the_latest_transaction() {
        let controller = NavigationController::new(CountingNavigationController {
            pushes: Rc::new(Cell::new(0)),
            pops: Rc::new(Cell::new(0)),
        });

        controller.push_builder(AnyViewBuilder::new(|| NavigationView::new("One", ())));
        controller.push_builder(AnyViewBuilder::new(|| NavigationView::new("Two", ())));

        assert!(!controller.transition_completed(1));
        assert!(controller.transition_cancelled(2));
        assert!(!controller.transition_completed(2));
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
        fn apply(&mut self, transaction: NavigationTransaction) {
            let mut inserted = transaction.inserted;
            if let Some(content) = inserted.pop() {
                *self.pushed.borrow_mut() = Some(content.build());
            }
        }
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
        retained_env.insert(Navigator(NavigationPath::<TestRoute>::new()));
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
        assert!(metadata.value.get::<Navigator<TestRoute>>().is_some());
    }
}
