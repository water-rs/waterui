//! Button component for `WaterUI`
//!
//! This module provides a Button component that allows users to trigger actions
//! when clicked.
//!
//! ![Button](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/button.svg)
//!
//! # Overview
//!
//! Buttons support two patterns for handling click actions:
//!
//! 1. **Simple actions** - No parameters, just execute code
//! 2. **Extractor-based actions** - Extract values from the environment at click time
//!
//! # Examples
//!
//! ## Simple Action
//!
//! The simplest form takes a closure with no parameters:
//!
//! ```rust
//! use waterui::prelude::*;
//!
//! # fn simple() {
//! button("Click me").action(|| {});
//! # }
//! ```
//!
//! ## State Injection
//!
//! Use `.state()` on the resulting view to inject cloneable state into the
//! environment, then extract it inside `action(...)` with `State<T>`:
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::reactive::binding;
//!
//! # fn injected() {
//! let counter: Binding<i32> = binding(0);
//!
//! button("Increment")
//!     .action(|State(count): State<Binding<i32>>| {
//!         *count.get_mut() += 1;
//!     })
//!     .state(&counter);
//!
//! let query: Binding<Str> = binding(Str::from(""));
//! let history: Binding<Vec<Str>> = binding(Vec::new());
//!
//! button("Search")
//!     .action(
//!         |State(query): State<Binding<Str>>, State(history): State<Binding<Vec<Str>>>| {
//!             history.with_mut(|history| history.push(query.snapshot()));
//!         },
//!     )
//!     .state(&query)
//!     .state(&history);
//! # }
//! ```
//!
//! ## Environment Extraction
//!
//! Any extractor type can be used directly as an `action(...)` parameter, and
//! an owned `Clone` type becomes one with `#[state]` — injected the same way
//! as `State<T>` values, with `.state(&value)`:
//!
//! ```rust
//! use waterui::prelude::*;
//!
//! #[state]
//! #[derive(Clone)]
//! struct CurrentUser(Str);
//! # impl CurrentUser {
//! #     fn rename(&self, _name: Str) {}
//! # }
//!
//! # fn extracted(user: &CurrentUser) {
//! button("Rename")
//!     .action(|user: CurrentUser| {
//!         user.rename(Str::from("Ada"));
//!     })
//!     .state(user);
//! # }
//! ```
//!
//! ## Combined State and Extraction
//!
//! `State<T>` and `#[state]` extractors compose in a single handler over the
//! same `.state(&value)` channel:
//!
//! ```rust
//! use waterui::prelude::*;
//!
//! # use waterui::reactive::binding;
//! # #[derive(Clone, Default)]
//! # struct FormData {
//! #     email: Str,
//! # }
//! # #[state]
//! # #[derive(Clone)]
//! # struct ApiClient;
//! # impl ApiClient {
//! #     fn submit(&self, _form: FormData) {}
//! # }
//! # fn combined() {
//! # let form_data: Binding<FormData> = binding(FormData::default());
//! # let client = ApiClient;
//! button("Submit")
//!     .action(|State(data): State<Binding<FormData>>, client: ApiClient| {
//!         client.submit(data.snapshot());
//!     })
//!     .state(&form_data)
//!     .state(&client);
//! # }
//! ```
//!
//! ## Async Actions
//!
//! All patterns support async variants with `action_async`. An async button
//! action is single-flight: while its spawned future runs, further presses are
//! ignored, the button's interaction state reports disabled, and accessibility
//! reports busy until the future completes or is dropped. Chain
//! [`.pending(&binding)`](Button::pending) in either order with
//! [`.action_async(...)`](Button::action_async) to mirror the in-flight state
//! into your own `Binding<bool>`:
//!
//! ```rust
//! use waterui::prelude::*;
//!
//! # use waterui::reactive::binding;
//! # async fn fetch_from_server() -> Str {
//! #     Str::from("payload")
//! # }
//! # fn asynchronous() {
//! # let result_binding: Binding<Str> = binding(Str::from(""));
//! # let fetching: Binding<bool> = binding(false);
//! button("Fetch Data")
//!     .action_async(|State(result): State<Binding<Str>>| async move {
//!         let data = fetch_from_server().await;
//!         result.set(data);
//!     })
//!     .pending(&fetching)
//!     .state(&result_binding);
//! # }
//! ```
//!
//! ## Button Styles
//!
//! Use `.style()` or convenience methods to change the button's visual appearance:
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui_controls::ButtonStyle;
//!
//! # fn styled() {
//! // Using style method
//! button("Primary Action")
//!     .style(ButtonStyle::BorderedProminent)
//!     .action(|| { /* ... */ });
//!
//! // Using convenience methods
//! button("Learn More")
//!     .link()
//!     .action(|| { /* ... */ });
//!
//! button("Secondary")
//!     .bordered()
//!     .action(|| { /* ... */ });
//!
//! button("Subtle")
//!     .plain()
//!     .action(|| { /* ... */ });
//! # }
//! ```

use crate::size::ControlSize;
use core::cell::RefCell;
use core::fmt;
use core::future::Future;
use fmt::Debug;

use alloc::boxed::Box;
use alloc::rc::Rc;
use executor_core::spawn_local;
use nami::{Binding, Computed, Signal as _, SignalExt as _, binding, zip::zip};
use waterui_accessibility::{
    AccessibilityLabel, AccessibilityRole, AccessibilityState, AccessibilityStateSignal,
    default_role,
};
use waterui_core::backend::{
    Backend, ChildSlot, HookSlot, HookableParent, MapSlots, ParentHookScope, offer_hookable_parent,
};
use waterui_core::env::Installed;
use waterui_core::handler::ReportExtraction;
use waterui_core::handler::{BoxedAction, ExtractionError, Handler};
use waterui_core::view::hook::Hook;
use waterui_core::{AnyView, Environment, Metadata, Native, NativeView, OneOf2, View};
use waterui_interaction::gesture::{GestureObserver, TapGesture};
use waterui_interaction::interaction::{Disabled, InteractionReport, InteractionState};
use waterui_text::styled::StyledStr;

use crate::label::{
    ControlLabel, IntoAccessibilityLabel, IntoLabel, Label, LabelDisplayMode, LabelKind,
};
use crate::shortcut::Shortcut;

// ============================================================================
// Macros for reducing boilerplate
// ============================================================================

/// Implements style convenience methods for a button builder type.
macro_rules! impl_style_methods {
    () => {
        /// Sets the button style to plain (no background or border).
        /// Suitable for low-emphasis actions or toolbar buttons.
        #[must_use]
        pub const fn plain(self) -> Self {
            self.with_builtin_style(ButtonStyle::Plain)
        }

        /// Sets the button style to link (hyperlink appearance).
        /// Used for URL navigation and text-based links.
        #[must_use]
        pub const fn link(self) -> Self {
            self.with_builtin_style(ButtonStyle::Link)
        }

        /// Sets the button style to borderless.
        /// Similar to plain but with more interactive feedback.
        #[must_use]
        pub const fn borderless(self) -> Self {
            self.with_builtin_style(ButtonStyle::Borderless)
        }

        /// Sets the button style to bordered.
        /// Suitable for secondary actions.
        #[must_use]
        pub const fn bordered(self) -> Self {
            self.with_builtin_style(ButtonStyle::Bordered)
        }

        /// Sets the button style to bordered prominent.
        /// Use for primary actions that should stand out.
        #[must_use]
        pub const fn bordered_prominent(self) -> Self {
            self.with_builtin_style(ButtonStyle::BorderedProminent)
        }
    };
}

// ============================================================================
// ButtonStyle
// ============================================================================

/// Visual style options for buttons.
///
/// Different button styles provide different visual emphasis and behavior.
/// The actual rendering depends on the platform's native UI toolkit.
///
/// # Examples
///
/// ```rust
/// use waterui::prelude::*;
/// use waterui_controls::ButtonStyle;
///
/// # fn styles() {
/// // Primary action with prominent styling
/// let _ = button("Submit")
///     .style(ButtonStyle::BorderedProminent)
///     .action(|| {});
///
/// // Secondary action with subtle styling
/// let _ = button("Cancel").style(ButtonStyle::Plain).action(|| {});
///
/// // Navigation link
/// let _ = button("Learn More").style(ButtonStyle::Link).action(|| {});
/// # }
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ButtonStyle {
    /// The default button style, determined by the platform and context.
    /// On macOS/iOS, this typically renders as a rounded rectangle with background.
    #[default]
    Automatic,
    /// A plain text button without any background or border.
    /// Suitable for low-emphasis actions or toolbar buttons.
    Plain,
    /// A button styled as a hyperlink, typically with underlined blue text.
    /// Used for URL navigation and text-based links.
    Link,
    /// A button without a visible border, but may show hover/press effects.
    /// Similar to Plain but with more interactive feedback.
    Borderless,
    /// A button with a subtle border or background.
    /// Suitable for secondary actions.
    Bordered,
    /// A prominent button with filled background color.
    /// Use for primary actions that should stand out.
    BorderedProminent,
    /// A Liquid Glass capsule: the button floats over its content in the
    /// platform's glass, taking its highlight from what is behind it.
    ///
    /// Glass is an Apple primitive. Backends without it render this as their
    /// bordered style, since the emphasis is the same.
    Glass,
    /// A Liquid Glass capsule filled with the accent color, for the primary
    /// action among glass controls.
    ///
    /// Backends without glass render this as their bordered-prominent style.
    GlassProminent,
}

/// A button style installed on a subtree becomes the default for buttons that
/// did not pick one, the same way [`LabelDisplayMode`] adapts labels to their
/// surroundings.
impl waterui_core::plugin::Plugin for ButtonStyle {}

// ============================================================================
// Semantic roles and custom button styles
// ============================================================================

/// Semantic role of a [`Button`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ButtonRole {
    /// Cancels the current flow or dismisses the current context.
    Cancel,
    /// Performs a destructive action such as deleting data.
    Destructive,
}

/// The label view passed to a custom [`Button`] style.
pub struct ButtonLabel<L = ControlLabel<HookSlot>>(L);

impl<L> fmt::Debug for ButtonLabel<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ButtonLabel")
    }
}

impl<L: View> View for ButtonLabel<L> {
    fn body(self, _env: &Environment) -> impl View {
        self.0
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        self.0.dispatch(backend, env, cx)
    }
}

/// Semantic configuration passed to [`CustomButtonStyle::make_body`].
pub struct ButtonStyleConfiguration<L> {
    label: L,
    role: Option<ButtonRole>,
    size: ControlSize,
    interaction: Computed<InteractionState>,
}

