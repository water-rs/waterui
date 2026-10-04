//! The view that hosts content laid out by Rust.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UIView` subclass, forwards to `UIView`'s own
//! implementation of each method it overrides, and reads the view's trait
//! collection. Every override has the signature `UIView` declares, and
//! `UIKit` calls them, and trait collections are read, on the main thread.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::ptr;
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2::sel;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSArray, NSMutableSet, NSObject, NSObjectProtocol};
use objc2_ui_kit::{
    UIEdgeInsets, UIEvent, UIGestureRecognizerState, UIHoverGestureRecognizer, UIPress,
    UIPressesEvent, UITraitEnvironment, UIView,
};

use crate::callback::guarded;
use crate::geometry::{EdgeInsets, Edges, MeasureProposal, Point, Rect, Size};
use crate::keys::{self, KeyEvent};
use crate::pointer::{PointerEvent, PointerEvents};

/// What a [`HostView`]'s hit-test handler decides for a point.
#[derive(Debug, Clone)]
pub enum HitTest {
    /// Whatever `UIKit` would decide: the deepest subview under the point, or
    /// the host view itself.
    Default,
    /// Nothing here: the touch goes to whatever lies beneath the host view.
    Pass,
    /// Whatever `UIKit` would decide, except the host view itself: a
    /// container's own hits belong to whatever lies beneath it.
    PassIfSelf,
    /// This view receives the touch.
    View(Retained<UIView>),
}

type LayoutHandler = Rc<dyn Fn(&HostView)>;
type ResizeHandler = Rc<dyn Fn(&HostView, Size)>;
type HitTestHandler = Rc<dyn Fn(&HostView, Point) -> HitTest>;
type WindowHandler = Rc<dyn Fn(&HostView)>;
type MeasureHandler = Rc<dyn Fn(&HostView, MeasureProposal) -> Size>;
type PrimaryContentHandler = Rc<dyn Fn(&HostView) -> Option<Retained<UIView>>>;
type ScrollSurfaceHandler = Rc<dyn Fn(&HostView) -> Vec<Retained<UIView>>>;
type PointerHandler = Rc<dyn Fn(&HostView, PointerEvent) -> bool>;
type KeyHandler = Rc<dyn Fn(&HostView, &KeyEvent) -> bool>;

/// The handlers a [`HostView`] calls, and the state it keeps for them.
#[derive(Default)]
pub struct HostViewIvars {
    layout: RefCell<Option<LayoutHandler>>,
    resize: RefCell<Option<ResizeHandler>>,
    hit_test: RefCell<Option<HitTestHandler>>,
    window: RefCell<Option<WindowHandler>>,
    superview: RefCell<Option<WindowHandler>>,
    measure: RefCell<Option<MeasureHandler>>,
    primary_content: RefCell<Option<PrimaryContentHandler>>,
    scroll_surface_candidates: RefCell<Option<ScrollSurfaceHandler>>,
    /// Whether the view's own content is laid out against its bounds — the
    /// answer to "does this view manage its own safe area".
    manages_safe_area: Cell<bool>,
    /// The edges this view erases from the safe-area insets its subtree
    /// sees — what `cocoaUiIgnoredSafeAreaEdges` reports: bits 0–3 the
    /// `Edges` mask, bit 4 marking the view an ignore-safe-area wrapper so
    /// a reader can tell "ignores nothing" from "not an ignorer".
    ignored_safe_area_edges: Cell<u8>,
    /// Whether the Auto Layout width is tracked for intrinsic size; see
    /// [`set_intrinsic_auto_layout`](HostView::set_intrinsic_auto_layout).
    intrinsic_auto_layout: Cell<bool>,
    /// The width the intrinsic-content-size query was last invalidated for.
    last_auto_layout_width: Cell<f64>,
    /// The size the resize handler was last told about.
    reported_size: Cell<CGSize>,
    /// Whether this is a view controller's root view, which always fills its
    /// window; see [`window_root`].
    fills_window: Cell<bool>,
    pointer: RefCell<Option<PointerHandler>>,
    /// Which pointer events the pointer handler wants.
    pointer_events: Cell<PointerEvents>,
    /// Whether the pointer is currently over the view.
    pointer_inside: Cell<bool>,
    /// The recognizer serving the pointer handler, kept so the target
    /// stays alive.
    hover_recognizer: RefCell<Option<Retained<UIHoverGestureRecognizer>>>,
    key: RefCell<Option<KeyHandler>>,
}

