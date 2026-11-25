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

use alloc::{boxed::Box, collections::BTreeMap, rc::Rc, vec, vec::Vec};
use core::{
    any::{Any, TypeId},
    cell::{Cell, RefCell},
    fmt::{self, Debug},
    marker::PhantomData,
};

use nami::{Binding, Computed, Signal as _, SignalExt as _};
#[cfg(feature = "serde")]
use serde::{Serialize, de::DeserializeOwned};
use waterui_controls::{Button, ButtonStyle, IconSlot, IntoLabel, LabelKind, button};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::handler::{BoxedAction, Handler, ReportExtraction, boxed_action};
use waterui_core::metadata::MetadataKey;
use waterui_core::{
    AnyView, Environment, Error, IntoSignal, Metadata, Native, NativeView, OneOf3, Retain,
    SerialDispatch, Str, View, contract_violation, extract::ExtractionState, extract::Extractor,
    extract::State, flatten_signal, handler::ViewBuilder, layout::StretchAxis,
};
use waterui_graphics::color::{Color, WorkingColor};
use waterui_text::IntoText;
use waterui_text::Text;

pub use path::{ErasedNavigationRoute, NavigationPath, NavigationPathReplacement};
pub use router::NavigationRouter;
pub use search::NavigationSearch;
pub use split::{
    ColumnWidth, NativeNavigationSplitStyle, NavigationSplitColumnVisibility,
    NavigationSplitLayout, NavigationSplitStyle, NavigationSplitView, split_style,
};
pub use tab::{BadgeValue, Tab, TabBarMinimizeBehavior, TabRole, Tabs, TabsLayout, tab_style};
use transition::TransitionKeyRegistry;
pub use transition::{
    AnyNavigationTransition, CoreTransitionMarker, IntoNavigationTransition, IntoTransitionSpec,
    KeyedNavigationTransitionDestination, KeyedNavigationTransitionSource,
    NativeNavigationTransition, NavigationTransition, NavigationTransitionDestination,
    NavigationTransitionDirection, NavigationTransitionMarker, NavigationTransitionPair,
    NavigationTransitionPairMarker, NavigationTransitionSource, NavigationTransitionViewExt,
    RetainedNavigationTransition, TransitionSpecMarker, navigation_transition,
};
pub use waterui_controls::{IconSlot as NavigationIconSlot, SlotBuilder};

/// A navigation view whose content and toolbar items have been lowered into
/// a backend's child slot `S`.
pub type ResolvedNavigationView<S> = NavigationView<S, ResolvedNavigationToolbar<S>>;

/// A view that combines a navigation bar with content.
///
/// The `NavigationView` contains a navigation bar with a title and other
/// configuration options, along with the actual content to display. It
/// keeps its content's type `C` and toolbar's type `T`; a backend receives
/// [`ResolvedNavigationView<S>`] with both in its child slot.
#[must_use]
pub struct NavigationView<C, T = ()> {
    /// The navigation bar for this view
    pub bar: Bar<T>,
    /// The content to display in this view
    pub content: C,
    /// Reactive and callback state associated with this destination.
    #[doc(hidden)]
    pub state: NavigationDestinationState,
    /// How this destination arrives, when it wants something other than the
    /// motion its stack declared.
    ///
    /// A stack-wide transition cannot name a *matched* pair, because the pair
    /// differs per destination: a gallery's tile and the page it opens share an
    /// identity that the next tile does not. Declaring it here is what lets each
    /// destination name its own — the same place SwiftUI puts it.
    #[doc(hidden)]
    pub transition: Option<AnyNavigationTransition>,
}

impl<C, T: Debug> Debug for NavigationView<C, T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NavigationView")
            .field("bar", &self.bar)
            .field("content", &core::any::type_name::<C>())
            .field("state", &self.state)
            .field("transition", &self.transition)
            .finish()
    }
}

impl<C, T> NavigationView<C, T> {
    /// Takes the content out, leaving the bar, state and transition.
    pub fn split_content(self) -> (C, NavigationView<(), T>) {
        let Self {
            bar,
            content,
            state,
            transition,
        } = self;
        (
            content,
            NavigationView {
                bar,
                content: (),
                state,
                transition,
            },
        )
    }

    /// The same view with its content mapped through `f`.
    pub fn map_content<D>(self, f: impl FnOnce(C) -> D) -> NavigationView<D, T> {
        let Self {
            bar,
            content,
            state,
            transition,
        } = self;
        NavigationView {
            bar,
            content: f(content),
            state,
            transition,
        }
    }
}

impl<C: View, T: NavigationToolbarItems> NavigationView<C, T> {
    /// Lowers this view's content and toolbar items into child slot `S`
    /// without erasing through `AnyView`.
    #[doc(hidden)]
    pub fn into_resolved<S: ChildSlot>(self) -> ResolvedNavigationView<S> {
        let Self {
            bar,
            content,
            state,
            transition,
        } = self;
        NavigationView {
            bar: bar.map_toolbar(NavigationToolbarItems::into_resolved::<S>),
            content: S::from_view(content),
            state,
            transition,
        }
    }
}

/// Reactive policy and lifecycle handlers attached to one destination.
#[doc(hidden)]
#[non_exhaustive]
pub struct NavigationDestinationState {
    /// Whether a user or system initiated pop may begin.
    pop_enabled: Computed<bool>,
    /// Called for a real user/system pop request, including a denied request.
    pop_attempted: Option<BoxedAction>,
    /// Called after the destination becomes active.
    appear: Option<BoxedAction>,
    /// Called after the destination ceases to be active.
    disappear: Option<BoxedAction>,
    /// Called only after a completed pop removes the destination.
    pop: Option<BoxedAction>,
}

impl NavigationDestinationState {
    /// Whether a user or system initiated pop may begin.
    #[must_use]
    pub const fn pop_enabled(&self) -> &Computed<bool> {
        &self.pop_enabled
    }

    /// Called for a real user/system pop request, including a denied request.
    #[must_use]
    pub const fn pop_attempted(&self) -> Option<&BoxedAction> {
        self.pop_attempted.as_ref()
    }

    /// Called after the destination becomes active.
    #[must_use]
    pub const fn appear(&self) -> Option<&BoxedAction> {
        self.appear.as_ref()
    }

    /// Called after the destination ceases to be active.
    #[must_use]
    pub const fn disappear(&self) -> Option<&BoxedAction> {
        self.disappear.as_ref()
    }

    /// Called only after a completed pop removes the destination.
    #[must_use]
    pub const fn pop(&self) -> Option<&BoxedAction> {
        self.pop.as_ref()
    }
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
            handler(env).report_extraction();
        }
        self.pop_enabled.snapshot()
    }

    /// Reports that the destination became active.
    #[doc(hidden)]
    pub fn appeared(&mut self, env: &Environment) {
        if let Some(handler) = &mut self.appear {
            handler(env).report_extraction();
        }
    }

    /// Reports that the destination ceased to be active.
    #[doc(hidden)]
    pub fn disappeared(&mut self, env: &Environment) {
        if let Some(handler) = &mut self.disappear {
            handler(env).report_extraction();
        }
    }

    /// Reports that a completed pop removed the destination.
    #[doc(hidden)]
    pub fn popped(&mut self, env: &Environment) {
        if let Some(handler) = &mut self.pop {
            handler(env).report_extraction();
        }
    }
}

/// Monotonically increasing identifier scoped to one navigation stack.
pub type NavigationTransactionId = u64;

type PathPopHandler = Rc<dyn Fn(usize)>;

/// A page a navigation stack shows, built when its backend inserts it: a
/// [`NavigationView`] whose content is in the child slot `S`.
///
/// A page keeps the environment of the stack that holds it, when the stack
/// scoped one; building the page scopes its content to it.
pub struct NavigationPage<S> {
    build: PageBuild<S>,
    env: Option<Environment>,
}

/// Builds a page, scoped to its stack's environment when there is one.
type PageBuild<S> = Box<dyn FnOnce(Option<&Environment>) -> ResolvedNavigationView<S>>;

impl<S> Debug for NavigationPage<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NavigationPage").finish_non_exhaustive()
    }
}

impl<S: ChildSlot> NavigationPage<S> {
    /// A page built by `build`, its content and toolbar converted into the slot `S`.
    pub fn new<V: View, T: NavigationToolbarItems>(
        build: impl FnOnce() -> NavigationView<V, T> + 'static,
    ) -> Self {
        Self {
            build: Box::new(move |env| {
                let view = build();
                match env {
                    Some(env) => scoped_navigation_view::<S, _, _>(view, env),
                    None => view.into_resolved::<S>(),
                }
            }),
            env: None,
        }
    }
}

impl<S: 'static> NavigationPage<S> {
    /// The same page with its content and toolbar mapped through `f`: a backend holds
    /// them in its own slot.
    #[must_use]
    pub fn map_slot<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> NavigationPage<T> {
        let Self { build, env } = self;
        NavigationPage {
            build: Box::new(move |scope| MapSlots::map_slots(build(scope), f)),
            env,
        }
    }
}