impl<L> fmt::Debug for ButtonStyleConfiguration<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ButtonStyleConfiguration")
            .field("role", &self.role)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl<L> ButtonStyleConfiguration<L> {
    /// Borrows the button's label view.
    #[must_use]
    pub const fn label(&self) -> &L {
        &self.label
    }

    /// Extracts the button's label view.
    #[must_use]
    pub fn into_label(self) -> L {
        self.label
    }

    /// Takes the configuration apart into `(label, role, size, interaction_state)`.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        L,
        Option<ButtonRole>,
        ControlSize,
        Computed<InteractionState>,
    ) {
        (self.label, self.role, self.size, self.interaction)
    }

    /// The semantic role assigned to this button, if any.
    #[must_use]
    pub const fn role(&self) -> Option<ButtonRole> {
        self.role
    }

    /// The control size requested for this button.
    #[must_use]
    pub const fn size(&self) -> ControlSize {
        self.size
    }

    /// Reactive interaction state flags for this button
    /// (hovered, focused, pressed, disabled, etc.).
    #[must_use]
    pub const fn interaction_state(&self) -> &Computed<InteractionState> {
        &self.interaction
    }

    /// Whether a pointer or key is currently pressing the button.
    #[must_use]
    pub fn is_pressed(&self) -> Computed<bool> {
        self.interaction
            .map(|state| state.contains(InteractionState::PRESSED))
            .computed()
    }

    /// Whether a pointer is hovering over the button.
    #[must_use]
    pub fn is_hovered(&self) -> Computed<bool> {
        self.interaction
            .map(|state| state.contains(InteractionState::HOVERED))
            .computed()
    }

    /// Whether the button currently shows keyboard focus.
    #[must_use]
    pub fn is_focused(&self) -> Computed<bool> {
        self.interaction
            .map(|state| state.contains(InteractionState::FOCUSED))
            .computed()
    }

    /// Whether the button is currently disabled.
    #[must_use]
    pub fn is_disabled(&self) -> Computed<bool> {
        self.interaction
            .map(|state| state.contains(InteractionState::DISABLED))
            .computed()
    }
}

/// Action trigger passed to a [`PrimitiveButtonStyle`] through
/// [`PrimitiveButtonStyleConfiguration`].
#[derive(Clone)]
pub struct ButtonTrigger {
    action: Rc<RefCell<BoxedAction<()>>>,
    env: Environment,
    pending: Option<Computed<bool>>,
}

impl fmt::Debug for ButtonTrigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ButtonTrigger")
    }
}

impl ButtonTrigger {
    fn is_pending(&self) -> bool {
        self.pending.as_ref().is_some_and(Computed::snapshot)
    }

    /// Triggers the button's action unless the button is currently disabled
    /// or its async action is in flight.
    pub fn trigger(&self) {
        if Disabled::resolve(&self.env, false).snapshot() || self.is_pending() {
            return;
        }
        (self.action.borrow_mut())(&self.env).report_extraction();
    }

    /// Triggers the button's action with `runtime_env` layered over the
    /// captured button environment, unless the button is currently disabled
    /// or its async action is in flight.
    pub fn trigger_with(&self, runtime_env: &Environment) {
        let action_env = self.env.layered_on(runtime_env);
        if Disabled::resolve(&action_env, false).snapshot() || self.is_pending() {
            return;
        }
        (self.action.borrow_mut())(&action_env).report_extraction();
    }
}

/// Semantic configuration passed to [`PrimitiveButtonStyle::make_body`].
pub struct PrimitiveButtonStyleConfiguration<L> {
    label: L,
    trigger: ButtonTrigger,
    role: Option<ButtonRole>,
    size: ControlSize,
    interaction: Computed<InteractionState>,
}

impl<L> fmt::Debug for PrimitiveButtonStyleConfiguration<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrimitiveButtonStyleConfiguration")
            .field("role", &self.role)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl<L> PrimitiveButtonStyleConfiguration<L> {
    /// Borrows the button's label view.
    #[must_use]
    pub const fn label(&self) -> &L {
        &self.label
    }

    /// Extracts the button's label view.
    #[must_use]
    pub fn into_label(self) -> L {
        self.label
    }

    /// Borrows the handle that triggers the button's action.
    #[must_use]
    pub const fn trigger(&self) -> &ButtonTrigger {
        &self.trigger
    }

    /// Takes the configuration apart into `(label, trigger, role, size, interaction_state)`.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        L,
        ButtonTrigger,
        Option<ButtonRole>,
        ControlSize,
        Computed<InteractionState>,
    ) {
        (
            self.label,
            self.trigger,
            self.role,
            self.size,
            self.interaction,
        )
    }

    /// The semantic role assigned to this button, if any.
    #[must_use]
    pub const fn role(&self) -> Option<ButtonRole> {
        self.role
    }

    /// The control size requested for this button.
    #[must_use]
    pub const fn size(&self) -> ControlSize {
        self.size
    }

    /// Reactive interaction state flags for this button
    /// (hovered, focused, pressed, disabled, etc.).
    #[must_use]
    pub const fn interaction_state(&self) -> &Computed<InteractionState> {
        &self.interaction
    }

    /// Whether a pointer or key is currently pressing the button.
    #[must_use]
    pub fn is_pressed(&self) -> Computed<bool> {
        self.interaction
            .map(|state| state.contains(InteractionState::PRESSED))
            .computed()
    }

    /// Whether a pointer is hovering over the button.
    #[must_use]
    pub fn is_hovered(&self) -> Computed<bool> {
        self.interaction
            .map(|state| state.contains(InteractionState::HOVERED))
            .computed()
    }

    /// Whether the button currently shows keyboard focus.
    #[must_use]
    pub fn is_focused(&self) -> Computed<bool> {
        self.interaction
            .map(|state| state.contains(InteractionState::FOCUSED))
            .computed()
    }

    /// Whether the button is currently disabled.
    #[must_use]
    pub fn is_disabled(&self) -> Computed<bool> {
        self.interaction
            .map(|state| state.contains(InteractionState::DISABLED))
            .computed()
    }
}

/// A custom button style that defines visual appearance while preserving
/// standard tap activation, disabled gating, interaction reporting, and
/// accessibility semantics.
///
/// Apply a custom style to one button with [`Button::style`] or to an entire
/// view subtree with `ViewExt::button_style`.
pub trait CustomButtonStyle: 'static {
    /// Builds the visual body of a button from its semantic `configuration`.
    fn make_body<L: View>(
        &self,
        configuration: ButtonStyleConfiguration<L>,
    ) -> impl View + use<Self, L>;
}

/// A custom button style that controls both visual appearance and activation
/// behavior via [`ButtonTrigger`].
///
/// Apply a primitive style to one button with [`Button::style`] or to an
/// entire view subtree with `ViewExt::button_style`.
pub trait PrimitiveButtonStyle: 'static {
    /// Builds the interactive body of a button from its semantic `configuration`.
    fn make_body<L: View>(
        &self,
        configuration: PrimitiveButtonStyleConfiguration<L>,
    ) -> impl View + use<Self, L>;
}

mod sealed {
    pub trait Sealed {}
}

#[doc(hidden)]
#[derive(Debug)]
pub struct BuiltinStyleMarker;

#[doc(hidden)]
#[derive(Debug)]
pub struct CustomStyleMarker;

#[doc(hidden)]
#[derive(Debug)]
pub struct PrimitiveStyleMarker;

/// Wrapper applying a [`CustomButtonStyle`].
#[derive(Debug, Clone)]
pub struct CustomButton<S>(S);

/// Wrapper applying a [`PrimitiveButtonStyle`].
#[derive(Debug, Clone)]
pub struct PrimitiveButton<S>(S);

impl sealed::Sealed for ButtonStyle {}
impl<S: CustomButtonStyle> sealed::Sealed for CustomButton<S> {}
impl<S: PrimitiveButtonStyle> sealed::Sealed for PrimitiveButton<S> {}

/// Types accepted by [`Button::style`] and `ViewExt::button_style`:
/// built-in [`ButtonStyle`], [`CustomButtonStyle`], or [`PrimitiveButtonStyle`].
pub trait IntoButtonStyle<Marker>: Sized + 'static {
    /// Concrete stored style representation.
    #[doc(hidden)]
    type Style: SealedButtonStyle;

    /// Converts this value into the stored style representation.
    #[doc(hidden)]
    fn into_button_style(self) -> Self::Style;
}

impl IntoButtonStyle<BuiltinStyleMarker> for ButtonStyle {
    type Style = Self;

    fn into_button_style(self) -> Self::Style {
        self
    }
}

impl<S: CustomButtonStyle> IntoButtonStyle<CustomStyleMarker> for S {
    type Style = CustomButton<Self>;

    fn into_button_style(self) -> Self::Style {
        CustomButton(self)
    }
}

impl<S: PrimitiveButtonStyle> IntoButtonStyle<PrimitiveStyleMarker> for S {
    type Style = PrimitiveButton<Self>;

    fn into_button_style(self) -> Self::Style {
        PrimitiveButton(self)
    }
}

#[doc(hidden)]
pub trait SealedButtonStyle: sealed::Sealed + Sized + 'static {
    fn button_body<L, Action>(button: Button<L, Action, Self>, env: &Environment) -> impl View
    where
        L: LabelKind,
        Action: FnMut(&Environment) -> Result<(), ExtractionError> + 'static;

    fn dispatch_button<B, L, Action>(
        button: Button<L, Action, Self>,
        backend: &mut B,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output
    where
        B: Backend,
        L: LabelKind,
        Action: FnMut(&Environment) -> Result<(), ExtractionError> + 'static;

    fn hook_apply(
        &self,
        target: ButtonStyleTarget,
        env: &Environment,
    ) -> Result<impl View, ButtonStyleTarget>;
}

/// Shared in-flight state for a single-flight async button action, plus an
/// optional caller-supplied `Binding<bool>` mirror that can be attached before
/// or after [`Button::action_async`].
#[derive(Clone)]
struct PendingState {
    in_flight: Binding<bool>,
    mirror: Rc<RefCell<Option<Binding<bool>>>>,
}

impl PendingState {
    fn new() -> Self {
        Self {
            in_flight: binding(false),
            mirror: Rc::new(RefCell::new(None)),
        }
    }

    fn with_mirror(mirror: Binding<bool>) -> Self {
        Self {
            in_flight: binding(false),
            mirror: Rc::new(RefCell::new(Some(mirror))),
        }
    }

    fn attach_mirror(&self, mirror: &Binding<bool>) {
        let is_pending = self.in_flight.snapshot();
        *self.mirror.borrow_mut() = Some(mirror.clone());
        if is_pending {
            mirror.set(true);
        }
    }

