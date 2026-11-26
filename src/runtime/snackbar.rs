//! Snackbar notification system for `WaterUI`.
//!
//! Snackbars provide brief messages about app processes. They automatically
//! disappear after a timeout and can optionally include a leading icon, an
//! action button, and a close button. Visual presentation, layout, stacking
//! spacing, and entrance/exit animations belong to each backend.
//!
//! # Examples
//!
//! Attach [`ViewExt::snackbar_host`] to a root view and extract
//! [`SnackbarManager`] in any descendant handler:
//!
//! ```rust
//! use core::time::Duration;
//!
//! use waterui::prelude::*;
//! use waterui::snackbar::{Snackbar, SnackbarManager, SnackbarPosition};
//!
//! fn view() -> impl View {
//!     vstack((
//!         // Show a simple message
//!         button("Save").action(|manager: SnackbarManager| {
//!             manager.show(Snackbar::new("File saved"));
//!         }),
//!         // With action and custom duration
//!         button("Delete").action(|manager: SnackbarManager| {
//!             manager.show(
//!                 Snackbar::new("Item deleted")
//!                     .action("Undo", || {})
//!                     .duration(Duration::from_secs(5)),
//!             );
//!         }),
//!         // Custom position
//!         button("Reconnect").action(|manager: SnackbarManager| {
//!             manager.show(
//!                 Snackbar::new("Connection restored")
//!                     .position(SnackbarPosition::TopCenter),
//!             );
//!         }),
//!     ))
//!     .snackbar_host()
//! }
//! ```

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::fmt::{self, Debug, Formatter};
use core::marker::PhantomData;
use core::time::Duration;

use executor_core::spawn_local;
use nami::{Binding, Computed, Signal as _, SignalExt as _};
use waterui_controls::{IntoLabel, Label, SemanticLabel, slot::SlotBuilder};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::extract::State;
use waterui_core::handler::{
    BoxedEventAction, EventHandler, Handler, ReportExtraction, SharedAction, ViewBuilder,
    boxed_event_handler, shared_action,
};
use waterui_core::metadata::MetadataKey;
use waterui_core::plugin::Plugin;
use waterui_core::{AnyView, Environment, Metadata, View};
use waterui_macros::state;
#[cfg(feature = "flow-markdown")]
use waterui_markdown::CodeCopyFeedback;
#[cfg(feature = "flow-markdown")]
use waterui_text::code::OnCopied;
use waterui_text::{IntoText, Text};

use crate::ViewExt;

/// Maximum number of snackbars visible at once *per placement*. When a new
/// snackbar arrives over this limit at the same placement, the oldest live one
/// at that placement is dismissed with [`SnackbarDismissReason::Replaced`].
const MAX_VISIBLE_SNACKBARS: usize = 3;

/// Position for snackbar display.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnackbarPosition {
    /// Bottom center of the screen (default).
    #[default]
    BottomCenter,
    /// Top center of the screen.
    TopCenter,
    /// Bottom leading (left in LTR).
    BottomLeading,
    /// Bottom trailing (right in LTR).
    BottomTrailing,
}

impl SnackbarPosition {
    /// Whether this placement anchors to the top edge.
    #[must_use]
    pub const fn is_top(self) -> bool {
        matches!(self, Self::TopCenter)
    }
}

/// An action button for the snackbar.
#[derive(Clone)]
pub struct SnackbarAction {
    /// The action button label.
    label: Label<SemanticLabel>,
    /// The callback when action is pressed.
    handler: SharedAction<()>,
}

impl SnackbarAction {
    /// The semantic action button label.
    #[must_use]
    pub const fn label(&self) -> &Label<SemanticLabel> {
        &self.label
    }
}

impl core::fmt::Debug for SnackbarAction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SnackbarAction")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// Why a snackbar presentation was dismissed, reported to
/// [`Snackbar::on_dismiss`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnackbarDismissReason {
    /// The snackbar's action button was pressed.
    Action,
    /// The auto-dismiss duration elapsed.
    Timeout,
    /// The trailing close button was pressed.
    Closed,
    /// The presentation was evicted to make room for a newer snackbar at the
    /// same placement.
    Replaced,
    /// The presentation was dismissed programmatically through
    /// [`SnackbarManager::dismiss`].
    Programmatic,
}

/// The snackbar's dismissal callback, shared by reference across every
/// presentation of a cloned [`Snackbar`].
#[derive(Clone)]
struct DismissalCallback(Rc<RefCell<BoxedEventAction<SnackbarDismissReason>>>);

impl DismissalCallback {
    fn call(&self, reason: SnackbarDismissReason, env: &waterui_core::Environment) {
        (self.0.borrow_mut())(reason, env).report_extraction();
    }
}

impl core::fmt::Debug for DismissalCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DismissalCallback").finish_non_exhaustive()
    }
}

/// Cloneable factory that constructs a snackbar's leading icon view on demand.
#[doc(hidden)]
pub struct IconFactory<I>(Rc<dyn Fn() -> I>);

