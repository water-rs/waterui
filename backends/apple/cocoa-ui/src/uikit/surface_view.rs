//! A view whose pixels a GPU pipeline owns.
//!
//! [`SurfaceView`] is the host a render surface attaches to: it hosts a
//! presentation layer the renderer presents into, and reports the geometry,
//! attachment and pointer changes a frame loop needs — a resized frame, a
//! moved window, hover/touch position, pinch, pan and double-tap gestures.
//! Its content arrives out of band (presented into the layer), so the view
//! itself never draws.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UIView` subclass plus the `NSObject` target
//! the gesture recognizers talk to, and implements
//! `UIGestureRecognizerDelegate` so pinch and pan fire together. `UIKit` calls
//! every override on the main thread; the recognizers install and remove
//! with the view.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::PlatformView;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::sel;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::NSSet;
use objc2_quartz_core::CALayer;
use objc2_ui_kit::{
    UIGestureRecognizer, UIGestureRecognizerDelegate, UIGestureRecognizerState,
    UIHoverGestureRecognizer, UIPanGestureRecognizer, UIPinchGestureRecognizer, UIScrollView,
    UITapGestureRecognizer, UITouch, UITraitCollection, UIView,
};

use crate::callback::guarded;
use crate::input::{EventPhase, GesturePhase, PointerInteraction};

/// Receives lifecycle events of the view — layout, attachment.
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
    /// The layer the renderer presents frames into, a sublayer of `layer`.
    presentation_layer: RefCell<Option<Retained<CALayer>>>,
    gesture_target: RefCell<Option<Retained<GestureTarget>>>,
    recognizers: RefCell<Vec<Retained<UIGestureRecognizer>>>,
}

impl fmt::Debug for SurfaceViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SurfaceViewIvars").finish_non_exhaustive()
    }
}

/// A hover callback: pointer position or exit.
type HoverHandler = Rc<dyn Fn(Option<kurbo::Point>)>;
type TapHandler = Rc<dyn Fn()>;
/// The ivars of a [`GestureTarget`].
#[derive(Default)]
struct GestureTargetIvars {
    hover: RefCell<Option<HoverHandler>>,
    pinch: RefCell<Option<InteractionHandler>>,
    pan: RefCell<Option<InteractionHandler>>,
    double_tap: RefCell<Option<TapHandler>>,
}

impl fmt::Debug for GestureTargetIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GestureTargetIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements; the class holds
    // main-thread closures and does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiSurfaceGestureTarget"]
    #[thread_kind = MainThreadOnly]
    #[ivars = GestureTargetIvars]
    #[derive(Debug)]
    /// Receives the gesture recognizers' actions for a [`SurfaceView`].
    struct GestureTarget;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for GestureTarget {}

    impl GestureTarget {
        // SAFETY: `hover:` is the `UIGestureRecognizer` action signature.
        #[unsafe(method(hover:))]
        fn hover(&self, gesture: &UIHoverGestureRecognizer) {
            guarded("CocoaUiSurfaceGestureTarget hover:", || {
                let Some(handler) = self.ivars().hover.borrow().clone() else {
                    return;
                };
                match gesture.state() {
                    UIGestureRecognizerState::Began | UIGestureRecognizerState::Changed => {
                        let location = gesture.locationInView(gesture.view().as_deref());
                        handler(Some(kurbo::Point::new(location.x, location.y)));
                    }
                    UIGestureRecognizerState::Ended | UIGestureRecognizerState::Cancelled => {
                        handler(None);
                    }
                    _ => {}
                }
            });
        }

        // SAFETY: `pinch:` is the `UIGestureRecognizer` action signature.
        #[unsafe(method(pinch:))]
        fn pinch(&self, gesture: &UIPinchGestureRecognizer) {
            guarded("CocoaUiSurfaceGestureTarget pinch:", || {
                let Some(handler) = self.ivars().pinch.borrow().clone() else {
                    return;
                };
                let Some(phase) = gesture_phase(gesture.state()) else {
                    return;
                };
                let location = gesture.locationInView(gesture.view().as_deref());
                handler(PointerInteraction::Pinch {
                    phase,
                    magnitude: gesture.scale(),
                    center: kurbo::Point::new(location.x, location.y),
                });
            });
        }

        // SAFETY: `pan:` is the `UIGestureRecognizer` action signature.
        #[unsafe(method(pan:))]
        fn pan(&self, gesture: &UIPanGestureRecognizer) {
            guarded("CocoaUiSurfaceGestureTarget pan:", || {
                let Some(handler) = self.ivars().pan.borrow().clone() else {
                    return;
                };
                let Some(phase) = gesture_phase(gesture.state()) else {
                    return;
                };
                let offset = gesture.translationInView(gesture.view().as_deref());
                handler(PointerInteraction::Pan {
                    phase: match phase {
                        GesturePhase::Began => EventPhase::Began,
                        GesturePhase::Changed => EventPhase::Changed,
                        GesturePhase::Ended | GesturePhase::Cancelled => EventPhase::Ended,
                    },
                    offset_x: offset.x,
                    offset_y: offset.y,
                });
            });
        }

        // SAFETY: `doubleTap:` is the `UIGestureRecognizer` action
        // signature.
        #[unsafe(method(doubleTap:))]
        fn double_tap(&self, gesture: &UITapGestureRecognizer) {
            guarded("CocoaUiSurfaceGestureTarget doubleTap:", || {
                if gesture.state() != UIGestureRecognizerState::Recognized {
                    return;
                }
                if let Some(handler) = self.ivars().double_tap.borrow().clone() {
                    handler();
                }
            });
        }
    }
);

