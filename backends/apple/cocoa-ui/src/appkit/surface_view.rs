//! A view whose pixels a GPU pipeline owns.
//!
//! [`SurfaceView`] is the host a render surface attaches to: it layer-hosts a
//! presentation layer the renderer presents into, and reports the geometry,
//! visibility and pointer changes a frame loop needs — a resized frame, a
//! moved window, a hidden view, a hovered pointer, pinch and pan gestures.
//! Its content arrives out of band (presented into the layer), so the view
//! itself never draws.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSView` subclass plus the `NSObject` target
//! the magnification recognizer talks to. `AppKit` calls every override on the
//! main thread; the gestures install and remove with the view.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::sel;
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
};
use objc2_app_kit::{
    NSEvent, NSEventPhase, NSMagnificationGestureRecognizer, NSResponder, NSTrackingArea,
    NSTrackingAreaOptions, NSView,
};
use objc2_foundation::NSRect;
use objc2_quartz_core::CALayer;

use crate::PlatformView;
use crate::callback::guarded;
use crate::input::{EventPhase, GesturePhase, PointerInteraction};

/// Receives lifecycle events of the view — layout, attachment, visibility.
type LifecycleHandler = Rc<dyn Fn()>;

/// Receives pointer and gesture input for the surface's own interaction.
type InteractionHandler = Rc<dyn Fn(PointerInteraction)>;

/// The ivars of a [`SurfaceView`].
#[derive(Default)]
pub struct SurfaceViewIvars {
    on_layout: RefCell<Option<LifecycleHandler>>,
    on_window_changed: RefCell<Option<LifecycleHandler>>,
    on_backing_changed: RefCell<Option<LifecycleHandler>>,
    on_visibility_changed: RefCell<Option<LifecycleHandler>>,
    on_interaction: RefCell<Option<InteractionHandler>>,
    tracking_area: RefCell<Option<Retained<NSTrackingArea>>>,
    /// The layer the renderer presents frames into, owned by the view's
    /// host layer.
    presentation_layer: RefCell<Option<Retained<CALayer>>>,
    gesture_target: RefCell<Option<Retained<GestureTarget>>>,
}

impl fmt::Debug for SurfaceViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SurfaceViewIvars").finish_non_exhaustive()
    }
}

/// Receives a pinch recognizer's phase, magnitude and surface-local center.
type PinchHandler = Rc<dyn Fn(GesturePhase, f64, kurbo::Point)>;

/// The ivars of a [`GestureTarget`].
struct GestureTargetIvars {
    on_pinch: RefCell<Option<PinchHandler>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements; the class holds a
    // main-thread closure and does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiSurfaceGestureTarget"]
    #[thread_kind = MainThreadOnly]
    #[ivars = GestureTargetIvars]
    /// Receives the magnification recognizer's action for a [`SurfaceView`].
    struct GestureTarget;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for GestureTarget {}

    impl GestureTarget {
        // SAFETY: `pinch:` is the `NSGestureRecognizer` action signature.
        #[unsafe(method(pinch:))]
        fn pinch(&self, gesture: &NSMagnificationGestureRecognizer) {
            guarded("CocoaUiSurfaceGestureTarget pinch:", || {
                use objc2_app_kit::NSGestureRecognizerState as State;
                let Some(handler) = self.ivars().on_pinch.borrow().clone() else {
                    return;
                };
                let Some(view) = gesture.view().and_then(|v| v.downcast::<NSView>().ok())
                else {
                    return;
                };
                let phase = match gesture.state() {
                    State::Began => GesturePhase::Began,
                    State::Changed => GesturePhase::Changed,
                    State::Ended => GesturePhase::Ended,
                    State::Cancelled | State::Failed => GesturePhase::Cancelled,
                    _ => return,
                };
                let location = gesture.locationInView(Some(&view));
                let center = kurbo::Point::new(
                    location.x,
                    view.bounds().size.height - location.y,
                );
                handler(phase, gesture.magnification(), center);
            });
        }
    }
);

