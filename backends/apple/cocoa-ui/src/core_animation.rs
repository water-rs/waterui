//! Core Animation, the compositor both frameworks draw through.

use std::cell::Cell;

use block2::RcBlock;
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
use objc2_core_foundation::{
    CFIndex, CFOptionFlags, CFRunLoop, CFRunLoopActivity, CFRunLoopObserver, kCFRunLoopCommonModes,
};
use objc2_quartz_core::CALayer;
use objc2_quartz_core::CATransaction;
#[cfg(target_os = "macos")]
use objc2_quartz_core::CATransform3D;

/// Commits the current Core Animation transaction to the render server now,
/// rather than at the end of this turn of the run loop.
///
/// After this returns, every layer change made so far has been handed to the
/// compositor, which is the point at which a frame can be said to have been
/// submitted.
pub fn flush_transaction() {
    CATransaction::flush();
}

/// Runs `body` inside a transaction with implicit actions disabled.
///
/// Layer property changes `body` makes — geometry, contents — apply
/// atomically and instantly, which is what a content stream wants: frames
/// from a GPU surface are already time-stamped, so animating their landing
/// would smear them.
pub fn without_animation(body: impl FnOnce()) {
    CATransaction::begin();
    // SAFETY: `setDisableActions:` is a plain property setter of the
    // transaction's copy semantics; the class method exists on every
    // supported release.
    CATransaction::setDisableActions(true);
    body();
    CATransaction::commit();
}

/// The order of Core Animation's own commit observer on the main run loop
/// (`CA::Transaction::observer_callback`, on `BeforeWaiting | Exit`).
/// [`on_commit`]'s observer sits right after it, [`before_commit`]'s right
/// before it.
const CORE_ANIMATION_COMMIT_ORDER: CFIndex = 2_000_000;

/// Runs `body` once, after the main run loop's current turn commits its
/// Core Animation transaction.
///
/// Commit is the boundary after which a layer added this turn is on
/// screen, so an animation started then animates from its initial value
/// instead of landing already at its end state.
///
/// The boundary is a one-shot main run-loop observer ordered after Core
/// Animation's commit observer, not the transaction's completion block:
/// that block waits for every animation added to the transaction after it
/// to finish, and a Liquid Glass tab bar adds animations of infinite
/// duration while it is built, so a completion block registered before it
/// never runs (#3881). Bodies registered in one turn run in registration
/// order. One registered while that turn's observers are already being
/// called — from inside the commit, or from another `on_commit` body —
/// runs after the next turn's commit.
///
/// The marker is the proof `body` — which need not be `Send` — is
/// registered on the thread the main run loop calls it on.
///
/// # Panics
///
/// Never in practice: every process with a main thread has a main run loop.
pub fn on_commit(mtm: MainThreadMarker, body: impl FnOnce() + 'static) {
    let run_loop = CFRunLoop::main().expect("every process with a main thread has a main run loop");
    observe_turn_once(mtm, CORE_ANIMATION_COMMIT_ORDER + 1, move || {
        body();
        // Layer changes `body` made join a transaction opened after this
        // turn's commit; wake the loop so the next turn commits them
        // instead of leaving them until some other event arrives.
        run_loop.wake_up();
    });
}

/// Runs `body` once, just before the main run loop's current turn commits
/// its Core Animation transaction.
///
/// The observer is ordered right before Core Animation's commit observer:
/// it runs after the turn's sources, timers and display-link callbacks,
/// and layer changes `body` makes ride that same commit.
///
/// Registration follows [`on_commit`]'s rules: bodies registered in one
/// turn run in registration order, and one registered while that turn's
/// observers are already being called — from another `before_commit`
/// body — runs before the next turn's commit.
///
pub fn before_commit(mtm: MainThreadMarker, body: impl FnOnce() + 'static) {
    observe_turn_once(mtm, CORE_ANIMATION_COMMIT_ORDER - 1, body);
}

