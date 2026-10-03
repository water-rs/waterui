//! The view that hosts content laid out by Rust.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSView` subclass and forwards to `NSView`'s
//! own implementation of each method it overrides. Every override has the
//! signature `NSView` declares, and `AppKit` calls them on the main thread;
//! `isFlipped` may be asked from any thread and answers a constant.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::ptr;
use std::rc::Rc;

use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSDragOperation, NSDraggingInfo, NSEvent, NSScreen, NSTrackingArea, NSTrackingAreaOptions,
    NSView,
};
use objc2_foundation::{NSArray, NSEdgeInsets, NSObjectProtocol, NSPoint, NSRect, NSSize};

use super::drag_drop::{DragInfo, DropHandlers};
use crate::callback::guarded;
use crate::geometry::{EdgeInsets, MeasureProposal, Point, Rect, Size};
use crate::keys::{self, KeyEvent};
use crate::pointer::{PointerEvent, PointerEvents};

/// What a [`HostView`]'s hit-test handler decides for a point.
#[derive(Debug, Clone)]
pub enum HitTest {
    /// Whatever `AppKit` would decide: the deepest subview under the point,
    /// or the host view itself.
    Default,
    /// Nothing here: the event goes to whatever lies beneath the host view.
    Pass,
    /// Whatever `AppKit` would decide, except the host view itself: a
    /// container's own hits belong to whatever lies beneath it.
    PassIfSelf,
    /// This view receives the event.
    View(Retained<NSView>),
}

type LayoutHandler = Rc<dyn Fn(&HostView)>;
type ResizeHandler = Rc<dyn Fn(&HostView, Size)>;
type HitTestHandler = Rc<dyn Fn(&HostView, Point) -> HitTest>;
type WindowHandler = Rc<dyn Fn(&HostView)>;
type MeasureHandler = Rc<dyn Fn(&HostView, MeasureProposal) -> Size>;
type PrimaryContentHandler = Rc<dyn Fn(&HostView) -> Option<Retained<NSView>>>;
type ScrollSurfaceHandler = Rc<dyn Fn(&HostView) -> Vec<Retained<NSView>>>;
type HiddenHandler = Rc<dyn Fn(&HostView, bool)>;
type MouseHandler = Rc<dyn Fn(&HostView, &NSEvent)>;
type PointerHandler = Rc<dyn Fn(&HostView, PointerEvent) -> bool>;
type KeyHandler = Rc<dyn Fn(&HostView, &KeyEvent) -> bool>;
type RightMouseHandler = Rc<dyn Fn(&HostView, &NSEvent)>;

/// The handlers a [`HostView`] calls.
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
    hidden: RefCell<Option<HiddenHandler>>,
    mouse_down: RefCell<Option<MouseHandler>>,
    mouse_dragged: RefCell<Option<MouseHandler>>,
    drop: RefCell<Option<Rc<DropHandlers>>>,
    pointer: RefCell<Option<PointerHandler>>,
    /// Which pointer events the pointer handler wants.
    pointer_events: Cell<PointerEvents>,
    /// Whether the pointer is currently over the view.
    pointer_inside: Cell<bool>,
    /// The tracking area serving the pointer handler, recreated by
    /// `updateTrackingAreas`.
    tracking_area: RefCell<Option<Retained<NSTrackingArea>>>,
    key: RefCell<Option<KeyHandler>>,
    right_mouse: RefCell<Option<RightMouseHandler>>,
    /// `viewDidChangeBackingProperties` subscribers — backing-scale changes.
    backing_changed: RefCell<Option<WindowHandler>>,
    /// Whether the view's own content is laid out against its bounds — the
    /// answer to "does this view manage its own safe area".
    manages_safe_area: std::cell::Cell<bool>,
    /// Whether the Auto Layout width is tracked for intrinsic size; see
    /// [`set_intrinsic_auto_layout`](HostView::set_intrinsic_auto_layout).
    intrinsic_auto_layout: std::cell::Cell<bool>,
    /// The width the intrinsic-content-size query was last invalidated for.
    last_auto_layout_width: std::cell::Cell<f64>,
}

