//! Drag and drop: `UIDragInteraction` and `UIDropInteraction` with Rust
//! closures as their delegates.
//!
//! A drag carries typed data on the pasteboard through `NSItemProvider`s; a
//! value that must stay inside the process rides on the drag item's
//! `localObject` instead, as a [`LocalPayload`]. Drop sessions read payloads
//! back off their local drag session; drags from other applications never
//! carry one.
//!
//! The returned [`DragSource`]/[`DropTarget`] own the interaction and its
//! delegate. Keep them for as long as the view should drag or accept drops —
//! they live in the leaf's `KeepAlive` — and drop them to detach.
//!
//! # Safety
//!
//! The `unsafe` here defines the delegate and payload classes and drives the
//! interactions `UIKit` declares. The delegate methods have the signatures
//! `UIKit` sends, and every call is a main-thread call: the classes are
//! `MainThreadOnly` and the interactions attach to views.

use std::any::Any;
use std::fmt;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, ProtocolObject};
use objc2::{
    AnyThread, ClassType, DefinedClass, DowncastTarget, MainThreadMarker, MainThreadOnly,
    define_class, msg_send,
};
use objc2_foundation::{
    NSArray, NSData, NSError, NSItemProvider, NSItemProviderRepresentationVisibility,
    NSObjectProtocol, NSProgress, NSString, NSURL,
};
use objc2_ui_kit::{
    UIDragDropSession, UIDragInteraction, UIDragInteractionDelegate, UIDragItem, UIDragSession,
    UIDropInteraction, UIDropInteractionDelegate, UIDropOperation, UIDropProposal, UIDropSession,
    UIView,
};

use crate::callback::guarded;

/// The operation a drop session proposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DropOperation {
    /// The drag is not accepted here.
    Forbidden,
    /// The drag copies its value into the destination.
    Copy,
    /// The drag moves its value into the destination.
    Move,
    /// The drag is cancelled.
    Cancel,
}

impl DropOperation {
    const fn native(self) -> UIDropOperation {
        match self {
            Self::Forbidden => UIDropOperation::Forbidden,
            Self::Copy => UIDropOperation::Copy,
            Self::Move => UIDropOperation::Move,
            Self::Cancel => UIDropOperation::Cancel,
        }
    }
}

struct PayloadIvars {
    payload: Rc<dyn Any>,
}

define_class!(
    /// An `Rc<dyn Any>` travelling as a drag item's `localObject` — how a
    /// same-process drop destination receives a typed value unserialized.
    ///
    // SAFETY: `NSObject` has no subclassing requirements; the class holds a
    // payload and does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiLocalPayload"]
    #[thread_kind = MainThreadOnly]
    #[ivars = PayloadIvars]
    struct LocalPayload;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for LocalPayload {}
);

impl LocalPayload {
    fn new(mtm: MainThreadMarker, payload: Rc<dyn Any>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PayloadIvars { payload });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

impl fmt::Debug for LocalPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalPayload").finish_non_exhaustive()
    }
}

/// One item a drag begins with: its pasteboard provider, and the payload a
/// same-process destination may read off the item.
#[derive(Debug)]
pub struct DragItemSpec {
    /// The `NSItemProvider` carrying the item's platform representation.
    pub provider: Retained<NSItemProvider>,
    /// The value a same-process destination reads; `localObject` otherwise.
    pub local: Option<Rc<dyn Any>>,
}

/// An item provider carrying plain text.
#[must_use]
pub fn text_item_provider(text: &str) -> Retained<NSItemProvider> {
    let text = NSString::from_str(text);
    NSItemProvider::initWithObject(NSItemProvider::alloc(), ProtocolObject::from_ref(&*text))
}

/// An item provider carrying a URL.
///
/// # Panics
///
/// If `url` is not a valid URL — the payload's URL invariant makes this
/// unreachable, mirroring the Swift `fatalError`.
#[must_use]
pub fn url_item_provider(url: &str) -> Retained<NSItemProvider> {
    let url = NSURL::URLWithString(&NSString::from_str(url))
        .unwrap_or_else(|| panic!("invalid drag URL: {url:?}"));
    NSItemProvider::initWithObject(NSItemProvider::alloc(), ProtocolObject::from_ref(&*url))
}

