//! Drag and drop: `NSDraggingSource` sessions and `NSDraggingInfo` payloads
//! with Rust values behind them.
//!
//! A drag carries typed data on the pasteboard through `NSPasteboardItem`s;
//! a value that must stay inside the process rides on the [`DragSource`]
//! object instead, which the destination reads back off
//! `NSDraggingInfo.draggingSource`. Drags from other applications never
//! carry one.
//!
//! [`DropHandlers`] installs the matching `NSDraggingDestination` behavior
//! on a `HostView`; the view keeps them for as long as it accepts drops.
//!
//! # Safety
//!
//! The `unsafe` here defines the source class and drives the pasteboard,
//! image and dragging-session calls `AppKit` declares. The delegate method
//! has the signature `AppKit` sends, and every call is a main-thread call:
//! the class is `MainThreadOnly` and the APIs are view APIs.

use std::any::Any;
use std::fmt;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, ProtocolObject};
use objc2::{
    AnyThread, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
};
use objc2_app_kit::{
    NSDraggingContext, NSDraggingInfo, NSDraggingItem, NSDraggingSession, NSDraggingSource,
    NSEvent, NSImage, NSPasteboardItem, NSPasteboardTypeFileURL, NSPasteboardTypeString,
    NSPasteboardTypeURL, NSPasteboardURLReadingFileURLsOnlyKey, NSView,
};
use objc2_core_graphics::CGContext;
use objc2_foundation::{
    NSArray, NSDictionary, NSNumber, NSObjectProtocol, NSPoint, NSRect, NSString, NSURL,
};

use crate::callback::guarded;

struct SourceIvars {
    payload: Option<Rc<dyn Any>>,
    on_end: Rc<dyn Fn()>,
}

define_class!(
    /// The `NSDraggingSource` behind [`begin_drag`]: it proposes copy/move
    /// and reports the session's end.
    ///
    // SAFETY: `NSObject` has no subclassing requirements; the class holds a
    // payload and a closure and does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiDragSource"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SourceIvars]
    struct DragSource;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for DragSource {}

    // SAFETY: each method has the signature `NSDraggingSource` declares.
    unsafe impl NSDraggingSource for DragSource {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn source_operation_mask(
            &self,
            _session: &NSDraggingSession,
            _context: NSDraggingContext,
        ) -> objc2_app_kit::NSDragOperation {
            objc2_app_kit::NSDragOperation::Copy | objc2_app_kit::NSDragOperation::Move
        }

        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn ended(
            &self,
            _session: &NSDraggingSession,
            _screen_point: NSPoint,
            _operation: objc2_app_kit::NSDragOperation,
        ) {
            guarded("CocoaUiDragSource endedAtPoint:", || {
                (self.ivars().on_end)();
            });
        }
    }
);

impl DragSource {
    fn new(
        mtm: MainThreadMarker,
        payload: Option<Rc<dyn Any>>,
        on_end: Rc<dyn Fn()>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SourceIvars { payload, on_end });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

impl fmt::Debug for DragSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DragSource").finish_non_exhaustive()
    }
}

/// One item a drag begins with: the pasteboard item carrying the value, the
/// frame it drags from, and the image shown under the pointer.
#[derive(Debug)]
pub struct DragItemSpec {
    /// The `NSPasteboardItem` carrying the item's platform representation.
    pub item: Retained<NSPasteboardItem>,
    /// The frame the drag image occupies, in the view's coordinate space.
    pub frame: NSRect,
    /// The image shown while dragging.
    pub image: Retained<NSImage>,
}

/// A pasteboard item carrying plain text.
#[must_use]
pub fn text_item(text: &str) -> Retained<NSPasteboardItem> {
    let item = NSPasteboardItem::new();
    // SAFETY: `AppKit`'s pasteboard-type statics are valid global strings.
    let kind = unsafe { NSPasteboardTypeString };
    item.setString_forType(&NSString::from_str(text), kind);
    item
}

/// A pasteboard item carrying a URL.
#[must_use]
pub fn url_item(url: &str) -> Retained<NSPasteboardItem> {
    let item = NSPasteboardItem::new();
    // SAFETY: `AppKit`'s pasteboard-type statics are valid global strings.
    let kind = unsafe { NSPasteboardTypeURL };
    item.setString_forType(&NSString::from_str(url), kind);
    item
}

/// A pasteboard item carrying a file URL.
#[must_use]
pub fn file_url_item(url: &str) -> Retained<NSPasteboardItem> {
    let item = NSPasteboardItem::new();
    // SAFETY: `AppKit`'s pasteboard-type statics are valid global strings.
    let kind = unsafe { NSPasteboardTypeFileURL };
    item.setString_forType(&NSString::from_str(url), kind);
    item
}

/// A pasteboard item declaring `type_identifier` with an empty payload —
/// the marker a process-scoped drag carries while its value travels on the
/// drag source object only.
#[must_use]
pub fn marker_item(type_identifier: &str) -> Retained<NSPasteboardItem> {
    let item = NSPasteboardItem::new();
    item.setString_forType(&NSString::new(), &NSString::from_str(type_identifier));
    item
}