impl fmt::Debug for HostViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostViewIvars")
            .field("layout", &self.layout.borrow().is_some())
            .field("resize", &self.resize.borrow().is_some())
            .field("hit_test", &self.hit_test.borrow().is_some())
            .field("window", &self.window.borrow().is_some())
            .field("superview", &self.superview.borrow().is_some())
            .field(
                "scroll_surface_candidates",
                &self.scroll_surface_candidates.borrow().is_some(),
            )
            .field("primary_content", &self.primary_content.borrow().is_some())
            .field("hidden", &self.hidden.borrow().is_some())
            .field("mouse_down", &self.mouse_down.borrow().is_some())
            .field("mouse_dragged", &self.mouse_dragged.borrow().is_some())
            .field("drop", &self.drop.borrow().is_some())
            .field("backing_changed", &self.backing_changed.borrow().is_some())
            .field("last_auto_layout_width", &self.last_auto_layout_width.get())
            .field("measure", &self.measure.borrow().is_some())
            .field("manages_safe_area", &self.manages_safe_area.get())
            .field("intrinsic_auto_layout", &self.intrinsic_auto_layout.get())
            .field("pointer", &self.pointer.borrow().is_some())
            .field("pointer_events", &self.pointer_events.get())
            .field("pointer_inside", &self.pointer_inside.get())
            .field("tracking_area", &self.tracking_area.borrow().is_some())
            .field("key", &self.key.borrow().is_some())
            .field("right_mouse", &self.right_mouse.borrow().is_some())
            .finish()
    }
}