/// An item provider whose only representation is `type_identifier` with
/// empty data.
///
/// The marker a process-scoped drag carries: it keeps the item well-formed
/// while the payload itself travels in `localObject` only.
#[must_use]
pub fn marker_item_provider(type_identifier: &str) -> Retained<NSItemProvider> {
    let provider = NSItemProvider::init(NSItemProvider::alloc());
    let handler = RcBlock::new(
        |completion: NonNull<
            block2::DynBlock<dyn Fn(*mut NSData, *mut NSError)>,
        >| -> *mut NSProgress {
            let data = NSData::new();
            // SAFETY: `completion` is the block UIKit passes; it is live for
            // this call and takes ownership of neither argument.
            unsafe {
                (*completion.as_ptr()).call((
                    Retained::as_ptr(&data).cast_mut(),
                    std::ptr::null_mut(),
                ));
            }
            std::ptr::null_mut()
        },
    );
    // SAFETY: the handler block is an `RcBlock`, which is sendable.
    unsafe {
        provider.registerDataRepresentationForTypeIdentifier_visibility_loadHandler(
            &NSString::from_str(type_identifier),
            NSItemProviderRepresentationVisibility::OwnProcess,
            &handler,
        );
    }
    provider
}

struct DragDelegateIvars {
    items: Rc<dyn Fn() -> Vec<DragItemSpec>>,
    mtm: MainThreadMarker,
}

define_class!(
    /// The `UIDragInteractionDelegate` producing the session's drag items.
    ///
    // SAFETY: `NSObject` has no subclassing requirements; the class holds a
    // closure and does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiDragDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DragDelegateIvars]
    struct DragDelegate;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for DragDelegate {}

    // SAFETY: `dragInteraction:itemsForBeginningSession:` has the signature
    // `UIDragInteractionDelegate` declares.
    unsafe impl UIDragInteractionDelegate for DragDelegate {
        #[unsafe(method_id(dragInteraction:itemsForBeginningSession:))]
        fn items_for_beginning(
            &self,
            _interaction: &UIDragInteraction,
            _session: &ProtocolObject<dyn UIDragSession>,
        ) -> Retained<NSArray<UIDragItem>> {
            guarded("CocoaUiDragDelegate itemsForBeginningSession:", || {
                let items = (self.ivars().items)()
                    .into_iter()
                    .map(|spec| {
                        let item = UIDragItem::initWithItemProvider(
                            UIDragItem::alloc(self.ivars().mtm),
                            &spec.provider,
                        );
                        if let Some(payload) = spec.local {
                            // SAFETY: attaching a main-thread object as the
                            // item's local object.
                            unsafe {
                                item.setLocalObject(Some(&*LocalPayload::new(
                                    self.ivars().mtm,
                                    payload,
                                )));
                            }
                        }
                        item
                    })
                    .collect::<Vec<_>>();
                NSArray::from_retained_slice(&items)
            })
        }
    }
);

impl fmt::Debug for DragDelegate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DragDelegate").finish_non_exhaustive()
    }
}

/// A `UIDragInteraction` on a view, owned with its delegate.
///
/// The `items` closure runs when a drag begins and answers the items the
/// drag carries. Dropping the source removes the interaction from its view.
#[must_use = "dropping the source removes the interaction"]
#[derive(Debug)]
pub struct DragSource {
    interaction: Retained<UIDragInteraction>,
    view: objc2::rc::Weak<UIView>,
    #[allow(dead_code)]
    delegate: Retained<DragDelegate>,
}

impl Drop for DragSource {
    fn drop(&mut self) {
        if let Some(view) = self.view.load() {
            view.removeInteraction(ProtocolObject::from_ref(&*self.interaction));
        }
    }
}

