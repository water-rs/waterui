//! Native drag and drop support for `WaterUI`.
//!
//! A drag carries one typed value. [`ViewExt::draggable`](crate::ViewExt::draggable)
//! takes any [`Transferable`] value, and
//! [`ViewExt::drop_destination`](crate::ViewExt::drop_destination) accepts exactly
//! the drags whose value has the type of its handler's first argument. A
//! destination is neither highlighted nor called for a drag of another type.
//!
//! The framework's own transferable types map onto the platform pasteboard, so
//! they travel between applications:
//!
//! - [`Str`]: plain text (`UTType.plainText`, MIME `text/plain`)
//! - [`Url`]: a URL (`UTType.url`, MIME `text/uri-list`)
//! - [`Files`]: a list of files (`UTType.fileURL`, Android `ClipData` URIs)
//!
//! Any other type an application marks [`Transferable`] travels within the
//! process only: it never reaches the pasteboard, and drags from other
//! applications never produce it.
//!
//! The drag and drop system integrates with native platform APIs:
//!
//! - **macOS**: `NSDraggingSource` / `NSDraggingDestination`
//! - **iOS**: `UIDragInteraction` / `UIDropInteraction`
//! - **Android**: `View.startDragAndDrop()` / `OnDragListener`
//!
//! # Example
//!
//! ```rust
//! use waterui::drag_drop::Transferable;
//! use waterui::prelude::*;
//! use waterui::Str;
//! use waterui::reactive::impl_constant;
//!
//! // Text travels to other applications as plain text.
//! let source = text!("Drag me!").draggable(Str::from("Hello, World!"));
//! let target = text!("Drop text here").drop_destination(|text: Str| {
//!     let _received = text;
//! });
//!
//! // An application type travels within the process only.
//! #[derive(Debug, Clone, PartialEq)]
//! struct TabId(u64);
//! impl Transferable for TabId {}
//! impl_constant!(TabId);
//!
//! let tab = text!("Tab 1").draggable(TabId(1));
//! let tab_bar = text!("Tabs").drop_destination(|tab: TabId| {
//!     let _moved = tab;
//! });
//! ```

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::{Any, TypeId};
use core::fmt;
use nami::Computed;
use nami::{Signal, SignalExt};
use suiteki::Str;
use waterui_core::{
    Environment,
    handler::{BoxedAction, BoxedEventAction, EventHandler, boxed_action, boxed_event_handler},
    metadata::{Metadata, MetadataKey},
};
use waterui_url::Url;

use crate::reactive::Binding;

/// A value that a drag can carry.
///
/// [`Str`], [`Url`] and [`Files`] are transferable and map onto the platform
/// pasteboard. An application marks its own types transferable with an empty
/// implementation; such values travel within the process only.
///
/// ```rust
/// use waterui::drag_drop::Transferable;
///
/// #[derive(Debug, Clone)]
/// struct TabId(u64);
/// impl Transferable for TabId {}
/// ```
pub trait Transferable: Clone + 'static {}

impl Transferable for Str {}
impl Transferable for Url {}
impl Transferable for Files {}

/// The files an OS file drag carries, as file URLs.
///
/// On Android the URLs are `content://` URIs granted to the receiving
/// activity; everywhere else they are `file://` URLs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Files(Vec<Url>);

impl Files {
    /// Creates a file list from its URLs.
    #[must_use]
    pub fn new(urls: impl IntoIterator<Item = Url>) -> Self {
        Self(urls.into_iter().collect())
    }

    /// The URLs of the files.
    #[must_use]
    pub fn urls(&self) -> &[Url] {
        &self.0
    }

    /// Consumes the list, returning the URLs of the files.
    #[must_use]
    pub fn into_urls(self) -> Vec<Url> {
        self.0
    }
}

nami::impl_constant!(Files);

/// What kind of value a drag carries, or a drop destination accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransferKind {
    /// Plain text: [`Str`].
    Text,
    /// A URL: [`Url`].
    Url,
    /// A list of files: [`Files`].
    Files,
    /// An application type, identified by its [`TypeId`]. It travels within
    /// the process only.
    InProcess(TypeId),
}

impl TransferKind {
    /// The kind of the transferable type `T`.
    #[must_use]
    pub fn of<T: Transferable>() -> Self {
        let id = TypeId::of::<T>();
        if id == TypeId::of::<Str>() {
            Self::Text
        } else if id == TypeId::of::<Url>() {
            Self::Url
        } else if id == TypeId::of::<Files>() {
            Self::Files
        } else {
            Self::InProcess(id)
        }
    }

