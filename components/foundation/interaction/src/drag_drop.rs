//! Native drag and drop contracts for `WaterUI`.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::{Any, TypeId, type_name};
use core::fmt;

use nami::{
    Binding, Computed, Signal,
    cache::Cached,
    constant::Constant,
    distinct::Distinct,
    impl_constant,
    map::{Map, map},
    scan::Scan,
    signal::WithMetadata,
};
use suiteki::Str;
use waterui_core::{
    Environment, Error, contract_violation,
    extract::Extractor,
    handler::{
        BoxedAction, BoxedEventAction, EventHandler, Handler, ReportExtraction, boxed_action,
        boxed_event_handler,
    },
    layout::Point,
    metadata::{Metadata, MetadataKey},
};
use waterui_url::Url;

/// Contextual details for a drag-enter or drop event, in the destination's
/// local coordinate space.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct DropInfo {
    /// Location in the destination view's local coordinate space (logical points).
    location: Point,
}

impl DropInfo {
    /// Creates drop info carrying `location` in the destination's local
    /// coordinate space.
    #[must_use]
    pub const fn new(location: Point) -> Self {
        Self { location }
    }

    /// Location of the drag or drop in the destination's local coordinate
    /// space (logical points).
    #[must_use]
    pub const fn location(&self) -> Point {
        self.location
    }
}

impl Extractor for DropInfo {
    fn extract(env: &Environment) -> Result<Self, Error> {
        env.get::<Self>().copied().ok_or_else(|| {
            Error::msg(
                "`DropInfo` is only available inside `drop_destination` and `on_enter` handlers",
            )
        })
    }
}

/// A value that a drag can carry.
///
/// [`Str`], [`Url`] and [`Files`] are transferable and map onto the platform
/// pasteboard. An application marks its own types transferable with
/// `#[derive(Transferable)]` (or an empty `impl Transferable`); such values
/// travel within the process only.
///
/// ```rust
/// use waterui_interaction::drag_drop::Transferable;
///
/// #[derive(Debug, Clone)]
/// struct TabId(u64);
/// impl Transferable for TabId {}
/// ```
pub trait Transferable: Clone + 'static {}

impl Transferable for Str {}
impl Transferable for Url {}
impl Transferable for Files {}

/// Converts a plain [`Transferable`] value or a reactive signal of one into
/// a [`DragPayload`] computation.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be dragged",
    label = "expected a `Transferable` value or a reactive signal producing one",
    note = "mark your type with `#[derive(Clone, Transferable)]`, or pass a `Binding<T>` / `Computed<T>` / mapped signal where `T: Transferable`"
)]
pub trait IntoDragPayload {
    /// Converts `self` into the reactive [`DragPayload`] carried by [`Draggable`].
    fn into_drag_payload(self) -> Computed<DragPayload>;
}

fn signal_drag_payload<S>(signal: S) -> Computed<DragPayload>
where
    S: Signal + Clone + 'static,
    S::Output: Transferable,
{
    Computed::new(map(signal, DragPayload::new::<S::Output>))
}

impl<T: Transferable> IntoDragPayload for T {
    fn into_drag_payload(self) -> Computed<DragPayload> {
        Computed::constant(DragPayload::new(self))
    }
}

impl<T: Transferable> IntoDragPayload for Binding<T> {
    fn into_drag_payload(self) -> Computed<DragPayload> {
        signal_drag_payload(self)
    }
}

impl<T: Transferable> IntoDragPayload for Computed<T> {
    fn into_drag_payload(self) -> Computed<DragPayload> {
        signal_drag_payload(self)
    }
}

impl<C, F, Output> IntoDragPayload for Map<C, F, Output>
where
    C: Signal,
    F: 'static + Clone + Fn(C::Output) -> Output,
    Output: Transferable,
{
    fn into_drag_payload(self) -> Computed<DragPayload> {
        signal_drag_payload(self)
    }
}

impl<T: Transferable> IntoDragPayload for Constant<T> {
    fn into_drag_payload(self) -> Computed<DragPayload> {
        signal_drag_payload(self)
    }
}

impl<C> IntoDragPayload for Cached<C>
where
    C: Signal,
    C::Output: Transferable,
{
    fn into_drag_payload(self) -> Computed<DragPayload> {
        signal_drag_payload(self)
    }
}

impl<S> IntoDragPayload for Distinct<S>
where
    S: Signal,
    S::Output: Transferable + PartialEq,
{
    fn into_drag_payload(self) -> Computed<DragPayload> {
        signal_drag_payload(self)
    }
}

impl<T: Transferable> IntoDragPayload for Scan<T> {
    fn into_drag_payload(self) -> Computed<DragPayload> {
        signal_drag_payload(self)
    }
}