/// Installs a drag interaction on `view`; `items` answers the drag items
/// each session begins with.
pub fn drag_source(view: &UIView, items: impl Fn() -> Vec<DragItemSpec> + 'static) -> DragSource {
    let mtm = view.mtm();
    let delegate: Retained<DragDelegate> = {
        let this = DragDelegate::alloc(mtm).set_ivars(DragDelegateIvars {
            items: Rc::new(items),
            mtm,
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    };
    let interaction = UIDragInteraction::initWithDelegate(
        UIDragInteraction::alloc(mtm),
        ProtocolObject::from_ref(&*delegate),
    );
    interaction.setEnabled(true);
    view.addInteraction(ProtocolObject::from_ref(&*interaction));
    DragSource {
        interaction,
        view: objc2::rc::Weak::new(view),
        delegate,
    }
}

/// A drop session as the delegate sees it.
pub struct DropSession<'a> {
    session: &'a ProtocolObject<dyn UIDropSession>,
}

impl fmt::Debug for DropSession<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DropSession").finish_non_exhaustive()
    }
}

impl DropSession<'_> {
    /// The payloads of the session's accepted same-process drag items.
    /// Other applications' drags carry no [`LocalPayload`].
    #[must_use]
    pub fn local_payloads(&self) -> Vec<Rc<dyn Any>> {
        let Some(local) = self.session.localDragSession() else {
            return Vec::new();
        };
        local
            .items()
            .iter()
            .filter_map(|item| {
                item.localObject()
                    .and_then(|object| object.downcast::<LocalPayload>().ok())
            })
            .map(|payload| Rc::clone(&payload.ivars().payload))
            .collect()
    }

    /// Whether the session can produce an `NSString`.
    #[must_use]
    pub fn can_load_strings(&self) -> bool {
        // SAFETY: `NSString` implements `NSItemProviderReading`.
        unsafe { self.session.canLoadObjectsOfClass(NSString::class()) }
    }

    /// Whether the session can produce an `NSURL`.
    #[must_use]
    pub fn can_load_urls(&self) -> bool {
        // SAFETY: `NSURL` implements `NSItemProviderReading`.
        unsafe { self.session.canLoadObjectsOfClass(NSURL::class()) }
    }

    /// Whether the session carries a `public.file-url` item.
    #[must_use]
    pub fn has_file_urls(&self) -> bool {
        let file_url = NSString::from_str("public.file-url");
        let identifiers = NSArray::from_slice(&[&*file_url]);
        self.session
            .hasItemsConformingToTypeIdentifiers(&identifiers)
    }

    /// Loads every `NSURL` the session can produce, then calls `handler`.
    pub fn load_urls(&self, handler: impl Fn(Vec<String>) + 'static) {
        self.load_objects::<NSURL>(move |urls| {
            handler(
                urls.iter()
                    .filter_map(|url| url.absoluteString())
                    .map(|string| string.to_string())
                    .collect(),
            );
        });
    }

    /// Loads every `NSString` the session can produce, then calls `handler`.
    pub fn load_strings(&self, handler: impl Fn(Vec<Retained<NSString>>) + 'static) {
        self.load_objects::<NSString>(handler);
    }

    fn load_objects<T>(&self, handler: impl Fn(Vec<Retained<T>>) + 'static)
    where
        T: DowncastTarget + 'static,
    {
        let block = RcBlock::new(
            move |objects: NonNull<
                NSArray<ProtocolObject<dyn objc2_foundation::NSItemProviderReading>>,
            >| {
                // SAFETY: `objects` is the array UIKit passes; it is live for
                // this call.
                let objects = unsafe { objects.as_ref() };
                let loaded = objects
                    .iter()
                    .filter_map(|object| {
                        let object: &AnyObject = object.as_ref();
                        object.downcast_ref::<T>().map(objc2::Message::retain)
                    })
                    .collect();
                handler(loaded);
            },
        );
        // SAFETY: `T` must implement `NSItemProviderReading` — the two callers
        // pass only `NSString` and `NSURL`.
        unsafe {
            let _progress = self
                .session
                .loadObjectsOfClass_completion(T::class(), &block);
        }
    }
}