impl<I> Clone for IconFactory<I> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<I> Debug for IconFactory<I> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("IconFactory").finish_non_exhaustive()
    }
}

/// Cloneable view adapter backed by an [`IconFactory`], allowing non-`Clone`
/// views inside a snackbar's icon slot.
#[doc(hidden)]
pub struct BuiltIcon<V>(IconFactory<V>);

impl<V> Clone for BuiltIcon<V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<V> Debug for BuiltIcon<V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("BuiltIcon").finish_non_exhaustive()
    }
}

impl<V: View> View for BuiltIcon<V> {
    fn body(self, _env: &Environment) -> impl View {
        (self.0.0)()
    }
}

/// Configuration for a snackbar notification.
///
/// Use the builder pattern to configure the snackbar before showing it.
///
/// # Examples
///
/// ```rust
/// use core::time::Duration;
///
/// use waterui::snackbar::Snackbar;
///
/// let snackbar = Snackbar::new("Message sent")
///     .duration(Duration::from_secs(3));
/// ```
pub struct Snackbar<I = NoIcon> {
    /// The message to display.
    message: Text,
    /// Optional icon builder to display before the message.
    icon: Option<IconFactory<I>>,
    /// Optional action button.
    action: Option<SnackbarAction>,
    /// Display duration before auto-dismiss.
    duration: Duration,
    /// Position on screen.
    position: SnackbarPosition,
    /// Whether to show a trailing close button.
    closeable: bool,
    /// Callback reporting each presentation's dismissal and its reason.
    dismissal: Option<DismissalCallback>,
    /// State injected into the action and dismissal handler environments.
    captured_env: waterui_core::Environment,
}

impl<I> Clone for Snackbar<I> {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            icon: self.icon.clone(),
            action: self.action.clone(),
            duration: self.duration,
            position: self.position,
            closeable: self.closeable,
            dismissal: self.dismissal.clone(),
            captured_env: self.captured_env.clone(),
        }
    }
}

impl<I> core::fmt::Debug for Snackbar<I> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Snackbar")
            .field("message", &self.message)
            .field("has_icon", &self.icon.is_some())
            .field("action", &self.action)
            .field("duration", &self.duration)
            .field("position", &self.position)
            .field("closeable", &self.closeable)
            .field("dismissal", &self.dismissal)
            .field("captured_env", &self.captured_env)
            .finish()
    }
}

/// Marker type for a [`Snackbar`] with no leading icon.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoIcon;

mod icon_sealed {
    pub trait Sealed {}
}

/// Leading-icon type accepted by [`Snackbar`], [`SnackbarManager`], and [`SnackbarHost`]:
/// either [`NoIcon`] or any [`View`].
#[doc(hidden)]
pub trait SnackbarIconSlot: icon_sealed::Sealed + Sized + 'static {
    type IconView: View + Clone;

    fn into_icon_view(factory: IconFactory<Self>) -> Option<Self::IconView>;

    fn from_plain(snackbar: Snackbar<NoIcon>) -> Snackbar<Self>;
}

impl icon_sealed::Sealed for NoIcon {}

impl SnackbarIconSlot for NoIcon {
    type IconView = ();

    fn into_icon_view(_factory: IconFactory<Self>) -> Option<Self::IconView> {
        None
    }

    fn from_plain(snackbar: Snackbar<NoIcon>) -> Snackbar<Self> {
        snackbar
    }
}

impl<V: View + 'static> icon_sealed::Sealed for V {}

impl<V: View + 'static> SnackbarIconSlot for V {
    type IconView = BuiltIcon<Self>;

    fn into_icon_view(factory: IconFactory<Self>) -> Option<Self::IconView> {
        Some(BuiltIcon(factory))
    }

    fn from_plain(snackbar: Snackbar<NoIcon>) -> Snackbar<Self> {
        snackbar.into()
    }
}

impl<V: View> From<Snackbar<NoIcon>> for Snackbar<V> {
    fn from(snackbar: Snackbar<NoIcon>) -> Self {
        Self {
            message: snackbar.message,
            icon: None,
            action: snackbar.action,
            duration: snackbar.duration,
            position: snackbar.position,
            closeable: snackbar.closeable,
            dismissal: snackbar.dismissal,
            captured_env: snackbar.captured_env,
        }
    }
}

impl Snackbar<NoIcon> {
    /// Creates a new snackbar with the specified message.
    ///
    /// Accepts any [`IntoText`] value so string literals resolve through the
    /// active translation catalog in the surrounding [`Environment`].
    #[must_use]
    pub fn new(message: impl IntoText) -> Self {
        Self {
            message: message.into_text(),
            icon: None,
            action: None,
            duration: Duration::from_secs(3),
            position: SnackbarPosition::default(),
            closeable: false,
            dismissal: None,
            captured_env: waterui_core::Environment::new(),
        }
    }
}

