//! In-window modal sheet presentation (`Sheet`).
//!
//! A [`Sheet`] presents arbitrary content modally inside its host window while
//! its `is_presented` [`Binding<bool>`] is `true`:
//!
//! ```rust,no_run
//! use waterui::prelude::*;
//! use waterui::sheet::{Sheet, SheetDetent, SheetDismiss};
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
//!                     button("Done").action(|sheet: SheetDismiss| sheet.dismiss()),
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

use nami::Binding;
use waterui_accessibility::AccessibilityRole;
use waterui_core::handler::{AnyViewBuilder, Handler, SharedAction, ViewBuilder, shared_action};
use waterui_core::layout::{
    Layout, LayoutError, PlacementInput, Point, ProposalSize, Rect, Size, StretchAxis, SubView,
    SubviewPlacement,
};
use waterui_core::metadata::MetadataKey;
use waterui_core::plugin::Plugin;
use waterui_core::{AnyView, Environment, OneOf2, View};
use waterui_graphics::color::Color;
use waterui_layout::container::FixedContainer;
use waterui_layout::padding::EdgeInsets;
use waterui_layout::stack::{Alignment, HorizontalAlignment, vstack, zstack};
use waterui_text::text::{IntoText, Text};

use crate::shape::{CornerRadius, RoundedRectangle, ShapeExt};
use crate::theme::color::{MutedForeground, Scrim, Surface};
use crate::{Computed, SignalExt as _, Str, ViewExt};

/// M3 `sheet.bottom.docked.container.max-width`, in logical points.
const SHEET_MAX_WIDTH: f32 = 640.0;
/// M3 `sheet.bottom.docked.container.shape` extra-large corner radius, in logical points.
const SHEET_CORNER_RADIUS: f32 = 28.0;
/// M3 `sheet.bottom.docked.drag-handle.width`, in logical points.
const DRAG_HANDLE_WIDTH: f32 = 32.0;
/// M3 `sheet.bottom.docked.drag-handle.height`, in logical points.
const DRAG_HANDLE_HEIGHT: f32 = 4.0;
/// Vertical padding around the M3 bottom-sheet drag handle, in logical points.
const DRAG_HANDLE_PADDING: f32 = 12.0;
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

/// Environment handle installed inside a presented [`Sheet`] so handlers inside
/// the sheet can dismiss it programmatically.
///
/// ```rust,no_run
/// use waterui::prelude::*;
/// use waterui::sheet::SheetDismiss;
///
/// fn sheet_body() -> impl View {
///     button("Close").action(|sheet: SheetDismiss| sheet.dismiss())
/// }
/// ```
#[waterui_macros::state]
#[derive(Clone)]
pub struct SheetDismiss {
    is_presented: Binding<bool>,
}

impl fmt::Debug for SheetDismiss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SheetDismiss").finish_non_exhaustive()
    }
}