    fn is_pending(&self) -> bool {
        self.in_flight.snapshot()
    }

    fn signal(&self) -> Computed<bool> {
        self.in_flight.clone().computed()
    }

    fn set(&self, pending: bool) {
        self.in_flight.set(pending);
        let mirror = self.mirror.borrow().clone();
        if let Some(mirror) = mirror {
            mirror.set(pending);
        }
    }

    fn begin(&self) -> PendingGuard {
        self.set(true);
        PendingGuard {
            state: self.clone(),
        }
    }
}

/// Drop-safe RAII guard that clears [`PendingState`] when an async action
/// finishes, unwinds, or is cancelled by dropping its future.
struct PendingGuard {
    state: PendingState,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.state.set(false);
    }
}

/// Pending-state storage carried by a [`Button`]: either a live tracker owned
/// by `.pending(..)` / `.action_async(..)`, or a read-only signal preserved
/// across a [`ButtonConfig`] / [`ButtonStyleTarget`] round-trip.
#[derive(Clone)]
enum ButtonPending {
    Live(PendingState),
    Signal(Computed<bool>),
}

impl ButtonPending {
    fn signal(&self) -> Computed<bool> {
        match self {
            Self::Live(state) => state.signal(),
            Self::Signal(signal) => signal.clone(),
        }
    }
}

fn effective_disabled(env: &Environment, pending: Option<&Computed<bool>>) -> Computed<bool> {
    let env_disabled = Disabled::resolve(env, false);
    match pending {
        Some(pending) => zip(env_disabled, pending.clone())
            .map(|(disabled, is_pending)| disabled || is_pending)
            .computed(),
        None => env_disabled,
    }
}

fn button_interaction_signals(
    env: &Environment,
    pending: Option<&Computed<bool>>,
) -> (Binding<InteractionState>, Computed<InteractionState>) {
    let disabled = effective_disabled(env, pending);
    let initial = if disabled.snapshot() {
        InteractionState::DISABLED
    } else {
        InteractionState::empty()
    };
    let report = binding(initial);
    let interaction = zip(report.clone(), disabled)
        .map(|(flags, is_disabled)| {
            if is_disabled {
                flags | InteractionState::DISABLED
            } else {
                flags - InteractionState::DISABLED
            }
        })
        .computed();
    (report, interaction)
}

#[cfg(feature = "toggle")]
pub(crate) fn interaction_signals(
    env: &Environment,
) -> (Binding<InteractionState>, Computed<InteractionState>) {
    button_interaction_signals(env, None)
}

fn wrap_pending_accessibility_state<V: View>(
    env: &Environment,
    view: V,
    pending: Option<Computed<bool>>,
) -> OneOf2<V, Metadata<AccessibilityStateSignal, V>> {
    match pending {
        Some(pending) => {
            let state = AccessibilityStateSignal::resolve(
                env,
                pending.map(|busy| {
                    if busy {
                        AccessibilityState::new().busy(true).disabled(true)
                    } else {
                        AccessibilityState::new()
                    }
                }),
            );
            OneOf2::B(Metadata::new(view, state))
        }
        None => OneOf2::A(view),
    }
}

pub(crate) fn wrap_default_accessibility_label<V: View>(
    env: &Environment,
    view: V,
    accessibility_label: &Computed<StyledStr>,
) -> OneOf2<V, Metadata<AccessibilityLabel, V>> {
    if env.get::<AccessibilityLabel>().is_some() {
        OneOf2::A(view)
    } else {
        OneOf2::B(Metadata::new(
            view,
            AccessibilityLabel::new(accessibility_label.map(|s| s.to_plain()).computed()),
        ))
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "composes one custom-styled button from its semantic fields, action, pending signal, and env"
)]
fn compose_custom_button<L: View, S: CustomButtonStyle>(
    style: &S,
    label: L,
    accessibility_label: &Computed<StyledStr>,
    action: Rc<RefCell<BoxedAction<()>>>,
    role: Option<ButtonRole>,
    size: ControlSize,
    pending: Option<Computed<bool>>,
    env: &Environment,
) -> impl View + use<L, S> {
    let (report, interaction) = button_interaction_signals(env, pending.as_ref());
    let body = style.make_body(ButtonStyleConfiguration {
        label,
        role,
        size,
        interaction,
    });
    let tap_pending = pending.clone();
    let gesture =
        GestureObserver::without_event(TapGesture::new(), move |action_env: Environment| {
            let is_pending = tap_pending.as_ref().is_some_and(Computed::snapshot);
            if !Disabled::resolve(&action_env, false).snapshot() && !is_pending {
                (action.borrow_mut())(&action_env).report_extraction();
            }
        });
    let with_gesture = Metadata::new(body, gesture);
    let with_state = wrap_pending_accessibility_state(env, with_gesture, pending);
    let with_label = wrap_default_accessibility_label(env, with_state, accessibility_label);
    let with_role = default_role(env, with_label, AccessibilityRole::Button);
    Installed::new(with_role, InteractionReport(report))
}

#[expect(
    clippy::too_many_arguments,
    reason = "composes one primitive-styled button from its semantic fields, action, pending signal, and env"
)]
fn compose_primitive_button<L: View, S: PrimitiveButtonStyle>(
    style: &S,
    label: L,
    accessibility_label: &Computed<StyledStr>,
    action: Rc<RefCell<BoxedAction<()>>>,
    role: Option<ButtonRole>,
    size: ControlSize,
    pending: Option<Computed<bool>>,
    env: &Environment,
) -> impl View + use<L, S> {
    let (report, interaction) = button_interaction_signals(env, pending.as_ref());
    let trigger = ButtonTrigger {
        action,
        env: env.clone(),
        pending: pending.clone(),
    };
    let body = style.make_body(PrimitiveButtonStyleConfiguration {
        label,
        trigger,
        role,
        size,
        interaction,
    });
    let with_state = wrap_pending_accessibility_state(env, body, pending);
    let with_label = wrap_default_accessibility_label(env, with_state, accessibility_label);
    let with_role = default_role(env, with_label, AccessibilityRole::Button);
    Installed::new(with_role, InteractionReport(report))
}

fn resolve_custom_label<L: LabelKind>(
    label: Label<L>,
    env: &Environment,
) -> (Label<L>, Computed<StyledStr>) {
    let label = label
        .default_text_line_limit(core::num::NonZeroUsize::MIN)
        .resolve(env);
    let a11y_label = label.accessibility_label();
    (label, a11y_label)
}

impl SealedButtonStyle for ButtonStyle {
    fn button_body<L, Action>(button: Button<L, Action, Self>, env: &Environment) -> impl View
    where
        L: LabelKind,
        Action: FnMut(&Environment) -> Result<(), ExtractionError> + 'static,
    {
        Native::new(render_button_config::<_, _, AnyView>(button, env))
    }

    fn dispatch_button<B, L, Action>(
        button: Button<L, Action, Self>,
        backend: &mut B,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output
    where
        B: Backend,
        L: LabelKind,
        Action: FnMut(&Environment) -> Result<(), ExtractionError> + 'static,
    {
        backend.native(render_button_config::<_, _, B::Slot>(button, env), env, cx)
    }

    fn hook_apply(
        &self,
        target: ButtonStyleTarget,
        _env: &Environment,
    ) -> Result<impl View, ButtonStyleTarget> {
        let data = *target.0;
        let mode = data.label.display_mode_preference();
        let action = data.action;
        Ok(Button {
            label: Label::from_control_label(data.label, mode),
            action: move |env: &Environment| (action.borrow_mut())(env),
            style: *self,
            explicit_style: true,
            role: data.role,
            size: data.size,
            pending: data.pending.map(ButtonPending::Signal),
            shortcut: data.shortcut,
        })
    }
}

impl<S: CustomButtonStyle> SealedButtonStyle for CustomButton<S> {
    fn button_body<L, Action>(button: Button<L, Action, Self>, env: &Environment) -> impl View
    where
        L: LabelKind,
        Action: FnMut(&Environment) -> Result<(), ExtractionError> + 'static,
    {
        let (label, a11y_label) = resolve_custom_label(button.label, env);
        let action: Rc<RefCell<BoxedAction<()>>> = Rc::new(RefCell::new(Box::new(button.action)));
        compose_custom_button(
            &button.style.0,
            label,
            &a11y_label,
            action,
            button.role,
            button.size,
            button.pending.as_ref().map(ButtonPending::signal),
            env,
        )
    }

    fn dispatch_button<B, L, Action>(
        button: Button<L, Action, Self>,
        backend: &mut B,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output
    where
        B: Backend,
        L: LabelKind,
        Action: FnMut(&Environment) -> Result<(), ExtractionError> + 'static,
    {
        let (label, a11y_label) = resolve_custom_label(button.label, env);
        let action: Rc<RefCell<BoxedAction<()>>> = Rc::new(RefCell::new(Box::new(button.action)));
        compose_custom_button(
            &button.style.0,
            label,
            &a11y_label,
            action,
            button.role,
            button.size,
            button.pending.as_ref().map(ButtonPending::signal),
            env,
        )
        .dispatch(backend, env, cx)
    }

    fn hook_apply(
        &self,
        target: ButtonStyleTarget,
        env: &Environment,
    ) -> Result<impl View, ButtonStyleTarget> {
        let data = *target.0;
        let a11y_label = data.label.accessibility_label();
        let label = ButtonLabel(data.label);
        Ok(compose_custom_button(
            &self.0,
            label,
            &a11y_label,
            data.action,
            data.role,
            data.size,
            data.pending,
            env,
        ))
    }
}

impl<S: PrimitiveButtonStyle> SealedButtonStyle for PrimitiveButton<S> {
    fn button_body<L, Action>(button: Button<L, Action, Self>, env: &Environment) -> impl View
    where
        L: LabelKind,
        Action: FnMut(&Environment) -> Result<(), ExtractionError> + 'static,
    {
        let (label, a11y_label) = resolve_custom_label(button.label, env);
        let action: Rc<RefCell<BoxedAction<()>>> = Rc::new(RefCell::new(Box::new(button.action)));
        compose_primitive_button(
            &button.style.0,
            label,
            &a11y_label,
            action,
            button.role,
            button.size,
            button.pending.as_ref().map(ButtonPending::signal),
            env,
        )
    }