impl<S> NavigationPage<S> {
    /// The page scoped to `env`, which its content and bar build under.
    fn in_environment(mut self, env: Option<Environment>) -> Self {
        if env.is_some() {
            self.env = env;
        }
        self
    }

    /// Builds the page.
    pub fn build(self) -> ResolvedNavigationView<S> {
        (self.build)(self.env.as_ref())
    }
}

/// A page on its way to a receiver: a [`NavigationPage`] in the slot of
/// whatever pushed it, which the receiver takes in its own slot.
struct PendingPage(Box<dyn Any>);

impl PendingPage {
    fn new<S: ChildSlot>(page: NavigationPage<S>) -> Self {
        Self(Box::new(page))
    }

    /// The page in the receiver's slot `S`. A pushed page arrives in that slot
    /// or erased; [`NavigationController::push`] refuses any other.
    fn into_slot<S: ChildSlot>(self) -> NavigationPage<S> {
        match self.0.downcast::<NavigationPage<S>>() {
            Ok(page) => *page,
            Err(page) => page
                .downcast::<NavigationPage<AnyView>>()
                .expect("a navigation page reaches its receiver in the receiver's slot or erased")
                .map_slot(S::from_view),
        }
    }
}

/// One atomic change projected from navigation state into a backend stack.
///
/// A transaction always replaces a suffix: the first [`Self::retained_prefix`]
/// entries stay untouched, the [`Self::removed`] entries above them are
/// discarded, and [`Self::inserted`] is appended in their place.
pub struct NavigationTransaction<S> {
    /// Stack-scoped identifier acknowledged by
    /// [`NavigationController::transition_completed`] or
    /// [`NavigationController::transition_cancelled`].
    pub id: NavigationTransactionId,
    /// Number of destination entries retained from the root.
    pub retained_prefix: usize,
    /// Number of entries removed after the retained prefix.
    pub removed: usize,
    /// Pages inserted after the retained prefix.
    pub inserted: Vec<NavigationPage<S>>,
}

impl<S> Debug for NavigationTransaction<S> {
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
    /// The child slot the receiver takes pages' content in: its backend's.
    type Slot: ChildSlot;

    /// Applies one complete stack mutation.
    fn apply(&mut self, transaction: NavigationTransaction<Self::Slot>);
}

/// A transaction before its pages are taken in the receiver's slot.
struct PendingTransaction {
    id: NavigationTransactionId,
    retained_prefix: usize,
    removed: usize,
    inserted: Vec<PendingPage>,
}

/// [`CustomNavigationController`] behind the controller's shared state.
trait Receiver {
    fn apply(&mut self, transaction: PendingTransaction);
}

impl<R: CustomNavigationController> Receiver for R {
    fn apply(&mut self, transaction: PendingTransaction) {
        let PendingTransaction {
            id,
            retained_prefix,
            removed,
            inserted,
        } = transaction;
        CustomNavigationController::apply(
            self,
            NavigationTransaction {
                id,
                retained_prefix,
                removed,
                inserted: inserted
                    .into_iter()
                    .map(PendingPage::into_slot::<R::Slot>)
                    .collect(),
            },
        );
    }
}

/// Shared interior state of one [`NavigationController`].
///
/// The projected stack is split at [`Self::path_depth`]: entries below it are
/// owned by an explicit [`NavigationPath`] and may only be removed by mutating
/// that path, while entries above it were pushed by a destination-building
/// [`NavigationLink`] and are owned by the controller alone. Keeping the split
/// explicit is what lets both link kinds coexist in one stack.
struct NavigationControllerState {
    receiver: RefCell<Box<dyn Receiver>>,
    /// The receiver's child slot, which a pushed page must arrive in unless
    /// it arrives erased.
    slot: TypeId,
    retained: RefCell<Option<Retain>>,
    retained_environment: RefCell<Option<Environment>>,
    depth: Cell<usize>,
    path_depth: Cell<usize>,
    next_transaction_id: Cell<NavigationTransactionId>,
    pending_transaction_id: Cell<Option<NavigationTransactionId>>,
    path_pop_handler: RefCell<Option<PathPopHandler>>,
    native_applied_pops: Cell<usize>,
    /// Transactions issued while the receiver's `apply` is still running; they
    /// are delivered after it returns, in order, rather than re-entering it.
    outbox: SerialDispatch<PendingTransaction>,
}

/// A receiver that handles navigation actions.
/// For renderers to implement navigation handling.
#[derive(Clone)]
pub struct NavigationController {
    state: Rc<NavigationControllerState>,
}

impl Extractor for NavigationController {
    fn extract(env: &Environment) -> Result<Self, Error> {
        State::<Self>::extract(env).map(|State(controller)| controller)
    }

    fn extract_from_action(env: &Environment, state: &mut ExtractionState) -> Result<Self, Error> {
        State::<Self>::extract_from_action(env, state).map(|State(controller)| controller)
    }
}

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
    pub fn new<R: CustomNavigationController>(receiver: R) -> Self {
        Self {
            state: Rc::new(NavigationControllerState {
                receiver: RefCell::new(Box::new(receiver)),
                slot: TypeId::of::<R::Slot>(),
                retained: RefCell::new(None),
                retained_environment: RefCell::new(None),
                depth: Cell::new(0),
                path_depth: Cell::new(0),
                next_transaction_id: Cell::new(1),
                pending_transaction_id: Cell::new(None),
                outbox: SerialDispatch::new(),
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

    /// Whether a page in the slot `S` reaches this controller's receiver: one
    /// in the receiver's own slot, or an erased one.
    fn takes<S: ChildSlot>(&self) -> bool {
        let slot = TypeId::of::<S>();
        slot == self.state.slot || slot == TypeId::of::<AnyView>()
    }

    /// The page as it travels to the receiver, in the controller's scope.
    fn pending<S: ChildSlot>(&self, page: NavigationPage<S>) -> PendingPage {
        PendingPage::new(page.in_environment(self.retained_environment()))
    }

    /// Pushes a page onto the stack.
    ///
    /// The entry is owned by the controller rather than by any explicit path, so
    /// it pops without disturbing route state and is discarded when the path
    /// underneath it changes.
    ///
    /// # Contract violation
    ///
    /// A page arrives in the child slot of the backend that realizes the
    /// stack, or erased. A page in another backend's slot is reported: a debug
    /// build panics; a release build logs it and pushes nothing.
    pub fn push<S: ChildSlot>(&self, page: NavigationPage<S>) {
        if !self.takes::<S>() {
            contract_violation!(
                "a navigation page in the slot `{}` was pushed onto a stack realized by another backend; nothing is pushed",
                core::any::type_name::<S>()
            );
            return;
        }
        let page = self.pending(page);
        let depth = self.state.depth.get();
        self.state.depth.set(depth + 1);
        self.commit(depth, 0, vec![page]);
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
    fn apply_path<S: ChildSlot>(&self, retained_prefix: usize, inserted: Vec<NavigationPage<S>>) {
        if !self.takes::<S>() {
            contract_violation!(
                "navigation path pages in the slot `{}` were projected onto a stack realized by another backend; the path is not shown",
                core::any::type_name::<S>()
            );
            return;
        }
        // A path's pages build in the stack's own environment, where the
        // backend realizes them; only a link-pushed page carries the
        // controller's scope.
        let inserted: Vec<PendingPage> = inserted.into_iter().map(PendingPage::new).collect();
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
    fn commit(&self, retained_prefix: usize, removed: usize, inserted: Vec<PendingPage>) {
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
        // A mutation issued while `apply` is still running only enqueues; the
        // delivery in flight drains it once `apply` returns, so the receiver is
        // never called re-entrantly.
        self.state.outbox.deliver(
            PendingTransaction {
                id,
                retained_prefix,
                removed,
                inserted,
            },
            |transaction| {
                self.state.pending_transaction_id.set(Some(transaction.id));
                self.state.receiver.borrow_mut().apply(transaction);
            },
        );
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
    /// # Contract violation
    ///
    /// A controller drives one explicit path. Installing a second handler is
    /// reported: a debug build panics; a release build logs it and the later
    /// handler replaces the earlier one.
    pub fn install_path_pop_handler(&self, handler: impl Fn(usize) + 'static) {
        let previous = self.state.path_pop_handler.replace(Some(Rc::new(handler)));
        if previous.is_some() {
            contract_violation!(
                "a navigation controller cannot drive more than one explicit path; the later path handler replaces the earlier one"
            );
        }
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
    /// # Contract violation
    ///
    /// A zero `count` is reported: a debug build panics; a release build logs
    /// it and pops nothing.
    fn split_pop(&self, count: usize) -> (usize, usize) {
        if count == 0 {
            contract_violation!("navigation pop count must be non-zero; nothing is popped");
            return (0, 0);
        }
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
    /// # Contract violation
    ///
    /// A zero `count` is reported: a debug build panics; a release build logs
    /// it and pops nothing.
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
    /// # Contract violation
    ///
    /// A zero `count`, or an explicit path that fails to absorb the pop in the
    /// same update, is reported: a debug build panics; a release build logs it
    /// and drops the pop that was not applied. A `count` beyond the projected
    /// stack depth pops what there is.
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
            let unabsorbed = self.state.native_applied_pops.replace(0);
            if unabsorbed != 0 {
                contract_violation!(
                    "a native navigation pop must be absorbed by the explicit path in the same update, \
                     but the path did not shrink by {from_path}; the unabsorbed pop is dropped"
                );
            }
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
}

/// `view` with its bar and content scoped to `env`, its environment-dependent
/// fields resolved against it.
fn scoped_navigation_view<S: ChildSlot, V: View, T: NavigationToolbarItems>(
    view: NavigationView<V, T>,
    env: &Environment,
) -> ResolvedNavigationView<S> {
    let mut resolved = view
        .map_content(|content| Metadata::new(content, env.clone()))
        .into_resolved::<S>();
    resolved.resolve_native_fields(env);
    for item in resolved.bar.toolbar.items_mut() {
        item.env = Some(env.clone());
    }
    resolved
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

// Written out rather than `#[state]`: the impl is conditional —
// `Navigator<T>: Clone` holds only when `T: Clone` — while `#[state]` copies
// the type's own generics verbatim and would emit an unconditional impl.
impl<T: Clone + 'static> Extractor for Navigator<T> {
    fn extract(env: &Environment) -> Result<Self, Error> {
        <State<Self> as Extractor>::extract(env).map(|state| state.0)
    }

    fn extract_from_action(env: &Environment, state: &mut ExtractionState) -> Result<Self, Error> {
        <State<Self> as Extractor>::extract_from_action(env, state).map(|value| value.0)
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

/// A resolved navigation view reaches its backend with its content and toolbar
/// items in the backend's child slot.
impl<S: 'static> MapSlots<S> for ResolvedNavigationView<S> {
    type With<T: 'static> = ResolvedNavigationView<T>;

    fn map_slots<T: 'static>(
        self,
        f: impl Fn(S) -> T + Clone + 'static,
    ) -> ResolvedNavigationView<T> {
        let Self {
            bar,
            content,
            state,
            transition,
        } = self;
        NavigationView {
            bar: bar.map_toolbar(|tb| tb.map_slots(f.clone())),
            content: f(content),
            state,
            transition,
        }
    }
}

impl<S: 'static> MapSlots<S> for NavigationStackLayout<S> {
    type With<T: 'static> = NavigationStackLayout<T>;

    fn map_slots<T: 'static>(
        self,
        f: impl Fn(S) -> T + Clone + 'static,
    ) -> NavigationStackLayout<T> {
        self.map_slot(f)
    }
}

impl<S: 'static> NativeView for ResolvedNavigationView<S> {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

impl<V: View, T: NavigationToolbarItems> View for NavigationView<V, T> {
    /// The erased form, for code that expands views by hand.
    fn body(self, env: &Environment) -> impl View {
        let mut resolved = self.into_resolved::<AnyView>();
        resolved.resolve_native_fields(env);
        Native::new(resolved)
    }

    /// Reaches the backend with the content and toolbar items in the backend's
    /// child slot.
    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_resolved::<B::Slot>(), env, cx)
    }
}

/// The display mode for the navigation bar title.
///
/// Controls how the navigation bar title is displayed, similar to `SwiftUI`'s
/// `navigationBarTitleDisplayMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each title display mode variant by variant; a new title display mode must fail their build instead of falling into a wildcard arm"
)]
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
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each toolbar placement variant by variant; a new toolbar placement must fail their build instead of falling into a wildcard arm"
)]
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