impl<I> Snackbar<I> {
    /// Adds a leading icon builder to the snackbar.
    ///
    /// Any [`ViewBuilder`] works, including closures returning non-`Clone`
    /// modified views (for example `|| mdi::check_circle().padding()`). For
    /// portable apps prefer a packaged icon crate such as
    /// `waterui-icons-material-icon` (`waterui_apple::SfSymbol` is only
    /// available on Apple backends). Pair with
    /// [`ViewExt::snackbar_host_with_icons`] or [`SnackbarManager::host`] when
    /// showing snackbars that carry an icon of type `B::Output`.
    #[must_use]
    pub fn icon<B: ViewBuilder>(self, icon: B) -> Snackbar<B::Output> {
        Snackbar {
            message: self.message,
            icon: Some(IconFactory(Rc::new(move || icon.build()))),
            action: self.action,
            duration: self.duration,
            position: self.position,
            closeable: self.closeable,
            dismissal: self.dismissal,
            captured_env: self.captured_env,
        }
    }

    /// Adds an action button to the snackbar.
    ///
    /// # Examples
    ///
    /// Simple action:
    /// ```rust
    /// use waterui::snackbar::Snackbar;
    ///
    /// let snackbar = Snackbar::new("Message sent").action("Undo", || {});
    /// ```
    ///
    /// With injected state:
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::snackbar::Snackbar;
    ///
    /// let items = binding::<Vec<String>>(Vec::new());
    /// let snackbar = Snackbar::new("Item deleted")
    ///     .state(&items)
    ///     .action("Undo", |State(items): State<Binding<Vec<String>>>| {
    ///         items.get_mut().clear();
    ///     });
    /// ```
    #[must_use]
    pub fn action<Args>(
        mut self,
        label: impl IntoLabel,
        handler: impl Handler<Args, ()> + 'static,
    ) -> Self {
        self.action = Some(SnackbarAction {
            label: label.into_label().into_semantic(),
            handler: shared_action(handler),
        });
        self
    }

    /// Registers a handler called when a presentation of this snackbar is
    /// dismissed, with the [`SnackbarDismissReason`] it went away.
    ///
    /// The handler runs exactly once per presentation, after the backend's exit
    /// presentation completes and the snackbar has been removed from the stack.
    /// Every dismissal reports through it: `Action` when the action button is
    /// pressed, `Timeout` when the duration elapses, `Closed` from the close
    /// button, `Replaced` when a newer snackbar at the same placement evicts
    /// it, and `Programmatic` for [`SnackbarManager::dismiss`]. Each `show` of
    /// a cloned snackbar is an independent presentation and reports its own
    /// dismissal.
    ///
    /// The reason arrives as the handler's first parameter; values injected
    /// with `.state()` extract as further parameters, exactly like `action`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::snackbar::{Snackbar, SnackbarDismissReason};
    ///
    /// let items = binding::<Vec<String>>(Vec::new());
    /// let snackbar = Snackbar::new("Item deleted")
    ///     .state(&items)
    ///     .on_dismiss(
    ///         |reason: SnackbarDismissReason, State(items): State<Binding<Vec<String>>>| {
    ///             if reason != SnackbarDismissReason::Action {
    ///                 items.get_mut().clear();
    ///             }
    ///         },
    ///     );
    /// ```
    #[must_use]
    pub fn on_dismiss<Args>(
        mut self,
        handler: impl EventHandler<SnackbarDismissReason, Args> + 'static,
    ) -> Self {
        self.dismissal = Some(DismissalCallback(Rc::new(RefCell::new(
            boxed_event_handler(handler),
        ))));
        self
    }

    /// Sets the auto-dismiss duration.
    ///
    /// Default is 3 seconds.
    #[must_use]
    pub const fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Sets the position on screen.
    ///
    /// Default is `SnackbarPosition::BottomCenter`.
    #[must_use]
    pub const fn position(mut self, position: SnackbarPosition) -> Self {
        self.position = position;
        self
    }

    /// Shows a trailing close button that dismisses this snackbar. Pair it with
    /// `.duration(Duration::ZERO)` for a snackbar that stays until the user
    /// closes it.
    #[must_use]
    pub const fn closeable(mut self) -> Self {
        self.closeable = true;
        self
    }

    /// Injects cloneable state into this snackbar's handler environment,
    /// where both `action` and `on_dismiss` handlers can extract it.
    #[must_use]
    pub fn state<T: Clone + 'static>(mut self, state: &T) -> Self {
        self.captured_env = self.captured_env.extending(State(state.clone()));
        self
    }
}

/// One active snackbar presentation handed to a native backend.
///
/// The backend owns all visual presentation (layout, surface styling, close
/// affordance geometry, stacking offsets, and entrance/exit animations) and
/// calls [`Self::trigger_action`], [`Self::close`], and
/// [`Self::complete_dismissal`] to drive the semantic lifecycle.
#[doc(hidden)]
pub struct SnackbarPresentation<S> {
    id: u64,
    message: Text,
    icon: Option<SlotBuilder<S>>,
    action: Option<SnackbarAction>,
    position: SnackbarPosition,
    closeable: bool,
    captured_env: Environment,
    dismissing: Computed<bool>,
    on_dismiss_request: Rc<dyn Fn(SnackbarDismissReason)>,
    on_dismiss_complete: Rc<dyn Fn()>,
}