    fn dispatch_button<B, L, Action>(
        button: Button<L, Action, Self>,
        backend: &mut B,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output
    where
        B: Backend,
        L: LabelKind,
        Action: FnMut(&Environment) -> Result<(), ExtractionError> + 'static,
    {
        let (label, a11y_label) = resolve_custom_label(button.label, env);
        let action: Rc<RefCell<BoxedAction<()>>> = Rc::new(RefCell::new(Box::new(button.action)));
        compose_primitive_button(
            &button.style.0,
            label,
            &a11y_label,
            action,
            button.role,
            button.size,
            button.pending.as_ref().map(ButtonPending::signal),
            env,
        )
        .dispatch(backend, env, cx)
    }

    fn hook_apply(
        &self,
        target: ButtonStyleTarget,
        env: &Environment,
    ) -> Result<impl View, ButtonStyleTarget> {
        let data = *target.0;
        let a11y_label = data.label.accessibility_label();
        let label = ButtonLabel(data.label);
        Ok(compose_primitive_button(
            &self.0,
            label,
            &a11y_label,
            data.action,
            data.role,
            data.size,
            data.pending,
            env,
        ))
    }
}

struct ButtonStyleTargetData {
    label: ControlLabel<HookSlot>,
    action: Rc<RefCell<BoxedAction<()>>>,
    role: Option<ButtonRole>,
    size: ControlSize,
    pending: Option<Computed<bool>>,
    shortcut: Option<Shortcut>,
}

/// Internal hook target representing a [`Button`] eligible for subtree
/// styling via `ViewExt::button_style`.
#[doc(hidden)]
pub struct ButtonStyleTarget(Box<ButtonStyleTargetData>);

impl fmt::Debug for ButtonStyleTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ButtonStyleTarget")
            .field("role", &self.0.role)
            .field("size", &self.0.size)
            .finish_non_exhaustive()
    }
}

impl NativeView for ButtonStyleTarget {}

/// Hook that applies a button style to every non-explicitly-styled [`Button`]
/// in a subtree.
#[doc(hidden)]
#[derive(Debug)]
pub struct ButtonStyleHook<S>(Rc<S>);

impl<S> Clone for ButtonStyleHook<S> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<S: SealedButtonStyle> ButtonStyleHook<S> {
    /// Creates a subtree button-style hook.
    #[must_use]
    pub fn new(style: S) -> Self {
        Self(Rc::new(style))
    }
}

impl<S: SealedButtonStyle> Hook for ButtonStyleHook<S> {
    type Target = ButtonStyleTarget;

    fn apply(&self, target: Self::Target, env: &Environment) -> Result<impl View, Self::Target> {
        self.0.hook_apply(target, env)
    }
}

/// Pending button configuration saved while offering [`ButtonStyleTarget`] to hooks,
/// allowing exact restoration without type erasure if every hook declines.
#[doc(hidden)]
pub struct ButtonStylePending<S> {
    label: ControlLabel<S>,
    action: Rc<RefCell<BoxedAction<()>>>,
    style: ButtonStyle,
    explicit_style: bool,
    role: Option<ButtonRole>,
    size: ControlSize,
    pending: Option<Computed<bool>>,
    shortcut: Option<Shortcut>,
}

impl<S> fmt::Debug for ButtonStylePending<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ButtonStylePending")
            .field("style", &self.style)
            .field("explicit_style", &self.explicit_style)
            .field("role", &self.role)
            .field("size", &self.size)
            .field("shortcut", &self.shortcut)
            .finish_non_exhaustive()
    }
}

impl<S: 'static> ButtonStylePending<S> {
    /// Restores the original [`ButtonConfig<S>`] after all hooks declined.
    #[must_use]
    pub fn restore(self) -> ButtonConfig<S> {
        let action = self.action;
        ButtonConfig {
            label: self.label,
            action: Box::new(move |env: &Environment| (action.borrow_mut())(env)),
            style: self.style,
            explicit_style: self.explicit_style,
            role: self.role,
            size: self.size,
            pending: self.pending,
            shortcut: self.shortcut,
        }
    }
}

// ============================================================================
// Button configuration and view implementation
// ============================================================================

/// Configuration for a button component.
///
/// This struct contains all the data needed to render a button on native platforms.
/// It is produced by [`Button::config`] and consumed by the native backend.
///
/// # Layout Behavior
///
/// Buttons size themselves to fit their label content and never stretch to fill
/// extra space. In a stack, they take only the space they need.
#[non_exhaustive]
pub struct ButtonConfig<S> {
    /// The semantic label displayed on the button.
    ///
    /// [`ControlLabel`] carries both the visual chrome (text + icon + display mode)
    /// and the spoken text used by assistive technology. A hook receiving a
    /// `ButtonConfig` gets a typed [`ControlLabel`], so it can wrap or restyle it
    /// without stripping away the semantic identity.
    label: ControlLabel<S>,
    /// The action to execute when the button is clicked.
    action: BoxedAction<()>,
    /// The visual style of the button.
    style: ButtonStyle,
    /// Whether the visual style was set explicitly on the button.
    explicit_style: bool,
    /// Semantic role of the button, if any.
    role: Option<ButtonRole>,
    /// How large the button is drawn.
    size: ControlSize,
    /// Reactive in-flight flag for an async button action.
    pending: Option<Computed<bool>>,
    /// Keyboard shortcut armed while the button is mounted and enabled.
    shortcut: Option<Shortcut>,
}

impl<S> ButtonConfig<S> {
    /// The semantic label displayed on the button.
    #[must_use]
    pub const fn label(&self) -> &ControlLabel<S> {
        &self.label
    }

    /// The action to execute when the button is clicked.
    #[must_use]
    pub const fn action(&self) -> &BoxedAction<()> {
        &self.action
    }

    /// The action to execute when the button is clicked, for calling it.
    pub const fn action_mut(&mut self) -> &mut BoxedAction<()> {
        &mut self.action
    }

    /// Splits the value into its label, action, style and size.
    pub fn into_parts(self) -> (ControlLabel<S>, BoxedAction<()>, ButtonStyle, ControlSize) {
        (self.label, self.action, self.style, self.size)
    }

    /// The visual style of the button.
    #[must_use]
    pub const fn style(&self) -> ButtonStyle {
        self.style
    }

    /// Semantic role of the button, if any.
    #[must_use]
    pub const fn role(&self) -> Option<ButtonRole> {
        self.role
    }

    /// How large the button is drawn.
    #[must_use]
    pub const fn size(&self) -> ControlSize {
        self.size
    }

    /// Reactive signal reporting whether this button's asynchronous action is
    /// currently in flight, if the button carries an async action.
    #[must_use]
    pub const fn pending(&self) -> Option<&Computed<bool>> {
        self.pending.as_ref()
    }

    /// Keyboard shortcut armed while this button is mounted and enabled, if any.
    #[must_use]
    pub fn shortcut(&self) -> Option<Shortcut> {
        self.shortcut.clone()
    }

    /// Whether the button chose its style explicitly.
    #[doc(hidden)]
    #[must_use]
    pub const fn has_explicit_style(&self) -> bool {
        self.explicit_style
    }
}

impl<S: 'static> HookableParent<S> for ButtonConfig<S> {
    type StyleTarget = ButtonStyleTarget;
    type Pending = ButtonStylePending<S>;

    fn is_style_hookable(&self) -> bool {
        !self.explicit_style
    }

    fn into_style_pending(
        self,
        map_slot: impl Fn(S) -> HookSlot + Clone + 'static,
    ) -> (ButtonStyleTarget, ButtonStylePending<S>) {
        let style_label = self.label.clone().map_slots(map_slot);
        let action: Rc<RefCell<BoxedAction<()>>> = Rc::new(RefCell::new(self.action));
        let target = ButtonStyleTarget(Box::new(ButtonStyleTargetData {
            label: style_label,
            action: Rc::clone(&action),
            role: self.role,
            size: self.size,
            pending: self.pending.clone(),
            shortcut: self.shortcut.clone(),
        }));
        let pending = ButtonStylePending {
            label: self.label,
            action,
            style: self.style,
            explicit_style: self.explicit_style,
            role: self.role,
            size: self.size,
            pending: self.pending,
            shortcut: self.shortcut,
        };
        (target, pending)
    }

    fn restore_pending(pending: Self::Pending) -> Self {
        pending.restore()
    }
}

impl<S> Debug for ButtonConfig<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ButtonConfig")
    }
}

impl<S: 'static> NativeView for ButtonConfig<S> {}

impl<S: 'static> MapSlots<S> for ButtonConfig<S> {
    type With<T: 'static> = ButtonConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> ButtonConfig<T> {
        ButtonConfig {
            label: self.label.map_slots(f),
            action: self.action,
            style: self.style,
            explicit_style: self.explicit_style,
            role: self.role,
            size: self.size,
            pending: self.pending,
            shortcut: self.shortcut,
        }
    }

    fn offer_style_hook<Scope: ParentHookScope<S>>(
        self,
        scope: &mut Scope,
        env: &Environment,
        cx: Scope::Ctx,
    ) -> Result<Scope::Output, (Self, Scope::Ctx)> {
        offer_hookable_parent(self, scope, env, cx)
    }
}

impl ButtonStyle {
    /// This style in `env`. `Automatic` means "whatever this context calls
    /// for", and a surrounding scope can say what that is — a list row wants
    /// row-style buttons, not the filled containers a screen's primary
    /// action gets — while an explicit style still wins.
    pub(crate) fn resolve(self, env: &Environment) -> Self {
        match (self, env.get::<Self>()) {
            (Self::Automatic, Some(ambient)) => *ambient,
            (style, _) => style,
        }
    }
}

fn render_button_config<L, Action, S>(
    button: Button<L, Action, ButtonStyle>,
    env: &Environment,
) -> ButtonConfig<S>
where
    L: LabelKind,
    Action: FnMut(&Environment) -> Result<(), ExtractionError> + 'static,
    S: ChildSlot,
{
    ButtonConfig {
        // A button label stays one truncated line — the SwiftUI and Material
        // baseline — instead of folding into a paragraph under compression.
        // Applied here so themed hooks and native backends agree, while an
        // explicit `Text::line_limit` from the application still wins.
        label: button
            .label
            .default_text_line_limit(core::num::NonZeroUsize::MIN)
            .into_control_label::<S>(env),
        action: Box::new(button.action),
        style: button.style.resolve(env),
        explicit_style: button.explicit_style,
        role: button.role,
        size: button.size,
        pending: button.pending.as_ref().map(ButtonPending::signal),
        shortcut: button.shortcut,
    }
}