impl fmt::Debug for HostViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostViewIvars")
            .field("layout", &self.layout.borrow().is_some())
            .field("resize", &self.resize.borrow().is_some())
            .field("hit_test", &self.hit_test.borrow().is_some())
            .field("reported_size", &self.reported_size.get())
            .field("fills_window", &self.fills_window.get())
            .field("window", &self.window.borrow().is_some())
            .field("superview", &self.superview.borrow().is_some())
            .field("measure", &self.measure.borrow().is_some())
            .field("primary_content", &self.primary_content.borrow().is_some())
            .field(
                "scroll_surface_candidates",
                &self.scroll_surface_candidates.borrow().is_some(),
            )
            .field(
                "ignored_safe_area_edges",
                &self.ignored_safe_area_edges.get(),
            )
            .field("last_auto_layout_width", &self.last_auto_layout_width.get())
            .field("manages_safe_area", &self.manages_safe_area.get())
            .field("intrinsic_auto_layout", &self.intrinsic_auto_layout.get())
            .field("pointer", &self.pointer.borrow().is_some())
            .field("pointer_events", &self.pointer_events.get())
            .field("pointer_inside", &self.pointer_inside.get())
            .field(
                "hover_recognizer",
                &self.hover_recognizer.borrow().is_some(),
            )
            .field("key", &self.key.borrow().is_some())
            .finish_non_exhaustive()
    }
}

/// The closure a [`HoverTarget`] runs.
type HoverHandler = Rc<dyn Fn(&UIHoverGestureRecognizer)>;

/// The ivars of a [`HoverTarget`]: the closure it runs.
#[derive(Default)]
pub struct HoverTargetIvars {
    handler: RefCell<Option<HoverHandler>>,
}

impl fmt::Debug for HoverTargetIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HoverTargetIvars")
            .field("handler", &self.handler.borrow().is_some())
            .finish()
    }
}

define_class!(
    // SAFETY: `NSObject` asks a subclass to initialize through `init`, which
    // `HoverTarget::new` does, and the class does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiHoverTarget"]
    #[thread_kind = MainThreadOnly]
    #[ivars = HoverTargetIvars]
    #[derive(Debug)]
    /// The action target of a [`UIHoverGestureRecognizer`] attached to a
    /// [`HostView`]: the recognizer owns it, it forwards to the view's
    /// pointer handler.
    struct HoverTarget;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for HoverTarget {}

    impl HoverTarget {
        // SAFETY: see the module safety note.
        #[unsafe(method(cocoaUiHover:))]
        fn cocoa_ui_hover(&self, recognizer: &UIHoverGestureRecognizer) {
            guarded("HoverTarget cocoaUiHover:", || {
                let handler = self.ivars().handler.borrow().clone();
                if let Some(handler) = handler {
                    handler(recognizer);
                }
            });
        }
    }
);