impl<S> Clone for SnackbarPresentation<S> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            message: self.message.clone(),
            icon: self.icon.clone(),
            action: self.action.clone(),
            position: self.position,
            closeable: self.closeable,
            captured_env: self.captured_env.clone(),
            dismissing: self.dismissing.clone(),
            on_dismiss_request: Rc::clone(&self.on_dismiss_request),
            on_dismiss_complete: Rc::clone(&self.on_dismiss_complete),
        }
    }
}

impl<S> Debug for SnackbarPresentation<S> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnackbarPresentation")
            .field("id", &self.id)
            .field("message", &self.message)
            .field("has_icon", &self.icon.is_some())
            .field("action", &self.action)
            .field("position", &self.position)
            .field("closeable", &self.closeable)
            .finish_non_exhaustive()
    }
}

impl<S> SnackbarPresentation<S> {
    /// Stable monotonic presentation identifier.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// The snackbar's localized reactive message.
    #[must_use]
    pub const fn message(&self) -> &Text {
        &self.message
    }

    /// Optional leading icon builder in the backend's child slot `S`.
    #[must_use]
    pub const fn icon(&self) -> Option<&SlotBuilder<S>> {
        self.icon.as_ref()
    }

    /// Optional trailing action button.
    #[must_use]
    pub const fn action(&self) -> Option<&SnackbarAction> {
        self.action.as_ref()
    }

    /// Screen placement anchor for this snackbar.
    #[must_use]
    pub const fn position(&self) -> SnackbarPosition {
        self.position
    }

    /// Whether the backend should show a close button.
    #[must_use]
    pub const fn closeable(&self) -> bool {
        self.closeable
    }

    /// Reactive flag that flips from `false` to `true` once this presentation
    /// begins dismissing (via timeout, action, close, eviction, or programmatic
    /// dismissal).
    #[must_use]
    pub const fn dismissing(&self) -> &Computed<bool> {
        &self.dismissing
    }

    /// Runs the snackbar's action handler against the live `env` layered under
    /// any state captured via [`Snackbar::state`], then requests dismissal with
    /// [`SnackbarDismissReason::Action`].
    pub fn trigger_action(&self, env: &Environment) {
        if let Some(action) = &self.action {
            action.handler.run(&self.captured_env.layered_on(env));
        }
        (self.on_dismiss_request)(SnackbarDismissReason::Action);
    }

    /// Requests dismissal with [`SnackbarDismissReason::Closed`].
    pub fn close(&self) {
        (self.on_dismiss_request)(SnackbarDismissReason::Closed);
    }

    /// Notifies the manager that the backend's exit presentation has finished,
    /// removing the presentation from the active stack and firing `on_dismiss`.
    pub fn complete_dismissal(&self) {
        (self.on_dismiss_complete)();
    }
}

impl<S: 'static> SnackbarPresentation<S> {
    /// Maps this presentation's icon slot into another slot type `T`.
    #[must_use]
    pub fn map_slots<T: 'static>(
        self,
        f: impl Fn(S) -> T + Clone + 'static,
    ) -> SnackbarPresentation<T> {
        SnackbarPresentation {
            id: self.id,
            message: self.message,
            icon: self.icon.map(|builder| builder.map_slot(f)),
            action: self.action,
            position: self.position,
            closeable: self.closeable,
            captured_env: self.captured_env,
            dismissing: self.dismissing,
            on_dismiss_request: self.on_dismiss_request,
            on_dismiss_complete: self.on_dismiss_complete,
        }
    }
}

/// Resolved snackbar host metadata consumed by native backends, carrying the
/// reactive stack of active [`SnackbarPresentation`]s whose icons live in `S`.
#[doc(hidden)]
pub struct SnackbarHostConfig<S> {
    presentations: Computed<Vec<SnackbarPresentation<S>>>,
}

impl<S> Debug for SnackbarHostConfig<S> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnackbarHostConfig").finish_non_exhaustive()
    }
}

impl<S: 'static> MetadataKey for SnackbarHostConfig<S> {}

impl<S> SnackbarHostConfig<S> {
    /// Reactive list of currently active snackbar presentations, oldest first.
    #[must_use]
    pub const fn presentations(&self) -> &Computed<Vec<SnackbarPresentation<S>>> {
        &self.presentations
    }
}

impl<S: 'static> SnackbarHostConfig<S> {
    fn from_active<I: SnackbarIconSlot>(
        active: &Binding<Vec<ActiveSnackbar<I>>>,
        manager: SnackbarManager<I>,
    ) -> Self
    where
        S: ChildSlot,
    {
        let presentations = active
            .map(move |items| {
                items
                    .into_iter()
                    .map(|item| item.into_presentation::<S>(&manager))
                    .collect()
            })
            .computed();
        Self { presentations }
    }

    /// Maps every active presentation's icon slot through `f`.
    #[must_use]
    pub fn map_slots<T: 'static>(
        self,
        f: impl Fn(S) -> T + Clone + 'static,
    ) -> SnackbarHostConfig<T> {
        SnackbarHostConfig {
            presentations: self
                .presentations
                .map(move |items| {
                    items
                        .into_iter()
                        .map(|item| item.map_slots(f.clone()))
                        .collect()
                })
                .computed(),
        }
    }
}