/// Action configured on a [`Button`].
///
/// Any `FnMut(&Environment) -> Result<(), ExtractionError> + 'static` implements
/// this trait; [`NoAction`] intentionally does not so a button without an action
/// cannot be rendered as a [`View`].
#[diagnostic::on_unimplemented(
    message = "a `Button` requires an action before it can be used as a `View`",
    label = "this button has no action configured",
    note = "call `.action(...)`, `.action_async(...)`, or `.action_boxed(...)` on the button"
)]
pub trait ButtonAction: FnMut(&Environment) -> Result<(), ExtractionError> + 'static {}

impl<F> ButtonAction for F where F: FnMut(&Environment) -> Result<(), ExtractionError> + 'static {}

impl<L, Action, Style> View for Button<L, Action, Style>
where
    L: LabelKind,
    Action: ButtonAction,
    Style: SealedButtonStyle,
{
    fn body(self, env: &Environment) -> impl View {
        Style::button_body(self, env)
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        Style::dispatch_button(self, backend, env, cx)
    }
}

impl<S: View> From<ButtonConfig<S>> for Button<ControlLabel<S>, BoxedAction<()>> {
    fn from(value: ButtonConfig<S>) -> Self {
        let mode = value.label.display_mode_preference();
        Self {
            label: Label::from_control_label(value.label, mode),
            action: value.action,
            style: value.style,
            explicit_style: value.explicit_style,
            role: value.role,
            size: value.size,
            pending: value.pending.map(ButtonPending::Signal),
            shortcut: value.shortcut,
        }
    }
}

impl<L, Action> Button<L, Action>
where
    L: LabelKind,
    Action: ButtonAction,
{
    /// Returns the configuration for this button.
    #[must_use]
    pub fn config(self) -> ButtonConfig<Label<L>> {
        ButtonConfig {
            label: self.label.into_unscoped_control_label(),
            action: Box::new(self.action),
            style: self.style,
            explicit_style: self.explicit_style,
            role: self.role,
            size: self.size,
            pending: self.pending.as_ref().map(ButtonPending::signal),
            shortcut: self.shortcut,
        }
    }
}

// ============================================================================
// Button struct
// ============================================================================

/// A button component that triggers an action when clicked.
///
/// Create buttons using the [`button`] convenience function or [`Button::new`].
/// Configure the action using one of these patterns:
///
/// - [`action`](Button::action) - Simple closure with no parameters
/// - [`action`](Button::action) with extractor parameters - Extract from the environment at click time
/// - `ViewExt::state` - Inject local cloneable state for later extraction
///
/// See the [module documentation](self) for detailed examples.
pub struct Button<L, Action, Style = ButtonStyle> {
    label: Label<L>,
    action: Action,
    style: Style,
    explicit_style: bool,
    role: Option<ButtonRole>,
    size: ControlSize,
    pending: Option<ButtonPending>,
    shortcut: Option<Shortcut>,
}

impl<L, Action, Style> Button<L, Action, Style> {
    /// Returns the semantic label displayed by this button.
    #[must_use]
    pub const fn label(&self) -> &Label<L> {
        &self.label
    }

    /// Returns the semantic role assigned to this button, if any.
    #[must_use]
    pub const fn button_role(&self) -> Option<ButtonRole> {
        self.role
    }

    /// Sets the semantic role of the button.
    #[must_use]
    pub const fn role(mut self, role: ButtonRole) -> Self {
        self.role = Some(role);
        self
    }

    /// Extracts the label, action handler, and semantic role for action conversions.
    #[doc(hidden)]
    pub fn into_action_parts(self) -> (Label<L>, Action, Option<ButtonRole>) {
        (self.label, self.action, self.role)
    }

    /// Returns the size this button is drawn at.
    #[must_use]
    pub const fn button_size(&self) -> ControlSize {
        self.size
    }

    /// Sets how large the button is drawn. Buttons default to
    /// [`ControlSize::Small`].
    #[must_use]
    pub const fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }
}

impl<L, Action> Button<L, Action, ButtonStyle> {
    /// Returns the visual style requested for this button.
    #[must_use]
    pub const fn button_style(&self) -> ButtonStyle {
        self.style
    }

    const fn with_builtin_style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self.explicit_style = true;
        self
    }

    impl_style_methods!();
}

impl<L: Debug, Action, Style: Debug> Debug for Button<L, Action, Style> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Button")
            .field("label", &self.label)
            .field("style", &self.style)
            .field("role", &self.role)
            .field("size", &self.size)
            .field("shortcut", &self.shortcut)
            .finish_non_exhaustive()
    }
}

impl<L: Clone, Action: Clone, Style: Clone> Clone for Button<L, Action, Style> {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            action: self.action.clone(),
            style: self.style.clone(),
            explicit_style: self.explicit_style,
            role: self.role,
            size: self.size,
            pending: self.pending.clone(),
            shortcut: self.shortcut.clone(),
        }
    }
}

/// Marker type for a [`Button`] that does not yet have an action configured.
///
/// `Button<L, NoAction, Style>` does not implement [`View`], so forgetting
/// `.action(...)` or `.action_async(...)` is a compile error:
///
/// ```compile_fail,E0277
/// use waterui::prelude::*;
///
/// fn missing_action() -> impl View {
///     button("Save")
/// }
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct NoAction;

impl<L> Button<L, NoAction, ButtonStyle> {
    /// Creates a new button with the specified semantic label.
    ///
    /// The button has no action until [`action`](Self::action),
    /// [`action_async`](Self::action_async), or [`action_boxed`](Self::action_boxed)
    /// is called, and does not implement [`View`] before then.
    ///
    /// # Arguments
    ///
    /// * `label` - The semantic label to display on the button
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui::prelude::*;
    /// # use waterui_controls::Button;
    /// # fn demo() {
    /// let btn = Button::new("Click me").action(|| {});
    /// # }
    /// ```
    #[must_use]
    pub fn new<I: IntoLabel<Kind = L>>(label: I) -> Self {
        Self {
            label: label.into_label(),
            action: NoAction,
            style: ButtonStyle::Automatic,
            explicit_style: false,
            role: None,
            size: ControlSize::Small,
            pending: None,
            shortcut: None,
        }
    }
}

impl<L, Action, Style> Button<L, Action, Style> {
    /// Sets the visual style of the button.
    ///
    /// Accepts a built-in [`ButtonStyle`], a [`CustomButtonStyle`], or a
    /// [`PrimitiveButtonStyle`].
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui::prelude::*;
    /// # use waterui_controls::ButtonStyle;
    /// # fn submit_form() {}
    /// # fn demo() {
    /// button("Submit")
    ///     .style(ButtonStyle::BorderedProminent)
    ///     .action(|| submit_form());
    /// # }
    /// ```
    #[must_use]
    pub fn style<NewStyle, Marker>(
        self,
        style: NewStyle,
    ) -> Button<L, Action, <NewStyle as IntoButtonStyle<Marker>>::Style>
    where
        NewStyle: IntoButtonStyle<Marker>,
    {
        Button {
            label: self.label,
            action: self.action,
            style: style.into_button_style(),
            explicit_style: true,
            role: self.role,
            size: self.size,
            pending: self.pending,
            shortcut: self.shortcut,
        }
    }

    /// Overrides the spoken accessibility text announced by assistive technology.
    ///
    /// This rewrites only the textual portion of the semantic label, preserving
    /// any icon attached at construction time. The visual label rendered on
    /// screen is unaffected.
    #[must_use]
    pub fn a11y_label<M>(mut self, label: impl IntoAccessibilityLabel<M>) -> Self {
        self.label = self.label.accessibility_text(label);
        self
    }

    /// Sets the visual presentation mode of the button's label.
    ///
    /// The semantic identity is always preserved for assistive technology
    /// regardless of the chosen visual mode.
    #[must_use]
    pub const fn label_style(mut self, mode: LabelDisplayMode) -> Self {
        self.label.set_display_mode(mode);
        self
    }

    /// Visually hides the button's label while keeping its semantic text in
    /// the accessibility tree.
    ///
    /// Shortcut for `.label_style(LabelDisplayMode::Hidden)`.
    #[must_use]
    pub const fn hide_label(self) -> Self {
        self.label_style(LabelDisplayMode::Hidden)
    }

    /// Assigns a keyboard shortcut that activates this button while it is
    /// mounted and enabled.
    #[must_use]
    pub const fn keyboard_shortcut(mut self, shortcut: Shortcut) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    /// Mirrors whether this button's asynchronous action is currently in flight
    /// into `pending`.
    ///
    /// Composes in either order with [`action_async`](Self::action_async):
    /// `button("Save").action_async(..).pending(&saving)` and
    /// `button("Save").pending(&saving).action_async(..)` behave identically.
    /// `pending` becomes `true` while the spawned future runs and is cleared to
    /// `false` when it completes or is dropped.
    #[must_use]
    pub fn pending(mut self, pending: &Binding<bool>) -> Self {
        match &self.pending {
            Some(ButtonPending::Live(state)) => state.attach_mirror(pending),
            Some(ButtonPending::Signal(_)) | None => {
                self.pending = Some(ButtonPending::Live(PendingState::with_mirror(
                    pending.clone(),
                )));
            }
        }
        self
    }

    /// Sets the action to be performed when the button is clicked.
    ///
    /// This is the simplest action form - a closure that takes no parameters.
    /// Use this when the action doesn't need any external state or
    /// environment values.
    ///
    /// # Arguments
    ///
    /// * `action` - A closure to execute when clicked
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui::prelude::*;
    /// # fn demo() {
    /// button("Click me").action(|| {});
    /// # }
    /// ```
    #[must_use]
    pub fn action<F, Args>(
        self,
        action: F,
    ) -> Button<L, impl FnMut(&Environment) -> Result<(), ExtractionError>, Style>
    where
        F: Handler<Args, ()> + 'static,
    {
        self.action_boxed(waterui_core::handler::boxed_action(action))
    }

    /// Sets an already type-erased action to be performed when the button is clicked.
    #[must_use]
    pub fn action_boxed(self, action: BoxedAction<()>) -> Button<L, BoxedAction<()>, Style> {
        Button {
            label: self.label,
            action,
            style: self.style,
            explicit_style: self.explicit_style,
            role: self.role,
            size: self.size,
            pending: self.pending,
            shortcut: self.shortcut,
        }
    }

