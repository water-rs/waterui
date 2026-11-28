//! In-window modal sheet presentation (`Sheet`).
//!
//! A [`Sheet`] presents arbitrary content modally inside its host window while
//! its `is_presented` [`Binding<bool>`] is `true`:
//!
//! ```rust,no_run
//! use waterui::prelude::*;
//! use waterui::sheet::{Sheet, SheetDetent};
//!
//! fn view() -> impl View {
//!     let open = binding(false);
//!     vstack((
//!         button("Open").action({
//!             let open = open.clone();
//!             move || open.set(true)
//!         }),
//!         text("Workspace").sheet(
//!             Sheet::new(&open, || {
//!                 vstack((
//!                     text("Filter options"),
//!                     button("Done").action(|dismiss: Dismiss| dismiss.dismiss()),
//!                 ))
//!             })
//!             .detents([SheetDetent::Medium, SheetDetent::Large]),
//!         ),
//!     ))
//! }
//! ```
//!
//! While presented, the sheet is window-modal: the content beneath is inert to
//! pointer, keyboard, and assistive technology, focus moves into the sheet and
//! returns to the previously focused element on dismissal, and Escape, the
//! platform back action, scrim tap, or interactive swipe dismissal invokes the
//! optional [`Sheet::on_dismiss`] handler and writes `is_presented = false`
//! when [`Sheet::dismissible`] is `true`.
//!
//! Backends realize the same semantic object natively where the platform
//! supplies one: `UISheetPresentationController` (or full-screen modal
//! `UIViewController`) on `UIKit`, `NSWindow` sheets (`beginSheet:`) on
//! `AppKit`, and a self-drawn modal sheet layer on Hydrolysis (desktop,
//! Android, web, headless, semantic).

use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use crate::slot::SlotBuilder;
use nami::Binding;
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::handler::{Handler, SharedAction, ViewBuilder, shared_action};
use waterui_core::metadata::MetadataKey;
use waterui_core::plugin::Plugin;
use waterui_core::{AnyView, Dismiss, Environment, Metadata, View};
use waterui_text::text::{IntoText, Text};

/// Medium detent proportion of available window height (matches iOS `mediumDetent` / M3 50% sheet).
const MEDIUM_DETENT_FRACTION: f32 = 0.5;
/// Large detent proportion of available window height (matches iOS `largeDetent` / M3 expanded sheet).
const LARGE_DETENT_FRACTION: f32 = 0.9;

/// Translation key for the default accessible name of a sheet that did not
/// declare an explicit label.
pub const SHEET_DEFAULT_LABEL_KEY: &str = "waterui.sheet.label";

/// Height detent for a [`Sheet`].
///
/// Platforms that support interactive detents (`UIKit`'s
/// `UISheetPresentationController`) expose every declared detent so the user
/// can drag between them; platforms with fixed-height sheets size to the first
/// declared detent.
#[derive(Debug, Clone, Copy, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each sheet detent variant by variant; a new detent must fail their build instead of falling into a wildcard arm"
)]
pub enum SheetDetent {
    /// Approximately half the available window height.
    Medium,
    /// Nearly full available window height with sheet chrome.
    Large,
    /// A fraction of the available window height in `0.0..=1.0`.
    Fraction(f32),
    /// A fixed sheet height in logical points, clamped to the available height.
    Height(f32),
    /// Covers the entire window with no scrim gap or sheet margins.
    FullScreen,
}

impl SheetDetent {
    /// Resolves this detent to a logical height within `available_height`.
    #[must_use]
    pub fn resolve_height(self, available_height: f32) -> f32 {
        let available = available_height.max(0.0);
        match self {
            Self::Medium => available * MEDIUM_DETENT_FRACTION,
            Self::Large => available * LARGE_DETENT_FRACTION,
            Self::Fraction(fraction) => available * fraction.clamp(0.0, 1.0),
            Self::Height(height) => height.clamp(0.0, available),
            Self::FullScreen => available,
        }
    }
}

