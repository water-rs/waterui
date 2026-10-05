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

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSNotification, NSObjectProtocol};
use objc2_ui_kit::{
    UIColor, UICoordinateSpace, UIKeyboardWillChangeFrameNotification,
    UIKeyboardWillHideNotification, UIKeyboardWillShowNotification, UIScrollView,
    UIScrollViewContentInsetAdjustmentBehavior, UIScrollViewDelegate,
    UITextFieldTextDidBeginEditingNotification, UITextViewTextDidBeginEditingNotification, UIView,
    UIViewAnimationOptions, UIViewNoIntrinsicMetric,
};

use crate::callback::guarded;
use crate::geometry::{EdgeInsets, Point, Size};
use crate::notification::{NotificationName, NotificationObserver};
use crate::uikit::keyboard;

/// The handler [`ScrollView`] calls after `UIKit` lays it out.
type LayoutHandler = Rc<dyn Fn(&ScrollView)>;
/// The handler [`ScrollView`] calls when its scroll position changes.
type ScrollHandler = Rc<dyn Fn(&ScrollView)>;

/// The per-instance handlers [`ScrollView`] stores.
#[derive(Default)]
pub struct ScrollViewIvars {
    layout: RefCell<Option<LayoutHandler>>,
    scroll: RefCell<Option<ScrollHandler>>,
    /// The keyboard and focus observers, live while the view sits in a
    /// window.
    observers: RefCell<Vec<NotificationObserver>>,
    /// The software keyboard's frame in the window's coordinates, last
    /// reported by a keyboard notification.
    keyboard_frame: Cell<CGRect>,
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