/// The closures a [`DropTarget`] calls during a drop session.
pub struct DropHandlers {
    /// Whether the session's drag can be dropped here at all.
    pub can_handle: Rc<dyn Fn(&DropSession<'_>) -> bool>,
    /// An accepted drag entered the view.
    pub entered: Rc<dyn Fn(&DropSession<'_>)>,
    /// The session updated; answers the proposed operation.
    pub update: Rc<dyn Fn(&DropSession<'_>) -> DropOperation>,
    /// An accepted drag left the view without dropping.
    pub exited: Rc<dyn Fn(&DropSession<'_>)>,
    /// The user dropped; deliver.
    pub perform: Rc<dyn Fn(&DropSession<'_>)>,
}

impl fmt::Debug for DropHandlers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DropHandlers").finish_non_exhaustive()
    }
}

define_class!(
    /// The `UIDropInteractionDelegate` reporting a session to Rust closures.
    ///
    // SAFETY: `NSObject` has no subclassing requirements; the class holds
    // closures and does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiDropDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DropHandlers]
    struct DropDelegate;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for DropDelegate {}

    // SAFETY: each method has the signature `UIDropInteractionDelegate`
    // declares.
    unsafe impl UIDropInteractionDelegate for DropDelegate {
        #[unsafe(method(dropInteraction:canHandleSession:))]
        fn can_handle(
            &self,
            _interaction: &UIDropInteraction,
            session: &ProtocolObject<dyn UIDropSession>,
        ) -> bool {
            guarded("CocoaUiDropDelegate canHandleSession:", || {
                (self.ivars().can_handle)(&DropSession { session })
            })
        }

        #[unsafe(method(dropInteraction:sessionDidEnter:))]
        fn session_did_enter(
            &self,
            _interaction: &UIDropInteraction,
            session: &ProtocolObject<dyn UIDropSession>,
        ) {
            guarded("CocoaUiDropDelegate sessionDidEnter:", || {
                (self.ivars().entered)(&DropSession { session });
            });
        }

        #[unsafe(method_id(dropInteraction:sessionDidUpdate:))]
        fn session_did_update(
            &self,
            _interaction: &UIDropInteraction,
            session: &ProtocolObject<dyn UIDropSession>,
        ) -> Retained<UIDropProposal> {
            guarded("CocoaUiDropDelegate sessionDidUpdate:", || {
                let operation = (self.ivars().update)(&DropSession { session });
                UIDropProposal::initWithDropOperation(
                    UIDropProposal::alloc(self.mtm()),
                    operation.native(),
                )
            })
        }

        #[unsafe(method(dropInteraction:sessionDidExit:))]
        fn session_did_exit(
            &self,
            _interaction: &UIDropInteraction,
            session: &ProtocolObject<dyn UIDropSession>,
        ) {
            guarded("CocoaUiDropDelegate sessionDidExit:", || {
                (self.ivars().exited)(&DropSession { session });
            });
        }

        #[unsafe(method(dropInteraction:performDrop:))]
        fn perform_drop(
            &self,
            _interaction: &UIDropInteraction,
            session: &ProtocolObject<dyn UIDropSession>,
        ) {
            guarded("CocoaUiDropDelegate performDrop:", || {
                (self.ivars().perform)(&DropSession { session });
            });
        }
    }
);

impl fmt::Debug for DropDelegate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DropDelegate").finish_non_exhaustive()
    }
}

/// A `UIDropInteraction` on a view, owned with its delegate.
///
/// Dropping it removes the interaction from its view.
#[must_use = "dropping the target removes the interaction"]
#[derive(Debug)]
pub struct DropTarget {
    interaction: Retained<UIDropInteraction>,
    view: objc2::rc::Weak<UIView>,
    #[allow(dead_code)]
    delegate: Retained<DropDelegate>,
}

impl Drop for DropTarget {
    fn drop(&mut self) {
        if let Some(view) = self.view.load() {
            view.removeInteraction(ProtocolObject::from_ref(&*self.interaction));
        }
    }
}

/// Installs a drop interaction on `view`, driven by `handlers`.
pub fn drop_target(view: &UIView, handlers: DropHandlers) -> DropTarget {
    let mtm = view.mtm();
    let delegate: Retained<DropDelegate> = {
        let this = DropDelegate::alloc(mtm).set_ivars(handlers);
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    };
    let interaction = UIDropInteraction::initWithDelegate(
        UIDropInteraction::alloc(mtm),
        ProtocolObject::from_ref(&*delegate),
    );
    view.addInteraction(ProtocolObject::from_ref(&*interaction));
    DropTarget {
        interaction,
        view: objc2::rc::Weak::new(view),
        delegate,
    }
}