    /// Returns `true` if values of this kind can reach the platform pasteboard.
    #[must_use]
    pub const fn is_platform(self) -> bool {
        !matches!(self, Self::InProcess(_))
    }
}

/// The value one drag carries, with its concrete type erased.
///
/// Backends read a drag source's payload when the drag begins, build one from
/// a platform drag with [`DragPayload::new`] over [`Str`], [`Url`] or
/// [`Files`], and hand it to [`DropDestination::deliver`].
#[derive(Clone)]
pub struct DragPayload {
    kind: TransferKind,
    value: Rc<dyn Any>,
}

impl fmt::Debug for DragPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DragPayload")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// How a [`DragPayload`] is written to the platform pasteboard.
#[derive(Debug, Clone, Copy)]
pub enum PlatformRepresentation<'a> {
    /// Plain text.
    Text(&'a Str),
    /// A URL.
    Url(&'a Url),
    /// A list of files.
    Files(&'a Files),
    /// An application value; it stays in the process and has no pasteboard form.
    InProcess,
}

impl DragPayload {
    /// Wraps a transferable value.
    #[must_use]
    pub fn new<T: Transferable>(value: T) -> Self {
        Self {
            kind: TransferKind::of::<T>(),
            value: Rc::new(value),
        }
    }

    /// The kind of value this payload carries.
    #[must_use]
    pub const fn kind(&self) -> TransferKind {
        self.kind
    }

    /// The carried value, if it has type `T`.
    #[must_use]
    pub fn downcast_ref<T: Transferable>(&self) -> Option<&T> {
        self.value.downcast_ref()
    }

    /// The carried value as the platform pasteboard represents it.
    #[must_use]
    pub fn platform_representation(&self) -> PlatformRepresentation<'_> {
        match self.kind {
            TransferKind::Text => PlatformRepresentation::Text(self.expect_value()),
            TransferKind::Url => PlatformRepresentation::Url(self.expect_value()),
            TransferKind::Files => PlatformRepresentation::Files(self.expect_value()),
            TransferKind::InProcess(_) => PlatformRepresentation::InProcess,
        }
    }

    fn expect_value<T: Transferable>(&self) -> &T {
        self.downcast_ref().unwrap_or_else(|| {
            panic!(
                "drag payload of kind {:?} does not hold a {}",
                self.kind,
                core::any::type_name::<T>()
            )
        })
    }
}

/// Metadata that makes a view draggable.
///
/// When attached to a view, the view becomes a drag source. Users can initiate
/// a drag operation by:
/// - **macOS**: Click and drag
/// - **iOS/Android**: Long-press and drag
///
/// The payload is read when the drag begins.
pub struct Draggable {
    payload: Computed<DragPayload>,
}

impl fmt::Debug for Draggable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Draggable").finish_non_exhaustive()
    }
}

impl MetadataKey for Draggable {}

impl Draggable {
    /// Creates draggable metadata carrying `payload`: a plain transferable value,
    /// or any signal of one (a `Binding` or `Computed`) to carry its value at the
    /// time the drag begins.
    #[must_use]
    pub fn new<S>(payload: S) -> Self
    where
        S: Signal + Clone + 'static,
        S::Output: Transferable,
    {
        Self {
            payload: Computed::new(payload.map(DragPayload::new::<S::Output>)),
        }
    }

    /// The payload a drag starting now carries.
    #[must_use]
    pub fn payload(&self) -> DragPayload {
        self.payload.snapshot()
    }
}

/// Metadata that makes a view a drop destination.
///
/// The destination accepts the drags whose payload has the type of its drop
/// handler's first argument; backends highlight it and deliver to it only for
/// those drags.
///
/// # Example
///
/// ```rust
/// use waterui::drag_drop::{DropDestination, TransferKind};
/// use waterui::Str;
///
/// let destination = DropDestination::new(|text: Str| {
///     let _dropped = text;
/// });
/// assert_eq!(destination.accepted_kind(), TransferKind::Text);
/// ```
pub struct DropDestination {
    accepted_kind: TransferKind,
    on_drop: BoxedEventAction<DragPayload>,
    on_enter: Option<BoxedAction<()>>,
    on_exit: Option<BoxedAction<()>>,
}

impl fmt::Debug for DropDestination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DropDestination")
            .field("accepted_kind", &self.accepted_kind)
            .finish_non_exhaustive()
    }
}

impl MetadataKey for DropDestination {}