impl SheetDismiss {
    /// Dismisses the sheet by writing `false` to its presentation binding.
    pub fn dismiss(&self) {
        self.is_presented.set(false);
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
#[derive(Clone)]
pub struct Sheet {
    is_presented: Binding<bool>,
    content: AnyViewBuilder<AnyView>,
    detents: Vec<SheetDetent>,
    dismissible: bool,
    on_dismiss: Option<SharedAction>,
    label: Option<Text>,
}

impl MetadataKey for Sheet {}

impl fmt::Debug for Sheet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sheet")
            .field("detents", &self.detents)
            .field("dismissible", &self.dismissible)
            .field("has_on_dismiss", &self.on_dismiss.is_some())
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl Sheet {
    /// Creates a modal sheet bound to `is_presented` presenting `content`.
    ///
    /// Defaults to `[SheetDetent::Large]` and `dismissible = true`.
    #[must_use]
    pub fn new(is_presented: &Binding<bool>, content: impl ViewBuilder) -> Self {
        Self {
            is_presented: is_presented.clone(),
            content: AnyViewBuilder::new(content).erase(),
            detents: vec![SheetDetent::Large],
            dismissible: true,
            on_dismiss: None,
            label: None,
        }
    }

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
    /// [`SheetDismiss::dismiss`] closes the sheet directly without running this
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

    /// Builds a fresh content view for a presentation.
    #[doc(hidden)]
    pub fn build_content(&self) -> AnyView {
        self.content.build()
    }

    /// Returns the [`SheetDismiss`] environment handle bound to this sheet.
    #[doc(hidden)]
    #[must_use]
    pub fn dismiss_handle(&self) -> SheetDismiss {
        SheetDismiss {
            is_presented: self.is_presented.clone(),
        }
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

    /// The self-drawn realization a backend's modal layer presents: a
    /// full-window `Scrim` backdrop (tapping it invokes the user dismiss path)
    /// and the detent-sized modal sheet surface carrying `AccessibilityRole::Dialog`.
    #[doc(hidden)]
    pub fn modal_layer(&self) -> impl View + use<> {
        let detent = self.primary_detent();
        let scrim_sheet = self.clone();
        let scrim = Color::new(Scrim)
            .on_tap(move |env: Environment| scrim_sheet.run_user_dismiss(&env))
            .a11y_hidden(true);
        FixedContainer::new(SheetLayout { detent }, (scrim, SheetSurface(self.clone())))
    }
}

/// Layout placing child 0 (scrim) across the full window and child 1 (sheet
/// surface) at the active [`SheetDetent`].
#[derive(Debug, Clone, Copy)]
struct SheetLayout {
    detent: SheetDetent,
}

impl Layout for SheetLayout {
    fn stretch_axis(&self, _children: &[StretchAxis]) -> StretchAxis {
        StretchAxis::Both
    }

    fn size_that_fits(&self, proposal: ProposalSize, _children: &[&dyn SubView]) -> Size {
        Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        )
    }

    fn place(
        &self,
        bounds: Rect,
        _proposal: ProposalSize,
        children: &[&dyn SubView],
    ) -> Vec<SubviewPlacement> {
        let mut placements = Vec::with_capacity(children.len());
        if !children.is_empty() {
            let full_proposal = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));
            placements.push(SubviewPlacement::new(bounds, full_proposal));
        }
        if children.len() > 1 {
            let sheet_rect = match self.detent {
                SheetDetent::FullScreen => bounds,
                detent => {
                    let width = bounds.width().min(SHEET_MAX_WIDTH);
                    let height = detent.resolve_height(bounds.height()).min(bounds.height());
                    let x = (bounds.width() - width).mul_add(0.5, bounds.x());
                    let y = bounds.y() + (bounds.height() - height);
                    Rect::new(Point::new(x, y), Size::new(width, height))
                }
            };
            let sheet_proposal =
                ProposalSize::new(Some(sheet_rect.width()), Some(sheet_rect.height()));
            placements.push(SubviewPlacement::new(sheet_rect, sheet_proposal));
        }
        placements
    }

    fn resolve_extent(
        &self,
        input: PlacementInput,
        _children: &[&dyn SubView],
    ) -> Result<Rect, LayoutError> {
        Ok(input.bounds)
    }
}

/// The sheet surface container drawn on Hydrolysis's modal layer: carries
/// `AccessibilityRole::Dialog`, resolves the accessible label, and wraps the
/// user content with [`SheetContentScope`].
struct SheetSurface(Sheet);

impl View for SheetSurface {
    fn body(self, env: &Environment) -> impl View {
        let sheet = self.0;
        let spoken: Computed<Str> = sheet
            .label
            .as_ref()
            .map_or_else(
                || Text::localized_or(SHEET_DEFAULT_LABEL_KEY, "Sheet").resolve(env),
                |label| label.resolve(env),
            )
            .content()
            .map(|styled| styled.to_plain())
            .computed();
        let content = sheet.build_content().with(SheetContentScope);
        let full_screen = sheet.primary_detent() == SheetDetent::FullScreen;
        if full_screen {
            let surface = Color::new(Surface).on_tap(|| {}).a11y_hidden(true);
            OneOf2::A(
                zstack((surface, content))
                    .a11y_label(spoken)
                    .a11y_role(AccessibilityRole::Dialog),
            )
        } else {
            let body = if sheet.dismissible {
                let handle = RoundedRectangle::new(CornerRadius::fraction(1.0))
                    .fill(Color::new(MutedForeground).with_opacity(0.4))
                    .size(DRAG_HANDLE_WIDTH, DRAG_HANDLE_HEIGHT)
                    .padding_with(EdgeInsets::new(
                        DRAG_HANDLE_PADDING,
                        0.0,
                        DRAG_HANDLE_PADDING,
                        0.0,
                    ))
                    .a11y_hidden(true);
                OneOf2::A(vstack((handle, content)).alignment(HorizontalAlignment::Center))
            } else {
                OneOf2::B(content)
            };
            let surface = RoundedRectangle::new(CornerRadius::points(SHEET_CORNER_RADIUS))
                .fill(Color::new(Surface))
                .on_tap(|| {})
                .a11y_hidden(true);
            OneOf2::B(
                zstack((surface, body))
                    .alignment(Alignment::Top)
                    .a11y_label(spoken)
                    .a11y_role(AccessibilityRole::Dialog),
            )
        }
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
        let presented = Binding::bool(true);
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
        let presented = Binding::bool(true);
        let calls = Rc::new(Cell::new(0_u32));
        let calls_ref = Rc::clone(&calls);
        let sheet =
            Sheet::new(&presented, || ()).on_dismiss(move || calls_ref.set(calls_ref.get() + 1));

        sheet.dismiss_handle().dismiss();
        assert!(!presented.snapshot());
        assert_eq!(calls.get(), 0);
    }
}