/// Marker for a custom-view [`NavigationToolbarItem`].
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CustomToolbarItem;

/// Marker for a custom-view [`NavigationToolbarItem`] carrying a typed icon.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CustomIconToolbarItem;

/// Marker for a semantic-action [`NavigationToolbarItem`].
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ActionToolbarItem;

/// One semantic native toolbar item before lowering into a backend's child slot.
#[must_use]
#[non_exhaustive]
pub struct NavigationToolbarItem<C, Kind = CustomToolbarItem> {
    placement: NavigationToolbarPlacement,
    content: C,
    kind: PhantomData<Kind>,
}

impl<C, Kind> Debug for NavigationToolbarItem<C, Kind> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NavigationToolbarItem")
            .field("placement", &self.placement)
            .finish_non_exhaustive()
    }
}

impl<C, Kind> NavigationToolbarItem<C, Kind> {
    /// Semantic placement resolved by each platform backend.
    #[must_use]
    pub const fn placement(&self) -> NavigationToolbarPlacement {
        self.placement
    }
}

impl NavigationToolbarItem<(), CustomToolbarItem> {
    /// Creates a toolbar item with arbitrary content.
    ///
    /// Arbitrary content carries no name or icon of its own, so a backend can
    /// only place the view as given. [`Self::action`] is what lets each platform
    /// draw the item its own way.
    pub const fn new<V: View>(
        placement: NavigationToolbarPlacement,
        content: V,
    ) -> NavigationToolbarItem<V, CustomToolbarItem> {
        NavigationToolbarItem {
            placement,
            content,
            kind: PhantomData,
        }
    }

    /// Creates a semantic toolbar action.
    ///
    /// The label's name and icon stay typed until the backend lowers the
    /// destination into its child slot, so every backend can draw the item
    /// the way its own chrome does without `AnyView` erasure.
    pub fn action<I: IntoLabel, Args: 'static>(
        placement: NavigationToolbarPlacement,
        label: I,
        handler: impl Handler<Args>,
    ) -> NavigationToolbarItem<Button<I::Kind, BoxedAction<()>>, ActionToolbarItem> {
        let content = button(label.into_label())
            .style(ButtonStyle::Plain)
            .action_boxed(boxed_action(handler));
        NavigationToolbarItem {
            placement,
            content,
            kind: PhantomData,
        }
    }
}

impl<C: View> NavigationToolbarItem<C, CustomToolbarItem> {
    /// Attaches a typed icon view to this custom toolbar item without erasing
    /// through `AnyView`.
    pub fn icon<I: View + Clone + 'static>(
        self,
        icon: I,
    ) -> NavigationToolbarItem<(C, I), CustomIconToolbarItem> {
        NavigationToolbarItem {
            placement: self.placement,
            content: (self.content, icon),
            kind: PhantomData,
        }
    }
}

/// A single authored toolbar item lowered into child slot `S`.
#[doc(hidden)]
pub trait NavigationToolbarItemSource: 'static {
    /// Lowers this item into child slot `S`.
    fn into_resolved<S: ChildSlot>(self) -> ResolvedNavigationToolbarItem<S>;
}

impl<V: View> NavigationToolbarItemSource for NavigationToolbarItem<V, CustomToolbarItem> {
    fn into_resolved<S: ChildSlot>(self) -> ResolvedNavigationToolbarItem<S> {
        ResolvedNavigationToolbarItem {
            placement: self.placement,
            content: S::from_view(self.content),
            title: None,
            icon: None,
            env: None,
        }
    }
}

impl<V: View, I: View + Clone + 'static> NavigationToolbarItemSource
    for NavigationToolbarItem<(V, I), CustomIconToolbarItem>
{
    fn into_resolved<S: ChildSlot>(self) -> ResolvedNavigationToolbarItem<S> {
        let (content, icon) = self.content;
        ResolvedNavigationToolbarItem {
            placement: self.placement,
            content: S::from_view(content),
            title: None,
            icon: Some(IconSlot::from_view(icon)),
            env: None,
        }
    }
}

impl<L: LabelKind> NavigationToolbarItemSource
    for NavigationToolbarItem<Button<L, BoxedAction<()>>, ActionToolbarItem>
{
    fn into_resolved<S: ChildSlot>(self) -> ResolvedNavigationToolbarItem<S> {
        let title = Some(self.content.label().semantic_text().clone());
        let icon = self.content.label().icon_slot::<S>();
        ResolvedNavigationToolbarItem {
            placement: self.placement,
            content: S::from_view(self.content),
            title,
            icon,
            env: None,
        }
    }
}

/// One resolved native toolbar item carrying its view and optional icon in the
/// backend's child slot `S`.
#[doc(hidden)]
#[must_use]
#[non_exhaustive]
pub struct ResolvedNavigationToolbarItem<S> {
    /// Semantic placement resolved by each platform backend.
    placement: NavigationToolbarPlacement,
    /// Item content in the backend's child slot.
    content: S,
    /// The item's name, when it was declared as a semantic label.
    title: Option<Text>,
    /// The item's icon, when it was declared as a semantic label.
    icon: Option<IconSlot<S>>,
    /// Scoped navigation environment attached by a pushed page.
    env: Option<Environment>,
}

impl<S: 'static> ResolvedNavigationToolbarItem<S> {
    /// Maps the item's content and icon slots through `f`.
    pub fn map_slots<T: 'static>(
        self,
        f: impl Fn(S) -> T + Clone + 'static,
    ) -> ResolvedNavigationToolbarItem<T> {
        ResolvedNavigationToolbarItem {
            placement: self.placement,
            content: f(self.content),
            title: self.title,
            icon: self.icon.map(|icon| icon.map_slot(f)),
            env: self.env,
        }
    }
}

