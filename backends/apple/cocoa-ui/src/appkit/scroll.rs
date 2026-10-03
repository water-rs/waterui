//! The `AppKit` scroll view: an `NSScrollView` that carries one document.
//!
//! [`ScrollView`] is the leaf surface a scroll container renders into. The
//! clip view (`contentView`) is the viewport; the document view is the
//! scrollable canvas and is flipped so its origin sits at the top-left. The
//! caller frames the document to the scroll extent and mounts its content
//! wherever the placement math puts it.
//!
//! Scrolling position reporting reuses
//! [`crate::notification::observe_object`]: enable
//! [`NSClipView::setPostsBoundsChangedNotifications`] on [`Self::clip_view`]
//! and observe [`NSViewBoundsDidChangeNotification`].
//!
//! # Safety
//!
//! `unsafe` here subclasses `NSScrollView`/`NSView`, calls `super`, and
//! forwards `AppKit` callbacks into stored `Rc` handlers. The superclasses
//! are main-thread classes and the class is marked `MainThreadOnly`; every
//! override guards the handler call with
//! [`crate::callback::guarded`].

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSClipView, NSScrollView, NSView, NSViewNoIntrinsicMetric};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize};

use crate::callback::guarded;
use crate::geometry::{Point, Size};

/// The handler [`ScrollView`] calls after `AppKit` lays it out.
type LayoutHandler = Rc<dyn Fn(&ScrollView)>;
/// The handler [`ScrollView`] calls after `AppKit` tiles the scrollers.
type TileHandler = Rc<dyn Fn(&ScrollView)>;

/// The per-instance handlers [`ScrollView`] stores.
#[derive(Default)]
pub struct ScrollViewIvars {
    layout: RefCell<Option<LayoutHandler>>,
    tile: RefCell<Option<TileHandler>>,
}

impl std::fmt::Debug for ScrollViewIvars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScrollViewIvars").finish_non_exhaustive()
    }
}

define_class!(
    #[unsafe(super(NSScrollView))]
    #[name = "CocoaUiScrollView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ScrollViewIvars]
    #[derive(Debug)]
    /// A scrollable surface whose document view hosts one laid-out child.
    pub struct ScrollView;

    unsafe impl NSObjectProtocol for ScrollView {}

    impl ScrollView {
        /// Keeps `AppKit`'s tiling, then reports it so the consumer can ask
        /// for a layout pass when the clip area changed.
        #[unsafe(method(tile))]
        fn tile_override(&self) {
            guarded("ScrollView tile", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), tile] };
                if let Some(handler) = self.ivars().tile.borrow().as_ref().cloned() {
                    handler(self);
                }
            });
        }

        /// Keeps `AppKit`'s layout, then hands layout to the consumer.
        #[unsafe(method(layout))]
        fn layout_override(&self) {
            guarded("ScrollView layout", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), layout] };
                if let Some(handler) = self.ivars().layout.borrow().as_ref().cloned() {
                    handler(self);
                }
            });
        }

        /// The scroll view offers no intrinsic size: it fills the space its
        /// layout gives it.
        #[unsafe(method(intrinsicContentSize))]
        fn intrinsic_content_size_override(&self) -> NSSize {
            // SAFETY: reads a constant `AppKit` owns for the process's
            // lifetime.
            let no_metric = unsafe { NSViewNoIntrinsicMetric };
            NSSize::new(no_metric, no_metric)
        }
    }
);

/// The per-instance state [`FlippedView`] stores: none.
#[derive(Debug, Default)]
pub struct FlippedViewIvars;

define_class!(
    #[unsafe(super(NSView))]
    #[name = "CocoaUiFlippedView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = FlippedViewIvars]
    #[derive(Debug)]
    /// The scroll document: a plain view whose coordinate origin is its
    /// top-left corner, matching `UIKit` and `SwiftUI` scroll geometry.
    pub struct FlippedView;

    unsafe impl NSObjectProtocol for FlippedView {}

    impl FlippedView {
        /// Reports the flipped coordinate system `NSScrollView` scrolls
        /// naturally in.
        #[unsafe(method(isFlipped))]
        fn is_flipped_override(&self) -> bool {
            true
        }
    }
);

impl FlippedView {
    /// An empty flipped document view.
    #[must_use]
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FlippedViewIvars);
        // SAFETY: standard `NSView` init on a main-thread class.
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }
}

impl ScrollView {
    /// A scroll view showing a scroller for each enabled axis, no backdrop,
    /// and an empty flipped document to mount content onto.
    ///
    /// `vertical` and `horizontal` enable the matching scroll axis.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, vertical: bool, horizontal: bool) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ScrollViewIvars::default());
        // SAFETY: standard `NSScrollView` init on a main-thread class.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.setHasVerticalScroller(vertical);
        this.setHasHorizontalScroller(horizontal);
        this.setAutohidesScrollers(true);
        this.setDrawsBackground(false);
        let document = FlippedView::new(mtm);
        this.setDocumentView(Some(&document));
        this
    }

    /// The document view children of the scroll content mount onto.
    #[must_use]
    pub fn document_view(&self) -> Option<Retained<NSView>> {
        self.documentView()
    }

    /// The clip view: the viewport and the bounds-changed notification
    /// source for scroll-position reporting.
    #[must_use]
    pub fn clip_view(&self) -> Retained<NSClipView> {
        self.contentView()
    }

    /// The size of the visible scroll area — the clip view's bounds.
    #[must_use]
    pub fn viewport_size(&self) -> Size {
        self.contentView().bounds().size.into()
    }

    /// Frames the document view to the scrollable extent, origin at zero.
    pub fn set_document_extent(&self, extent: Size) {
        if let Some(document) = self.documentView() {
            document.setFrame(NSRect::new(NSPoint::ZERO, extent.into()));
        }
    }

    /// The scroll position: the clip view's bounds origin in document
    /// coordinates.
    #[must_use]
    pub fn content_offset(&self) -> Point {
        self.contentView().bounds().origin.into()
    }

    /// Jumps the scroll position to `point`, top-left origin.
    pub fn scroll_to(&self, point: Point) {
        let clip = self.contentView();
        clip.scrollToPoint(point.into());
        self.reflectScrolledClipView(&clip);
    }

    /// Runs `handler` after every `AppKit` layout pass, replacing the
    /// previous handler.
    pub fn set_layout_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().layout.replace(Some(Rc::new(handler)));
    }

    /// Runs `handler` after every `AppKit` tile pass, replacing the previous
    /// handler. Use it to spot clip-view resizes `layout` does not cover.
    pub fn set_tile_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().tile.replace(Some(Rc::new(handler)));
    }

    /// Marks the view as needing layout on the next pass.
    pub fn set_needs_layout(&self) {
        self.setNeedsLayout(true);
    }

    /// Runs any pending layout immediately.
    pub fn layout_if_needed(&self) {
        self.layoutSubtreeIfNeeded();
    }
}