/// Marker plugin placed around user content inside a self-drawn sheet layer so
/// backends distinguish the sheet's own modal surface container from any view
/// inside the sheet's content subtree.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct SheetContentScope;

impl Plugin for SheetContentScope {}

/// An in-window modal sheet driven by a [`Binding<bool>`].
///
/// Attach it to a view with [`ViewExt::sheet`]. While `is_presented` reads
/// `true`, the sheet presents modally over its host window: everything beneath
/// is inert to pointer, keyboard, and assistive technology, focus enters the
/// sheet on present and returns to the previously focused element on dismiss.
///
/// Each presentation invokes the content builder freshly; while the sheet stays
/// open its view tree persists with fine-grained reactivity, and dismissal
/// drops the sheet subtree.
///
/// One sheet is presented per window at a time; a second one presented while
/// one is up waits in presentation order.
pub struct Sheet<S = AnyView> {
    is_presented: Binding<bool>,
    content: SlotBuilder<S>,
    detents: Vec<SheetDetent>,
    dismissible: bool,
    on_dismiss: Option<SharedAction>,
    label: Option<Text>,
}

impl<S> Clone for Sheet<S> {
    fn clone(&self) -> Self {
        Self {
            is_presented: self.is_presented.clone(),
            content: self.content.clone(),
            detents: self.detents.clone(),
            dismissible: self.dismissible,
            on_dismiss: self.on_dismiss.clone(),
            label: self.label.clone(),
        }
    }
}

impl<S: 'static> MetadataKey for Sheet<S> {}

impl<S> fmt::Debug for Sheet<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sheet")
            .field("detents", &self.detents)
            .field("dismissible", &self.dismissible)
            .field("has_on_dismiss", &self.on_dismiss.is_some())
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl<V: View> Sheet<V> {
    /// Creates a modal sheet bound to `is_presented` presenting `content`.
    ///
    /// Defaults to `[SheetDetent::Large]` and `dismissible = true`.
    #[must_use]
    pub fn new(is_presented: &Binding<bool>, content: impl ViewBuilder<Output = V>) -> Self {
        Self {
            is_presented: is_presented.clone(),
            content: SlotBuilder::new(move || content.build()),
            detents: vec![SheetDetent::Large],
            dismissible: true,
            on_dismiss: None,
            label: None,
        }
    }
}

impl<S> Sheet<S> {
    /// Sets the supported detents for this sheet.
    ///
    /// An empty iterator leaves the default `[SheetDetent::Large]`.
    #[must_use]
    pub fn detents(mut self, detents: impl IntoIterator<Item = SheetDetent>) -> Self {
        let collected: Vec<SheetDetent> = detents.into_iter().collect();
        if !collected.is_empty() {
            self.detents = collected;
        }
        self
    }

    /// Sets whether user-initiated dismissal gestures (Escape, platform back
    /// action, scrim tap, interactive swipe-down) dismiss the sheet.
    ///
    /// Defaults to `true`. Set to `false` for mandatory flows or blocking
    /// progress sheets that must only close programmatically.
    #[must_use]
    pub const fn dismissible(mut self, dismissible: bool) -> Self {
        self.dismissible = dismissible;
        self
    }

    /// Installs a handler invoked when the user dismisses the sheet through
    /// Escape, the platform back action, a scrim tap, or interactive swipe-down.
    ///
    /// Programmatically writing `is_presented.set(false)` or calling
    /// [`Dismiss::dismiss`] closes the sheet directly without running this
    /// handler.
    #[must_use]
    pub fn on_dismiss<P>(mut self, handler: impl Handler<P, ()> + 'static) -> Self {
        self.on_dismiss = Some(shared_action(handler));
        self
    }

    /// Sets the accessible label for the sheet's modal dialog container.
    #[must_use]
    pub fn label(mut self, label: impl IntoText) -> Self {
        self.label = Some(label.into_text());
        self
    }

