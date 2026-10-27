//! Property sampling (§3): a transition never writes a property the
//! content owns. Its sampled alpha goes to the transition host — an object
//! only the transition writes — and its sampled transform to the host's
//! single content child, so both compose with the content's own opacity
//! and transform through the native layer tree. On `AppKit` the transform
//! is the host layer's `sublayerTransform`: `AppKit` rewrites a
//! layer-backed child's own `transform` at display (#3504), while the
//! host's sublayer transform reaches the same single child untouched.

use alloc::rc::Rc;
use core::cell::RefCell;

use cocoa_ui::{PlatformView, Retained, view};
use waterui_core::layout::Size;
use waterui_core::transition::{
    TransitionGeometry, TransitionPhase, TransitionProperties, TransitionState,
};

use crate::transition::TransitionClaim;
use crate::transition::clock::TransitionDriver;
use crate::transition::host::TransitionHost;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The transform `properties` applies around the centre of a `size` slot:
/// scale, then rotation, about the centre, then translation — the
/// documented `TransitionProperties` compose order, in top-left
/// coordinates. `CATransform3DConcat(a, b)` applies `a` first, so the
/// chain reads in application order, as `affine_transform` does on `UIKit`.
#[cfg(target_os = "macos")]
fn layer_transform(
    properties: &TransitionProperties,
    size: cocoa_ui::Size,
) -> objc2_quartz_core::CATransform3D {
    use objc2_quartz_core::CATransform3D;
    let cx = size.width * 0.5;
    let cy = size.height * 0.5;
    let translation = CATransform3D::new_translation(
        f64::from(properties.translation[0]),
        f64::from(properties.translation[1]),
        0.0,
    );
    let to_centre = CATransform3D::new_translation(cx, cy, 0.0);
    let rotate = CATransform3D::new_rotation(f64::from(properties.rotation), 0.0, 0.0, 1.0);
    let scale = CATransform3D::new_scale(
        f64::from(properties.scale[0]),
        f64::from(properties.scale[1]),
        1.0,
    );
    let back = CATransform3D::new_translation(-cx, -cy, 0.0);
    back.concat(scale)
        .concat(rotate)
        .concat(to_centre)
        .concat(translation)
}

/// The `CGAffineTransform` spelling of [`layer_transform`] for `UIKit`:
/// `UIView.transform` already applies about the view's centre.
#[cfg(target_os = "ios")]
fn affine_transform(properties: &TransitionProperties) -> objc2_core_foundation::CGAffineTransform {
    let (sin, cos) = f64::from(properties.rotation).sin_cos();
    let sx = f64::from(properties.scale[0]);
    let sy = f64::from(properties.scale[1]);
    objc2_core_foundation::CGAffineTransform {
        a: sx * cos,
        b: sx * sin,
        c: -sy * sin,
        d: sy * cos,
        tx: f64::from(properties.translation[0]),
        ty: f64::from(properties.translation[1]),
    }
}

/// Where one sample lands: `opacity` on the transition host's own alpha,
/// the transform on its single content child — the live content view or
/// the ghost's retired root (§3.1).
#[derive(Clone)]
pub struct PropertyTarget {
    /// The transition host — its alpha composites above the content, and
    /// on `AppKit` its layer's `sublayerTransform` carries the transform.
    host: Retained<HostView>,
    /// The host's single child — on `UIKit` its own transform carries the
    /// sample and composes with anything the content applies inside itself.
    #[cfg(target_os = "ios")]
    child: Retained<PlatformView>,
}

impl core::fmt::Debug for PropertyTarget {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PropertyTarget").finish_non_exhaustive()
    }
}

impl PropertyTarget {
    /// The target over `host` and its content child.
    #[cfg_attr(
        target_os = "ios",
        expect(
            clippy::missing_const_for_fn,
            reason = "only macOS layer-backs the host; the iOS body makes no call"
        )
    )]
    pub fn new(host: Retained<HostView>, child: Retained<PlatformView>) -> Self {
        // `AppKit` transforms the single child through the host layer's
        // `sublayerTransform`, so the child itself is not kept.
        #[cfg(target_os = "macos")]
        let () = {
            view::ensure_layer_backed(&host);
            drop(child);
        };
        Self {
            host,
            #[cfg(target_os = "ios")]
            child,
        }
    }

    /// Writes one sample inside `core_animation::without_animation`.
    ///
    /// # Panics
    ///
    /// When a property is not finite or `blur` is negative — the same
    /// contract `record_blurred` declares in Hydrolysis — and when `blur`
    /// is nonzero: blur lands in stage 8.
    pub fn apply(&self, sample: &TransitionProperties) {
        assert!(
            sample.opacity.is_finite()
                && sample.scale.iter().all(|value| value.is_finite())
                && sample.translation.iter().all(|value| value.is_finite())
                && sample.rotation.is_finite()
                && sample.blur.is_finite()
                && sample.blur >= 0.0,
            "waterui-apple: a transition sampled {sample:?} — every property must be finite \
             and the blur radius non-negative"
        );
        assert!(sample.blur == 0.0, "waterui-apple: blur lands in stage 8");
        cocoa_ui::core_animation::without_animation(|| {
            view::set_alpha(&self.host, f64::from(sample.opacity));
            #[cfg(target_os = "macos")]
            {
                let layer = view::layer(&self.host).expect("a layer-backed transition host");
                let size = view::bounds(&self.host).size;
                cocoa_ui::core_animation::set_sublayer_transform(
                    &layer,
                    layer_transform(sample, size),
                );
            }
            #[cfg(target_os = "ios")]
            view::set_transform(&self.child, affine_transform(sample));
        });
    }
}