impl<S: 'static> MapSlots<S> for SnackbarHostConfig<S> {
    type With<T: 'static> = SnackbarHostConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> SnackbarHostConfig<T> {
        Self::map_slots(self, f)
    }
}

/// One active snackbar entry owned by [`SnackbarManager`].
struct ActiveSnackbar<I: 'static = NoIcon> {
    id: u64,
    snackbar: Snackbar<I>,
    dismissing: Binding<bool>,
    dismiss_reason: Rc<Cell<Option<SnackbarDismissReason>>>,
}

impl<I: 'static> Clone for ActiveSnackbar<I> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            snackbar: self.snackbar.clone(),
            dismissing: self.dismissing.clone(),
            dismiss_reason: Rc::clone(&self.dismiss_reason),
        }
    }
}

impl<I: SnackbarIconSlot> ActiveSnackbar<I> {
    fn into_presentation<S: ChildSlot>(
        self,
        manager: &SnackbarManager<I>,
    ) -> SnackbarPresentation<S> {
        let id = self.id;
        let icon = self.snackbar.icon.and_then(|factory| {
            let icon_view = I::into_icon_view(factory)?;
            Some(SlotBuilder::new(move || S::from_view(icon_view.clone())))
        });
        let manager_for_dismiss = manager.clone();
        let manager_for_complete = manager.clone();
        SnackbarPresentation {
            id,
            message: self.snackbar.message,
            icon,
            action: self.snackbar.action,
            position: self.snackbar.position,
            closeable: self.snackbar.closeable,
            captured_env: self.snackbar.captured_env,
            dismissing: self.dismissing.computed(),
            on_dismiss_request: Rc::new(move |reason| {
                manager_for_dismiss.dismiss_presentation(id, reason);
            }),
            on_dismiss_complete: Rc::new(move || {
                manager_for_complete.complete_dismissal(id);
            }),
        }
    }
}

/// Internal state for the snackbar manager.
struct SnackbarManagerState<I: 'static = NoIcon> {
    /// Active snackbars, oldest first.
    active: Binding<Vec<ActiveSnackbar<I>>>,
    /// Monotonic presentation identifier used to ignore stale timers.
    next_id: u64,
}

/// Manages snackbar notifications in a FIFO queue.
///
/// Attach [`ViewExt::snackbar_host`] (or [`ViewExt::snackbar_host_with_icons`]
/// / [`SnackbarManager::host`]) to an ancestor view to install the manager and
/// its overlay, then extract `SnackbarManager` in descendant handlers.
///
/// # Examples
///
/// ```rust
/// use waterui::prelude::*;
/// use waterui::snackbar::{Snackbar, SnackbarManager};
///
/// fn my_view() -> impl View {
///     button("Save")
///         .action(|manager: SnackbarManager| {
///             manager.show(Snackbar::new("File saved successfully"));
///         })
///         .snackbar_host()
/// }
/// ```
#[state]
pub struct SnackbarManager<I: 'static = NoIcon> {
    state: Rc<RefCell<SnackbarManagerState<I>>>,
}

impl<I: 'static> Clone for SnackbarManager<I> {
    fn clone(&self) -> Self {
        Self {
            state: Rc::clone(&self.state),
        }
    }
}

impl<I: 'static> Plugin for SnackbarManager<I> {}

impl<I: 'static> core::fmt::Debug for SnackbarManager<I> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SnackbarManager").finish()
    }
}

impl<I: SnackbarIconSlot> SnackbarManager<I> {
    fn create() -> (Self, Binding<Vec<ActiveSnackbar<I>>>) {
        let active = Binding::new(Vec::new());
        let state = Rc::new(RefCell::new(SnackbarManagerState {
            active: active.clone(),
            next_id: 0,
        }));
        (Self { state }, active)
    }

    /// Builds a view subtree paired with a snackbar host, inferring `I` from
    /// how `build` uses the handed manager and installing it for descendants.
    pub fn host<R: View>(build: impl FnOnce(Self) -> R) -> impl View {
        let (manager, active) = Self::create();
        let content = attach_manager(build(manager.clone()), &manager);
        SnackbarHostView {
            content,
            manager,
            active,
        }
    }