    /// The presentation binding the sheet was created with.
    #[must_use]
    pub const fn is_presented(&self) -> &Binding<bool> {
        &self.is_presented
    }

    /// The declared detents (always non-empty).
    #[must_use]
    pub fn configured_detents(&self) -> &[SheetDetent] {
        &self.detents
    }

    /// The primary (initial) detent.
    #[must_use]
    pub fn primary_detent(&self) -> SheetDetent {
        self.detents.first().copied().unwrap_or(SheetDetent::Large)
    }

    /// Whether user-initiated cancel/dismiss gestures are enabled.
    #[must_use]
    pub const fn is_dismissible(&self) -> bool {
        self.dismissible
    }

    /// The optional accessible label declared for the sheet.
    #[must_use]
    pub const fn accessible_label(&self) -> Option<&Text> {
        self.label.as_ref()
    }

    /// Builds a fresh content slot for a presentation.
    #[doc(hidden)]
    #[must_use]
    pub fn build_content(&self) -> S {
        self.content.build()
    }

    /// Returns the [`Dismiss`] environment handle bound to this sheet.
    #[doc(hidden)]
    #[must_use]
    pub fn dismiss_handle(&self) -> Dismiss {
        let is_presented = self.is_presented.clone();
        Dismiss::new(move || is_presented.set(false))
    }

    /// Runs the user-initiated dismissal path (Escape, platform back action,
    /// scrim tap, interactive swipe-down): when [`Self::is_dismissible`] is
    /// `true`, runs [`Self::on_dismiss`] (if installed) and writes
    /// `is_presented = false`. When `dismissible` is `false`, does nothing.
    #[doc(hidden)]
    pub fn run_user_dismiss(&self, env: &Environment) {
        if !self.dismissible {
            return;
        }
        if let Some(handler) = &self.on_dismiss {
            handler.run(env);
        }
        self.is_presented.set(false);
    }
}

impl<S: 'static> Sheet<S> {
    /// Maps the sheet's built content slot through `f`.
    #[must_use]
    pub fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> Sheet<T> {
        Sheet {
            is_presented: self.is_presented,
            content: self.content.map_slot(f),
            detents: self.detents,
            dismissible: self.dismissible,
            on_dismiss: self.on_dismiss,
            label: self.label,
        }
    }
}

impl<S: 'static> MapSlots<S> for Sheet<S> {
    type With<T: 'static> = Sheet<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> Sheet<T> {
        Self::map_slots(self, f)
    }
}

/// View wrapper that attaches a [`Sheet`] with its modal content projected
/// into the backend's child slot without `AnyView` erasure.
#[doc(hidden)]
#[derive(Debug)]
pub struct SheetView<Content, V> {
    content: Content,
    sheet: Sheet<V>,
}

impl<Content, V> SheetView<Content, V> {
    /// Creates a wrapper attaching `sheet` to `content`.
    #[must_use]
    pub const fn new(content: Content, sheet: Sheet<V>) -> Self {
        Self { content, sheet }
    }
}