/// One insertion's in-flight sampling (§3.2): the state, the frozen
/// geometry it samples against, and the display driver. The site drops it
/// at retirement — the removal starts a fresh clock at progress 0 — and
/// the run drops itself from the site's slot on completion.
pub struct InsertionRun {
    /// The transition clock — `Insertion` phase, progress at 1.
    state: TransitionState,
    /// The host bounds at start plus the claim's direction.
    geometry: TransitionGeometry,
    /// The host each tick samples through — the live wrapper, so the
    /// recorded transform stays current for the layout re-apply.
    host: Rc<TransitionHost>,
    /// The display driver — held so the run ticks.
    driver: TransitionDriver,
}

impl core::fmt::Debug for InsertionRun {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InsertionRun").finish_non_exhaustive()
    }
}

impl InsertionRun {
    /// Starts the insertion of `host`'s content: the absent (progress-1)
    /// sample applies synchronously, before the host's first layout, so
    /// the first presented frame is never fully present. Answers the slot
    /// the site stores — the run clears it on completion.
    pub fn start(
        host: &Rc<TransitionHost>,
        claim: &TransitionClaim,
        env: &waterui_backend_core::Environment,
    ) -> Rc<RefCell<Option<Self>>> {
        let _ = env; // GPU services land with the pixel insertion (stage 9)
        let state = claim
            .spec
            .start(TransitionPhase::Insertion, claim.reduce_motion);
        if state.captures_content() {
            crate::transition::transition_violation!(
                "waterui-apple: pixel transitions land in stage 9 — a `captures_content` \
                 insertion has no realization yet; use a property transition (opacity, scale, \
                 move, rotation) or an asymmetric transition"
            );
        }
        let size = view::bounds(&host.host).size;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the layout contract is f32; kit geometry is f64"
        )]
        let geometry = TransitionGeometry::new(
            Size::new(size.width as f32, size.height as f32),
            claim.direction,
        );
        let slot: Rc<RefCell<Option<Self>>> = Rc::new(RefCell::new(None));
        // A host with no window never ticks — `FrameClock::start` on a
        // windowless view stops the clock for good — and a zero-duration
        // insertion has nothing left to sample; both end at identity at
        // once rather than freeze on the absent sample.
        if state.is_complete() || !view::has_window(&host.host) {
            host.apply_sample(&TransitionProperties::IDENTITY);
            return slot;
        }
        host.apply_sample(&state.properties(geometry));
        let mtm =
            cocoa_ui::MainThreadMarker::new().expect("a transition starts on the main thread");
        let driver = TransitionDriver::start(mtm, &host.host, {
            let slot = Rc::downgrade(&slot);
            move |delta| {
                let Some(slot) = slot.upgrade() else {
                    return;
                };
                let done = {
                    let mut borrowed = slot.borrow_mut();
                    let run = borrowed
                        .as_mut()
                        .expect("the run lives until its completion task");
                    if run.state.advance(delta) {
                        let sample = run.state.properties(run.geometry);
                        run.host.apply_sample(&sample);
                        false
                    } else {
                        // Completion: restore the identity sample, then the
                        // run drops itself — never inside its own tick.
                        run.host.apply_sample(&TransitionProperties::IDENTITY);
                        true
                    }
                };
                if done {
                    cocoa_ui::main_queue::enqueue_local(mtm, move |_mtm| {
                        slot.borrow_mut().take();
                    });
                }
            }
        });
        *slot.borrow_mut() = Some(Self {
            state,
            geometry,
            host: Rc::clone(host),
            driver,
        });
        slot
    }
}

impl Drop for InsertionRun {
    /// Dropping the run — at retirement or from the completion task —
    /// stops its clock at the driver's boundary.
    fn drop(&mut self) {
        self.driver.stop();
    }
}