    /// Sets a single-flight asynchronous action to be performed when the button
    /// is clicked.
    ///
    /// The future is spawned on the local executor and detached. While it runs,
    /// further presses are ignored, the button's interaction state reports
    /// disabled, accessibility reports busy, and any binding attached with
    /// [`pending`](Self::pending) mirrors `true` until the future completes or
    /// is dropped.
    ///
    /// # Arguments
    ///
    /// * `action` - A closure that returns a future
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui::prelude::*;
    /// # mod api {
    /// #     use waterui::Str;
    /// #     pub async fn fetch_data() -> Str {
    /// #         Str::from("payload")
    /// #     }
    /// # }
    /// # fn process(_data: Str) {}
    /// # fn demo() {
    /// button("Fetch").action_async(|| async {
    ///     let data = api::fetch_data().await;
    ///     process(data);
    /// });
    /// # }
    /// ```
    pub fn action_async<F, Fut, Args>(
        self,
        action: F,
    ) -> Button<L, impl FnMut(&Environment) -> Result<(), ExtractionError>, Style>
    where
        F: Handler<Args, Fut> + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let mut action = action;
        let pending = match self.pending {
            Some(ButtonPending::Live(state)) => state,
            Some(ButtonPending::Signal(_)) | None => PendingState::new(),
        };
        let action_pending = pending.clone();
        Button {
            label: self.label,
            // Single-flight: ignore presses while a spawned future is in flight.
            // A failed extraction means the action does not run: no future is
            // spawned, pending stays false, and the failure travels to the
            // button's dispatcher.
            action: move |env: &Environment| {
                if action_pending.is_pending() {
                    return Ok(());
                }
                let future = action.call(env)?;
                let guard = action_pending.begin();
                spawn_local(async move {
                    let _guard = guard;
                    future.await;
                })
                .detach();
                Ok(())
            },
            style: self.style,
            explicit_style: self.explicit_style,
            role: self.role,
            size: self.size,
            pending: Some(ButtonPending::Live(pending)),
            shortcut: self.shortcut,
        }
    }
}

#[cfg(feature = "menu")]
impl<L: LabelKind, Action, Style> From<Button<L, Action, Style>> for crate::menu::Command
where
    Action: ButtonAction,
{
    fn from(value: Button<L, Action, Style>) -> Self {
        let Button {
            label,
            mut action,
            style: _,
            explicit_style: _,
            role,
            size: _,
            pending,
            shortcut,
        } = value;
        let command =
            Self::from_semantic_label(label.into_semantic()).action(move |env: Environment| {
                action(&env).report_extraction();
            });
        let command = match role {
            Some(role) => command.role(role),
            None => command,
        };
        let command = match shortcut {
            Some(shortcut) => command.shortcut(shortcut),
            None => command,
        };
        match pending {
            Some(pending) => command.disabled(pending.signal()),
            None => command,
        }
    }
}

#[cfg(feature = "menu")]
impl<L: LabelKind, Action, Style> From<Button<L, Action, Style>> for crate::menu::MenuItem
where
    Action: ButtonAction,
{
    fn from(value: Button<L, Action, Style>) -> Self {
        Self::Command(value.into())
    }
}

// ============================================================================
// Convenience function
// ============================================================================

/// Creates a new button with the specified label.
///
/// This is a convenience function equivalent to [`Button::new`].
///
/// # Arguments
///
/// * `label` - The semantic label to display on the button
///
/// # Example
///
/// ```rust
/// use waterui::prelude::*;
///
/// # fn demo() {
/// button("Click me").action(|| {});
/// # }
/// ```
pub fn button<L: IntoLabel>(label: L) -> Button<L::Kind, NoAction> {
    Button::new(label)
}

#[cfg(all(test, feature = "menu"))]
mod tests {
    use core::any::Any;

    use super::*;
    use waterui_core::{Native, NativeView, Signal};

    struct SearchIconProbe(bool);

    #[derive(Clone, Copy)]
    struct SearchIcon;

    impl NativeView for SearchIcon {
        fn realize_provided(self, context: &mut dyn Any, _env: &Environment) -> Result<(), Self> {
            if let Some(probe) = context.downcast_mut::<SearchIconProbe>() {
                probe.0 = true;
                return Ok(());
            }
            Err(self)
        }
    }

    impl View for SearchIcon {
        fn body(self, _env: &Environment) -> impl View {
            Native::new(self)
        }
    }

    #[test]
    fn menu_command_preserves_label_through_config_render_roundtrip() {
        crate::init_test_executor();
        let rendered = Button::from(
            button(crate::label::label("Edit").icon(SearchIcon))
                .action(|| {})
                .config(),
        );
        let command: crate::menu::Command = rendered.into();
        assert_eq!(
            command
                .label()
                .semantic_text()
                .resolve(&Environment::default())
                .content()
                .snapshot()
                .to_plain()
                .as_str(),
            "Edit"
        );
        let icon = command
            .label()
            .icon_slot::<AnyView>()
            .expect("label icon slot should be preserved");
        let mut probe = SearchIconProbe(false);
        assert!(icon.probe(&mut probe, &Environment::default()));
        assert!(probe.0);
    }

    #[test]
    fn a11y_label_overrides_text_only() {
        crate::init_test_executor();
        // Calling .a11y_label("X") must rewrite the spoken text
        // while preserving the visible text and icon attached at construction time.
        let rendered = Button::from(
            button(crate::label::label("Edit").icon(SearchIcon))
                .a11y_label("Edit document")
                .action(|| {})
                .config(),
        );
        let command: crate::menu::Command = rendered.into();
        assert_eq!(
            command
                .label()
                .semantic_text()
                .resolve(&Environment::default())
                .content()
                .snapshot()
                .to_plain()
                .as_str(),
            "Edit"
        );
        assert_eq!(
            command
                .label()
                .clone()
                .resolve(&Environment::default())
                .accessibility_label()
                .snapshot()
                .to_plain()
                .as_str(),
            "Edit document"
        );
        let icon = command
            .label()
            .icon_slot::<AnyView>()
            .expect("icon should be preserved when only the spoken text is overridden");
        let mut probe = SearchIconProbe(false);
        assert!(icon.probe(&mut probe, &Environment::default()));
        assert!(probe.0);
    }

    #[test]
    fn menu_command_conversion_preserves_button_role() {
        crate::init_test_executor();
        let destructive: crate::menu::Command = button("Delete")
            .role(ButtonRole::Destructive)
            .action(|| {})
            .into();
        assert_eq!(destructive.command_role(), Some(ButtonRole::Destructive));

        let cancel: crate::menu::Command = button("Cancel")
            .role(ButtonRole::Cancel)
            .action(|| {})
            .into();
        assert_eq!(cancel.command_role(), Some(ButtonRole::Cancel));

        let plain: crate::menu::Command = button("Open").action(|| {}).into();
        assert_eq!(plain.command_role(), None);
    }
}

#[cfg(test)]
mod custom_style_tests {
    use alloc::rc::Rc;
    use core::cell::{Cell, RefCell};
    use nami::{Signal as _, binding};
    use waterui_accessibility::{AccessibilityLabel, AccessibilityRole};
    use waterui_core::backend::peel_metadata;
    use waterui_core::{AnyView, Environment, View};
    use waterui_interaction::gesture::GestureObserver;
    use waterui_interaction::interaction::{Disabled, InteractionReport, InteractionState};

    use super::{
        ButtonRole, ButtonStyleConfiguration, CustomButtonStyle, PrimitiveButtonStyle,
        PrimitiveButtonStyleConfiguration, button,
    };
    use crate::size::ControlSize;

    #[derive(Clone)]
    struct RecordedConfig {
        role: Option<ButtonRole>,
        size: ControlSize,
        disabled: nami::Computed<bool>,
        pressed: nami::Computed<bool>,
        hovered: nami::Computed<bool>,
        focused: nami::Computed<bool>,
        interaction: nami::Computed<InteractionState>,
    }

    struct RecordingCustomStyle {
        recorded: Rc<RefCell<Option<RecordedConfig>>>,
    }

    impl CustomButtonStyle for RecordingCustomStyle {
        fn make_body<L: View>(
            &self,
            configuration: ButtonStyleConfiguration<L>,
        ) -> impl View + use<L> {
            *self.recorded.borrow_mut() = Some(RecordedConfig {
                role: configuration.role(),
                size: configuration.size(),
                disabled: configuration.is_disabled(),
                pressed: configuration.is_pressed(),
                hovered: configuration.is_hovered(),
                focused: configuration.is_focused(),
                interaction: configuration.interaction_state().clone(),
            });
            configuration.into_label()
        }
    }

    struct RecordedPrimitiveConfig {
        role: Option<ButtonRole>,
        size: ControlSize,
        disabled: nami::Computed<bool>,
    }

    struct RecordingPrimitiveStyle {
        recorded: Rc<RefCell<Option<RecordedPrimitiveConfig>>>,
        trigger_slot: Rc<RefCell<Option<super::ButtonTrigger>>>,
    }

    impl PrimitiveButtonStyle for RecordingPrimitiveStyle {
        fn make_body<L: View>(
            &self,
            configuration: PrimitiveButtonStyleConfiguration<L>,
        ) -> impl View + use<L> {
            *self.recorded.borrow_mut() = Some(RecordedPrimitiveConfig {
                role: configuration.role(),
                size: configuration.size(),
                disabled: configuration.is_disabled(),
            });
            *self.trigger_slot.borrow_mut() = Some(configuration.trigger().clone());
            configuration.into_label()
        }
    }