impl<Content: View, V: View> View for SheetView<Content, V> {
    fn body(self, _env: &Environment) -> impl View {
        Metadata::new(self.content, self.sheet.map_slots(AnyView::new))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.metadata(
            Metadata::new(
                B::Slot::from_view(self.content),
                self.sheet.map_slots(B::Slot::from_view),
            ),
            env,
            cx,
        )
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use core::cell::Cell;

    use nami::{Binding, Signal};
    use waterui_core::Environment;

    use super::{Sheet, SheetDetent};

    #[test]
    fn detents_resolve_expected_heights() {
        assert!((SheetDetent::Medium.resolve_height(800.0) - 400.0).abs() < f32::EPSILON);
        assert!((SheetDetent::Large.resolve_height(800.0) - 720.0).abs() < f32::EPSILON);
        assert!((SheetDetent::Fraction(0.25).resolve_height(800.0) - 200.0).abs() < f32::EPSILON);
        assert!((SheetDetent::Fraction(1.5).resolve_height(800.0) - 800.0).abs() < f32::EPSILON);
        assert!((SheetDetent::Height(320.0).resolve_height(800.0) - 320.0).abs() < f32::EPSILON);
        assert!((SheetDetent::Height(1200.0).resolve_height(800.0) - 800.0).abs() < f32::EPSILON);
        assert!((SheetDetent::FullScreen.resolve_height(800.0) - 800.0).abs() < f32::EPSILON);
    }

    #[test]
    fn user_dismiss_respects_dismissible_and_invokes_on_dismiss() {
        let presented = Binding::new(true);
        let calls = Rc::new(Cell::new(0_u32));
        let calls_ref = Rc::clone(&calls);

        let non_dismissible = Sheet::new(&presented, || ())
            .dismissible(false)
            .on_dismiss(move || calls_ref.set(calls_ref.get() + 1));
        non_dismissible.run_user_dismiss(&Environment::new());
        assert!(presented.snapshot());
        assert_eq!(calls.get(), 0);

        let calls_ref = Rc::clone(&calls);
        let dismissible =
            Sheet::new(&presented, || ()).on_dismiss(move || calls_ref.set(calls_ref.get() + 1));
        dismissible.run_user_dismiss(&Environment::new());
        assert!(!presented.snapshot());
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn dismiss_handle_closes_without_invoking_on_dismiss() {
        let presented = Binding::new(true);
        let calls = Rc::new(Cell::new(0_u32));
        let calls_ref = Rc::clone(&calls);
        let sheet =
            Sheet::new(&presented, || ()).on_dismiss(move || calls_ref.set(calls_ref.get() + 1));

        sheet.dismiss_handle().dismiss();
        assert!(!presented.snapshot());
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn sheet_view_dispatches_typed_slot_without_anyview() {
        use core::any::TypeId;

        use waterui_core::backend::{Backend, ChildSlot, cast};
        use waterui_core::metadata::MetadataKey;
        use waterui_core::view::hook::{HookList, HookableView};
        use waterui_core::{AnyView, Metadata, NativeView, View};

        use super::SheetView;

        struct ProbeSheetContent;

        impl View for ProbeSheetContent {
            fn body(self, _env: &Environment) -> impl View {}
        }

        struct RecordedSlot(TypeId);

        impl ChildSlot for RecordedSlot {
            fn from_view<V: View>(_view: V) -> Self {
                Self(TypeId::of::<V>())
            }
        }

        struct CapturingBackend {
            seen: Option<TypeId>,
        }

        impl Backend for CapturingBackend {
            type Output = ();
            type Ctx = ();
            type Slot = RecordedSlot;

            fn native<T: NativeView + 'static>(&mut self, _native: T, _env: &Environment, (): ()) {}

            fn metadata<K: MetadataKey>(
                &mut self,
                metadata: Metadata<K, Self::Slot>,
                _env: &Environment,
                (): (),
            ) {
                if let Ok(sheet) = cast::<K, Sheet<RecordedSlot>>(metadata.value) {
                    self.seen = Some(sheet.build_content().0);
                }
            }

            fn erased(&mut self, _view: AnyView, _env: &Environment, (): ()) {
                panic!("SheetView must dispatch statically without AnyView");
            }

            fn scoped<H: HookList, V: View>(
                &mut self,
                _hooks: H,
                view: V,
                env: &Environment,
                (): (),
            ) {
                view.dispatch(self, env, ());
            }

            fn hookable<T: HookableView>(&mut self, view: T, env: &Environment, (): ()) {
                self.native(view, env, ());
            }
        }

        let presented = Binding::new(true);
        let view = SheetView::new((), Sheet::new(&presented, || ProbeSheetContent));
        let mut backend = CapturingBackend { seen: None };
        view.dispatch(&mut backend, &Environment::new(), ());
        assert_eq!(backend.seen, Some(TypeId::of::<ProbeSheetContent>()));
    }
}