/// A raster of `view`'s current layer, for use as a drag image.
#[must_use]
pub fn view_snapshot(view: &NSView) -> Retained<NSImage> {
    let bounds = view.bounds();
    let layer = view.layer();
    let handler = RcBlock::new(move |_rect: NSRect| -> objc2::runtime::Bool {
        if let (Some(context), Some(layer)) = (
            objc2_app_kit::NSGraphicsContext::currentContext(),
            layer.as_deref(),
        ) {
            let cg = context.CGContext();
            // The layer renders in a flipped (top-left origin) context; the
            // image draws unflipped.
            CGContext::translate_ctm(Some(&cg), 0.0, bounds.size.height);
            CGContext::scale_ctm(Some(&cg), 1.0, -1.0);
            layer.renderInContext(&cg);
        }
        true.into()
    });
    NSImage::imageWithSize_flipped_drawingHandler(bounds.size, true, &handler)
}

/// Begins a dragging session on `view` for `event` carrying `items`, with
/// `payload` readable by same-process destinations off the drag source.
/// `on_end` runs when the session ends.
///
/// The returned session stays alive for the session's duration; the caller
/// may keep or drop it.
pub fn begin_drag(
    view: &NSView,
    event: &NSEvent,
    items: Vec<DragItemSpec>,
    payload: Option<Rc<dyn Any>>,
    on_end: impl Fn() + 'static,
) -> Retained<NSDraggingSession> {
    let mtm = view.mtm();
    let source = DragSource::new(mtm, payload, Rc::new(on_end));
    let dragging_items = items
        .into_iter()
        .map(|spec| {
            let item = NSDraggingItem::initWithPasteboardWriter(
                NSDraggingItem::alloc(),
                ProtocolObject::from_ref(&*spec.item),
            );
            // SAFETY: `contents` is the drag image AppKit documents.
            unsafe { item.setDraggingFrame_contents(spec.frame, Some(&*spec.image)) };
            item
        })
        .collect::<Vec<_>>();
    let items = NSArray::from_retained_slice(&dragging_items);
    view.beginDraggingSessionWithItems_event_source(
        &items,
        event,
        ProtocolObject::from_ref(&*source),
    )
}

/// A dragging-info object as a destination sees it.
pub struct DragInfo<'a> {
    info: &'a ProtocolObject<dyn NSDraggingInfo>,
}

impl fmt::Debug for DragInfo<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DragInfo").finish_non_exhaustive()
    }
}

impl<'a> DragInfo<'a> {
    pub(crate) fn new(info: &'a ProtocolObject<dyn NSDraggingInfo>) -> Self {
        Self { info }
    }

    /// The payload of a same-process [`begin_drag`] source, if this drag is
    /// one. Other applications' drags never carry one.
    #[must_use]
    pub fn source_payload(&self) -> Option<Rc<dyn Any>> {
        self.info
            .draggingSource()
            .and_then(|source| source.downcast::<DragSource>().ok())
            .and_then(|source| source.ivars().payload.clone())
    }

    /// The pasteboard types the drag carries.
    #[must_use]
    pub fn types(&self) -> Vec<Retained<NSString>> {
        self.info
            .draggingPasteboard()
            .types()
            .map(|types| types.iter().collect())
            .unwrap_or_default()
    }

    /// Whether the drag declares `type_identifier` on its pasteboard.
    #[must_use]
    pub fn has_type(&self, type_identifier: &str) -> bool {
        let type_identifier = NSString::from_str(type_identifier);
        self.types().iter().any(|t| **t == *type_identifier)
    }

    /// The file URLs the drag's pasteboard carries, as absolute URL strings.
    #[must_use]
    pub fn file_urls(&self) -> Vec<String> {
        let pasteboard = self.info.draggingPasteboard();
        let classes = NSArray::from_slice(&[NSURL::class()]);
        let file_urls_only = NSNumber::numberWithBool(true);
        let value: &AnyObject = &file_urls_only;
        // SAFETY: `AppKit`'s pasteboard-option-key statics are valid global
        // strings.
        let key = unsafe { NSPasteboardURLReadingFileURLsOnlyKey };
        let options = NSDictionary::from_slices(&[key], &[value]);
        // SAFETY: `NSURL` implements `NSPasteboardReading` and the options
        // dictionary is well-formed.
        unsafe {
            pasteboard
                .readObjectsForClasses_options(&classes, Some(&*options))
                .map(|objects| {
                    objects
                        .iter()
                        .filter_map(|object: Retained<AnyObject>| {
                            object
                                .downcast_ref::<NSURL>()
                                .and_then(objc2_foundation::NSURL::absoluteString)
                                .map(|string| string.to_string())
                        })
                        .collect()
                })
                .unwrap_or_default()
        }
    }

    /// The first `NSString` the pasteboard carries for `type_identifier`.
    #[must_use]
    pub fn string(&self, type_identifier: &str) -> Option<Retained<NSString>> {
        self.info
            .draggingPasteboard()
            .stringForType(&NSString::from_str(type_identifier))
    }
}

/// The closures a `HostView` drop destination calls during a drag.
pub struct DropHandlers {
    /// A drag entered the view; answers the operation to accept it with.
    pub entered: Rc<dyn Fn(&DragInfo<'_>) -> objc2_app_kit::NSDragOperation>,
    /// The drag updated inside the view; answers the proposed operation.
    pub updated: Rc<dyn Fn(&DragInfo<'_>) -> objc2_app_kit::NSDragOperation>,
    /// The drag left the view without dropping.
    pub exited: Rc<dyn Fn(&DragInfo<'_>)>,
    /// The user dropped; answers whether the drop was performed.
    pub perform: Rc<dyn Fn(&DragInfo<'_>) -> bool>,
}

impl fmt::Debug for DropHandlers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DropHandlers").finish_non_exhaustive()
    }
}
