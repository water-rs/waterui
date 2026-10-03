//! Core Animation, the compositor both frameworks draw through.

#[cfg(target_os = "macos")]
use objc2::rc::Retained;
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

/// Runs `body` when the current implicit transaction commits.
///
/// Commit is the boundary after which a layer added this transaction is on
/// screen, so an animation started then animates from its initial value
/// instead of landing already at its end state. `CATransaction` replaces any
/// completion block already installed, as the framework's own setter does.
pub fn on_commit(body: impl FnOnce() + 'static) {
    use std::cell::RefCell;

    // The completion block is `Fn`, but Core Animation evaluates it once;
    // the option still guards a double evaluation.
    let body = RefCell::new(Some(body));
    let block = block2::RcBlock::new(move || {
        if let Some(body) = body.borrow_mut().take() {
            body();
        }
    });
    // SAFETY: the block lives in the transaction, which calls it once on
    // commit and releases it afterwards.
    unsafe { CATransaction::setCompletionBlock(Some(&block)) };
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
    /// A physically driven spring.
    Spring {
        /// Spring stiffness.
        stiffness: f64,
        /// Spring damping.
        damping: f64,
    },
}

/// Runs `body` while `timing` plays its animatable changes: a
/// `UIViewPropertyAnimator` on iOS, an `NSAnimationContext` group on macOS.
///
/// # Panics
///
/// When called off the main thread.
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

    let mtm = objc2::MainThreadMarker::new().expect("animation runs on the main thread");
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
            Timing::Spring { stiffness, damping } => (
                0.0,
                ProtocolObject::from_retained(
                    UISpringTimingParameters::initWithMass_stiffness_damping_initialVelocity(
                        UISpringTimingParameters::alloc(mtm),
                        1.0,
                        stiffness,
                        damping,
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
/// A spring has no authored duration in `UIKit`; `NSAnimationContext` wants
/// one, so it is estimated from the stiffness and damping and clamped the
/// way the framework consumer's spring timing expects.
#[cfg(target_os = "macos")]
pub fn animate_with(timing: Timing, body: impl FnOnce() + 'static) {
    match timing {
        Timing::Bezier {
            duration,
            control_points,
        } => animate(duration, Some(control_points), body),
        Timing::Spring { stiffness, damping } => {
            let estimated = 2.0 * (1.0 / stiffness).sqrt() * damping;
            animate(estimated.clamp(0.1, 2.0), None, body);
        }
    }
}

/// Writes `layer`'s transform with implicit actions disabled — an
/// `NSAnimationContext`-free set, for transforms `AppKit` would otherwise
/// animate as implicit layer changes.
///
/// `AppKit` owns a layer-backed view's layer geometry, so the transform is
/// set on the layer inside a `CATransaction` that suppresses the implicit
/// animation.
#[cfg(target_os = "macos")]
pub fn set_layer_transform(layer: &objc2_quartz_core::CALayer, transform: CATransform3D) {
    use objc2_quartz_core::CATransaction;

    CATransaction::begin();
    CATransaction::setDisableActions(true);
    layer.setTransform(transform);
    CATransaction::commit();
}

/// Moves `layer`'s transform to `transform`, animating the `transform`
/// keypath under `timing` — `None` writes it directly.
///
/// The explicit `CAAnimation` runs on the compositor so it survives the
/// layout passes that rewrite the layer's geometry; `key` identifies it so
/// a later change replaces it. The animation reads `fromValue` from the
/// presentation layer, so a mid-flight change starts from where the screen
/// actually is.
#[cfg(target_os = "macos")]
pub fn animate_layer_transform(
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
        set_layer_transform(layer, transform);
        return;
    };

    // The transform the screen currently shows: the presentation layer's
    // when an animation is already playing, the model's otherwise.
    // SAFETY: `presentationLayer` is a documented accessor on a live layer.
    let from = unsafe {
        layer.presentationLayer().map_or_else(
            || layer.transform(),
            |presentation| presentation.transform(),
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
            let basic = CABasicAnimation::animationWithKeyPath(Some(ns_string!("transform")));
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
        Timing::Spring { stiffness, damping } => {
            let spring = CASpringAnimation::animationWithKeyPath(Some(ns_string!("transform")));
            spring.setMass(1.0);
            spring.setStiffness(stiffness);
            spring.setDamping(damping);
            spring.setInitialVelocity(0.0);
            spring.setDuration(spring.settlingDuration());
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

    set_layer_transform(layer, transform);
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