impl<S> ResolvedNavigationToolbarItem<S> {
    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        NavigationToolbarPlacement,
        S,
        Option<Text>,
        Option<IconSlot<S>>,
        Option<Environment>,
    ) {
        (
            self.placement,
            self.content,
            self.title,
            self.icon,
            self.env,
        )
    }

    /// Semantic placement resolved by each platform backend.
    #[must_use]
    pub const fn placement(&self) -> NavigationToolbarPlacement {
        self.placement
    }

    /// Item content.
    #[must_use]
    pub const fn content(&self) -> &S {
        &self.content
    }

    /// The item's name, when it was declared as a semantic label.
    #[must_use]
    pub const fn title(&self) -> Option<&Text> {
        self.title.as_ref()
    }

    /// The item's icon, when it was declared as a semantic label.
    #[must_use]
    pub const fn icon(&self) -> Option<&IconSlot<S>> {
        self.icon.as_ref()
    }

    /// Scoped navigation environment attached by a pushed destination, if any.
    #[must_use]
    pub const fn env(&self) -> Option<&Environment> {
        self.env.as_ref()
    }
}

impl<S> Debug for ResolvedNavigationToolbarItem<S> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ResolvedNavigationToolbarItem")
            .field("placement", &self.placement)
            .field("title", &self.title)
            .field("icon", &self.icon)
            .field("env", &self.env)
            .finish_non_exhaustive()
    }
}

/// A sequence of authored navigation toolbar items lowered into child slot `S`.
pub trait NavigationToolbarItems: 'static {
    /// Appends every resolved item in declaration order into `out`.
    #[doc(hidden)]
    fn push_into<S: ChildSlot>(self, out: &mut Vec<ResolvedNavigationToolbarItem<S>>);

    /// Lowers all items into a [`ResolvedNavigationToolbar<S>`].
    #[doc(hidden)]
    fn into_resolved<S: ChildSlot>(self) -> ResolvedNavigationToolbar<S>
    where
        Self: Sized,
    {
        let mut items = Vec::new();
        self.push_into(&mut items);
        ResolvedNavigationToolbar { items }
    }
}

impl NavigationToolbarItems for () {
    fn push_into<S: ChildSlot>(self, _out: &mut Vec<ResolvedNavigationToolbarItem<S>>) {}
}

impl<Head, Tail> NavigationToolbarItems for (Head, Tail)
where
    Head: NavigationToolbarItems,
    Tail: NavigationToolbarItemSource,
{
    fn push_into<S: ChildSlot>(self, out: &mut Vec<ResolvedNavigationToolbarItem<S>>) {
        self.0.push_into(out);
        out.push(self.1.into_resolved::<S>());
    }
}

impl<Item: NavigationToolbarItemSource> NavigationToolbarItems for Vec<Item> {
    fn push_into<S: ChildSlot>(self, out: &mut Vec<ResolvedNavigationToolbarItem<S>>) {
        out.reserve(self.len());
        for item in self {
            out.push(item.into_resolved::<S>());
        }
    }
}

impl<Item: NavigationToolbarItemSource, const N: usize> NavigationToolbarItems for [Item; N] {
    fn push_into<S: ChildSlot>(self, out: &mut Vec<ResolvedNavigationToolbarItem<S>>) {
        out.reserve(N);
        for item in self {
            out.push(item.into_resolved::<S>());
        }
    }
}

impl<Items: NavigationToolbarItems> NavigationToolbarItems for NavigationToolbar<Items> {
    fn push_into<S: ChildSlot>(self, out: &mut Vec<ResolvedNavigationToolbarItem<S>>) {
        self.items.push_into(out);
    }
}

impl<Items: NavigationToolbarItems> NavigationToolbarItems for Option<Items> {
    fn push_into<S: ChildSlot>(self, out: &mut Vec<ResolvedNavigationToolbarItem<S>>) {
        if let Some(items) = self {
            items.push_into(out);
        }
    }
}

macro_rules! impl_one_of_toolbar_items {
    ($name:ident { $($variant:ident),+ }) => {
        impl<$($variant: NavigationToolbarItems),+> NavigationToolbarItems
            for waterui_core::$name<$($variant),+>
        {
            fn push_into<S: ChildSlot>(self, out: &mut Vec<ResolvedNavigationToolbarItem<S>>) {
                match self {
                    $(Self::$variant(items) => items.push_into(out),)+
                }
            }
        }
    };
}

impl_one_of_toolbar_items!(OneOf2 { A, B });
impl_one_of_toolbar_items!(OneOf3 { A, B, C });
impl_one_of_toolbar_items!(OneOf4 { A, B, C, D });
impl_one_of_toolbar_items!(OneOf5 { A, B, C, D, E });
impl_one_of_toolbar_items!(OneOf6 { A, B, C, D, E, F });
impl_one_of_toolbar_items!(OneOf7 {
    A,
    B,
    C,
    D,
    E,
    F,
    G
});
impl_one_of_toolbar_items!(OneOf8 {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H
});

/// Native toolbar contents attached to one navigation destination.
#[derive(Debug)]
#[non_exhaustive]
pub struct NavigationToolbar<Items = ()> {
    /// Authored semantic toolbar items in declaration order.
    items: Items,
}

impl Default for NavigationToolbar<()> {
    fn default() -> Self {
        Self { items: () }
    }
}

impl NavigationToolbar<()> {
    /// Creates a toolbar from semantic items (for example a `Vec` or array of
    /// [`NavigationToolbarItem`] values).
    #[must_use]
    pub const fn new<Items: NavigationToolbarItems>(items: Items) -> NavigationToolbar<Items> {
        NavigationToolbar { items }
    }
}

impl<Items: NavigationToolbarItems> NavigationToolbar<Items> {
    /// Appends one semantic toolbar item.
    #[must_use]
    pub fn item<Item: NavigationToolbarItemSource>(
        self,
        item: Item,
    ) -> NavigationToolbar<(Items, Item)> {
        NavigationToolbar {
            items: (self.items, item),
        }
    }
}

/// Resolved native toolbar items carried on [`ResolvedNavigationView<S>`].
#[doc(hidden)]
#[derive(Debug)]
#[non_exhaustive]
pub struct ResolvedNavigationToolbar<S> {
    items: Vec<ResolvedNavigationToolbarItem<S>>,
}

impl<S> Default for ResolvedNavigationToolbar<S> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<S> ResolvedNavigationToolbar<S> {
    /// Semantic toolbar items in declaration order.
    #[must_use]
    pub const fn items(&self) -> &[ResolvedNavigationToolbarItem<S>] {
        self.items.as_slice()
    }

    /// The items, for in-place edits.
    pub const fn items_mut(&mut self) -> &mut [ResolvedNavigationToolbarItem<S>] {
        self.items.as_mut_slice()
    }

    /// Takes the items out of the toolbar, leaving it empty.
    pub fn take_items(&mut self) -> Vec<ResolvedNavigationToolbarItem<S>> {
        core::mem::take(&mut self.items)
    }
}

impl<S: 'static> ResolvedNavigationToolbar<S> {
    /// Maps every toolbar item's content and icon slots through `f`.
    #[must_use]
    pub fn map_slots<T: 'static>(
        self,
        f: impl Fn(S) -> T + Clone + 'static,
    ) -> ResolvedNavigationToolbar<T> {
        ResolvedNavigationToolbar {
            items: self
                .items
                .into_iter()
                .map(|item| item.map_slots(f.clone()))
                .collect(),
        }
    }
}