    #[test]
    fn custom_style_exposes_semantic_config_and_wraps_accessibility_and_interaction() {
        let recorded = Rc::new(RefCell::new(None));
        let tapped = Rc::new(Cell::new(0u32));
        let disabled_binding = binding(false);
        let mut env = Environment::new();
        Disabled::install(&mut env, disabled_binding.clone());

        let view = button("Delete")
            .role(ButtonRole::Destructive)
            .size(ControlSize::Large)
            .hide_label()
            .action({
                let tapped = Rc::clone(&tapped);
                move || tapped.set(tapped.get() + 1)
            })
            .style(RecordingCustomStyle {
                recorded: Rc::clone(&recorded),
            });

        let body = View::body(view, &env);
        let recorded = recorded
            .borrow_mut()
            .take()
            .expect("custom style make_body must run");
        assert_eq!(recorded.role, Some(ButtonRole::Destructive));
        assert_eq!(recorded.size, ControlSize::Large);
        assert!(!recorded.disabled.snapshot());
        assert!(!recorded.pressed.snapshot());
        assert!(!recorded.hovered.snapshot());
        assert!(!recorded.focused.snapshot());

        // `Disabled` scope updates `is_disabled` and `interaction_state` reactively.
        disabled_binding.set(true);
        assert!(recorded.disabled.snapshot());
        assert!(
            recorded
                .interaction
                .snapshot()
                .contains(InteractionState::DISABLED)
        );
        disabled_binding.set(false);
        assert!(!recorded.disabled.snapshot());

        // Peel Metadata<Environment> (from Installed<InteractionReport>) ->
        // AccessibilityRole -> AccessibilityLabel -> GestureObserver.
        let env_view = peel_metadata::<Environment>(AnyView::new(body), &env)
            .expect("custom-styled button installs InteractionReport into child environment");
        let report_binding = env_view
            .value
            .get::<InteractionReport>()
            .expect("InteractionReport plugin must be installed")
            .0
            .clone();
        report_binding.set(InteractionState::HOVERED | InteractionState::PRESSED);
        assert!(recorded.hovered.snapshot());
        assert!(recorded.pressed.snapshot());
        assert!(!recorded.focused.snapshot());
        report_binding.set(InteractionState::FOCUSED);
        assert!(recorded.focused.snapshot());
        assert!(!recorded.pressed.snapshot());

        let mut inner_env = env_view.value;
        let role_view = peel_metadata::<AccessibilityRole>(env_view.content, &inner_env)
            .expect("custom-styled button attaches AccessibilityRole::Button");
        assert_eq!(role_view.value, AccessibilityRole::Button);

        let a11y_label = peel_metadata::<AccessibilityLabel>(role_view.content, &env)
            .expect("custom-styled button attaches AccessibilityLabel even when label is hidden");
        assert_eq!(a11y_label.value.resolve(&env).snapshot().as_str(), "Delete");

        let observer = peel_metadata::<GestureObserver>(a11y_label.content, &env)
            .expect("custom-styled button wraps body in TapGesture observer");
        let (_, action) = observer.value.into_parts();
        let mut action = action;

        // Enabled tap fires the button action.
        action(&inner_env).expect("tap action extraction succeeds");
        assert_eq!(tapped.get(), 1);

        // Disabled tap is blocked.
        Disabled::install(&mut inner_env, true);
        action(&inner_env).expect("disabled tap check succeeds");
        assert_eq!(tapped.get(), 1);
    }

    #[test]
    fn custom_style_respects_interaction_report_and_action_after_style() {
        let recorded = Rc::new(RefCell::new(None));
        let tapped = Rc::new(Cell::new(0u32));
        let env = Environment::new();

        // `.action(...)` chained *after* `.style(...)` preserves the custom style.
        let view = button("Save")
            .role(ButtonRole::Cancel)
            .style(RecordingCustomStyle {
                recorded: Rc::clone(&recorded),
            })
            .action({
                let tapped = Rc::clone(&tapped);
                move || tapped.set(tapped.get() + 1)
            });

        let body = View::body(view, &env);
        let recorded = recorded.borrow_mut().take().expect("style must run");
        assert_eq!(recorded.role, Some(ButtonRole::Cancel));

        // Simulate backend writing InteractionReport flags while walking the subtree.
        // Installed<InteractionReport> attaches the plugin to env before children resolve.
        assert!(env.get::<InteractionReport>().is_none());
        let _ = body;
    }

    #[test]
    fn primitive_style_exposes_trigger_and_gates_disabled() {
        let recorded = Rc::new(RefCell::new(None));
        let trigger_slot = Rc::new(RefCell::new(None));
        let tapped = Rc::new(Cell::new(0u32));
        let disabled_binding = binding(false);
        let mut env = Environment::new();
        Disabled::install(&mut env, disabled_binding.clone());

        let view = button("Send")
            .role(ButtonRole::Destructive)
            .size(ControlSize::ExtraLarge)
            .style(RecordingPrimitiveStyle {
                recorded: Rc::clone(&recorded),
                trigger_slot: Rc::clone(&trigger_slot),
            })
            .action({
                let tapped = Rc::clone(&tapped);
                move || tapped.set(tapped.get() + 1)
            });

        let body = View::body(view, &env);
        let recorded_cfg = recorded
            .borrow_mut()
            .take()
            .expect("primitive style must run");
        assert_eq!(recorded_cfg.role, Some(ButtonRole::Destructive));
        assert_eq!(recorded_cfg.size, ControlSize::ExtraLarge);
        assert!(!recorded_cfg.disabled.snapshot());
        let is_disabled = recorded_cfg.disabled;

        // Primitive style wraps InteractionReport -> AccessibilityRole -> AccessibilityLabel,
        // without injecting an automatic TapGesture.
        let env_view = peel_metadata::<Environment>(AnyView::new(body), &env)
            .expect("primitive-styled button installs InteractionReport into child environment");
        assert!(env_view.value.get::<InteractionReport>().is_some());
        let role_view = peel_metadata::<AccessibilityRole>(env_view.content, &env_view.value)
            .expect("primitive-styled button still publishes AccessibilityRole::Button");
        assert_eq!(role_view.value, AccessibilityRole::Button);
        let a11y_label = peel_metadata::<AccessibilityLabel>(role_view.content, &env)
            .expect("primitive-styled button preserves AccessibilityLabel");
        assert!(peel_metadata::<GestureObserver>(a11y_label.content, &env).is_none());

        let trigger = trigger_slot
            .borrow_mut()
            .take()
            .expect("primitive style received ButtonTrigger");
        trigger.trigger();
        assert_eq!(tapped.get(), 1);

        disabled_binding.set(true);
        assert!(is_disabled.snapshot());
        trigger.trigger();
        assert_eq!(tapped.get(), 1, "disabled button trigger must not fire");
    }

    struct ManualGate {
        ready: Cell<bool>,
        waker: RefCell<Option<core::task::Waker>>,
    }

    impl ManualGate {
        fn new() -> Rc<Self> {
            Rc::new(Self {
                ready: Cell::new(false),
                waker: RefCell::new(None),
            })
        }

        fn complete(&self) {
            self.ready.set(true);
            if let Some(waker) = self.waker.borrow_mut().take() {
                waker.wake();
            }
        }

        fn wait(self: Rc<Self>) -> GateFuture {
            GateFuture(self)
        }
    }

    struct GateFuture(Rc<ManualGate>);

    impl core::future::Future for GateFuture {
        type Output = ();

        fn poll(
            self: core::pin::Pin<&mut Self>,
            cx: &mut core::task::Context<'_>,
        ) -> core::task::Poll<()> {
            if self.0.ready.get() {
                core::task::Poll::Ready(())
            } else {
                *self.0.waker.borrow_mut() = Some(cx.waker().clone());
                core::task::Poll::Pending
            }
        }
    }

    #[test]
    fn action_async_is_single_flight_and_mirrors_pending_in_both_orders() {
        crate::init_test_executor();
        let env = Environment::new();

        for pending_first in [false, true] {
            let saving = binding(false);
            let calls = Rc::new(Cell::new(0u32));
            let gate_slot = Rc::new(RefCell::new(ManualGate::new()));
            let handler = {
                let calls = Rc::clone(&calls);
                let gate = Rc::clone(&gate_slot);
                move || {
                    calls.set(calls.get() + 1);
                    let current = Rc::clone(&gate.borrow());
                    async move {
                        current.wait().await;
                    }
                }
            };

            let mut cfg = if pending_first {
                button("Save")
                    .pending(&saving)
                    .action_async(handler)
                    .config()
            } else {
                button("Save")
                    .action_async(handler)
                    .pending(&saving)
                    .config()
            };
            let pending_signal = cfg
                .pending()
                .expect("async button config carries pending signal")
                .clone();

            assert!(!saving.snapshot());
            assert!(!pending_signal.snapshot());

            // First press starts the future; rapid double-tap is ignored.
            (cfg.action_mut())(&env).expect("first press succeeds");
            assert_eq!(calls.get(), 1);
            assert!(saving.snapshot());
            assert!(pending_signal.snapshot());

            (cfg.action_mut())(&env).expect("second press while pending is a no-op");
            assert_eq!(
                calls.get(),
                1,
                "rapid double-tap must not spawn a second future (pending_first={pending_first})"
            );

            // Poll once so the future registers its waker and stays suspended.
            let _ = waterui_testing::drain_parked_local_work();
            assert!(saving.snapshot());
            assert!(pending_signal.snapshot());

            // Another tap while suspended mid-await is still ignored.
            (cfg.action_mut())(&env).expect("press while suspended is a no-op");
            assert_eq!(calls.get(), 1);

            // Completing the future clears pending and re-arms the button.
            gate_slot.borrow().complete();
            let _ = waterui_testing::drain_parked_local_work();
            assert!(!saving.snapshot());
            assert!(!pending_signal.snapshot());

            // A fresh press after completion runs again.
            *gate_slot.borrow_mut() = ManualGate::new();
            (cfg.action_mut())(&env).expect("press after completion succeeds");
            assert_eq!(calls.get(), 2);
            assert!(saving.snapshot());
            assert!(pending_signal.snapshot());
            gate_slot.borrow().complete();
            let _ = waterui_testing::drain_parked_local_work();
            assert!(!saving.snapshot());
            assert!(!pending_signal.snapshot());
        }
    }