/// Runs `body` once from a one-shot main run-loop observer at `order` on
/// `BeforeWaiting | Exit`, in the common modes.
fn observe_turn_once(_mtm: MainThreadMarker, order: CFIndex, body: impl FnOnce() + 'static) {
    let run_loop = CFRunLoop::main().expect("every process with a main thread has a main run loop");
    // The handler is `Fn`; the run loop calls a non-repeating observer
    // once and then invalidates it, releasing the handler and `body`.
    let body = Cell::new(Some(body));
    let handler = RcBlock::new(move |_: *mut CFRunLoopObserver, _: CFRunLoopActivity| {
        if let Some(body) = body.take() {
            body();
        }
    });
    let activities: CFOptionFlags = (CFRunLoopActivity::BeforeWaiting | CFRunLoopActivity::Exit).0;
    // SAFETY: the default allocator, a valid activity mask and a live
    // handler block, which the observer copies.
    let observer =
        unsafe { CFRunLoopObserver::with_handler(None, activities, false, order, Some(&handler)) }
            .expect("CFRunLoopObserverCreateWithHandler answers an observer for a valid allocator, mask and handler");
    // SAFETY: `kCFRunLoopCommonModes` is a Core Foundation constant.
    run_loop.add_observer(Some(&observer), unsafe { kCFRunLoopCommonModes });
    // A registration made while the run loop is already calling this
    // turn's observers misses that pass; waking the loop guarantees the
    // next pass comes instead of the loop sleeping on it.
    run_loop.wake_up();
}

/// Runs `body` inside an `NSAnimationContext` group.
///
/// Animatable property changes `body` makes animate over `duration` seconds
/// and, when `control_points` is given, the cubic timing function they
/// describe.
#[cfg(target_os = "macos")]
pub fn animate(duration: f64, control_points: Option<[f32; 4]>, body: impl FnOnce() + 'static) {
    use std::cell::RefCell;
    use std::ptr::NonNull;

    use objc2_app_kit::NSAnimationContext;
    use objc2_quartz_core::CAMediaTimingFunction;

    // The changes block is `Fn`, and `NSAnimationContext` may evaluate it
    // once only; the option still guards a double evaluation.
    let body = RefCell::new(Some(body));
    let block = block2::RcBlock::new(move |context: NonNull<NSAnimationContext>| {
        // SAFETY: the group passes a live context for the block's duration.
        let context = unsafe { context.as_ref() };
        context.setDuration(duration);
        context.setAllowsImplicitAnimation(true);
        if let Some([x1, y1, x2, y2]) = control_points {
            let timing = CAMediaTimingFunction::functionWithControlPoints(x1, y1, x2, y2);
            context.setTimingFunction(Some(&timing));
        }
        if let Some(body) = body.borrow_mut().take() {
            body();
        }
    });
    NSAnimationContext::runAnimationGroup(&block);
}

/// Runs `body` while a cross-fade of `duration` seconds plays on `view`'s
/// layer: the view's new content dissolves in over the old.
#[cfg(target_os = "macos")]
pub fn cross_dissolve(view: &objc2_app_kit::NSView, duration: f64, body: impl FnOnce()) {
    use objc2_foundation::ns_string;
    use objc2_quartz_core::{CAMediaTiming, CATransition, kCATransitionFade};

    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        let transition = CATransition::new();
        // SAFETY: `kCATransitionFade` is a `CATransitionType` constant Core
        // Animation exports.
        transition.setType(unsafe { kCATransitionFade });
        transition.setDuration(duration);
        layer.addAnimation_forKey(&transition, Some(ns_string!("crossDissolve")));
    }
    body();
}

/// Runs `body` while a cross-fade of `duration` seconds plays on `view`: the
/// view's new content dissolves in over the old.
#[cfg(target_os = "ios")]
pub fn cross_dissolve(view: &objc2_ui_kit::UIView, duration: f64, body: impl FnOnce() + 'static) {
    use objc2_ui_kit::{UIView, UIViewAnimationOptions};

    // `UIView.transition` may evaluate its animations block more than once;
    // the body still runs a single time.
    let body = std::cell::RefCell::new(Some(body));
    let block = block2::RcBlock::new(move || {
        if let Some(body) = body.borrow_mut().take() {
            body();
        }
    });
    UIView::transitionWithView_duration_options_animations_completion(
        view,
        duration,
        UIViewAnimationOptions::TransitionCrossDissolve,
        Some(&block),
        None,
    );
}

/// A timing curve [`animate_with`] plays `body`'s animatable changes under.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Timing {
    /// A cubic-bezier timing curve over `duration` seconds.
    Bezier {
        /// Seconds the animation runs.
        duration: f64,
        /// The curve's two control points, `[x1, y1, x2, y2]`.
        control_points: [f32; 4],
    },
    /// A physical spring on a unit mass: the definition `WaterUI` core and
    /// the Cherenkov engine evaluate, whose stiffness, damping coefficient
    /// and settling duration every platform animator here takes.
    Spring(waterui_easing::Spring),
}