define_class!(
    // SAFETY: `NSView` asks a subclass to initialize through its designated
    // initializer, which `HostView::new` does, and the class does not
    // implement `Drop`.
    #[unsafe(super(NSView))]
    #[name = "CocoaUiHostView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = HostViewIvars]
    #[derive(Debug)]
    /// A layer-backed view whose layout, resizing and hit testing are Rust
    /// closures.
    ///
    /// Its coordinates are flipped: the origin is the top-left corner and `y`
    /// grows downward, as on iOS. Place subviews from the layout handler,
    /// which runs whenever `AppKit` lays the view out.
    pub struct HostView;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSView` subclass.
    unsafe impl NSObjectProtocol for HostView {}

    impl HostView {
        // SAFETY: see the module safety note.
        #[unsafe(method(isFlipped))]
        fn is_flipped_override(&self) -> bool {
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(layout))]
        fn layout_override(&self) {
            guarded("HostView layout", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), layout] };
                let handler = self.ivars().layout.borrow().clone();
                if let Some(handler) = handler {
                    handler(self);
                }
                self.track_intrinsic_width();
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(viewDidMoveToSuperview))]
        fn view_did_move_to_superview_override(&self) {
            guarded("HostView viewDidMoveToSuperview", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), viewDidMoveToSuperview] };
                let handler = self.ivars().superview.borrow().clone();
                if let Some(handler) = handler {
                    handler(self);
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(fittingSize))]
        fn fitting_size_override(&self) -> NSSize {
            guarded("HostView fittingSize", || {
                if let Some(handler) = self.ivars().measure.borrow().clone() {
                    return handler(self, MeasureProposal::UNBOUNDED).into();
                }
                // SAFETY: see the module safety note.
                unsafe { msg_send![super(self), fittingSize] }
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(intrinsicContentSize))]
        fn intrinsic_content_size_override(&self) -> NSSize {
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
                        return NSSize::new(intrinsic.width, constrained.height);
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
        // selector for the sibling backend's primary-content chain.
        #[unsafe(method_id(cocoaUiPrimaryContent))]
        fn primary_content_override(&self) -> Option<Retained<NSView>> {
            let handler = self.ivars().primary_content.borrow().clone();
            handler.and_then(|handler| handler(self))
        }

        // SAFETY: see the module safety note. Exposed under a `cocoaUi`
        // selector for the sibling backend's scroll-surface search.
        #[unsafe(method_id(cocoaUiScrollSurfaceCandidates))]
        fn scroll_surface_candidates_override(&self) -> Retained<NSArray<NSView>> {
            let handler = self.ivars().scroll_surface_candidates.borrow().clone();
            NSArray::from_retained_slice(&handler.map_or_else(Vec::new, |handler| handler(self)))
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(setFrameSize:))]
        fn set_frame_size_override(&self, new_size: NSSize) {
            guarded("HostView setFrameSize:", || {
                let old_size = self.frame().size;
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), setFrameSize: new_size] };
                if old_size == new_size {
                    return;
                }
                let handler = self.ivars().resize.borrow().clone();
                if let Some(handler) = handler {
                    handler(self, new_size.into());
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(setHidden:))]
        fn set_hidden_override(&self, hidden: bool) {
            guarded("HostView setHidden:", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), setHidden: hidden] };
                let handler = self.ivars().hidden.borrow().clone();
                if let Some(handler) = handler {
                    handler(self, hidden);
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window_override(&self) {
            guarded("HostView viewDidMoveToWindow", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), viewDidMoveToWindow] };
                let handler = self.ivars().window.borrow().clone();
                if let Some(handler) = handler {
                    handler(self);
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseDown:))]
        fn mouse_down_override(&self, event: &NSEvent) {
            guarded("HostView mouseDown:", || {
                let handler = self.ivars().mouse_down.borrow().clone();
                if let Some(handler) = handler {
                    handler(self, event);
                } else {
                    // SAFETY: see the module safety note.
                    let _: () = unsafe { msg_send![super(self), mouseDown: event] };
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged_override(&self, event: &NSEvent) {
            guarded("HostView mouseDragged:", || {
                let handler = self.ivars().mouse_dragged.borrow().clone();
                if let Some(handler) = handler {
                    handler(self, event);
                } else {
                    // SAFETY: see the module safety note.
                    let _: () = unsafe { msg_send![super(self), mouseDragged: event] };
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered_override(
            &self,
            sender: &ProtocolObject<dyn NSDraggingInfo>,
        ) -> NSDragOperation {
            guarded("HostView draggingEntered:", || {
                self.ivars().drop.borrow().clone().map_or_else(
                    // SAFETY: see the module safety note.
                    || unsafe { msg_send![super(self), draggingEntered: sender] },
                    |handlers| (handlers.entered)(&DragInfo::new(sender)),
                )
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated_override(
            &self,
            sender: &ProtocolObject<dyn NSDraggingInfo>,
        ) -> NSDragOperation {
            guarded("HostView draggingUpdated:", || {
                self.ivars().drop.borrow().clone().map_or_else(
                    // SAFETY: see the module safety note.
                    || unsafe { msg_send![super(self), draggingUpdated: sender] },
                    |handlers| (handlers.updated)(&DragInfo::new(sender)),
                )
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(draggingExited:))]
        fn dragging_exited_override(&self, sender: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
            guarded("HostView draggingExited:", || {
                if let Some(handlers) = self.ivars().drop.borrow().clone() {
                    if let Some(sender) = sender {
                        (handlers.exited)(&DragInfo::new(sender));
                    }
                } else {
                    // SAFETY: see the module safety note.
                    let _: () = unsafe { msg_send![super(self), draggingExited: sender] };
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation_override(
            &self,
            sender: &ProtocolObject<dyn NSDraggingInfo>,
        ) -> bool {
            guarded("HostView performDragOperation:", || {
                self.ivars().drop.borrow().clone().map_or_else(
                    // SAFETY: see the module safety note.
                    || unsafe { msg_send![super(self), performDragOperation: sender] },
                    |handlers| (handlers.perform)(&DragInfo::new(sender)),
                )
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(viewDidChangeBackingProperties))]
        fn view_did_change_backing_properties(&self) {
            guarded("HostView viewDidChangeBackingProperties", || {
                // SAFETY: see the module safety note.
                let _: () =
                    unsafe { msg_send![super(self), viewDidChangeBackingProperties] };
                let handler = self.ivars().backing_changed.borrow().clone();
                if let Some(handler) = handler {
                    handler(self);
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method_id(hitTest:))]
        fn hit_test_override(&self, point: NSPoint) -> Option<Retained<NSView>> {
            guarded("HostView hitTest:", || {
                let handler = self.ivars().hit_test.borrow().clone();
                match handler.map_or(HitTest::Default, |handler| handler(self, point.into())) {
                    // SAFETY: see the module safety note.
                    HitTest::Default => unsafe { msg_send![super(self), hitTest: point] },
                    HitTest::Pass => None,
                    HitTest::PassIfSelf => {
                        let this: &NSView = self;
                        // SAFETY: see the module safety note.
                        let hit: Option<Retained<NSView>> =
                            unsafe { msg_send![super(self), hitTest: point] };
                        hit.filter(|hit| !ptr::eq(&raw const **hit, this))
                    }
                    HitTest::View(view) => Some(view),
                }
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas_override(&self) {
            guarded("HostView updateTrackingAreas", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), updateTrackingAreas] };
                if let Some(area) = self.ivars().tracking_area.replace(None) {
                    self.removeTrackingArea(&area);
                }
                let events = self.ivars().pointer_events.get();
                if events == PointerEvents::NONE || self.ivars().pointer.borrow().is_none() {
                    return;
                }
                let mut options = NSTrackingAreaOptions::ActiveInKeyWindow
                    | NSTrackingAreaOptions::InVisibleRect;
                if events.wants(PointerEvent::Entered) || events.wants(PointerEvent::Exited) {
                    options |= NSTrackingAreaOptions::MouseEnteredAndExited;
                }
                if events.wants(PointerEvent::Moved(Point::ZERO)) {
                    options |= NSTrackingAreaOptions::MouseMoved;
                }
                if events.wants(PointerEvent::CursorUpdate) {
                    options |= NSTrackingAreaOptions::CursorUpdate;
                }
                // SAFETY: `self` owns the area until `updateTrackingAreas`
                // removes it, and the options describe it.
                let area = unsafe {
                    NSTrackingArea::initWithRect_options_owner_userInfo(
                        NSTrackingArea::alloc(),
                        NSRect::ZERO,
                        options,
                        Some(AsRef::<objc2::runtime::AnyObject>::as_ref(self)),
                        None,
                    )
                };
                self.addTrackingArea(&area);
                self.ivars().tracking_area.replace(Some(area));
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseEntered:))]
        fn mouse_entered_override(&self, event: &NSEvent) {
            guarded("HostView mouseEntered:", || {
                self.ivars().pointer_inside.set(true);
                self.deliver_pointer(PointerEvent::Entered, event);
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved_override(&self, event: &NSEvent) {
            guarded("HostView mouseMoved:", || {
                // SAFETY: `event` is a live mouse event.
                let point = self.convertPoint_fromView(event.locationInWindow(), None);
                self.deliver_pointer(PointerEvent::Moved(point.into()), event);
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseExited:))]
        fn mouse_exited_override(&self, event: &NSEvent) {
            guarded("HostView mouseExited:", || {
                self.ivars().pointer_inside.set(false);
                self.deliver_pointer(PointerEvent::Exited, event);
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(cursorUpdate:))]
        fn cursor_update_override(&self, event: &NSEvent) {
            guarded("HostView cursorUpdate:", || {
                if self.deliver_pointer(PointerEvent::CursorUpdate, event) {
                    return;
                }
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), cursorUpdate: event] };
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(keyDown:))]
        fn key_down_override(&self, event: &NSEvent) {
            guarded("HostView keyDown:", || {
                let handler = self.ivars().key.borrow().clone();
                let key = keys::key_event(event);
                if handler.is_some_and(|handler| handler(self, &key)) {
                    return;
                }
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), keyDown: event] };
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down_override(&self, event: &NSEvent) {
            guarded("HostView rightMouseDown:", || {
                let handler = self.ivars().right_mouse.borrow().clone();
                if let Some(handler) = handler {
                    handler(self, event);
                    return;
                }
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), rightMouseDown: event] };
            });
        }
    }
);

impl HostView {
    /// A host view occupying `frame` in its future superview's coordinates.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, frame: Rect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(HostViewIvars::default());
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer.
        let view: Retained<Self> =
            unsafe { msg_send![super(this), initWithFrame: NSRect::from(frame)] };
        view.setWantsLayer(true);
        view
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

    /// Lets `handler` decide which view receives a mouse event at a point,
    /// replacing any handler set before.
    ///
    /// The point is in the coordinates of the host view's superview, as
    /// `AppKit` hit testing is.
    pub fn set_hit_test_handler(&self, handler: impl Fn(&Self, Point) -> HitTest + 'static) {
        self.ivars().hit_test.replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` when the view's backing store properties change —
    /// typically a move to a differently scaled display.
    pub fn set_backing_changed_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().backing_changed.replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` every time the view moves into or out of a window —
    /// `viewDidMoveToWindow`, the point where a focus request becomes
    /// possible or is lost.
    pub fn set_window_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().window.replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` every time the view moves into or out of a superview —
    /// `viewDidMoveToSuperview`, the point where an enclosing scroll surface
    /// may have changed.
    pub fn set_superview_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().superview.replace(Some(Rc::new(handler)));
    }

    /// Sends `event` to the pointer handler when it wants it; returns
    /// whether it consumed it.
    fn deliver_pointer(&self, event: PointerEvent, _native: &NSEvent) -> bool {
        if !self.ivars().pointer_events.get().wants(event) {
            return false;
        }
        let handler = self.ivars().pointer.borrow().clone();
        handler.is_some_and(|handler| handler(self, event))
    }

    /// Calls `handler` for each pointer `event` in `events`, replacing any
    /// handler set before.
    ///
    /// A tracking area covering the view's visible rect serves the events;
    /// [`PointerEvent::Moved`]'s point is in the view's own coordinates.
    /// The handler's return value is only meaningful for
    /// [`PointerEvent::CursorUpdate`]: `true` stops the event there,
    /// `false` lets `AppKit` continue to the next responder.
    pub fn set_pointer_handler(
        &self,
        events: PointerEvents,
        handler: impl Fn(&Self, PointerEvent) -> bool + 'static,
    ) {
        self.ivars().pointer.replace(Some(Rc::new(handler)));
        self.ivars().pointer_events.set(events);
        self.updateTrackingAreas();
    }

    /// Whether the pointer is currently over this view.
    #[must_use]
    pub fn is_pointer_inside(&self) -> bool {
        self.ivars().pointer_inside.get()
    }

    /// Lets `handler` decide whether a key press on this view is consumed,
    /// replacing any handler set before. A consumed `keyDown` goes no
    /// further; an unconsumed one is passed up the responder chain.
    pub fn set_key_handler(&self, handler: impl Fn(&Self, &KeyEvent) -> bool + 'static) {
        self.ivars().key.replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` on a secondary click (`rightMouseDown`), replacing
    /// any handler set before.
    pub fn set_right_mouse_handler(&self, handler: impl Fn(&Self, &NSEvent) + 'static) {
        self.ivars().right_mouse.replace(Some(Rc::new(handler)));
    }

    /// Lets `handler` answer the view's intrinsic measurements,
    /// `fittingSize` and `intrinsicContentSize`, for a layout container.
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

    /// The primary content the sibling backend's wrappers descend to — the
    /// answer `cocoaUiPrimaryContent` reports, and the link a scroll-surface
    /// or safe-area query follows into the view.
    pub fn set_primary_content_handler(
        &self,
        handler: impl Fn(&Self) -> Option<Retained<NSView>> + 'static,
    ) {
        self.ivars().primary_content.replace(Some(Rc::new(handler)));
    }

    /// The children that may be the scroll surface surrounding bars follow —
    /// the answer `cocoaUiScrollSurfaceCandidates` reports.
    pub fn set_scroll_surface_handler(
        &self,
        handler: impl Fn(&Self) -> Vec<Retained<NSView>> + 'static,
    ) {
        self.ivars()
            .scroll_surface_candidates
            .replace(Some(Rc::new(handler)));
    }

    /// Runs `handler` when the view's `isHidden` flag changes — how a tab
    /// container tells a navigation stack inside the hidden page to release
    /// the window toolbar, `setNavigationChromeActive(_:)`'s equivalent.
    pub fn set_hidden_handler(&self, handler: impl Fn(&Self, bool) + 'static) {
        self.ivars().hidden.replace(Some(Rc::new(handler)));
    }

    /// Whether a hidden handler is registered — a container walking a pane's
    /// subtree flips only the views that asked for the notification.
    #[must_use]
    pub fn wants_hidden_events(&self) -> bool {
        self.ivars().hidden.borrow().is_some()
    }

    /// Calls `handler` on `mouseDown`, replacing any handler set before.
    /// Without a handler the event goes to `NSView`'s implementation.
    pub fn set_mouse_down_handler(&self, handler: impl Fn(&Self, &NSEvent) + 'static) {
        self.ivars().mouse_down.replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` on `mouseDragged`, replacing any handler set before.
    /// Without a handler the event goes to `NSView`'s implementation.
    pub fn set_mouse_dragged_handler(&self, handler: impl Fn(&Self, &NSEvent) + 'static) {
        self.ivars().mouse_dragged.replace(Some(Rc::new(handler)));
    }

    /// Makes the view a drop destination reporting to `handlers`, replacing
    /// any handlers set before.
    ///
    /// `types` are the pasteboard types the view registers for; an empty
    /// slice unregisters the view as a destination.
    pub fn set_drop_handlers(
        &self,
        types: &[&objc2_foundation::NSString],
        handlers: Option<DropHandlers>,
    ) {
        if let Some(handlers) = handlers {
            self.ivars().drop.replace(Some(Rc::new(handlers)));
            self.registerForDraggedTypes(&NSArray::from_slice(types));
        } else {
            self.ivars().drop.take();
            self.unregisterDraggedTypes();
        }
    }

    /// Adds `view` above the existing subviews.
    pub fn add_subview(&self, view: &NSView) {
        self.addSubview(view);
    }

    /// Removes `view`, one of this view's subviews.
    ///
    /// # Panics
    ///
    /// If `view` is not a subview of this view.
    pub fn remove_subview(&self, view: &NSView) {
        let this: &NSView = self;
        // SAFETY: a main-thread read of the view's superview, which the view
        // hierarchy keeps alive while `view` is in it.
        let parent = unsafe { view.superview() };
        assert!(
            parent.is_some_and(|parent| ptr::eq(&raw const *parent, this)),
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
        self.setNeedsLayout(true);
    }

    /// Runs any pending layout pass of this view and its subviews now.
    pub fn layout_if_needed(&self) {
        self.layoutSubtreeIfNeeded();
    }

    /// Device pixels per point where the view is drawn: its window's backing
    /// scale, or the main screen's before it is in a window.
    ///
    /// `None` when the view is in no window and the system has no screen.
    #[must_use]
    pub fn display_scale(&self) -> Option<f64> {
        self.window().map_or_else(
            || NSScreen::mainScreen(self.mtm()).map(|screen| screen.backingScaleFactor()),
            |window| Some(window.backingScaleFactor()),
        )
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

    /// The distances from each edge within which content is obscured by the
    /// window's title bar or toolbar.
    #[must_use]
    pub fn safe_area_insets(&self) -> EdgeInsets {
        let NSEdgeInsets {
            top,
            left,
            bottom,
            right,
        } = self.safeAreaInsets();
        EdgeInsets::new(top, left, bottom, right)
    }
}