const fn gesture_phase(state: UIGestureRecognizerState) -> Option<GesturePhase> {
    match state {
        UIGestureRecognizerState::Began => Some(GesturePhase::Began),
        UIGestureRecognizerState::Changed => Some(GesturePhase::Changed),
        UIGestureRecognizerState::Ended => Some(GesturePhase::Ended),
        UIGestureRecognizerState::Cancelled | UIGestureRecognizerState::Failed => {
            Some(GesturePhase::Cancelled)
        }
        _ => None,
    }
}

define_class!(
    // SAFETY: `UIView` asks a subclass to initialize through its designated
    // initializer, which `SurfaceView::new` does, and the class does not
    // implement `Drop`.
    #[unsafe(super(UIView))]
    #[name = "CocoaUiSurfaceView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SurfaceViewIvars]
    #[derive(Debug)]
    /// The layer-hosted view a GPU pipeline presents into.
    pub struct SurfaceView;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIView` subclass.
    unsafe impl NSObjectProtocol for SurfaceView {}

    // SAFETY: `UIGestureRecognizerDelegate`'s methods are implemented with
    // their declared signatures.
    unsafe impl UIGestureRecognizerDelegate for SurfaceView {
        // SAFETY: both recognizers are UIKit's.
        #[unsafe(method(gestureRecognizer:shouldRecognizeSimultaneouslyWithGestureRecognizer:))]
        fn gesture_recognizer_should_recognize_simultaneously_with_gesture_recognizer(
            &self,
            gesture_recognizer: &UIGestureRecognizer,
            other_gesture_recognizer: &UIGestureRecognizer,
        ) -> bool {
            let this_scroll = gesture_recognizer
                .view()
                .is_some_and(|view| view.downcast_ref::<UIScrollView>().is_some());
            let other_scroll = other_gesture_recognizer
                .view()
                .is_some_and(|view| view.downcast_ref::<UIScrollView>().is_some());
            if this_scroll || other_scroll {
                return true.into();
            }
            // Pinch and pan fire together; anything else is exclusive.
            let is_pinch = gesture_recognizer.downcast_ref::<UIPinchGestureRecognizer>().is_some()
                || other_gesture_recognizer
                    .downcast_ref::<UIPinchGestureRecognizer>()
                    .is_some();
            let is_pan = gesture_recognizer.downcast_ref::<UIPanGestureRecognizer>().is_some()
                || other_gesture_recognizer
                    .downcast_ref::<UIPanGestureRecognizer>()
                    .is_some();
            is_pinch && is_pan
        }
    }

    impl SurfaceView {
        // SAFETY: see the module safety note.
        #[unsafe(method(layoutSubviews))]
        fn layout_subviews_override(&self) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), layoutSubviews] };
            let handler = self.ivars().on_layout.borrow().clone();
            if let Some(handler) = handler {
                handler();
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(didMoveToWindow))]
        fn did_move_to_window_override(&self) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), didMoveToWindow] };
            let handler = self.ivars().on_window_changed.borrow().clone();
            if let Some(handler) = handler {
                handler();
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(didMoveToSuperview))]
        fn did_move_to_superview_override(&self) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), didMoveToSuperview] };
            let handler = self.ivars().on_visibility_changed.borrow().clone();
            if let Some(handler) = handler {
                handler();
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(traitCollectionDidChange:))]
        fn trait_collection_did_change(&self, previous: Option<&UITraitCollection>) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), traitCollectionDidChange: previous] };
            let handler = self.ivars().on_backing_changed.borrow().clone();
            if let Some(handler) = handler {
                handler();
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(touchesBegan:withEvent:))]
        fn touches_began(&self, touches: &NSSet<UITouch>, event: Option<&objc2_ui_kit::UIEvent>) {
            let _ = event;
            if let Some(position) = self.touch_point(touches) {
                self.emit(PointerInteraction::PrimaryDown {
                    position,
                    click_count: 1,
                });
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(touchesMoved:withEvent:))]
        fn touches_moved(&self, touches: &NSSet<UITouch>, event: Option<&objc2_ui_kit::UIEvent>) {
            let _ = event;
            if let Some(position) = self.touch_point(touches) {
                self.emit(PointerInteraction::Moved(Some(position)));
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(touchesEnded:withEvent:))]
        fn touches_ended(&self, touches: &NSSet<UITouch>, event: Option<&objc2_ui_kit::UIEvent>) {
            let _ = touches;
            let _ = event;
            self.emit(PointerInteraction::PrimaryUp);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(touchesCancelled:withEvent:))]
        fn touches_cancelled(&self, touches: &NSSet<UITouch>, event: Option<&objc2_ui_kit::UIEvent>) {
            let _ = touches;
            let _ = event;
            self.emit(PointerInteraction::PrimaryUp);
        }
    }
);

impl SurfaceView {
    /// A layer-hosted surface host.
    ///
    /// The returned view's [`presentation_layer`](Self::presentation_layer)
    /// is where frames present; it fills the view's layer.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SurfaceViewIvars::default());
        // SAFETY: `initWithFrame:` is `UIView`'s designated initializer.
        let view: Retained<Self> =
            unsafe { msg_send![super(this), initWithFrame: objc2_core_foundation::CGRect::ZERO] };

        // The presentation layer is opaque-free and stretched to fit; frames
        // are rendered at device-pixel size, so the layer must not rescale
        // them — `contentsScale` carries that.
        let presentation = CALayer::new();
        presentation.setOpaque(false);
        // SAFETY: `kCAGravityResize` is a `CAContentsGravity` constant.
        // SAFETY: `kCAGravityResize` is a `CAContentsGravity` constant.
        presentation.setContentsGravity(unsafe { objc2_quartz_core::kCAGravityResize });
        view.layer().addSublayer(&presentation);
        view.ivars().presentation_layer.replace(Some(presentation));

        let target: Retained<GestureTarget> = {
            let this = GestureTarget::alloc(mtm).set_ivars(GestureTargetIvars::default());
            // SAFETY: `init` is `NSObject`'s designated initializer.
            unsafe { msg_send![super(this), init] }
        };

        // SAFETY: `initWithTarget:action:` retains the recognizer↔target
        // pair; the view owns the target through `gesture_target`.
        unsafe {
            let hover = UIHoverGestureRecognizer::initWithTarget_action(
                UIHoverGestureRecognizer::alloc(mtm),
                Some(&*target),
                Some(sel!(hover:)),
            );
            view.addGestureRecognizer(&hover);

            let pinch = UIPinchGestureRecognizer::initWithTarget_action(
                UIPinchGestureRecognizer::alloc(mtm),
                Some(&*target),
                Some(sel!(pinch:)),
            );
            pinch.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*view)));
            view.addGestureRecognizer(&pinch);

            let pan = UIPanGestureRecognizer::initWithTarget_action(
                UIPanGestureRecognizer::alloc(mtm),
                Some(&*target),
                Some(sel!(pan:)),
            );
            // Two fingers so a single-finger scroll never reads as a pan.
            pan.setMinimumNumberOfTouches(2);
            pan.setCancelsTouchesInView(false);
            pan.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*view)));
            view.addGestureRecognizer(&pan);

            let double_tap = UITapGestureRecognizer::initWithTarget_action(
                UITapGestureRecognizer::alloc(mtm),
                Some(&*target),
                Some(sel!(doubleTap:)),
            );
            double_tap.setNumberOfTapsRequired(2);
            view.addGestureRecognizer(&double_tap);

            view.ivars().recognizers.replace(vec![
                hover.into_super(),
                pinch.into_super(),
                pan.into_super(),
                double_tap.into_super(),
            ]);
        }
        view.ivars().gesture_target.replace(Some(target));
        view
    }

    /// The layer a renderer presents frames into.
    ///
    /// # Panics
    ///
    /// Panics when called before `new` installs the layer — impossible.
    #[must_use]
    /// # Panics
    ///
    /// If called before `add_presentation_layer`.
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

    /// The window screen's scale — physical pixels per logical unit —
    /// `None` while the view is off-window.
    #[must_use]
    pub fn backing_scale(&self) -> Option<f64> {
        crate::view::window(self).map(|window| window.screen().scale())
    }

    /// Whether the view can present: on a window and not hidden inside the
    /// hierarchy.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        let mut hidden_ancestor = false;
        let mut ancestor = self.superview();
        while let Some(view) = ancestor {
            if view.isHidden() {
                hidden_ancestor = true;
                break;
            }
            ancestor = view.superview();
        }
        !self.isHidden() && !hidden_ancestor && crate::view::window(self).is_some()
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
    /// Registers `handler` for visibility transitions (hidden ancestors,
    /// window attach/detach).
    pub fn set_visibility_changed_handler(&self, handler: impl Fn() + 'static) {
        self.ivars()
            .on_visibility_changed
            .replace(Some(Rc::new(handler)));
    }

    /// Registers `handler` for backing-property changes (screen scale,
    /// dynamic range).
    pub fn set_backing_changed_handler(&self, handler: impl Fn() + 'static) {
        self.ivars()
            .on_backing_changed
            .replace(Some(Rc::new(handler)));
    }

    /// Registers `handler` for window attach/detach transitions.
    pub fn set_window_changed_handler(&self, handler: impl Fn() + 'static) {
        self.ivars()
            .on_window_changed
            .replace(Some(Rc::new(handler)));
    }

    /// Installs `handler` as the receiver of pointer and gesture input for
    /// the surface's own interaction, and the recognizers' target.
    pub fn set_interaction_handler(&self, handler: impl Fn(PointerInteraction) + 'static) {
        let handler: InteractionHandler = Rc::new(handler);
        if let Some(target) = self.ivars().gesture_target.borrow().as_ref() {
            let hover = handler.clone();
            target.ivars().hover.replace(Some(Rc::new(move |point| {
                hover(PointerInteraction::Moved(point));
            })));
            let pinch = handler.clone();
            target
                .ivars()
                .pinch
                .replace(Some(Rc::new(move |interaction| {
                    pinch(interaction);
                })));
            let pan = handler.clone();
            target.ivars().pan.replace(Some(Rc::new(move |interaction| {
                pan(interaction);
            })));
            let tap = handler.clone();
            target.ivars().double_tap.replace(Some(Rc::new(move || {
                tap(PointerInteraction::DoubleTap);
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

    /// The first touch's position in logical, surface-local points.
    fn touch_point(&self, touches: &NSSet<UITouch>) -> Option<kurbo::Point> {
        let touch = touches.iter().next()?;
        let this: &UIView = self;
        let point = touch.locationInView(Some(this));
        Some(kurbo::Point::new(point.x, point.y))
    }
}