impl HoverTarget {
    /// A target that runs `handler` when its recognizer fires.
    fn new(mtm: MainThreadMarker, handler: HoverHandler) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(HoverTargetIvars {
            handler: RefCell::new(Some(handler)),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

define_class!(
    // SAFETY: `UIView` asks a subclass to initialize through its designated
    // initializer, which `HostView::new` does, and the class does not
    // implement `Drop`.
    #[unsafe(super(UIView))]
    #[name = "CocoaUiHostView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = HostViewIvars]
    #[derive(Debug)]
    /// A view whose layout, resizing and hit testing are Rust closures.
    ///
    /// Place subviews from the layout handler, which runs whenever `UIKit`
    /// lays the view out.
    pub struct HostView;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIView` subclass.
    unsafe impl NSObjectProtocol for HostView {}

    impl HostView {
        // SAFETY: see the module safety note.
        #[unsafe(method(layoutSubviews))]
        fn layout_subviews_override(&self) {
            guarded("HostView layoutSubviews", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), layoutSubviews] };
                if self.ivars().fills_window.get()
                    && let Some(window) = self.window()
                {
                    self.setFrame(window.bounds());
                }
                let handler = self.ivars().layout.borrow().clone();
                if let Some(handler) = handler {
                    handler(self);
                }
                self.track_intrinsic_width();
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(didMoveToWindow))]
        fn did_move_to_window_override(&self) {
            guarded("HostView didMoveToWindow", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), didMoveToWindow] };
                let handler = self.ivars().window.borrow().clone();
                if let Some(handler) = handler {
                    handler(self);
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(didMoveToSuperview))]
        fn did_move_to_superview_override(&self) {
            guarded("HostView didMoveToSuperview", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), didMoveToSuperview] };
                let handler = self.ivars().superview.borrow().clone();
                if let Some(handler) = handler {
                    handler(self);
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(sizeThatFits:))]
        fn size_that_fits_override(&self, size: CGSize) -> CGSize {
            guarded("HostView sizeThatFits:", || {
                if let Some(handler) = self.ivars().measure.borrow().clone() {
                    return handler(self, MeasureProposal::fitted(size.into())).into();
                }
                // SAFETY: see the module safety note.
                unsafe { msg_send![super(self), sizeThatFits: size] }
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(intrinsicContentSize))]
        fn intrinsic_content_size_override(&self) -> CGSize {
            guarded("HostView intrinsicContentSize", || {
                if let Some(handler) = self.ivars().measure.borrow().clone() {
                    let intrinsic = handler(self, MeasureProposal::UNBOUNDED);
                    // Under Auto Layout the width is the parent's constraint,
                    // so the intrinsic height must be measured against it.
                    if self.ivars().intrinsic_auto_layout.get()
                        && !self.translatesAutoresizingMaskIntoConstraints()
                        && self.bounds().size.width > 0.0
                    {
                        let constrained = handler(
                            self,
                            MeasureProposal::width(self.bounds().size.width),
                        );
                        return CGSize::new(intrinsic.width, constrained.height);
                    }
                    return intrinsic.into();
                }
                // SAFETY: see the module safety note.
                unsafe { msg_send![super(self), intrinsicContentSize] }
            })
        }

        // SAFETY: see the module safety note. Exposed under a `cocoaUi`
        // selector for the sibling backend's safe-area rules; it reads an
        // ivar and performs no layout.
        #[unsafe(method(cocoaUiManagesSafeArea))]
        fn manages_safe_area_override(&self) -> bool {
            self.ivars().manages_safe_area.get()
        }

        // SAFETY: see the module safety note. Exposed under a `cocoaUi`
        // selector for the sibling backend's safe-area erasure; it reads an
        // ivar and performs no layout. Bits 0–3 are the `Edges` mask, bit 4
        // marks the view an ignore-safe-area wrapper.
        #[unsafe(method(cocoaUiIgnoredSafeAreaEdges))]
        fn ignored_safe_area_edges_override(&self) -> isize {
            isize::from(self.ivars().ignored_safe_area_edges.get())
        }

        // SAFETY: see the module safety note. Exposed under a `cocoaUi`
        // selector for the sibling backend's primary-content chain.
        #[unsafe(method_id(cocoaUiPrimaryContent))]
        fn primary_content_override(&self) -> Option<Retained<UIView>> {
            let handler = self.ivars().primary_content.borrow().clone();
            handler.and_then(|handler| handler(self))
        }

        // SAFETY: see the module safety note. Exposed under a `cocoaUi`
        // selector for the sibling backend's scroll-surface search.
        #[unsafe(method_id(cocoaUiScrollSurfaceCandidates))]
        fn scroll_surface_candidates_override(&self) -> Retained<NSArray<UIView>> {
            let handler = self.ivars().scroll_surface_candidates.borrow().clone();
            NSArray::from_retained_slice(&handler.map_or_else(Vec::new, |handler| handler(self)))
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(setFrame:))]
        fn set_frame_override(&self, frame: CGRect) {
            guarded("HostView setFrame:", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), setFrame: frame] };
                self.report_size();
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(setBounds:))]
        fn set_bounds_override(&self, bounds: CGRect) {
            guarded("HostView setBounds:", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), setBounds: bounds] };
                self.report_size();
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(safeAreaInsets))]
        fn safe_area_insets_override(&self) -> UIEdgeInsets {
            guarded("HostView safeAreaInsets", || {
                // A window root reports its window's insets, so content that
                // fills the window still learns where the window is obscured.
                if self.ivars().fills_window.get()
                    && let Some(window) = self.window()
                {
                    return window.safeAreaInsets();
                }
                // SAFETY: see the module safety note.
                unsafe { msg_send![super(self), safeAreaInsets] }
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method_id(hitTest:withEvent:))]
        fn hit_test_override(
            &self,
            point: CGPoint,
            event: Option<&UIEvent>,
        ) -> Option<Retained<UIView>> {
            guarded("HostView hitTest:withEvent:", || {
                let handler = self.ivars().hit_test.borrow().clone();
                match handler.map_or(HitTest::Default, |handler| handler(self, point.into())) {
                    HitTest::Default => self.default_hit_test(point, event),
                    HitTest::Pass => None,
                    HitTest::PassIfSelf => {
                        let this: &UIView = self;
                        self.default_hit_test(point, event)
                            .filter(|hit| !ptr::eq(&raw const **hit, this))
                    }
                    HitTest::View(view) => Some(view),
                }
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(pressesBegan:withEvent:))]
        fn presses_began_override(
            &self,
            presses: &objc2_foundation::NSSet<UIPress>,
            event: Option<&UIPressesEvent>,
        ) {
            guarded("HostView pressesBegan:withEvent:", || {
                let handler = self.ivars().key.borrow().clone();
                let unconsumed = NSMutableSet::<UIPress>::new();
                for press in presses {
                    let consumed = handler.as_ref().is_some_and(|handler| {
                        press
                            .key(self.mtm())
                            .is_some_and(|key| handler(self, &keys::key_event(&key)))
                    });
                    if !consumed {
                        unconsumed.addObject(&press);
                    }
                }
                // A key event where every press was consumed is fully
                // handled; only the leftover presses reach `super`.
                if unconsumed.count() != 0 {
                    // SAFETY: see the module safety note.
                    unsafe {
                        let _: () =
                            msg_send![super(self), pressesBegan: &*unconsumed, withEvent: event];
                    }
                }
            });
        }
    }
);

impl HostView {
    /// A host view occupying `frame` in its future superview's coordinates.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, frame: Rect) -> Retained<Self> {
        Self::with_ivars(mtm, frame, HostViewIvars::default())
    }

    fn with_ivars(mtm: MainThreadMarker, frame: Rect, ivars: HostViewIvars) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: `initWithFrame:` is `UIView`'s designated initializer.
        unsafe { msg_send![super(this), initWithFrame: CGRect::from(frame)] }
    }

    /// Calls `handler` at the end of every layout pass, replacing any handler
    /// set before. This is where subviews are given their frames.
    pub fn set_layout_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().layout.replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` with the new size every time the view's size changes,
    /// replacing any handler set before.
    pub fn set_resize_handler(&self, handler: impl Fn(&Self, Size) + 'static) {
        self.ivars().resize.replace(Some(Rc::new(handler)));
    }

    /// Lets `handler` decide which view receives a touch at a point,
    /// replacing any handler set before.
    ///
    /// The point is in the host view's own coordinates, as `UIKit` hit
    /// testing is.
    pub fn set_hit_test_handler(&self, handler: impl Fn(&Self, Point) -> HitTest + 'static) {
        self.ivars().hit_test.replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` every time the view moves into or out of a window —
    /// `didMoveToWindow`, the point where a focus request becomes possible
    /// or is lost.
    pub fn set_window_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().window.replace(Some(Rc::new(handler)));
    }

    /// Sends the hover recognizer's state change to the pointer handler
    /// when it wants that event.
    fn deliver_hover(&self, recognizer: &UIHoverGestureRecognizer) {
        let events = self.ivars().pointer_events.get();
        let handler = self.ivars().pointer.borrow().clone();
        let Some(handler) = handler else { return };
        match recognizer.state() {
            UIGestureRecognizerState::Began => {
                self.ivars().pointer_inside.set(true);
                if events.wants(PointerEvent::Entered) {
                    handler(self, PointerEvent::Entered);
                }
            }
            UIGestureRecognizerState::Changed => {
                if events.wants(PointerEvent::Moved(Point::ZERO)) {
                    let point = recognizer.locationInView(Some(self));
                    handler(self, PointerEvent::Moved(point.into()));
                }
            }
            UIGestureRecognizerState::Ended
            | UIGestureRecognizerState::Cancelled
            | UIGestureRecognizerState::Failed => {
                self.ivars().pointer_inside.set(false);
                if events.wants(PointerEvent::Exited) {
                    handler(self, PointerEvent::Exited);
                }
            }
            _ => {}
        }
    }

    /// Calls `handler` for each pointer `event` in `events`, replacing any
    /// handler set before.
    ///
    /// The events arrive through a `UIHoverGestureRecognizer`;
    /// [`PointerEvent::Moved`]'s point is in the view's own coordinates.
    /// `UIKit` has no cursor contract, so [`PointerEvent::CursorUpdate`] is
    /// never delivered and the handler's return value is ignored.
    pub fn set_pointer_handler(
        &self,
        events: PointerEvents,
        handler: impl Fn(&Self, PointerEvent) -> bool + 'static,
    ) {
        self.ivars().pointer.replace(Some(Rc::new(handler)));
        self.ivars().pointer_events.set(events);
        let mtm = self.mtm();
        let weak = Weak::new(self);
        let target = HoverTarget::new(
            mtm,
            Rc::new(move |recognizer: &UIHoverGestureRecognizer| {
                if let Some(view) = weak.load() {
                    view.deliver_hover(recognizer);
                }
            }),
        );
        // SAFETY: `target` owns the handler and `cocoaUiHover:` is its
        // declared action; the recognizer retains its target.
        let recognizer = unsafe {
            UIHoverGestureRecognizer::initWithTarget_action(
                UIHoverGestureRecognizer::alloc(mtm),
                Some(AsRef::<AnyObject>::as_ref(&*target)),
                Some(sel!(cocoaUiHover:)),
            )
        };
        self.addGestureRecognizer(&recognizer);
        self.ivars().hover_recognizer.replace(Some(recognizer));
    }

    /// Whether the pointer is currently over this view.
    #[must_use]
    pub fn is_pointer_inside(&self) -> bool {
        self.ivars().pointer_inside.get()
    }

    /// Lets `handler` decide whether a key press on this view is consumed,
    /// replacing any handler set before. A press `handler` consumes is
    /// removed from the set `pressesBegan` forwards; the rest continue up
    /// the responder chain.
    pub fn set_key_handler(&self, handler: impl Fn(&Self, &KeyEvent) -> bool + 'static) {
        self.ivars().key.replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` every time the view moves into or out of a superview —
    /// `didMoveToSuperview`, the point where an enclosing scroll surface may
    /// have changed.
    pub fn set_superview_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().superview.replace(Some(Rc::new(handler)));
    }

    /// Lets `handler` answer the view's intrinsic measurements,
    /// `sizeThatFits` and `intrinsicContentSize`, for a layout container.
    pub fn set_measure_handler(&self, handler: impl Fn(&Self, MeasureProposal) -> Size + 'static) {
        self.ivars().measure.replace(Some(Rc::new(handler)));
    }

    /// Whether the intrinsic content size reports the height the current
    /// Auto Layout width produces.
    ///
    /// When enabled and the view is parented under Auto Layout
    /// (`translatesAutoresizingMaskIntoConstraints` is off),
    /// `intrinsicContentSize` re-measures at the bounds width so wrapped
    /// content can grow vertically, and a width change during layout
    /// invalidates the intrinsic size so the constraint system re-queries.
    pub fn set_intrinsic_auto_layout(&self, enabled: bool) {
        self.ivars().intrinsic_auto_layout.set(enabled);
    }

    /// Whether the view manages its own safe area — the answer
    /// `wuiHandlesSafeArea` in the sibling backend reads, through the
    /// `cocoaUiManagesSafeArea` selector.
    pub fn set_manages_safe_area(&self, manages: bool) {
        self.ivars().manages_safe_area.set(manages);
    }

    /// The edges this view erases from the safe-area insets its subtree
    /// sees — what an ignore-safe-area wrapper reports through the
    /// `cocoaUiIgnoredSafeAreaEdges` selector the sibling backend's
    /// safe-area rect consults on its ancestor walk. Setting the marks
    /// bit 4, so a wrapper that ignores no edge still answers as an
    /// ignorer.
    pub fn set_ignored_safe_area_edges(&self, edges: Edges) {
        self.ivars()
            .ignored_safe_area_edges
            .set(edges.mask() | 0x10);
    }

    /// The primary content the sibling backend's wrappers descend to — the
    /// answer `cocoaUiPrimaryContent` reports, and the link a scroll-surface
    /// or safe-area query follows into the view.
    pub fn set_primary_content_handler(
        &self,
        handler: impl Fn(&Self) -> Option<Retained<UIView>> + 'static,
    ) {
        self.ivars().primary_content.replace(Some(Rc::new(handler)));
    }

    /// The children that may be the scroll surface surrounding bars follow —
    /// the answer `cocoaUiScrollSurfaceCandidates` reports.
    pub fn set_scroll_surface_handler(
        &self,
        handler: impl Fn(&Self) -> Vec<Retained<UIView>> + 'static,
    ) {
        self.ivars()
            .scroll_surface_candidates
            .replace(Some(Rc::new(handler)));
    }

    /// Adds `view` above the existing subviews.
    pub fn add_subview(&self, view: &UIView) {
        self.addSubview(view);
    }

    /// Removes `view`, one of this view's subviews.
    ///
    /// # Panics
    ///
    /// If `view` is not a subview of this view.
    pub fn remove_subview(&self, view: &UIView) {
        let this: &UIView = self;
        assert!(
            view.superview()
                .is_some_and(|parent| ptr::eq(&raw const *parent, this)),
            "{view:?} is not a subview of {this:?}"
        );
        view.removeFromSuperview();
    }

    /// Moves and resizes the view to `frame`, in its superview's coordinates.
    pub fn set_frame(&self, frame: Rect) {
        self.setFrame(frame.into());
    }

    /// Marks the view as needing a layout pass before it is next drawn.
    pub fn set_needs_layout(&self) {
        self.setNeedsLayout();
    }

    /// Runs any pending layout pass of this view and its subviews now.
    pub fn layout_if_needed(&self) {
        self.layoutIfNeeded();
    }

    /// Device pixels per point where the view is drawn, from its trait
    /// collection.
    ///
    /// `None` while the view is outside any window scene, where the scale is
    /// not yet known.
    #[must_use]
    pub fn display_scale(&self) -> Option<f64> {
        // SAFETY: see the module safety note.
        let scale = unsafe { self.traitCollection().displayScale() };
        (scale > 0.0).then_some(scale)
    }

    /// The distances from each edge within which content is obscured by bars,
    /// the status bar, or the rounded corners and sensor housing of the
    /// screen.
    #[must_use]
    pub fn safe_area_insets(&self) -> EdgeInsets {
        let UIEdgeInsets {
            top,
            left,
            bottom,
            right,
        } = self.safeAreaInsets();
        EdgeInsets::new(top, left, bottom, right)
    }

    /// When intrinsic size is tracked under Auto Layout, records the
    /// current width and invalidates it when the width changed.
    fn track_intrinsic_width(&self) {
        let ivars = self.ivars();
        if !ivars.intrinsic_auto_layout.get()
            || ivars.measure.borrow().is_none()
            || self.translatesAutoresizingMaskIntoConstraints()
        {
            return;
        }
        let width = self.bounds().size.width;
        if width > 0.0 {
            let previous = ivars.last_auto_layout_width.replace(width);
            if (previous - width).abs() > 0.0 {
                self.invalidateIntrinsicContentSize();
            }
        }
    }

    /// Tells the resize handler about the current size, once per size.
    fn report_size(&self) {
        let size = self.bounds().size;
        if self.ivars().reported_size.replace(size) == size {
            return;
        }
        let handler = self.ivars().resize.borrow().clone();
        if let Some(handler) = handler {
            handler(self, size.into());
        }
    }

    /// `UIKit`'s hit test, extended for a window root: when no subview claims
    /// the point, each subview is asked again, topmost first, with the point
    /// in its own coordinates, so content placed outside the root's bounds
    /// (into the safe area, say) still receives touches.
    fn default_hit_test(
        &self,
        point: CGPoint,
        event: Option<&UIEvent>,
    ) -> Option<Retained<UIView>> {
        // SAFETY: see the module safety note.
        let hit: Option<Retained<UIView>> =
            unsafe { msg_send![super(self), hitTest: point, withEvent: event] };
        let this: &UIView = self;
        if !self.ivars().fills_window.get() || !hit.as_deref().is_some_and(|hit| ptr::eq(hit, this))
        {
            return hit;
        }
        self.subviews()
            .to_vec()
            .into_iter()
            .rev()
            .find_map(|subview| {
                subview.hitTest_withEvent(self.convertPoint_toView(point, Some(&*subview)), event)
            })
            .or(hit)
    }
}

/// A host view that serves as a view controller's root view: it always fills
/// its window, reports its window's safe-area insets, and extends hit testing
/// to subviews placed outside its bounds.
pub fn window_root(mtm: MainThreadMarker) -> Retained<HostView> {
    HostView::with_ivars(
        mtm,
        Rect::ZERO,
        HostViewIvars {
            fills_window: Cell::new(true),
            ..HostViewIvars::default()
        },
    )
}