impl fmt::Debug for GestureTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GestureTarget").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSView` asks a subclass to initialize through its designated
    // initializer, which `SurfaceView::new` does, and the class does not
    // implement `Drop`.
    #[unsafe(super(NSView))]
    #[name = "CocoaUiSurfaceView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SurfaceViewIvars]
    #[derive(Debug)]
    /// The layer-hosting view a GPU pipeline presents into.
    ///
    /// The view is flipped and always layer-backed: its assigned layer hosts
    /// the presentation layer frames land in.
    pub struct SurfaceView;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSView` subclass.
    unsafe impl NSObjectProtocol for SurfaceView {}

    impl SurfaceView {
        // SAFETY: see the module safety note.
        #[unsafe(method(isFlipped))]
        fn is_flipped_override(&self) -> bool {
            true
        }

        // SAFETY: see the module safety note; the view is always layer-backed.
        #[unsafe(method(wantsLayer))]
        fn wants_layer_override(&self) -> bool {
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(setWantsLayer:))]
        fn set_wants_layer_override(&self, wants_layer: bool) {
            let _ = wants_layer;
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder_override(&self) -> bool {
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(layout))]
        fn layout_override(&self) {
            // SAFETY: `super(layout)` forwards to `NSView`.
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), layout] };
            self.emit_layout();
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), viewDidMoveToWindow] };
            let handler = self.ivars().on_window_changed.borrow().clone();
            if let Some(handler) = handler {
                handler();
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(viewDidChangeBackingProperties))]
        fn view_did_change_backing_properties(&self) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), viewDidChangeBackingProperties] };
            let handler = self.ivars().on_backing_changed.borrow().clone();
            if let Some(handler) = handler {
                handler();
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(viewDidHide))]
        fn view_did_hide(&self) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), viewDidHide] };
            let handler = self.ivars().on_visibility_changed.borrow().clone();
            if let Some(handler) = handler {
                handler();
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(viewDidUnhide))]
        fn view_did_unhide(&self) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), viewDidUnhide] };
            let handler = self.ivars().on_visibility_changed.borrow().clone();
            if let Some(handler) = handler {
                handler();
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas_override(&self) {
            guarded("SurfaceView updateTrackingAreas", || {
                // SAFETY: see the module safety note.
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), updateTrackingAreas] };
                if let Some(area) = self.ivars().tracking_area.borrow_mut().take() {
                    self.removeTrackingArea(&area);
                }
                if self.window().is_none() {
                    return;
                }
                // `.activeInKeyWindow` on purpose: pointer position feeds the
                // renderer's hover state, and a background window has no
                // hover to show. `.inVisibleRect` keeps the area in step with
                // the view's visible bounds.
                // SAFETY: see the module safety note.
                let area = unsafe {
                    NSTrackingArea::initWithRect_options_owner_userInfo(
                        NSTrackingArea::alloc(),
                        NSRect::ZERO,
                        NSTrackingAreaOptions::MouseEnteredAndExited
                            | NSTrackingAreaOptions::MouseMoved
                            | NSTrackingAreaOptions::ActiveInKeyWindow
                            | NSTrackingAreaOptions::InVisibleRect,
                        Some(self.as_ref()),
                        None,
                    )
                };
                self.addTrackingArea(&area);
                self.ivars().tracking_area.replace(Some(area));
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, event: &NSEvent) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), mouseEntered: event] };
            self.send_moved(event, true);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), mouseMoved: event] };
            self.send_moved(event, true);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, event: &NSEvent) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), mouseExited: event] };
            self.emit(PointerInteraction::Moved(None));
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), mouseDragged: event] };
            self.send_moved(event, true);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if let Some(window) = self.window() {
                let this: &NSResponder = self;
                window.makeFirstResponder(Some(this));
            }
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), mouseDown: event] };
            let position = self.local_point(event);
            self.emit(PointerInteraction::PrimaryDown {
                position,
                click_count: event.clickCount() as i64,
            });
            if event.clickCount() == 2 {
                self.emit(PointerInteraction::DoubleTap);
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), mouseUp: event] };
            self.emit(PointerInteraction::PrimaryUp);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            // Inside an `NSScrollView` the wheel keeps scrolling natively;
            // Option routes it to the surface's gesture channel instead.
            let explicit_surface_pan = event
                .modifierFlags()
                .contains(objc2_app_kit::NSEventModifierFlags::Option);
            if self.enclosingScrollView().is_some() && !explicit_surface_pan {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), scrollWheel: event] };
                return;
            }
            let delta_x = event.scrollingDeltaX();
            let delta_y = event.scrollingDeltaY();
            let phase = match event.phase() {
                NSEventPhase::Began => EventPhase::Began,
                NSEventPhase::Changed => EventPhase::Changed,
                NSEventPhase::Ended | NSEventPhase::Cancelled => EventPhase::Ended,
                _ => EventPhase::None,
            };
            self.emit(PointerInteraction::Pan {
                phase,
                offset_x: delta_x,
                offset_y: delta_y,
            });
        }
    }
);

impl SurfaceView {
    /// A layer-hosted surface host.
    ///
    /// The returned view's [`presentation_layer`](Self::presentation_layer)
    /// is where frames present; it already fills the view's host layer.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SurfaceViewIvars::default());
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };

        // The presentation layer is opaque-free and stretched to fit; frames
        // are rendered at device-pixel size, so the layer must not rescale
        // them — `contentsScale` carries that.
        let host = CALayer::new();
        let presentation = CALayer::new();
        presentation.setOpaque(false);
        // SAFETY: `kCAGravityResize` is a system constant.
        presentation.setContentsGravity(unsafe { objc2_quartz_core::kCAGravityResize });
        host.addSublayer(&presentation);
        view.setLayer(Some(&host));
        view.setWantsLayer(true);
        view.ivars().presentation_layer.replace(Some(presentation));

        let target: Retained<GestureTarget> = {
            let this = GestureTarget::alloc(mtm).set_ivars(GestureTargetIvars {
                on_pinch: RefCell::new(None),
            });
            // SAFETY: `init` is `NSObject`'s designated initializer.
            unsafe { msg_send![super(this), init] }
        };
        // SAFETY: `initWithTarget:action:` retains the recognizer↔target
        // pair; the view owns the target through `gesture_target`.
        let recognizer = unsafe {
            NSMagnificationGestureRecognizer::initWithTarget_action(
                NSMagnificationGestureRecognizer::alloc(mtm),
                Some(&*target),
                Some(sel!(pinch:)),
            )
        };
        view.addGestureRecognizer(&recognizer);
        view.ivars().gesture_target.replace(Some(target));
        view
    }

    /// The layer a renderer presents frames into.
    ///
    /// # Panics
    ///
    /// When the presentation layer is gone — it is created in `new` and never
    /// removed, so this cannot happen in a live view.
    #[must_use]
    pub fn presentation_layer(&self) -> Retained<CALayer> {
        self.ivars()
            .presentation_layer
            .borrow()
            .clone()
            .expect("presentation layer is created in `new`")
    }

    /// The view's bounds size in logical units.
    #[must_use]
    pub fn bounds_size(&self) -> crate::geometry::Size {
        crate::view::bounds(self).size
    }

    /// The window's backing scale — physical pixels per logical unit —
    /// `None` while the view is off-window.
    #[must_use]
    pub fn backing_scale(&self) -> Option<f64> {
        crate::view::window(self).map(|window| window.backingScaleFactor())
    }

    /// Whether the view can present: on a visible window that is not
    /// occluded, and not hidden inside the hierarchy.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        if self.isHiddenOrHasHiddenAncestor() {
            return false;
        }
        crate::view::window(self).is_some_and(|window| {
            window.isVisible()
                && window
                    .occlusionState()
                    .contains(objc2_app_kit::NSWindowOcclusionState::Visible)
        })
    }

    /// The view's bounds in `to`'s coordinate space.
    #[must_use]
    pub fn bounds_in(&self, to: &PlatformView) -> crate::geometry::Rect {
        crate::view::convert_rect(self, crate::view::bounds(self), Some(to))
    }

    /// The view as its platform view.
    #[must_use]
    pub fn as_platform_view(&self) -> &PlatformView {
        self
    }

    /// Calls `handler` after every layout pass.
    pub fn set_layout_handler(&self, handler: impl Fn() + 'static) {
        self.ivars().on_layout.replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` when the view moves into or out of a window.
    pub fn set_window_changed_handler(&self, handler: impl Fn() + 'static) {
        self.ivars()
            .on_window_changed
            .replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` when the backing store properties change — typically
    /// a display move with a different scale factor.
    pub fn set_backing_changed_handler(&self, handler: impl Fn() + 'static) {
        self.ivars()
            .on_backing_changed
            .replace(Some(Rc::new(handler)));
    }

    /// Calls `handler` when the view's effective visibility flips.
    pub fn set_visibility_changed_handler(&self, handler: impl Fn() + 'static) {
        self.ivars()
            .on_visibility_changed
            .replace(Some(Rc::new(handler)));
    }

    /// Installs `handler` as the receiver of pointer and gesture input for
    /// the surface's own interaction, and the pinch recognizer's target.
    pub fn set_interaction_handler(&self, handler: impl Fn(PointerInteraction) + 'static) {
        let handler: InteractionHandler = Rc::new(handler);
        if let Some(target) = self.ivars().gesture_target.borrow().as_ref() {
            let pinch = handler.clone();
            target
                .ivars()
                .on_pinch
                .replace(Some(Rc::new(move |phase, magnitude, center| {
                    pinch(PointerInteraction::Pinch {
                        phase,
                        magnitude,
                        center,
                    });
                })));
        }
        self.ivars().on_interaction.replace(Some(handler));
    }

    fn emit(&self, interaction: PointerInteraction) {
        let handler = self.ivars().on_interaction.borrow().clone();
        if let Some(handler) = handler {
            handler(interaction);
        }
    }

    fn emit_layout(&self) {
        let handler = self.ivars().on_layout.borrow().clone();
        if let Some(handler) = handler {
            handler();
        }
    }

    /// The event position in logical, surface-local points with y growing
    /// down.
    fn local_point(&self, event: &NSEvent) -> kurbo::Point {
        let point = self.convertPoint_fromView(event.locationInWindow(), None);
        kurbo::Point::new(point.x, self.bounds().size.height - point.y)
    }

    fn send_moved(&self, event: &NSEvent, inside: bool) {
        let _ = inside;
        self.emit(PointerInteraction::Moved(Some(self.local_point(event))));
    }
}