    /// Shows a snackbar. Snackbars stack simultaneously, oldest nearest the
    /// anchor edge. Each placement is an independent stack: a snackbar only
    /// interacts with others sharing its [`SnackbarPosition`], and when a
    /// placement already holds `MAX_VISIBLE_SNACKBARS` the oldest *at that
    /// placement* is dismissed to make room — different placements never evict
    /// each other.
    ///
    /// # Panics
    ///
    /// Panics if the snackbar presentation id counter overflows, which takes
    /// more than `u64::MAX` snackbars.
    pub fn show(&self, snackbar: impl Into<Snackbar<I>>) {
        let snackbar = snackbar.into();
        let duration = snackbar.duration;
        let position = snackbar.position;
        let id = {
            let mut state = self.state.borrow_mut();
            state.next_id = state
                .next_id
                .checked_add(1)
                .expect("a u64 id advancing once per shown snackbar does not overflow");
            state.next_id
        };
        let item = ActiveSnackbar {
            id,
            dismissing: Binding::new(false),
            dismiss_reason: Rc::new(Cell::new(None)),
            snackbar,
        };

        let active = self.state.borrow().active.clone();

        // Evict the oldest live snackbar AT THIS PLACEMENT if its stack is
        // already full, so the new one enters as the oldest exits.
        // Snackbars at other placements are untouched.
        let snapshot = active.snapshot();
        let live_here = snapshot
            .iter()
            .filter(|entry| {
                entry.dismiss_reason.get().is_none() && entry.snackbar.position == position
            })
            .count();
        if live_here >= MAX_VISIBLE_SNACKBARS {
            let oldest = snapshot
                .iter()
                .find(|entry| {
                    entry.dismiss_reason.get().is_none() && entry.snackbar.position == position
                })
                .map(|entry| entry.id);
            if let Some(oldest) = oldest {
                self.dismiss_presentation(oldest, SnackbarDismissReason::Replaced);
            }
        }
        drop(snapshot);

        active.get_mut().push(item);

        // A zero duration disables auto-dismiss: the snackbar stays until
        // dismissed by its action, close button, replacement, or `dismiss()`.
        if duration.is_zero() {
            return;
        }
        let manager = self.clone();
        spawn_local(async move {
            native_executor::sleep(duration).await;
            tracing::debug!("snackbar auto-dismiss timer fired for {id}");
            manager.dismiss_presentation(id, SnackbarDismissReason::Timeout);
        })
        .detach();
    }

    /// Dismisses the most recently shown snackbar.
    pub fn dismiss(&self) {
        let newest = self
            .state
            .borrow()
            .active
            .snapshot()
            .into_iter()
            .rev()
            .find(|item| item.dismiss_reason.get().is_none())
            .map(|item| item.id);
        if let Some(id) = newest {
            self.dismiss_presentation(id, SnackbarDismissReason::Programmatic);
        }
    }

    /// Marks one snackbar as dismissing so the backend runs its exit
    /// presentation and calls [`SnackbarPresentation::complete_dismissal`] when
    /// done. The `reason` of the first dismissal wins: later requests for a
    /// presentation already dismissing return without changing anything.
    fn dismiss_presentation(&self, id: u64, reason: SnackbarDismissReason) {
        let item = self
            .state
            .borrow()
            .active
            .snapshot()
            .into_iter()
            .find(|item| item.id == id);
        let Some(item) = item else {
            return;
        };
        if item.dismiss_reason.get().is_some() {
            return;
        }
        item.dismiss_reason.set(Some(reason));
        item.dismissing.set(true);
    }

    /// Removes a dismissed presentation from the active stack and fires its
    /// `on_dismiss` callback with the recorded dismissal reason.
    fn complete_dismissal(&self, id: u64) {
        let active = self.state.borrow().active.clone();
        let removed = {
            let mut list = active.get_mut();
            let Some(index) = list.iter().position(|item| item.id == id) else {
                return;
            };
            list.remove(index)
        };
        let reason = removed
            .dismiss_reason
            .get()
            .unwrap_or(SnackbarDismissReason::Programmatic);

        // The presentation is off the stack and no manager state is borrowed:
        // the callback may itself show or dismiss snackbars.
        if let Some(dismissal) = &removed.snackbar.dismissal {
            dismissal.call(reason, &removed.snackbar.captured_env);
        }
    }
}

fn attach_manager<V: View, I: SnackbarIconSlot>(
    content: V,
    manager: &SnackbarManager<I>,
) -> impl View + use<V, I> {
    let content = content.state(manager);
    #[cfg(feature = "flow-markdown")]
    let content = content.with(CodeCopyFeedback(OnCopied::new({
        let snackbar = manager.clone();
        move |_env| {
            snackbar.show(I::from_plain(Snackbar::new("Copied to clipboard")));
        }
    })));
    content
}

struct SnackbarHostView<V, I: 'static> {
    content: V,
    manager: SnackbarManager<I>,
    active: Binding<Vec<ActiveSnackbar<I>>>,
}

impl<V: View, I: SnackbarIconSlot> View for SnackbarHostView<V, I> {
    fn body(self, _env: &Environment) -> impl View {
        let overlay = SnackbarHostConfig::<AnyView>::from_active(&self.active, self.manager);
        Metadata::new(self.content, overlay)
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        let overlay = SnackbarHostConfig::<B::Slot>::from_active(&self.active, self.manager);
        backend.metadata(
            Metadata::new(B::Slot::from_view(self.content), overlay),
            env,
            cx,
        )
    }
}