impl<C, M> IntoDragPayload for WithMetadata<C, M>
where
    C: Signal,
    C::Output: Transferable,
    M: Clone + 'static,
{
    fn into_drag_payload(self) -> Computed<DragPayload> {
        signal_drag_payload(self)
    }
}

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

impl_constant!(Files);

/// What kind of value a drag carries, or a drop destination accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each transfer kind variant by variant; a new transfer kind must fail their build instead of falling into a wildcard arm"
)]
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
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each pasteboard representation variant by variant; a new pasteboard representation must fail their build instead of falling into a wildcard arm"
)]
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
            unreachable!(
                "`DragPayload::new` derives the kind from the stored type, so a payload of kind {:?} holds a {}",
                self.kind,
                type_name::<T>()
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
    /// or a reactive signal of one (`Binding`, `Computed`, or mapped signal) to
    /// carry its value at the time the drag begins.
    #[must_use]
    pub fn new(payload: impl IntoDragPayload) -> Self {
        Self {
            payload: payload.into_drag_payload(),
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
/// use suiteki::Str;
/// use waterui_interaction::drag_drop::{DropDestination, TransferKind};
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
                on_drop(value, env)
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

    /// Delivers a dropped payload and its local [`DropInfo`] to the handler.
    ///
    /// # Contract
    ///
    /// A backend delivers only accepted drags. A payload this destination does
    /// not [accept](Self::accepts) breaks the contract: a debug build panics,
    /// and a release build logs the error and drops the payload.
    pub fn deliver(&mut self, payload: DragPayload, info: DropInfo, env: &Environment) {
        if !self.accepts(&payload) {
            contract_violation!(
                "drop destination accepting {:?} was delivered a {:?} payload, which is dropped",
                self.accepted_kind,
                payload.kind()
            );
            return;
        }
        let env = env.extending(info);
        (self.on_drop)(payload, &env).report_extraction();
    }

    /// Reports that an accepted drag entered the destination's bounds at
    /// `info`'s local coordinates.
    pub fn enter(&mut self, info: DropInfo, env: &Environment) {
        if let Some(on_enter) = &mut self.on_enter {
            let env = env.extending(info);
            on_enter(&env).report_extraction();
        }
    }

    /// Reports that an accepted drag left the destination's bounds without dropping.
    pub fn exit(&mut self, env: &Environment) {
        if let Some(on_exit) = &mut self.on_exit {
            on_exit(env).report_extraction();
        }
    }

    /// Adds a handler for when an accepted drag enters the view bounds.
    ///
    /// The handler accepts any [`Extractor`] parameters — including
    /// [`DropInfo`] for the local entry point and [`State`](waterui_core::State)
    /// for injected state. Chains with any existing `on_enter` handler,
    /// executing both.
    #[must_use]
    pub fn on_enter<H, Args>(mut self, handler: H) -> Self
    where
        H: Handler<Args, ()>,
        Args: 'static,
    {
        self.on_enter = Some(chain(self.on_enter.take(), handler));
        self
    }

    /// Adds a handler for when an accepted drag exits the view bounds.
    ///
    /// The handler accepts any [`Extractor`] parameters (such as
    /// [`State`](waterui_core::State)). Chains with any existing `on_exit`
    /// handler, executing both.
    #[must_use]
    pub fn on_exit<H, Args>(mut self, handler: H) -> Self
    where
        H: Handler<Args, ()>,
        Args: 'static,
    {
        self.on_exit = Some(chain(self.on_exit.take(), handler));
        self
    }
}

fn chain<H, Args>(previous: Option<BoxedAction<()>>, handler: H) -> BoxedAction<()>
where
    H: Handler<Args, ()>,
    Args: 'static,
{
    let mut previous = previous;
    let mut handler = boxed_action(handler);
    Box::new(move |env| {
        if let Some(previous) = &mut previous {
            previous(env)?;
        }
        handler(env)
    })
}

/// Extension trait for `Metadata<DropDestination>` to easily bind hover state.
pub trait DropDestinationExt {
    /// Binds the drag hover state to a `Binding<bool>`.
    ///
    /// The binding becomes `true` when an accepted drag enters the view and
    /// `false` when it exits.
    #[must_use]
    fn drop_hover(self, is_hovering: &Binding<bool>) -> Self;

    /// Adds a handler for when an accepted drag enters the view bounds.
    ///
    /// Accepts any [`Extractor`] parameters, including [`DropInfo`] and
    /// [`State`](waterui_core::State). Chains with any existing `on_enter`
    /// handler, executing both.
    #[must_use]
    fn on_enter<H, Args>(self, handler: H) -> Self
    where
        H: Handler<Args, ()>,
        Args: 'static;

    /// Adds a handler for when an accepted drag exits the view bounds.
    ///
    /// Accepts any [`Extractor`] parameters (such as
    /// [`State`](waterui_core::State)). Chains with any existing `on_exit`
    /// handler, executing both.
    #[must_use]
    fn on_exit<H, Args>(self, handler: H) -> Self
    where
        H: Handler<Args, ()>,
        Args: 'static;
}

impl<C> DropDestinationExt for Metadata<DropDestination, C> {
    fn drop_hover(self, is_hovering: &Binding<bool>) -> Self {
        let enter = is_hovering.clone();
        let exit = is_hovering.clone();
        self.on_enter(move || enter.set(true))
            .on_exit(move || exit.set(false))
    }

    fn on_enter<H, Args>(mut self, handler: H) -> Self
    where
        H: Handler<Args, ()>,
        Args: 'static,
    {
        self.value = self.value.on_enter(handler);
        self
    }

    fn on_exit<H, Args>(mut self, handler: H) -> Self
    where
        H: Handler<Args, ()>,
        Args: 'static,
    {
        self.value = self.value.on_exit(handler);
        self
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use core::cell::Cell;
    use core::iter::empty;

    use waterui_core::State;

    use super::*;

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "was delivered a"))]
    fn delivering_an_unaccepted_payload_drops_it() {
        let delivered = Rc::new(Cell::new(false));
        let mut destination = DropDestination::new({
            let delivered = Rc::clone(&delivered);
            move |_: Str| delivered.set(true)
        });
        destination.deliver(
            DragPayload::new(Files::new(empty())),
            DropInfo::new(Point::zero()),
            &Environment::new(),
        );
        assert!(!delivered.get());
    }

    #[test]
    fn drop_and_enter_deliver_drop_info_and_extractors() {
        let indicator = Binding::new(None::<Point>);
        let landed = Binding::new(None::<(Str, Point)>);
        let env = Environment::new()
            .extending(State(indicator.clone()))
            .extending(State(landed.clone()));

        let mut destination = DropDestination::new(
            |text: Str, info: DropInfo, State(landed): State<Binding<Option<(Str, Point)>>>| {
                landed.set(Some((text, info.location())));
            },
        )
        .on_enter(
            |info: DropInfo, State(indicator): State<Binding<Option<Point>>>| {
                indicator.set(Some(info.location()));
            },
        )
        .on_exit(|State(indicator): State<Binding<Option<Point>>>| {
            indicator.set(None);
        });

        let entry_point = Point::new(14.0, 22.0);
        let drop_point = Point::new(42.0, 64.0);
        destination.enter(DropInfo::new(entry_point), &env);
        assert_eq!(indicator.snapshot(), Some(entry_point));

        destination.deliver(
            DragPayload::new(Str::from("note")),
            DropInfo::new(drop_point),
            &env,
        );
        assert_eq!(landed.snapshot(), Some((Str::from("note"), drop_point)));

        destination.exit(&env);
        assert_eq!(indicator.snapshot(), None);
    }

    #[test]
    fn drop_info_fails_extraction_outside_drop_handlers() {
        assert!(DropInfo::extract(&Environment::new()).is_err());
    }

    #[test]
    fn draggable_accepts_plain_transferable_and_reactive_signals() {
        use nami::{SignalExt as _, binding};

        #[derive(Debug, Clone, PartialEq)]
        struct PlainTab(u64);
        impl Transferable for PlainTab {}

        // Plain Transferable value without impl_constant!.
        let plain = Draggable::new(PlainTab(1));
        assert_eq!(
            plain.payload().downcast_ref::<PlainTab>(),
            Some(&PlainTab(1))
        );

        // Binding<T>.
        let tab = binding(PlainTab(2));
        let from_binding = Draggable::new(tab.clone());
        assert_eq!(
            from_binding.payload().downcast_ref::<PlainTab>(),
            Some(&PlainTab(2))
        );
        tab.set(PlainTab(3));
        assert_eq!(
            from_binding.payload().downcast_ref::<PlainTab>(),
            Some(&PlainTab(3))
        );

        // Computed<T>.
        let computed = tab.map(|t| PlainTab(t.0 * 10)).computed();
        let from_computed = Draggable::new(computed);
        assert_eq!(
            from_computed.payload().downcast_ref::<PlainTab>(),
            Some(&PlainTab(30))
        );

        // Map<...>.
        let mapped = tab.map(|t| PlainTab(t.0 + 1));
        let from_mapped = Draggable::new(mapped);
        assert_eq!(
            from_mapped.payload().downcast_ref::<PlainTab>(),
            Some(&PlainTab(4))
        );
    }
}