/// Configuration for a navigation bar.
///
/// Represents the appearance and behavior of a navigation bar, including
/// its title, color, and visibility.
#[derive(Debug)]
pub struct Bar<T = ()> {
    /// The title text displayed in the navigation bar.
    pub title: Option<Text>,
    /// Optional semantic subtitle rendered by native chrome.
    pub subtitle: Option<Text>,
    /// Semantic native toolbar content.
    pub toolbar: T,
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

impl<T: Default> Default for Bar<T> {
    fn default() -> Self {
        Self {
            title: None,
            subtitle: None,
            toolbar: T::default(),
            search: None,
            color: None,
            hidden: Computed::default(),
            display_mode: NavigationTitleDisplayMode::default(),
        }
    }
}

impl<T> Bar<T> {
    /// Maps the toolbar payload carried by this bar.
    pub fn map_toolbar<U>(self, f: impl FnOnce(T) -> U) -> Bar<U> {
        Bar {
            title: self.title,
            subtitle: self.subtitle,
            toolbar: f(self.toolbar),
            search: self.search,
            color: self.color,
            hidden: self.hidden,
            display_mode: self.display_mode,
        }
    }
}

/// Marks a subtree as a navigation link: activating it follows to a
/// destination.
///
/// The link itself renders and behaves as a plain button; this marker is how
/// a platform that draws a destination-following affordance around the row a
/// link sits in — the iOS disclosure chevron — recognizes one. It carries no
/// behavior, and a backend with no such affordance (a Mac list, say) ignores
/// it explicitly.
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
#[non_exhaustive]
pub enum BarColor {
    /// An author-specified color that has not been resolved yet.
    Unresolved(Computed<Color>),
    /// A color resolved against the effective environment.
    Resolved(Computed<WorkingColor>),
}

impl BarColor {
    /// Returns the resolved color, or `None` while still unresolved.
    #[must_use]
    pub const fn resolved(&self) -> Option<&Computed<WorkingColor>> {
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
    pub const fn expect_resolved(&self) -> &Computed<WorkingColor> {
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
#[non_exhaustive]
pub struct NavigationLink<Label, Content> {
    /// The label view displayed for this link
    label: Label,
    /// A function that creates the destination view when the link is activated
    content: Content,
}

impl<Label, Content> NavigationLink<Label, Content> {
    /// The label view displayed for this link
    #[must_use]
    pub const fn label(&self) -> &Label {
        &self.label
    }

    /// A function that creates the destination view when the link is activated
    #[must_use]
    pub const fn content(&self) -> &Content {
        &self.content
    }
}

/// A typed navigation link that pushes a route value onto a path-backed stack.
#[must_use]
#[derive(Debug)]
#[non_exhaustive]
pub struct NavigationValueLink<Label, T> {
    /// The label view displayed for this link.
    label: Label,
    /// The route value pushed when the link is activated.
    value: T,
}

impl<Label, T> NavigationValueLink<Label, T> {
    /// The label view displayed for this link.
    #[must_use]
    pub const fn label(&self) -> &Label {
        &self.label
    }

    /// The route value pushed when the link is activated.
    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }
}

impl<Label, Content, V, Tb> NavigationLink<Label, Content>
where
    Label: IntoLabel + 'static,
    Content: ViewBuilder<Output = NavigationView<V, Tb>>,
    V: View,
    Tb: NavigationToolbarItems,
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
///
/// `V` is the root's content type, `P` the path whose routes the pages above
/// the root show (`()` for a stack only links push onto), `F` the
/// destinations that build those pages, and `Tb` the root's toolbar type.
#[must_use]
pub struct NavigationStack<V, P, F, Tb = ()> {
    root: NavigationView<V, Tb>,
    path: P,
    destination: F,
    transition: AnyNavigationTransition,
}

impl<V, P, F, Tb: Debug> Debug for NavigationStack<V, P, F, Tb> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NavigationStack")
            .field("root", &self.root)
            .field("transition", &self.transition)
            .finish_non_exhaustive()
    }
}

/// The destinations of a stack whose path holds routes of several types, one
/// per route type, registered by [`NavigationStack::destination`].
#[doc(hidden)]
pub struct RouteDestinations<Prev, R, F> {
    prev: Prev,
    destination: Rc<F>,
    route: PhantomData<fn(R)>,
}

impl<Prev, R, F> Debug for RouteDestinations<Prev, R, F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RouteDestinations")
            .field("route", &core::any::type_name::<R>())
            .finish_non_exhaustive()
    }
}

/// Builds the page for one route of a heterogeneous path, in the child slot
/// `S`.
type RouteBuilder<S> = Box<dyn Fn(&ErasedNavigationRoute) -> NavigationPage<S>>;

/// A chain of [`RouteDestinations`], turned into its lookup table once the
/// stack's backend, and so the pages' slot, is known.
#[doc(hidden)]
pub trait RouteTable: 'static {
    /// Registers each destination in `table`, earliest first.
    fn register<S: ChildSlot>(self, table: &mut BTreeMap<TypeId, RouteBuilder<S>>);
}

impl RouteTable for () {
    fn register<S: ChildSlot>(self, _table: &mut BTreeMap<TypeId, RouteBuilder<S>>) {}
}

impl<Prev, R, F, D, Tb> RouteTable for RouteDestinations<Prev, R, F>
where
    Prev: RouteTable,
    R: Clone + PartialEq + 'static,
    F: 'static + Fn(R) -> NavigationView<D, Tb>,
    D: View,
    Tb: NavigationToolbarItems,
{
    fn register<S: ChildSlot>(self, table: &mut BTreeMap<TypeId, RouteBuilder<S>>) {
        self.prev.register(table);
        let destination = self.destination;
        let previous = table.insert(
            TypeId::of::<R>(),
            Box::new(move |route| {
                let route = route
                    .downcast_ref::<R>()
                    .expect("registered navigation destination type must match route type")
                    .clone();
                let destination = Rc::clone(&destination);
                NavigationPage::new(move || destination(route))
            }),
        );
        if previous.is_some() {
            contract_violation!(
                "navigation destination for `{}` was registered more than once; the later registration replaces the earlier one",
                core::any::type_name::<R>()
            );
        }
    }
}

/// The page a heterogeneous path's `route` shows.
fn route_page<S: ChildSlot>(
    table: &BTreeMap<TypeId, RouteBuilder<S>>,
    route: &ErasedNavigationRoute,
) -> NavigationPage<S> {
    let Some(build) = table.get(&route.type_id()) else {
        contract_violation!(
            "heterogeneous navigation route `{}` has no registered destination; an empty page is shown",
            route.type_name()
        );
        return NavigationPage::new(|| NavigationView::new("", ()));
    };
    build(route)
}

/// What a stack does once its backend placed the controller that realizes
/// it: drive the controller from its path and scope its pages.
type StackAttach = Box<dyn FnOnce(&NavigationController, &mut Environment)>;

/// A navigation stack as its backend receives it: the root page with its
/// content in the backend's child slot `S`, and how the stack attaches to the
/// [`NavigationController`] the backend realizes it with.
pub struct NavigationStackLayout<S> {
    root: NavigationPage<S>,
    attach: Option<StackAttach>,
    transition: AnyNavigationTransition,
}

impl<S> Debug for NavigationStackLayout<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NavigationStackLayout")
            .field("transition", &self.transition)
            .finish_non_exhaustive()
    }
}

impl<S: 'static> NativeView for NavigationStackLayout<S> {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

impl<S: 'static> NavigationStackLayout<S> {
    /// The same stack with its root page's content and toolbar mapped through `f`: a
    /// backend holds them in its own slot.
    #[must_use]
    pub fn map_slot<T: 'static>(
        self,
        f: impl Fn(S) -> T + Clone + 'static,
    ) -> NavigationStackLayout<T> {
        let Self {
            root,
            attach,
            transition,
        } = self;
        NavigationStackLayout {
            root: root.map_slot(f),
            attach,
            transition,
        }
    }
}

impl<S> NavigationStackLayout<S> {
    /// Returns the configured transition style.
    #[must_use]
    pub const fn transition_style(&self) -> &AnyNavigationTransition {
        &self.transition
    }

    /// Attaches the stack to the [`NavigationController`] its backend
    /// inserted into `env`, and builds the root page in the environment the
    /// stack's pages share.
    ///
    /// # Panics
    ///
    /// Panics when a path-backed stack is resolved in an environment without
    /// a [`NavigationController`]: its backend inserts one first.
    pub fn resolve(self, env: &Environment) -> ResolvedNavigationView<S> {
        let mut env = TransitionKeyRegistry::scoped(env);
        if let Some(attach) = self.attach {
            let controller = env.extract::<NavigationController>().expect(
                "a navigation stack's backend inserts its NavigationController before resolving the stack",
            );
            attach(&controller, &mut env);
        }
        self.root.in_environment(Some(env)).build()
    }
}

impl<V: View, Tb: NavigationToolbarItems> NavigationStack<V, (), (), Tb> {
    /// Creates a new navigation stack with the specified root view.
    ///
    /// # Arguments
    /// * `root` - The semantic root destination of the navigation stack
    pub fn new(root: NavigationView<V, Tb>) -> Self {
        Self {
            root,
            path: (),
            destination: (),
            transition: AnyNavigationTransition::new(navigation_transition::automatic()),
        }
    }
}

impl<V, P, F, Tb> NavigationStack<V, P, F, Tb> {
    /// Returns the configured transition style.
    #[must_use]
    pub const fn transition_style(&self) -> &AnyNavigationTransition {
        &self.transition
    }

    /// Sets the page transition for push and pop operations.
    ///
    /// Accepts any [`NavigationTransition`] (`navigation_transition::{automatic,
    /// fade, zoom, none, pair}`), any core `Transition` (such as
    /// `PropertyTransition::opacity()`), a `TransitionSpec`, or a
    /// [`NavigationTransitionPair`].
    ///
    /// This is the default for every page of the stack. A destination that
    /// declares its own [`NavigationView::page_transition`] (or a content-root
    /// `.transition(...)`) replaces it for that page's push and pop, in this
    /// order of precedence:
    ///
    /// 1. [`NavigationView::page_transition`] on the destination.
    /// 2. A `.transition(...)` on the page's content root when no page
    ///    transition is set on the `NavigationView`.
    /// 3. This stack transition.
    pub fn page_transition<M>(mut self, transition: impl IntoNavigationTransition<M>) -> Self {
        self.transition = transition.into_navigation_transition();
        self
    }
}