/// Runs `body` while `timing` plays its animatable changes: a
/// `UIViewPropertyAnimator` on iOS, an `NSAnimationContext` group on macOS.
///
/// The curve the animator plays: `UICubicTimingParameters` for `Bezier`,
/// a mass-1 `UISpringTimingParameters` with the spring's stiffness and
/// damping coefficient for `Spring` (the animator's `duration` argument is
/// unused there; the spring's physics sets it).
///
/// Off the main thread this breaks the contract: it is reported and `body`
/// runs at once, unanimated.
#[cfg(target_os = "ios")]
pub fn animate_with(timing: Timing, body: impl FnOnce() + 'static) {
    use objc2::MainThreadOnly;
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_core_foundation::{CGPoint, CGVector};
    use objc2_ui_kit::{
        UICubicTimingParameters, UISpringTimingParameters, UITimingCurveProvider, UIViewAnimating,
        UIViewPropertyAnimator,
    };
    use std::cell::RefCell;

    let Some(mtm) = objc2::MainThreadMarker::new() else {
        waterui_contract::contract_violation!(
            "animation runs on the main thread; the body runs unanimated"
        );
        body();
        return;
    };
    // `addAnimations` may evaluate its block once only; the option still
    // guards a double evaluation.
    let body = RefCell::new(Some(body));
    let block = block2::RcBlock::new(move || {
        if let Some(body) = body.borrow_mut().take() {
            body();
        }
    });
    let (duration, parameters): (f64, Retained<ProtocolObject<dyn UITimingCurveProvider>>) =
        match timing {
            Timing::Bezier {
                duration,
                control_points: [x1, y1, x2, y2],
            } => (
                duration,
                ProtocolObject::from_retained(
                    UICubicTimingParameters::initWithControlPoint1_controlPoint2(
                        UICubicTimingParameters::alloc(mtm),
                        CGPoint::new(f64::from(x1), f64::from(y1)),
                        CGPoint::new(f64::from(x2), f64::from(y2)),
                    ),
                ),
            ),
            Timing::Spring(spring) => (
                0.0,
                ProtocolObject::from_retained(
                    UISpringTimingParameters::initWithMass_stiffness_damping_initialVelocity(
                        UISpringTimingParameters::alloc(mtm),
                        1.0,
                        spring.stiffness(),
                        spring.damping_coefficient(),
                        CGVector::new(0.0, 0.0),
                    ),
                ),
            ),
        };
    let animator = UIViewPropertyAnimator::initWithDuration_timingParameters(
        UIViewPropertyAnimator::alloc(mtm),
        duration,
        &parameters,
    );
    animator.addAnimations(&block);
    animator.startAnimation();
}

/// Runs `body` while `timing` plays its animatable changes.
///
/// `NSAnimationContext` has no spring curve and wants a duration, so a
/// spring runs for its own settling duration: the time a unit step from rest
/// takes to settle, as `WaterUI`'s `Animation::duration` reports it.
#[cfg(target_os = "macos")]
pub fn animate_with(timing: Timing, body: impl FnOnce() + 'static) {
    match timing {
        Timing::Bezier {
            duration,
            control_points,
        } => animate(duration, Some(control_points), body),
        Timing::Spring(spring) => animate(spring.settling_duration(-1.0, 0.0), None, body),
    }
}

/// Writes `layer`'s `sublayerTransform` — the transform it applies to its
/// children — with implicit actions disabled, for transforms `AppKit`
/// would otherwise animate as implicit layer changes.
///
/// `AppKit` owns a layer-backed view's layer geometry and rewrites a
/// subview layer's `transform` at display time; `sublayerTransform` on
/// the parent is not part of that sync, so a view-level transform lives
/// here.
#[cfg(target_os = "macos")]
pub fn set_sublayer_transform(layer: &objc2_quartz_core::CALayer, transform: CATransform3D) {
    use objc2_quartz_core::CATransaction;

    CATransaction::begin();
    CATransaction::setDisableActions(true);
    layer.setSublayerTransform(transform);
    CATransaction::commit();
}