    #[test]
    fn action_async_resets_pending_when_future_is_dropped_or_cancelled() {
        use core::future::Future;
        use executor_core::LocalExecutor;
        use executor_core::async_task::{self, AsyncTask, Runnable};

        extern crate std;
        use std::collections::VecDeque;

        // Dedicated thread with a local executor whose queue we can explicitly
        // drop/cancel mid-flight to prove the RAII guard resets `pending` even
        // when the spawned future never runs to completion.
        let handle = std::thread::spawn(|| {
            std::thread_local! {
                static QUEUE: RefCell<VecDeque<Runnable>> = const { RefCell::new(VecDeque::new()) };
            }

            #[derive(Debug, Clone, Copy)]
            struct CancellableExecutor;

            impl LocalExecutor for CancellableExecutor {
                type Task<T: 'static> = AsyncTask<T>;

                fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
                where
                    Fut: Future + 'static,
                {
                    let (runnable, task) = async_task::spawn_local(fut, |runnable: Runnable| {
                        QUEUE.with(|q| q.borrow_mut().push_back(runnable));
                    });
                    runnable.schedule();
                    task
                }
            }

            executor_core::try_init_local_executor(CancellableExecutor)
                .expect("fresh thread has no local executor");

            let env = Environment::new();
            let saving = binding(false);
            let gate = ManualGate::new();
            let gate_for_action = Rc::clone(&gate);

            let mut cfg = button("Save")
                .action_async(move || {
                    let g = Rc::clone(&gate_for_action);
                    async move {
                        g.wait().await;
                    }
                })
                .pending(&saving)
                .config();
            let pending_signal = cfg.pending().expect("pending signal present").clone();

            // Case 1: drop the spawned runnable before it is ever polled.
            (cfg.action_mut())(&env).expect("press succeeds");
            assert!(saving.snapshot());
            assert!(pending_signal.snapshot());
            QUEUE.with(|q| q.borrow_mut().clear());
            assert!(
                !saving.snapshot() && !pending_signal.snapshot(),
                "dropping an unpolled future must reset pending"
            );

            // Case 2: poll until suspended on `gate.wait()`, wake it, and then
            // drop the scheduled runnable before its second poll (simulating
            // executor shutdown/cancellation mid-flight).
            (cfg.action_mut())(&env).expect("second press after drop succeeds");
            assert!(saving.snapshot());
            assert!(pending_signal.snapshot());
            let runnable = QUEUE
                .with(|q| q.borrow_mut().pop_front())
                .expect("runnable queued");
            runnable.run();
            assert!(saving.snapshot(), "still pending while suspended at await");
            if let Some(waker) = gate.waker.borrow_mut().take() {
                waker.wake();
            }
            QUEUE.with(|q| q.borrow_mut().clear());
            assert!(
                !saving.snapshot() && !pending_signal.snapshot(),
                "dropping a suspended future must reset pending"
            );

            // Case 3: a future that panics while running still resets pending
            // via `PendingGuard::drop` during unwind.
            let mut panic_cfg = button("Explode")
                .action_async(|| async {
                    panic!("simulated async action panic");
                })
                .pending(&saving)
                .config();
            let panic_pending = panic_cfg.pending().expect("pending signal present").clone();
            (panic_cfg.action_mut())(&env).expect("press succeeds");
            assert!(saving.snapshot());
            assert!(panic_pending.snapshot());
            let runnable = QUEUE
                .with(|q| q.borrow_mut().pop_front())
                .expect("panic runnable queued");
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                runnable.run();
            }));
            assert!(panicked.is_err(), "test future must panic");
            assert!(
                !saving.snapshot() && !panic_pending.snapshot(),
                "panicking future must reset pending via drop guard"
            );
        });
        handle.join().expect("cancellation test thread panicked");
    }

    #[test]
    fn action_async_failed_extraction_does_not_mark_pending() {
        crate::init_test_executor();
        let env = Environment::new();
        let saving = binding(false);
        let mut cfg = button("Save")
            .action_async(
                |waterui_core::extract::State(_missing): waterui_core::extract::State<
                    nami::Binding<i32>,
                >| async {},
            )
            .pending(&saving)
            .config();
        let pending_signal = cfg.pending().expect("pending signal present").clone();

        let err = (cfg.action_mut())(&env);
        assert!(err.is_err(), "missing state must fail extraction");
        assert!(!saving.snapshot());
        assert!(!pending_signal.snapshot());
    }

    #[test]
    fn custom_style_reflects_async_pending_in_interaction_and_a11y() {
        use waterui_accessibility::AccessibilityStateSignal;

        crate::init_test_executor();
        let env = Environment::new();

        let recorded = Rc::new(RefCell::new(None));
        let gate = ManualGate::new();
        let gate_for_custom = Rc::clone(&gate);
        let saving = binding(false);
        let calls = Rc::new(Cell::new(0u32));
        let calls_for_custom = Rc::clone(&calls);

        let view = button("Save")
            .action_async(move || {
                calls_for_custom.set(calls_for_custom.get() + 1);
                let g = Rc::clone(&gate_for_custom);
                async move {
                    g.wait().await;
                }
            })
            .pending(&saving)
            .style(RecordingCustomStyle {
                recorded: Rc::clone(&recorded),
            });

        let body = View::body(view, &env);
        let recorded_cfg = recorded.borrow_mut().take().expect("custom style ran");
        let env_view = peel_metadata::<Environment>(AnyView::new(body), &env)
            .expect("InteractionReport installed");
        let inner_env = env_view.value;
        let role_view =
            peel_metadata::<AccessibilityRole>(env_view.content, &inner_env).expect("role");
        let a11y_label =
            peel_metadata::<AccessibilityLabel>(role_view.content, &env).expect("label");
        let a11y_state = peel_metadata::<AccessibilityStateSignal>(a11y_label.content, &env)
            .expect("async custom button publishes AccessibilityStateSignal");
        let observer =
            peel_metadata::<GestureObserver>(a11y_state.content, &env).expect("tap observer");
        let (_, mut tap_action) = observer.value.into_parts();

        assert!(!recorded_cfg.disabled.snapshot());
        assert!(!a11y_state.value.state().snapshot().is_busy());

        tap_action(&inner_env).expect("tap succeeds");
        assert_eq!(calls.get(), 1);
        assert!(saving.snapshot());
        assert!(recorded_cfg.disabled.snapshot());
        assert!(
            recorded_cfg
                .interaction
                .snapshot()
                .contains(InteractionState::DISABLED)
        );
        assert!(a11y_state.value.state().snapshot().is_busy());
        assert!(a11y_state.value.state().snapshot().is_disabled());

        // Second tap while pending is blocked.
        tap_action(&inner_env).expect("blocked tap succeeds");
        assert_eq!(calls.get(), 1);

        let _ = waterui_testing::drain_parked_local_work();
        gate.complete();
        let _ = waterui_testing::drain_parked_local_work();

        assert!(!saving.snapshot());
        assert!(!recorded_cfg.disabled.snapshot());
        assert!(!a11y_state.value.state().snapshot().is_busy());
        assert!(!a11y_state.value.state().snapshot().is_disabled());
    }

    #[test]
    fn primitive_style_reflects_async_pending_in_interaction_and_a11y() {
        use waterui_accessibility::AccessibilityStateSignal;

        crate::init_test_executor();
        let env = Environment::new();
        let saving = binding(false);
        let prim_recorded = Rc::new(RefCell::new(None));
        let trigger_slot = Rc::new(RefCell::new(None));
        let prim_gate = ManualGate::new();
        let prim_gate_for_action = Rc::clone(&prim_gate);
        let prim_calls = Rc::new(Cell::new(0u32));
        let prim_calls_for_action = Rc::clone(&prim_calls);

        let prim_view = button("Send")
            .pending(&saving)
            .action_async(move || {
                prim_calls_for_action.set(prim_calls_for_action.get() + 1);
                let g = Rc::clone(&prim_gate_for_action);
                async move {
                    g.wait().await;
                }
            })
            .style(RecordingPrimitiveStyle {
                recorded: Rc::clone(&prim_recorded),
                trigger_slot: Rc::clone(&trigger_slot),
            });

        let prim_body = View::body(prim_view, &env);
        let prim_cfg = prim_recorded
            .borrow_mut()
            .take()
            .expect("primitive style ran");
        let prim_trigger = trigger_slot
            .borrow_mut()
            .take()
            .expect("primitive style received trigger");
        let prim_env_view = peel_metadata::<Environment>(AnyView::new(prim_body), &env)
            .expect("InteractionReport installed");
        let prim_role =
            peel_metadata::<AccessibilityRole>(prim_env_view.content, &prim_env_view.value)
                .expect("role");
        let prim_label =
            peel_metadata::<AccessibilityLabel>(prim_role.content, &env).expect("label");
        let prim_state = peel_metadata::<AccessibilityStateSignal>(prim_label.content, &env)
            .expect("async primitive button publishes AccessibilityStateSignal");

        prim_trigger.trigger();
        assert_eq!(prim_calls.get(), 1);
        assert!(prim_cfg.disabled.snapshot());
        assert!(prim_state.value.state().snapshot().is_busy());
        prim_trigger.trigger();
        assert_eq!(prim_calls.get(), 1);

        let _ = waterui_testing::drain_parked_local_work();
        prim_gate.complete();
        let _ = waterui_testing::drain_parked_local_work();
        assert!(!prim_cfg.disabled.snapshot());
        assert!(!prim_state.value.state().snapshot().is_busy());
    }

    #[cfg(feature = "menu")]
    #[test]
    fn menu_command_conversion_disables_while_async_button_is_pending() {
        crate::init_test_executor();
        let saving = binding(false);
        let gate = ManualGate::new();
        let gate_for_action = Rc::clone(&gate);
        let command: crate::menu::Command = button("Sync")
            .action_async(move || {
                let g = Rc::clone(&gate_for_action);
                async move {
                    g.wait().await;
                }
            })
            .pending(&saving)
            .into();

        assert!(!command.is_disabled().snapshot());
        let env = Environment::new();
        command.action().run(&env);
        assert!(saving.snapshot());
        assert!(command.is_disabled().snapshot());

        let _ = waterui_testing::drain_parked_local_work();
        gate.complete();
        let _ = waterui_testing::drain_parked_local_work();
        assert!(!saving.snapshot());
        assert!(!command.is_disabled().snapshot());
    }

    #[test]
    fn keyboard_shortcut_preserved_across_config_from_and_style_hook() {
        use super::{
            Button, ButtonConfig, ButtonStyle, ButtonStyleHook, ButtonStylePending, Hook as _,
            HookSlot, HookableParent as _, Native,
        };
        use crate::shortcut::{Shortcut, ShortcutNamedKey};

        let shortcut = Shortcut::new(ShortcutNamedKey::Enter).command();
        let cfg = button("Send")
            .keyboard_shortcut(shortcut.clone())
            .action(|| {})
            .config();
        assert_eq!(cfg.shortcut(), Some(shortcut.clone()));

        let round_trip = Button::from(cfg).config();
        assert_eq!(round_trip.shortcut(), Some(shortcut.clone()));

        let env = Environment::new();
        let hook = ButtonStyleHook::new(ButtonStyle::Bordered);
        let (target, pending) = round_trip.into_style_pending(HookSlot::new);
        let restored = ButtonStylePending::restore(pending);
        assert_eq!(restored.shortcut(), Some(shortcut.clone()));

        let styled = hook.apply(target, &env).expect("hook applies");
        let styled_body = AnyView::new(View::body(styled, &env));
        let native = styled_body
            .downcast_ref::<Native<ButtonConfig<AnyView>>>()
            .expect("built-in button style hook renders Native<ButtonConfig>");
        assert_eq!(native.as_inner().shortcut(), Some(shortcut));
    }

    #[cfg(feature = "menu")]
    #[test]
    fn menu_command_conversion_preserves_keyboard_shortcut() {
        use crate::shortcut::Shortcut;

        let shortcut = Shortcut::new('s').command().shift();
        let command: crate::menu::Command = button("Save As")
            .keyboard_shortcut(shortcut.clone())
            .action(|| {})
            .into();
        let resolved = command.resolve::<crate::NoopSlot>(&Environment::new());
        assert_eq!(resolved.shortcut(), Some(&shortcut));
    }
}