impl<V: View, T: 'static, Tb: NavigationToolbarItems>
    NavigationStack<V, NavigationPath<T>, (), Tb>
{
    /// Creates a new navigation stack with the specified navigation path and root view.
    ///
    /// # Arguments
    /// * `path` - The navigation path representing the current stack
    /// * `root` - The root view of the navigation stack
    pub fn with_path(path: NavigationPath<T>, root: NavigationView<V, Tb>) -> Self {
        Self {
            root,
            path,
            destination: (),
            transition: AnyNavigationTransition::new(navigation_transition::automatic()),
        }
    }
}

impl<V: View, T: 'static + PartialEq, Tb: NavigationToolbarItems>
    NavigationStack<V, NavigationPath<T>, (), Tb>
{
    /// Sets the destination builder for the navigation stack.
    ///
    /// # Arguments
    /// * `destination` - A function that creates a `NavigationView` from a path component
    pub fn destination<F, D, DTb>(
        self,
        destination: F,
    ) -> NavigationStack<V, NavigationPath<T>, F, Tb>
    where
        F: 'static + Fn(T) -> NavigationView<D, DTb>,
        D: View,
        DTb: NavigationToolbarItems,
    {
        NavigationStack {
            root: self.root,
            path: self.path,
            destination,
            transition: self.transition,
        }
    }
}

impl<V: View, Prev: RouteTable, Tb: NavigationToolbarItems>
    NavigationStack<V, NavigationPath<ErasedNavigationRoute>, Prev, Tb>
{
    /// Registers the destination for one concrete route type of a
    /// heterogeneous path.
    pub fn destination<R, F, D, DTb>(
        self,
        destination: F,
    ) -> NavigationStack<V, NavigationPath<ErasedNavigationRoute>, RouteDestinations<Prev, R, F>, Tb>
    where
        R: Clone + PartialEq + 'static,
        F: 'static + Fn(R) -> NavigationView<D, DTb>,
        D: View,
        DTb: NavigationToolbarItems,
    {
        NavigationStack {
            root: self.root,
            path: self.path,
            destination: RouteDestinations {
                prev: self.destination,
                destination: Rc::new(destination),
                route: PhantomData,
            },
            transition: self.transition,
        }
    }

    /// Registers a restorable destination for one concrete route type of a
    /// heterogeneous path, keying serialized routes by the explicit stable
    /// `key`.
    #[cfg(feature = "serde")]
    pub fn restorable_destination<R, F, D, DTb>(
        self,
        key: &'static str,
        destination: F,
    ) -> NavigationStack<V, NavigationPath<ErasedNavigationRoute>, RouteDestinations<Prev, R, F>, Tb>
    where
        R: Clone + PartialEq + Serialize + DeserializeOwned + 'static,
        F: 'static + Fn(R) -> NavigationView<D, DTb>,
        D: View,
        DTb: NavigationToolbarItems,
    {
        self.path.register_restoration::<R>(key);
        self.destination(destination)
    }
}

/// A [`NavigationStack`] that reaches its backend: it becomes the
/// [`NavigationStackLayout`] of the backend's child slot.
#[doc(hidden)]
pub trait IntoNavigationStackLayout: 'static {
    /// The stack with its pages built in the child slot `S`.
    fn into_layout<S: ChildSlot>(self) -> NavigationStackLayout<S>;
}

impl<V: View, Tb: NavigationToolbarItems> IntoNavigationStackLayout
    for NavigationStack<V, (), (), Tb>
{
    fn into_layout<S: ChildSlot>(self) -> NavigationStackLayout<S> {
        let Self {
            root, transition, ..
        } = self;
        NavigationStackLayout {
            root: NavigationPage::new(move || root),
            attach: None,
            transition,
        }
    }
}

impl<V, Tb, T, F, D, DTb> IntoNavigationStackLayout for NavigationStack<V, NavigationPath<T>, F, Tb>
where
    V: View,
    Tb: NavigationToolbarItems,
    T: 'static + Clone + PartialEq,
    F: 'static + Fn(T) -> NavigationView<D, DTb>,
    D: View,
    DTb: NavigationToolbarItems,
{
    fn into_layout<S: ChildSlot>(self) -> NavigationStackLayout<S> {
        let destination = Rc::new(self.destination);
        path_stack_layout(
            self.root,
            self.path,
            self.transition,
            |left, right| left == right,
            move |route: &T| {
                let destination = Rc::clone(&destination);
                let route = route.clone();
                NavigationPage::new(move || destination(route))
            },
        )
    }
}

impl<V, Tb, Prev, R, F> IntoNavigationStackLayout
    for NavigationStack<V, NavigationPath<ErasedNavigationRoute>, RouteDestinations<Prev, R, F>, Tb>
where
    V: View,
    Tb: NavigationToolbarItems,
    RouteDestinations<Prev, R, F>: RouteTable,
{
    fn into_layout<S: ChildSlot>(self) -> NavigationStackLayout<S> {
        let mut table = BTreeMap::new();
        self.destination.register(&mut table);
        path_stack_layout(
            self.root,
            self.path,
            self.transition,
            ErasedNavigationRoute::same_route,
            move |route| route_page(&table, route),
        )
    }
}

impl<V, P, F, Tb> View for NavigationStack<V, P, F, Tb>
where
    Self: IntoNavigationStackLayout,
{
    /// The erased form, for code that expands views by hand.
    fn body(self, env: &Environment) -> impl View {
        let stack_env = TransitionKeyRegistry::scoped(env);
        let mut layout = self.into_layout::<AnyView>();
        layout.transition = layout.transition.resolve(&stack_env);
        Native::new(layout)
    }

    /// Reaches the backend with every page built in the backend's child
    /// slot.
    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        let stack_env = TransitionKeyRegistry::scoped(env);
        let mut layout = self.into_layout::<B::Slot>();
        layout.transition = layout.transition.resolve(&stack_env);
        backend.native(layout, &stack_env, cx)
    }
}

/// Projects a new path snapshot onto the backend stack as one transaction.
///
/// `same` decides route identity — plain equality for a typed path, type-and-value
/// equality for a heterogeneous one — and `build` turns a surviving route into its
/// page.
fn reconcile_navigation_path<R: Clone + 'static, S: ChildSlot>(
    receiver: &NavigationController,
    current_path: &mut Vec<R>,
    next_path: Vec<R>,
    same: &impl Fn(&R, &R) -> bool,
    build: &impl Fn(&R) -> NavigationPage<S>,
) {
    let shared_prefix = shared_prefix_len(current_path, &next_path, same);
    let inserted = next_path.iter().skip(shared_prefix).map(build).collect();
    receiver.apply_path(shared_prefix, inserted);

    *current_path = next_path;
}

/// Subscribes a controller to a path, reconciling every snapshot into one
/// transaction.
fn subscribe_navigation_path<R: Clone + 'static, S: ChildSlot>(
    path: &NavigationPath<R>,
    receiver: &NavigationController,
    same: impl Fn(&R, &R) -> bool + 'static,
    build: impl Fn(&R) -> NavigationPage<S> + 'static,
) -> impl nami::watcher::WatcherGuard {
    let current_path = RefCell::new(Vec::new());
    // A path write issued while a reconcile still runs lands here
    // re-entrantly; queue it and let the dispatch in flight drain it once the
    // current reconcile returns, keeping snapshots in order.
    let pending = SerialDispatch::new();
    let receiver = receiver.clone();
    let reconcile = Rc::new(move |next_path: Vec<R>| {
        pending.deliver(next_path, |next_path| {
            reconcile_navigation_path(
                &receiver,
                &mut current_path.borrow_mut(),
                next_path,
                &same,
                &build,
            );
        });
    });
    // Subscribing delivers nothing: the path's current routes are the
    // controller's starting stack.
    reconcile(path.snapshot());
    path.watch(move |next_path| reconcile(next_path))
}

/// The layout of every path-backed stack: once attached, it publishes the
/// `Navigator`, installs the pop handler, and keeps the path subscription
/// alive on the controller so it outlives the root page.
fn path_stack_layout<V, Tb, R, S>(
    root: NavigationView<V, Tb>,
    path: NavigationPath<R>,
    transition: AnyNavigationTransition,
    same: impl Fn(&R, &R) -> bool + 'static,
    build: impl Fn(&R) -> NavigationPage<S> + 'static,
) -> NavigationStackLayout<S>
where
    V: View,
    Tb: NavigationToolbarItems,
    R: Clone + 'static,
    S: ChildSlot,
{
    let navigator = Navigator(path.clone());
    NavigationStackLayout {
        root: NavigationPage::new(move || root),
        attach: Some(Box::new(move |receiver, env| {
            env.insert(State(navigator));
            receiver.retain_environment(env.clone());
            receiver.install_path_pop_handler({
                let path = path.clone();
                move |count| path.pop_n(count)
            });
            let guard = subscribe_navigation_path(&path, receiver, same, build);
            receiver.retain(Retain::new(guard));
        })),
        transition,
    }
}