/// Moves `layer`'s `sublayerTransform` to `transform`, animating the
/// `sublayerTransform` keypath under `timing` — `None` writes it directly.
///
/// The explicit `CAAnimation` runs on the compositor so it survives the
/// layout passes that rewrite the layer's geometry; `key` identifies it so
/// a later change replaces it. The animation reads `fromValue` from the
/// presentation layer, so a mid-flight change starts from where the screen
/// actually is.
#[cfg(target_os = "macos")]
pub fn animate_sublayer_transform(
    layer: &objc2_quartz_core::CALayer,
    transform: CATransform3D,
    key: &str,
    timing: Option<Timing>,
) {
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSString, ns_string};
    use objc2_quartz_core::{
        CAAnimation, CABasicAnimation, CAMediaTiming, CAMediaTimingFunction, CASpringAnimation,
        kCAFillModeBoth,
    };

    let Some(timing) = timing else {
        set_sublayer_transform(layer, transform);
        return;
    };

    // The transform the screen currently shows: the presentation layer's
    // when an animation is already playing, the model's otherwise.
    // SAFETY: `presentationLayer` is a documented accessor on a live layer.
    let from = unsafe {
        layer.presentationLayer().map_or_else(
            || layer.sublayerTransform(),
            |presentation| presentation.sublayerTransform(),
        )
    };
    layer.removeAnimationForKey(&NSString::from_str(key));

    // `NSValue +valueWithCATransform3D:` wraps the transform for
    // `fromValue`/`toValue`; it has no generated binding, so the selector
    // is sent directly.
    // SAFETY: `valueWithCATransform3D:` is a class method on `NSValue`
    // taking the struct by value and answering a retained `NSValue`.
    let from_value: Retained<AnyObject> =
        unsafe { objc2::msg_send![objc2::class!(NSValue), valueWithCATransform3D: from] };
    // SAFETY: see `from_value`.
    let to_value: Retained<AnyObject> =
        unsafe { objc2::msg_send![objc2::class!(NSValue), valueWithCATransform3D: transform] };

    let animation: Retained<CAAnimation> = match timing {
        Timing::Bezier {
            duration,
            control_points: [x1, y1, x2, y2],
        } => {
            let basic =
                CABasicAnimation::animationWithKeyPath(Some(ns_string!("sublayerTransform")));
            basic.setDuration(duration);
            let function = CAMediaTimingFunction::functionWithControlPoints(x1, y1, x2, y2);
            basic.setTimingFunction(Some(&function));
            // SAFETY: the values are `NSValue` objects — `AnyObject`s.
            unsafe {
                basic.setFromValue(Some(&from_value));
                basic.setToValue(Some(&to_value));
            }
            // SAFETY: `CAAnimation` is `CABasicAnimation`'s superclass.
            unsafe { Retained::cast_unchecked(basic) }
        }
        Timing::Spring(parameters) => {
            let spring =
                CASpringAnimation::animationWithKeyPath(Some(ns_string!("sublayerTransform")));
            spring.setMass(1.0);
            spring.setStiffness(parameters.stiffness());
            spring.setDamping(parameters.damping_coefficient());
            spring.setAllowsOverdamping(true);
            spring.setInitialVelocity(0.0);
            spring.setDuration(parameters.settling_duration(-1.0, 0.0));
            // SAFETY: the values are `NSValue` objects — `AnyObject`s.
            unsafe {
                spring.setFromValue(Some(&from_value));
                spring.setToValue(Some(&to_value));
            }
            // SAFETY: `CAAnimation` is `CASpringAnimation`'s superclass.
            unsafe { Retained::cast_unchecked(spring) }
        }
    };
    animation.setRemovedOnCompletion(true);
    // SAFETY: `kCAFillModeBoth` is a `CAMediaTimingFillMode` constant Core
    // Animation exports.
    animation.setFillMode(unsafe { kCAFillModeBoth });

    set_sublayer_transform(layer, transform);
    layer.addAnimation_forKey(&animation, Some(&NSString::from_str(key)));
}
/// Sets a layer's frame — the presentation plane updates
/// batches inside a transaction.
pub fn set_frame(layer: &CALayer, frame: crate::geometry::Rect) {
    layer.setFrame(frame.into());
}

/// Sets a layer's contents scale to the display's backing factor.
pub fn set_contents_scale(layer: &CALayer, scale: f64) {
    layer.setContentsScale(scale);
}

/// The presentation plane's clear color.
pub fn set_background_clear(layer: &CALayer) {
    layer.setBackgroundColor(None);
}

/// `contentsGravity = kCAGravityResize` — presented content stretches to the
/// layer's bounds rather than rescaling point-for-point.
pub fn set_contents_gravity_resize(layer: &CALayer) {
    // SAFETY: `kCAGravityResize` is a Core Animation framework constant.
    layer.setContentsGravity(unsafe { objc2_quartz_core::kCAGravityResize });
}
