//! The `UIKit` scroll view: a `UIScrollView` that carries one laid-out
//! child.
//!
//! [`ScrollView`] is the leaf surface a scroll container renders into.
//! `UIKit` scrolls by translating the view's own content area, so the child
//! mounts on the scroll view itself and [`Self::set_content_extent`] sizes
//! the scrollable canvas; [`Self::content_offset`] is where that canvas is
//! clipped.
//!
//! Scroll-position reporting rides the view's own
//! `UIScrollViewDelegate`: install a handler with [`Self::on_scroll`].
//!
//! # Safety
//!
//! `unsafe` here subclasses `UIScrollView`, calls `super`, implements the
//! `UIScrollViewDelegate` protocol on the view itself, and forwards `UIKit`
//! callbacks into stored `Rc` handlers. The superclass is a main-thread
//! class and the class is marked `MainThreadOnly`; every override guards the
//! handler call with [`crate::callback::guarded`].

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::{CGRect, CGSize};
use objc2_foundation::NSObjectProtocol;
use objc2_ui_kit::{
    UIColor, UIScrollView, UIScrollViewContentInsetAdjustmentBehavior, UIScrollViewDelegate,
    UIViewNoIntrinsicMetric,
};

use crate::callback::guarded;
use crate::geometry::{EdgeInsets, Point, Size};

/// The handler [`ScrollView`] calls after `UIKit` lays it out.
type LayoutHandler = Rc<dyn Fn(&ScrollView)>;
/// The handler [`ScrollView`] calls when its scroll position changes.
type ScrollHandler = Rc<dyn Fn(&ScrollView)>;

/// The per-instance handlers [`ScrollView`] stores.
#[derive(Default)]
pub struct ScrollViewIvars {
    layout: RefCell<Option<LayoutHandler>>,
    scroll: RefCell<Option<ScrollHandler>>,
}

impl std::fmt::Debug for ScrollViewIvars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScrollViewIvars").finish_non_exhaustive()
    }
}

define_class!(
    #[unsafe(super(UIScrollView))]
    #[name = "CocoaUiScrollView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ScrollViewIvars]
    #[derive(Debug)]
    /// A scrollable surface that hosts one laid-out child directly.
    pub struct ScrollView;

    unsafe impl NSObjectProtocol for ScrollView {}

    unsafe impl UIScrollViewDelegate for ScrollView {
        /// Reports the new scroll position to the consumer.
        #[unsafe(method(scrollViewDidScroll:))]
        fn scroll_view_did_scroll(&self, _scroll_view: &UIScrollView) {
            guarded("ScrollView scrollViewDidScroll", || {
                if let Some(handler) = self.ivars().scroll.borrow().as_ref().cloned() {
                    handler(self);
                }
            });
        }
    }

    impl ScrollView {
        /// Keeps `UIKit`'s layout, then hands layout to the consumer.
        #[unsafe(method(layoutSubviews))]
        fn layout_subviews_override(&self) {
            guarded("ScrollView layoutSubviews", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), layoutSubviews] };
                if let Some(handler) = self.ivars().layout.borrow().as_ref().cloned() {
                    handler(self);
                }
            });
        }

        /// The scroll view offers no intrinsic size: it fills the space its
        /// layout gives it.
        #[unsafe(method(intrinsicContentSize))]
        fn intrinsic_content_size_override(&self) -> CGSize {
            // SAFETY: reads a constant `UIKit` owns for the process's
            // lifetime.
            let no_metric = unsafe { UIViewNoIntrinsicMetric };
            CGSize::new(no_metric, no_metric)
        }
    }
);

impl ScrollView {
    /// A scroll view showing an indicator and bouncing on each enabled axis,
    /// with the platform's automatic safe-area inset adjustment and a clear
    /// backdrop.
    ///
    /// `vertical` and `horizontal` enable the matching scroll axis.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, vertical: bool, horizontal: bool) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ScrollViewIvars::default());
        // SAFETY: see the module safety note.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        this.setShowsVerticalScrollIndicator(vertical);
        this.setShowsHorizontalScrollIndicator(horizontal);
        this.setAlwaysBounceVertical(vertical);
        this.setAlwaysBounceHorizontal(horizontal);
        this.setContentInsetAdjustmentBehavior(
            UIScrollViewContentInsetAdjustmentBehavior::Automatic,
        );
        this.setBackgroundColor(Some(&UIColor::clearColor()));
        // SAFETY: `this` implements `UIScrollViewDelegate` above, and the
        // delegate relationship is non-retained on a view we own.
        unsafe { this.setDelegate(Some(ProtocolObject::from_ref(&*this))) };
        this
    }

    /// The size of the visible scroll area.
    #[must_use]
    pub fn viewport_size(&self) -> Size {
        self.bounds().size.into()
    }

    /// The scrollable canvas `UIKit` clips `content_offset` against.
    #[must_use]
    pub fn content_extent(&self) -> Size {
        self.contentSize().into()
    }

    /// Sizes the scrollable canvas.
    pub fn set_content_extent(&self, extent: Size) {
        self.setContentSize(extent.into());
    }

    /// The scroll position: the point inside the canvas the top-left of the
    /// viewport shows.
    #[must_use]
    pub fn content_offset(&self) -> Point {
        self.contentOffset().into()
    }

    /// Sets the scroll position.
    pub fn set_content_offset(&self, offset: Point, animated: bool) {
        self.setContentOffset_animated(offset.into(), animated);
    }

    /// The insets `UIKit` currently applies around the canvas: safe-area and
    /// any content insets combined.
    #[must_use]
    pub fn adjusted_content_inset(&self) -> EdgeInsets {
        let objc2_ui_kit::UIEdgeInsets {
            top,
            left,
            bottom,
            right,
        } = self.adjustedContentInset();
        EdgeInsets::new(top, left, bottom, right)
    }

    /// Runs `handler` after every `UIKit` layout pass, replacing the
    /// previous handler.
    pub fn set_layout_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().layout.replace(Some(Rc::new(handler)));
    }

    /// Runs `handler` whenever the scroll position changes, replacing the
    /// previous handler.
    pub fn on_scroll(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().scroll.replace(Some(Rc::new(handler)));
    }

    /// Marks the view as needing layout on the next pass.
    pub fn set_needs_layout(&self) {
        self.setNeedsLayout();
    }

    /// Runs any pending layout immediately.
    pub fn layout_if_needed(&self) {
        self.layoutIfNeeded();
    }
}