        /// Installs the keyboard observers once the view has a window and
        /// drops them when it leaves one.
        #[unsafe(method(didMoveToWindow))]
        fn did_move_to_window_override(&self) {
            guarded("ScrollView didMoveToWindow", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), didMoveToWindow] };
                self.ivars().observers.borrow_mut().clear();
                self.ivars().keyboard_frame.set(CGRect::ZERO);
                if self.window().is_some() {
                    self.observe_keyboard();
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

    /// Registers the observers driving the scroll's keyboard behavior:
    /// keyboard notifications re-inset and re-scroll with `UIKit`'s own
    /// animation, and a text field gaining focus while the keyboard is up
    /// scrolls it clear.
    fn observe_keyboard(&self) {
        let mtm = self.mtm();
        let mut observers = self.ivars().observers.borrow_mut();
        // SAFETY: `UIKit` exports these notification names as constants
        // for the process's lifetime.
        for name in unsafe {
            [
                UIKeyboardWillShowNotification,
                UIKeyboardWillChangeFrameNotification,
                UIKeyboardWillHideNotification,
            ]
        } {
            let weak = Weak::new(self);
            observers.push(crate::notification::observe_with_notification(
                mtm,
                &NotificationName::framework(name),
                move |note| {
                    if let Some(scroll) = weak.load() {
                        scroll.apply_keyboard_change(note);
                    }
                },
            ));
        }
        // SAFETY: `UIKit` exports these notification names as constants
        // for the process's lifetime.
        for name in unsafe {
            [
                UITextFieldTextDidBeginEditingNotification,
                UITextViewTextDidBeginEditingNotification,
            ]
        } {
            let weak = Weak::new(self);
            observers.push(crate::notification::observe_with_notification(
                mtm,
                &NotificationName::framework(name),
                move |_note| {
                    if let Some(scroll) = weak.load() {
                        scroll.scroll_focused_clear(true);
                    }
                },
            ));
        }
    }

    /// The depth of the keyboard band inside this scroll's window frame on
    /// the bottom edge — the inset the content needs to scroll the tail
    /// clear of the keyboard region.
    fn keyboard_cover(&self) -> f64 {
        let keyboard = self.ivars().keyboard_frame.get();
        if keyboard.size.width <= 0.0 || keyboard.size.height <= 0.0 {
            return 0.0;
        }
        let frame = self.convertRect_toView(self.bounds(), None);
        let bottom = frame.origin.y + frame.size.height;
        let horizontal = keyboard.origin.x < frame.origin.x + frame.size.width
            && keyboard.origin.x + keyboard.size.width > frame.origin.x;
        if !horizontal || keyboard.origin.y + keyboard.size.height < bottom - 0.5 {
            return 0.0;
        }
        (bottom - keyboard.origin.y).clamp(0.0, frame.size.height)
    }

    /// Applies a keyboard notification: the band depth becomes the bottom
    /// content inset beyond `safeAreaInsets`, and a focused field inside is
    /// scrolled clear — all inside the animation `UIKit` plays for the
    /// change.
    fn apply_keyboard_change(&self, note: &NSNotification) {
        let Some(window) = self.window() else {
            return;
        };
        let Some(change) = keyboard::change(note) else {
            return;
        };
        let space = window.screen().coordinateSpace();
        let frame = window.convertRect_fromCoordinateSpace(change.frame, &space);
        self.ivars().keyboard_frame.set(frame);
        let options = UIViewAnimationOptions(
            (change.curve << 16) | UIViewAnimationOptions::BeginFromCurrentState.0,
        );
        let block = RcBlock::new({
            let scroll: Retained<Self> = Retained::from(self);
            move || {
                scroll.apply_keyboard_inset();
                scroll.scroll_focused_clear(false);
                scroll.layoutIfNeeded();
            }
        });
        // `mtm` guarantees the `UIKit` call stays on the main thread; the
        // options value is the documented `curve << 16` packing.
        UIView::animateWithDuration_delay_options_animations_completion(
            change.duration,
            0.0,
            options,
            &block,
            None,
            self.mtm(),
        );
    }

    /// Adds the covered keyboard depth to the bottom content inset, so the
    /// content tail scrolls up clear of the keyboard region. The adjusted
    /// inset ends at the band depth: `safeAreaInsets` already carries the
    /// container band the scroll crosses.
    fn apply_keyboard_inset(&self) {
        let covered = self.keyboard_cover();
        let mut inset = self.contentInset();
        inset.bottom = (covered - self.safeAreaInsets().bottom).max(0.0);
        self.setContentInset(inset);
    }

    /// Scrolls the current first responder inside this scroll the minimum
    /// distance that brings its frame clear of the keyboard band.
    fn scroll_focused_clear(&self, animated: bool) {
        let keyboard = self.ivars().keyboard_frame.get();
        if keyboard.size.height <= 0.0 {
            return;
        }
        let Some(responder) = first_responder(self) else {
            return;
        };
        let field = responder.convertRect_toView(responder.bounds(), None);
        let keyboard_top = keyboard.origin.y;
        if field.origin.y + field.size.height <= keyboard_top + 0.5 {
            return;
        }
        // The distance the viewport must move so the field's bottom clears
        // the band's top, in scroll coordinates.
        let own_frame = self.convertRect_toView(self.bounds(), None);
        let visible_bottom = keyboard_top - own_frame.origin.y;
        let field_bottom = field.origin.y + field.size.height - own_frame.origin.y;
        let delta = field_bottom - visible_bottom;
        if delta <= 0.5 {
            return;
        }
        let inset = self.adjustedContentInset();
        let max_y =
            (self.contentSize().height + inset.bottom - self.bounds().size.height).max(-inset.top);
        let offset = self.contentOffset();
        let new_y = (offset.y + delta).min(max_y);
        if new_y > offset.y + 0.5 {
            self.setContentOffset_animated(CGPoint::new(offset.x, new_y), animated);
        }
    }
}

/// The first responder inside `view`'s subtree, if any — a recursive
/// `isFirstResponder` walk of the view hierarchy.
fn first_responder(view: &UIView) -> Option<Retained<UIView>> {
    if view.isFirstResponder() {
        return Some(Retained::from(view));
    }
    view.subviews()
        .iter()
        .find_map(|subview| first_responder(&subview))
}