/// Hosts a [`SnackbarManager`] and its overlay for a view subtree.
///
/// Attach with [`ViewExt::snackbar_host`] (plain snackbars) or
/// [`ViewExt::snackbar_host_with_icons`] (snackbars carrying a typed icon view).
/// Each built instance creates its own manager and overlay stack, so multiple
/// windows or scenes stay isolated. When `flow-markdown` is enabled, this host
/// also installs `CodeCopyFeedback` so copying a markdown code block presents
/// `"Copied to clipboard"` through the hosted manager.
#[derive(Debug)]
pub struct SnackbarHost<V, I = NoIcon> {
    content: V,
    _icon: PhantomData<fn() -> I>,
}

impl<V, I> SnackbarHost<V, I> {
    /// Wraps `content` in a snackbar host.
    #[must_use]
    pub const fn new(content: V) -> Self {
        Self {
            content,
            _icon: PhantomData,
        }
    }
}

impl<V: View, I: SnackbarIconSlot> View for SnackbarHost<V, I> {
    fn body(self, _env: &Environment) -> impl View {
        let (manager, active) = SnackbarManager::<I>::create();
        let content = attach_manager(self.content, &manager);
        let overlay = SnackbarHostConfig::<AnyView>::from_active(&active, manager);
        Metadata::new(content, overlay)
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        let (manager, active) = SnackbarManager::<I>::create();
        let content = attach_manager(self.content, &manager);
        let overlay = SnackbarHostConfig::<B::Slot>::from_active(&active, manager);
        backend.metadata(Metadata::new(B::Slot::from_view(content), overlay), env, cx)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use waterui_core::binding;

    use super::super::parked_executor::install as install_executor;
    use super::*;

    fn new_manager() -> SnackbarManager {
        SnackbarManager::<NoIcon>::create().0
    }

    /// A presentation row built directly, so tests drive dismissal without a
    /// show timer or a running executor.
    fn presentation(id: u64, snackbar: Snackbar) -> ActiveSnackbar {
        ActiveSnackbar {
            id,
            dismissing: Binding::new(false),
            dismiss_reason: Rc::new(Cell::new(None)),
            snackbar,
        }
    }

    fn present(manager: &SnackbarManager, item: ActiveSnackbar) {
        manager.state.borrow().active.get_mut().push(item);
    }

    fn live_ids(manager: &SnackbarManager) -> Vec<u64> {
        manager
            .state
            .borrow()
            .active
            .snapshot()
            .iter()
            .map(|item| item.id)
            .collect()
    }

    /// A snackbar recording every reported reason.
    fn recording(snackbar: Snackbar) -> (Snackbar, Rc<RefCell<Vec<SnackbarDismissReason>>>) {
        let reasons = Rc::new(RefCell::new(Vec::new()));
        let log = Rc::clone(&reasons);
        (
            snackbar.on_dismiss(move |reason: SnackbarDismissReason| {
                log.borrow_mut().push(reason);
            }),
            reasons,
        )
    }

    #[test]
    fn building_or_showing_a_snackbar_reports_nothing() {
        let manager = new_manager();
        let (snackbar, reasons) = recording(Snackbar::new("hello"));
        present(&manager, presentation(1, snackbar.clone()));
        present(&manager, presentation(2, snackbar));
        assert!(reasons.borrow().is_empty());

        // A completion for an id that was never presented reports nothing either.
        manager.complete_dismissal(42);
        assert!(reasons.borrow().is_empty());
    }

    #[test]
    fn removal_reports_the_reason_once() {
        let manager = new_manager();
        let (snackbar, reasons) = recording(Snackbar::new("bye"));
        present(&manager, presentation(1, snackbar));

        manager.dismiss_presentation(1, SnackbarDismissReason::Timeout);
        manager.complete_dismissal(1);
        assert_eq!(live_ids(&manager), Vec::<u64>::new());
        assert_eq!(*reasons.borrow(), [SnackbarDismissReason::Timeout]);

        // A stale completion for the same presentation reports nothing more.
        manager.complete_dismissal(1);
        assert_eq!(*reasons.borrow(), [SnackbarDismissReason::Timeout]);
    }

    #[test]
    fn removal_without_a_callback_is_harmless() {
        let manager = new_manager();
        present(&manager, presentation(1, Snackbar::new("plain")));
        manager.dismiss_presentation(1, SnackbarDismissReason::Timeout);
        manager.complete_dismissal(1);
        assert_eq!(live_ids(&manager), Vec::<u64>::new());
    }

    #[test]
    fn the_callback_observes_removal_and_may_reenter_the_manager() {
        let manager = new_manager();
        let seen = Rc::new(Cell::new(false));
        let snackbar = Snackbar::new("going").on_dismiss({
            let manager = manager.clone();
            let seen = Rc::clone(&seen);
            move |_reason: SnackbarDismissReason| {
                // The presentation is already off the stack, and reentering
                // the manager does not panic on a held borrow.
                assert!(manager.state.borrow().active.snapshot().is_empty());
                manager.dismiss();
                seen.set(true);
            }
        });
        present(&manager, presentation(1, snackbar));

        manager.dismiss_presentation(1, SnackbarDismissReason::Programmatic);
        manager.complete_dismissal(1);
        assert!(seen.get());
    }

    #[test]
    fn each_presentation_of_a_cloned_snackbar_reports_its_own_reason() {
        let manager = new_manager();
        let (snackbar, reasons) = recording(Snackbar::new("twice"));
        present(&manager, presentation(1, snackbar.clone()));
        present(&manager, presentation(2, snackbar));

        manager.dismiss_presentation(1, SnackbarDismissReason::Replaced);
        manager.complete_dismissal(1);
        manager.dismiss_presentation(2, SnackbarDismissReason::Closed);
        manager.complete_dismissal(2);
        assert_eq!(
            *reasons.borrow(),
            [
                SnackbarDismissReason::Replaced,
                SnackbarDismissReason::Closed
            ]
        );
    }

    #[test]
    fn the_first_dismissal_reason_wins() {
        let manager = new_manager();
        let (snackbar, reasons) = recording(Snackbar::new("raced"));
        present(&manager, presentation(1, snackbar));

        // A timeout already started dismissal; the later programmatic dismiss
        // must not overwrite the recorded reason.
        manager.dismiss_presentation(1, SnackbarDismissReason::Timeout);
        manager.dismiss_presentation(1, SnackbarDismissReason::Programmatic);
        manager.complete_dismissal(1);
        assert_eq!(*reasons.borrow(), [SnackbarDismissReason::Timeout]);
    }

    #[test]
    fn the_handler_extracts_injected_state() {
        let manager = new_manager();
        let count = binding::<i32>(0);
        let snackbar = Snackbar::new("stateful").state(&count).on_dismiss(
            |reason: SnackbarDismissReason, State(count): State<Binding<i32>>| {
                assert_eq!(reason, SnackbarDismissReason::Action);
                count.set(count.snapshot() + 1);
            },
        );
        present(&manager, presentation(1, snackbar));

        manager.dismiss_presentation(1, SnackbarDismissReason::Action);
        manager.complete_dismissal(1);
        assert_eq!(count.snapshot(), 1);
    }

    #[test]
    fn fourth_snackbar_at_same_placement_evicts_oldest_only() {
        install_executor();
        let manager = new_manager();
        let (first, first_reasons) = recording(Snackbar::new("1").duration(Duration::ZERO));
        let (top, top_reasons) = recording(
            Snackbar::new("top")
                .position(SnackbarPosition::TopCenter)
                .duration(Duration::ZERO),
        );
        manager.show(first);
        manager.show(top);
        manager.show(Snackbar::new("2").duration(Duration::ZERO));
        manager.show(Snackbar::new("3").duration(Duration::ZERO));
        manager.show(Snackbar::new("4").duration(Duration::ZERO));

        let items = manager.state.borrow().active.snapshot();
        assert!(items[0].dismissing.snapshot());
        assert!(!items[1].dismissing.snapshot());
        assert!(!items[2].dismissing.snapshot());

        manager.complete_dismissal(items[0].id);
        assert_eq!(*first_reasons.borrow(), [SnackbarDismissReason::Replaced]);
        assert!(top_reasons.borrow().is_empty());
    }

    #[test]
    fn typed_icon_host_accepts_plain_and_icon_snackbars() {
        install_executor();
        let view = SnackbarManager::host(|manager| {
            manager.show(Snackbar::new("Plain"));
            manager.show(Snackbar::new("Icon").icon(|| "★"));
            "Content"
        });
        let _ = view.body(&Environment::new());
    }

    #[test]
    fn snackbar_icon_accepts_non_clone_modified_view_builder() {
        use waterui_layout::padding::Padding;
        use waterui_text::{Text, text};

        install_executor();
        let snackbar = Snackbar::new("Padded").icon(|| text("★").padding());
        let cloned = snackbar.clone();
        let host = text("Content").snackbar_host_with_icons::<Padding<Text>>();
        let _ = host.body(&Environment::new());

        let view = SnackbarManager::host(move |manager| {
            manager.show(snackbar);
            manager.show(cloned);
            "Content"
        });
        let _ = view.body(&Environment::new());
    }

    #[test]
    fn snackbar_message_and_action_resolve_through_translation_catalog() {
        use waterui_locale::{TranslationCatalog, locales};

        let mut env = Environment::new();
        env.insert(locales::FR);
        env.insert(
            TranslationCatalog::new()
                .add_toml(
                    locales::FR,
                    "\"File saved\" = \"Fichier enregistré\"\n\"Undo\" = \"Annuler\"",
                )
                .expect("test catalog must parse"),
        );

        let snackbar = Snackbar::new("File saved").action("Undo", || {});
        assert_eq!(
            snackbar
                .message
                .resolve(&env)
                .content()
                .snapshot()
                .to_plain()
                .as_str(),
            "Fichier enregistré"
        );
        let action = snackbar.action.as_ref().expect("action must be present");
        assert_eq!(
            action
                .label()
                .semantic_text()
                .resolve(&env)
                .content()
                .snapshot()
                .to_plain()
                .as_str(),
            "Annuler"
        );
    }
}