impl DropDestination {
    /// Creates a drop destination whose `on_drop` handler receives the dropped
    /// value as its first argument; the remaining arguments are extractors.
    pub fn new<T: Transferable, Args>(on_drop: impl EventHandler<T, Args>) -> Self {
        let mut on_drop = boxed_event_handler(on_drop);
        Self {
            accepted_kind: TransferKind::of::<T>(),
            on_drop: Box::new(move |payload: DragPayload, env: &Environment| {
                let value = payload.expect_value::<T>().clone();
                on_drop(value, env);
            }),
            on_enter: None,
            on_exit: None,
        }
    }

    /// The kind of payload this destination accepts.
    #[must_use]
    pub const fn accepted_kind(&self) -> TransferKind {
        self.accepted_kind
    }

    /// Returns `true` if this destination accepts `payload`.
    #[must_use]
    pub fn accepts(&self, payload: &DragPayload) -> bool {
        payload.kind() == self.accepted_kind
    }

    /// Delivers a dropped payload to the handler.
    ///
    /// # Panics
    ///
    /// Panics if this destination does not [accept](Self::accepts) `payload`;
    /// a backend delivers only accepted drags.
    pub fn deliver(&mut self, payload: DragPayload, env: &Environment) {
        assert!(
            self.accepts(&payload),
            "drop destination accepting {:?} was delivered a {:?} payload",
            self.accepted_kind,
            payload.kind()
        );
        (self.on_drop)(payload, env);
    }

    /// Reports that an accepted drag entered the destination's bounds.
    pub fn enter(&mut self, env: &Environment) {
        if let Some(on_enter) = &mut self.on_enter {
            on_enter(env);
        }
    }

    /// Reports that an accepted drag left the destination's bounds without dropping.
    pub fn exit(&mut self, env: &Environment) {
        if let Some(on_exit) = &mut self.on_exit {
            on_exit(env);
        }
    }

    /// Adds a callback for when an accepted drag enters the view bounds.
    ///
    /// This chains with any existing `on_enter` handler, executing both.
    #[must_use]
    pub fn on_enter(mut self, handler: impl FnMut() + 'static) -> Self {
        self.on_enter = Some(chain(self.on_enter.take(), handler));
        self
    }

    /// Adds a callback for when an accepted drag exits the view bounds.
    ///
    /// This chains with any existing `on_exit` handler, executing both.
    #[must_use]
    pub fn on_exit(mut self, handler: impl FnMut() + 'static) -> Self {
        self.on_exit = Some(chain(self.on_exit.take(), handler));
        self
    }
}

fn chain(previous: Option<BoxedAction<()>>, handler: impl FnMut() + 'static) -> BoxedAction<()> {
    let mut previous = previous;
    let mut handler = boxed_action(handler);
    Box::new(move |env| {
        if let Some(previous) = &mut previous {
            previous(env);
        }
        handler(env);
    })
}

// ============================================================================
// Drop Destination Extension
// ============================================================================

/// Extension trait for `Metadata<DropDestination>` to easily bind hover state.
pub trait DropDestinationExt {
    /// Binds the drag hover state to a `Binding<bool>`.
    ///
    /// The binding becomes `true` when an accepted drag enters the view and
    /// `false` when it exits.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::drag_drop::DropDestinationExt;
    /// use waterui::prelude::*;
    /// use waterui::Str;
    ///
    /// let is_hovering = binding::<bool>(false);
    ///
    /// // `drop_hover` extends the metadata a `drop_destination` produces.
    /// let target = text!("Drop here")
    ///     .drop_destination(|text: Str| {
    ///         let _dropped = text;
    ///     })
    ///     .drop_hover(&is_hovering);
    /// ```
    #[must_use]
    fn drop_hover(self, is_hovering: &Binding<bool>) -> Self;

    /// Adds a callback for when an accepted drag enters the view bounds.
    ///
    /// This chains with any existing `on_enter` handler, executing both.
    #[must_use]
    fn on_enter(self, handler: impl FnMut() + 'static) -> Self;

    /// Adds a callback for when an accepted drag exits the view bounds.
    ///
    /// This chains with any existing `on_exit` handler, executing both.
    #[must_use]
    fn on_exit(self, handler: impl FnMut() + 'static) -> Self;
}

impl DropDestinationExt for Metadata<DropDestination> {
    fn drop_hover(self, is_hovering: &Binding<bool>) -> Self {
        let enter = is_hovering.clone();
        let exit = is_hovering.clone();
        self.on_enter(move || enter.set(true))
            .on_exit(move || exit.set(false))
    }

    fn on_enter(mut self, handler: impl FnMut() + 'static) -> Self {
        self.value = self.value.on_enter(handler);
        self
    }

    fn on_exit(mut self, handler: impl FnMut() + 'static) -> Self {
        self.value = self.value.on_exit(handler);
        self
    }
}