impl<Label, Content, V, Tb> View for NavigationLink<Label, Content>
where
    Label: IntoLabel + 'static,
    Content: ViewBuilder<Output = NavigationView<V, Tb>>,
    V: View,
    Tb: NavigationToolbarItems,
{
    /// The erased form, for code that expands views by hand: the link pushes
    /// its destination erased.
    fn body(self, env: &Environment) -> impl View {
        self.link::<AnyView>(env)
    }

    /// Reaches the backend as a link pushing its destination in the
    /// backend's child slot.
    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        self.link::<B::Slot>(env).dispatch(backend, env, cx)
    }
}

impl<Label, Content, V, Tb> NavigationLink<Label, Content>
where
    Label: IntoLabel + 'static,
    Content: ViewBuilder<Output = NavigationView<V, Tb>>,
    V: View,
    Tb: NavigationToolbarItems,
{
    /// The link as a button pushing its destination built in the slot `S`.
    fn link<S: ChildSlot>(self, env: &Environment) -> impl View + use<Label, Content, V, Tb, S> {
        debug_assert!(
            env.get::<State<NavigationController>>().is_some(),
            "NavigationLink used outside of a navigation context"
        );

        let destination = Rc::new(self.content);
        Metadata::new(
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
                    let destination = Rc::clone(&destination);
                    receiver.push(NavigationPage::<S>::new(move || destination.build()));
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
        let link = if let Ok(navigator) = Navigator::<T>::extract(env) {
            OneOf3::A(
                button(self.label)
                    .style(ButtonStyle::Plain)
                    .action(move || navigator.push(value.clone())),
            )
        } else if let Ok(navigator) = Navigator::<ErasedNavigationRoute>::extract(env) {
            OneOf3::B(
                button(self.label)
                    .style(ButtonStyle::Plain)
                    .action(move || navigator.push(value.clone())),
            )
        } else {
            contract_violation!(
                "NavigationLink::value used outside of a compatible path-backed stack; the link does nothing when activated"
            );
            OneOf3::C(button(self.label).style(ButtonStyle::Plain).action(|| {}))
        };
        Metadata::new(link, NavigationLinkHint)
    }
}

impl<S> ResolvedNavigationView<S> {
    /// Resolves environment-dependent native fields without snapshotting signals.
    ///
    /// A backend resolves a navigation view against the environment it
    /// realizes it in before reading its bar, whether the view arrived as a
    /// view or as a page a stack built. Resolving again is harmless.
    #[doc(hidden)]
    pub fn resolve_native_fields(&mut self, env: &Environment) {
        if let Some(title) = self.bar.title.take() {
            self.bar.title = Some(Text::from(title.resolve(env)));
        }

        if let Some(subtitle) = self.bar.subtitle.take() {
            self.bar.subtitle = Some(Text::from(subtitle.resolve(env)));
        }

        if let Some(search) = &mut self.bar.search {
            search.set_prompt(waterui_text::Text::computed(
                search.prompt().resolve(env).into_content(),
            ));
        }

        // A toolbar item's name crosses the boundary as resolved content, so it
        // has to stop depending on the environment here — `Text::content` panics
        // on an environment-dependent text.
        for item in &mut self.bar.toolbar.items {
            if let Some(title) = item.title.take() {
                item.title = Some(Text::computed(title.resolve(env).into_content()));
            }
        }

        if let Some(BarColor::Unresolved(color)) = &self.bar.color {
            let resolved_env = env.clone();
            self.bar.color = Some(BarColor::Resolved(flatten_signal(
                color.clone().map(move |color| color.resolve(&resolved_env)),
            )));
        }

        if let Some(transition) = self.transition.take() {
            self.transition = Some(transition.resolve(env));
        }
    }
}

impl<C> NavigationView<C, ()> {
    /// Creates a new navigation view.
    ///
    /// # Arguments
    ///
    /// * `title` - The semantic title to display in the navigation bar
    /// * `content` - The content view to display
    pub fn new(title: impl IntoText, content: C) -> Self
    where
        C: View,
    {
        let bar = Bar {
            title: Some(title.into_text()),
            ..Default::default()
        };

        Self {
            bar,
            content,
            state: NavigationDestinationState::default(),
            transition: Option::None,
        }
    }
}

impl<C, T> NavigationView<C, T> {
    /// Sets how this destination arrives and leaves, overriding its stack's
    /// transition without shadowing `ViewExt::transition`.
    ///
    /// Accepts any [`NavigationTransition`] (`navigation_transition::{automatic,
    /// fade, zoom, none, pair}`), any core `Transition` (such as
    /// `PropertyTransition::opacity()`), a `TransitionSpec`, or a
    /// [`NavigationTransitionPair`].
    ///
    /// ```rust
    /// use waterui::navigation::{NavigationView, navigation_transition};
    /// use waterui::prelude::*;
    ///
    /// #[derive(Hash, PartialEq, Eq)]
    /// struct PhotoId(i32);
    ///
    /// fn photo_page(index: i32) -> NavigationView<impl View> {
    ///     NavigationView::new("Photo", text!("photo {index}"))
    ///         .page_transition(navigation_transition::zoom(PhotoId(index)))
    /// }
    /// ```
    pub fn page_transition<M>(mut self, transition: impl IntoNavigationTransition<M>) -> Self {
        self.transition = Some(transition.into_navigation_transition());
        self
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
    ///     .navigation_title("Settings")
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
        self.bar.subtitle = Some(subtitle.into_text());
        self
    }

    /// Installs semantic native toolbar content.
    pub fn navigation_toolbar<Items: NavigationToolbarItems>(
        self,
        toolbar: NavigationToolbar<Items>,
    ) -> NavigationView<C, Items> {
        NavigationView {
            bar: self.bar.map_toolbar(|_| toolbar.items),
            content: self.content,
            state: self.state,
            transition: self.transition,
        }
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

    /// Maps the destination's toolbar type, preserving its bar chrome, content,
    /// and lifecycle state.
    pub fn map_toolbar<U>(self, f: impl FnOnce(T) -> U) -> NavigationView<C, U> {
        NavigationView {
            bar: Bar {
                title: self.bar.title,
                subtitle: self.bar.subtitle,
                toolbar: f(self.bar.toolbar),
                search: self.bar.search,
                color: self.bar.color,
                hidden: self.bar.hidden,
                display_mode: self.bar.display_mode,
            },
            content: self.content,
            state: self.state,
            transition: self.transition,
        }
    }
}

/// Translation key for the back affordance every navigation backend draws.
pub const NAVIGATION_BACK_LABEL_KEY: &str = "waterui.navigation.back";

/// Semantic label for the back affordance.
///
/// Backends must render and speak this rather than a string of their own, so a
/// single catalog entry translates the back button on every platform at once.
#[must_use]
pub fn navigation_back_label() -> waterui_controls::label::Label<waterui_controls::label::TitleLabel>
{
    waterui_text::Text::localized_or(NAVIGATION_BACK_LABEL_KEY, "Back").into_label()
}

/// Convenience function to create a navigation view.
///
/// # Arguments
///
/// * `title` - The semantic title to display in the navigation bar
/// * `view` - The content view to display
pub fn navigation<V: View>(title: impl IntoText, view: V) -> NavigationView<V> {
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
        CustomNavigationController, IntoNavigationStackLayout, NavigationController,
        NavigationLink, NavigationPage, NavigationPath, NavigationStack, NavigationTransaction,
        NavigationValueLink, NavigationView, Navigator, ResolvedNavigationView, shared_prefix_len,
        subscribe_navigation_path,
    };
    use waterui_core::{
        AnyView, Environment, Metadata, View,
        backend::ChildSlot,
        extract::{Extractor, State},
    };

    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "the signature is fixed by the destination-builder closure the reconciler takes"
    )]
    fn route_destination(_route: &u8) -> NavigationPage<AnyView> {
        NavigationPage::new(|| NavigationView::new("Route", ()))
    }

    /// A page of `title`, erased.
    fn page(title: &'static str) -> NavigationPage<AnyView> {
        NavigationPage::new(move || NavigationView::new(title, ()))
    }

    fn equal<T: PartialEq>(left: &T, right: &T) -> bool {
        left == right
    }

    /// A stack's root page builds in the environment it resolves in.
    #[test]
    fn a_stack_root_resolves_in_its_environment() {
        let mut env = Environment::new();
        env.insert(BaseMarker);
        let root = NavigationStack::new(NavigationView::new("Root", ()))
            .into_layout::<AnyView>()
            .resolve(&env);
        let content = root
            .content
            .downcast::<Metadata<Environment, ()>>()
            .expect("the root's content keeps its type, scoped to the stack's environment");
        assert!(content.value.get::<BaseMarker>().is_some());
    }

    /// A child slot of a backend other than the receiver's.
    struct OtherSlot;

    impl ChildSlot for OtherSlot {
        fn from_view<V: View>(_view: V) -> Self {
            Self
        }
    }

    /// Takes pages in [`OtherSlot`] and counts them.
    struct OtherSlotController {
        pushes: Rc<Cell<usize>>,
    }

    impl CustomNavigationController for OtherSlotController {
        type Slot = OtherSlot;

        fn apply(&mut self, transaction: NavigationTransaction<OtherSlot>) {
            for page in transaction.inserted {
                let _: OtherSlot = page.build().content;
                self.pushes.set(self.pushes.get() + 1);
            }
        }
    }

    /// A receiver takes pages in its own slot, and erased ones converted
    /// into it.
    #[test]
    fn a_receiver_takes_its_own_slot_and_erased_pages() {
        let pushes = Rc::new(Cell::new(0));
        let controller = NavigationController::new(OtherSlotController {
            pushes: Rc::clone(&pushes),
        });
        controller.push(NavigationPage::<OtherSlot>::new(|| {
            NavigationView::new("Typed", ())
        }));
        controller.push(page("Erased"));
        assert_eq!(pushes.get(), 2);
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "was pushed onto a stack realized by another backend")
    )]
    fn a_page_in_another_backends_slot_is_not_pushed() {
        let pushes = Rc::new(Cell::new(0));
        let controller = NavigationController::new(CountingNavigationController {
            pushes: Rc::clone(&pushes),
            pops: Rc::new(Cell::new(0)),
        });
        controller.push(NavigationPage::<OtherSlot>::new(|| {
            NavigationView::new("Foreign", ())
        }));
        // Release outcome: nothing is pushed.
        assert_eq!(pushes.get(), 0);
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
        type Slot = AnyView;

        fn apply(&mut self, transaction: NavigationTransaction<AnyView>) {
            self.pushes
                .set(self.pushes.get() + transaction.inserted.len());
            self.pops.set(self.pops.get() + transaction.removed);
        }
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "navigation pop count must be non-zero")
    )]
    fn a_zero_pop_request_pops_nothing() {
        let pushes = Rc::new(Cell::new(0));
        let pops = Rc::new(Cell::new(0));
        let controller = NavigationController::new(CountingNavigationController {
            pushes: Rc::clone(&pushes),
            pops: Rc::clone(&pops),
        });
        controller.push(page("One"));

        // Release outcome: the request is ignored.
        controller.request_pop(0);

        assert_eq!(pops.get(), 0);
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(
            expected = "a navigation controller cannot drive more than one explicit path"
        )
    )]
    fn a_second_path_handler_replaces_the_first() {
        let controller = NavigationController::new(CountingNavigationController {
            pushes: Rc::new(Cell::new(0)),
            pops: Rc::new(Cell::new(0)),
        });
        let first = Rc::new(Cell::new(0));
        let second = Rc::new(Cell::new(0));
        controller.install_path_pop_handler({
            let first = Rc::clone(&first);
            move |count| first.set(first.get() + count)
        });
        controller.install_path_pop_handler({
            let second = Rc::clone(&second);
            move |count| second.set(second.get() + count)
        });

        controller.path_pop_handler()(1);

        assert_eq!((first.get(), second.get()), (0, 1));
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

        controller.push(page("Link"));
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

        controller.push(page("Link"));
        pops.set(0);
        path.replace([2_u8]);

        assert_eq!(
            pops.get(),
            2,
            "replacing the route must remove both the route entry and the link entry above it"
        );
    }

    /// A receiver that writes the watched path while `apply` is still running.
    struct PathWritingNavigationController {
        path: Rc<RefCell<Option<NavigationPath<u8>>>>,
        inside: Cell<bool>,
        applies: Cell<usize>,
        applied: Rc<RefCell<alloc::vec::Vec<(usize, usize, usize)>>>,
    }

    impl CustomNavigationController for PathWritingNavigationController {
        type Slot = AnyView;

        fn apply(&mut self, transaction: NavigationTransaction<AnyView>) {
            assert!(
                !self.inside.replace(true),
                "the receiver must not run re-entrantly"
            );
            self.applied.borrow_mut().push((
                transaction.retained_prefix,
                transaction.removed,
                transaction.inserted.len(),
            ));
            self.applies.set(self.applies.get() + 1);
            // The write must happen on a post-registration delivery: the
            // collection watcher fires its initial snapshot before it is
            // registered, so a write during the first apply can never notify.
            if self.applies.get() == 2 {
                self.path
                    .borrow()
                    .as_ref()
                    .expect("the test installs the path")
                    .replace([9_u8, 8]);
            }
            self.inside.set(false);
        }
    }

    /// A path mutation issued while the backend receiver is applying a
    /// transaction must reconcile after `apply` returns, not re-enter it.
    #[test]
    fn a_path_write_during_apply_is_reconciled_after_apply_returns() {
        let path = NavigationPath::from(vec![0_u8]);
        let path_slot = Rc::new(RefCell::new(None));
        let applied = Rc::new(RefCell::new(alloc::vec::Vec::new()));
        let controller = NavigationController::new(PathWritingNavigationController {
            path: Rc::clone(&path_slot),
            inside: Cell::new(false),
            applies: Cell::new(0),
            applied: Rc::clone(&applied),
        });
        *path_slot.borrow_mut() = Some(path.clone());

        let _guard = subscribe_navigation_path(&path, &controller, equal, route_destination);
        path.push(1);

        assert_eq!(
            &*applied.borrow(),
            &[(0, 0, 1), (1, 0, 1), (0, 2, 2)],
            "the nested replace reconciles against the state left by the in-flight apply"
        );
        assert_eq!(path.snapshot(), vec![9, 8]);
    }

    /// A receiver that commits another mutation while `apply` is still running.
    struct CommittingNavigationController {
        controller: Rc<RefCell<Option<NavigationController>>>,
        inside: Cell<bool>,
        fired: Cell<bool>,
        ids: Rc<RefCell<alloc::vec::Vec<u64>>>,
    }

    impl CustomNavigationController for CommittingNavigationController {
        type Slot = AnyView;

        fn apply(&mut self, transaction: NavigationTransaction<AnyView>) {
            assert!(
                !self.inside.replace(true),
                "the receiver must not run re-entrantly"
            );
            self.ids.borrow_mut().push(transaction.id);
            if !self.fired.replace(true) {
                self.controller
                    .borrow()
                    .as_ref()
                    .expect("the test installs the controller")
                    .push(page("Nested"));
            }
            self.inside.set(false);
        }
    }

    /// A commit issued while the backend receiver is applying a transaction
    /// must be delivered after `apply` returns, in order, not re-entrantly.
    #[test]
    fn a_commit_during_apply_is_delivered_after_apply_returns() {
        let slot = Rc::new(RefCell::new(None));
        let ids = Rc::new(RefCell::new(alloc::vec::Vec::new()));
        let controller = NavigationController::new(CommittingNavigationController {
            controller: Rc::clone(&slot),
            inside: Cell::new(false),
            fired: Cell::new(false),
            ids: Rc::clone(&ids),
        });
        *slot.borrow_mut() = Some(controller.clone());

        controller.push(page("Outer"));

        assert_eq!(&*ids.borrow(), &[1, 2]);
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

        controller.push(page("One"));
        controller.push(page("Two"));

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
        pushed: Rc<RefCell<Option<ResolvedNavigationView<AnyView>>>>,
    }

    impl CustomNavigationController for RecordingNavigationController {
        type Slot = AnyView;

        fn apply(&mut self, transaction: NavigationTransaction<AnyView>) {
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
        retained_env.insert(State(Navigator(NavigationPath::<TestRoute>::new())));
        controller.retain_environment(retained_env);

        controller.push(NavigationPage::<AnyView>::new(|| {
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
            .downcast::<Metadata<Environment, NavigationValueLink<&'static str, TestRoute>>>()
            .expect("pushed content should carry retained navigation environment");

        assert!(metadata.value.get::<BaseMarker>().is_some());
        assert!(Navigator::<TestRoute>::extract(&metadata.value).is_ok());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn restorable_destination_registers_explicit_key_for_stack_restoration() {
        use alloc::format;
        use serde::de::DeserializeSeed as _;
        use serde_json::{Deserializer as JsonDeserializer, to_string};

        let path = NavigationPath::heterogeneous();
        let _stack = NavigationStack::with_path(path.clone(), NavigationView::new("Home", ()))
            .restorable_destination("project", |id: u32| {
                NavigationView::new("Project", format!("Project {id}"))
            });

        path.push(9_u32);
        let encoded = to_string(&path).expect("restorable destination must serialize");
        assert_eq!(encoded, r#"[["project",9]]"#);

        let restored = NavigationPath::heterogeneous();
        let _restored_stack =
            NavigationStack::with_path(restored.clone(), NavigationView::new("Home", ()))
                .restorable_destination("project", |id: u32| {
                    NavigationView::new("Project", format!("Project {id}"))
                });
        let mut deserializer = JsonDeserializer::from_str(&encoded);
        restored
            .restoration()
            .deserialize(&mut deserializer)
            .expect("restorable destination must deserialize");

        let routes = restored.snapshot();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].downcast_ref::<u32>(), Some(&9));
    }
}
