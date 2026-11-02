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
use waterui_core::layout::{LayoutDirection, Size};
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
        f64::from(properties.translation()[0]),
        f64::from(properties.translation()[1]),
        0.0,
    );
    let to_centre = CATransform3D::new_translation(cx, cy, 0.0);
    let rotate = CATransform3D::new_rotation(f64::from(properties.rotation()), 0.0, 0.0, 1.0);
    let scale = CATransform3D::new_scale(
        f64::from(properties.scale()[0]),
        f64::from(properties.scale()[1]),
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
    let (sin, cos) = f64::from(properties.rotation()).sin_cos();
    let sx = f64::from(properties.scale()[0]);
    let sy = f64::from(properties.scale()[1]);
    objc2_core_foundation::CGAffineTransform {
        a: sx * cos,
        b: sx * sin,
        c: -sy * sin,
        d: sy * cos,
        tx: f64::from(properties.translation()[0]),
        ty: f64::from(properties.translation()[1]),
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
    /// contract `record_blurred` declares in Hydrolysis.
    pub fn apply(&self, sample: &TransitionProperties) {
        assert!(
            sample.opacity().is_finite()
                && sample.scale().iter().all(|value| value.is_finite())
                && sample.translation().iter().all(|value| value.is_finite())
                && sample.rotation().is_finite()
                && sample.blur().is_finite()
                && sample.blur() >= 0.0,
            "waterui-apple: a transition sampled {sample:?} — every property must be finite \
             and the blur radius non-negative"
        );
        cocoa_ui::core_animation::without_animation(|| {
            view::set_alpha(&self.host, f64::from(sample.opacity()));
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
    /// The claim's direction — the geometry freezes it with the host
    /// bounds when the run arms.
    direction: LayoutDirection,
    /// The host bounds when the run armed plus the claim's direction.
    geometry: TransitionGeometry,
    /// The host each tick samples through — the live wrapper, so the
    /// recorded transform stays current for the layout re-apply.
    host: Rc<TransitionHost>,
    /// The display driver — `None` until the host is in a window.
    driver: Option<TransitionDriver>,
    /// The site's environment — the lazy blur effect reads its GPU
    /// services from it.
    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
    env: waterui_backend_core::Environment,
    /// The lazy blur effect (§2.6) — created on the first sample whose
    /// `blur > 0`, kept when the radius returns to 0. The run's drop
    /// tears it down.
    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
    blur: Option<(
        waterui::reactive::Binding<f32>,
        crate::components::filtered::LiveEffect,
    )>,
    /// The `captures_content` realization (§2.7): the window overlay and
    /// its live-capture effect — built when the run arms. The run's drop
    /// tears it down.
    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
    pixel: Option<crate::transition::pixel::PixelInsertion>,
    /// Set once a nonzero radius could not be realized — one violation
    /// report per run.
    #[cfg(not(any(feature = "applied_filter", feature = "view_effect")))]
    blur_warned: core::cell::Cell<bool>,
}

impl core::fmt::Debug for InsertionRun {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InsertionRun").finish_non_exhaustive()
    }
}

/// The host's bounds as the geometry a sample is computed against.
fn host_geometry(host: &TransitionHost, direction: LayoutDirection) -> TransitionGeometry {
    let size = view::bounds(&host.host).size;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; kit geometry is f64"
    )]
    TransitionGeometry::new(Size::new(size.width as f32, size.height as f32), direction)
}

impl InsertionRun {
    /// Starts the insertion of `host`'s content: the absent (progress-1)
    /// sample applies synchronously, before the host's first layout, so
    /// the first presented frame is never fully present. A zero-duration
    /// insertion ends at identity at once. A host not yet in a window — a
    /// table cell built before the table adds it — arms when it enters
    /// one: its geometry freezes and its clock starts then, never before
    /// it can be seen. Answers the slot the site stores — the run clears
    /// it on completion.
    pub fn start(
        host: &Rc<TransitionHost>,
        claim: &TransitionClaim,
        env: &waterui_backend_core::Environment,
    ) -> Rc<RefCell<Option<Self>>> {
        #[cfg(not(any(feature = "applied_filter", feature = "view_effect")))]
        let _ = env; // GPU services land with the pixel insertion (stage 9)
        let state = claim
            .spec
            .start(TransitionPhase::Insertion, claim.reduce_motion);
        let slot: Rc<RefCell<Option<Self>>> = Rc::new(RefCell::new(None));
        // Without the filtered pipeline a `captures_content` transition
        // has no realization — a user-triggerable failure, reported and
        // ended at once at identity.
        #[cfg(not(any(feature = "applied_filter", feature = "view_effect")))]
        if state.captures_content() {
            crate::transition::transition_violation!(
                "waterui-apple: a pixel transition needs the filtered pipeline \
                 (the view_effect or applied_filter feature)"
            );
            host.apply_sample(&TransitionProperties::IDENTITY);
            return slot;
        }
        if state.is_complete() {
            host.apply_sample(&TransitionProperties::IDENTITY);
            return slot;
        }
        let geometry = host_geometry(host, claim.direction);
        host.apply_sample(&state.properties(geometry));
        *slot.borrow_mut() = Some(Self {
            state,
            direction: claim.direction,
            geometry,
            host: Rc::clone(host),
            driver: None,
            #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
            env: env.clone(),
            #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
            blur: None,
            #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
            pixel: None,
            #[cfg(not(any(feature = "applied_filter", feature = "view_effect")))]
            blur_warned: core::cell::Cell::new(false),
        });
        // Entering a window arms the run; entering one again — a row
        // scrolled away and back — re-arms its clock on the new window.
        host.host.set_window_handler({
            let slot = Rc::downgrade(&slot);
            move |view| {
                if view::has_window(view)
                    && let Some(slot) = slot.upgrade()
                {
                    Self::arm(&slot);
                }
            }
        });
        if view::has_window(&host.host) {
            Self::arm(&slot);
        }
        slot
    }

    /// Starts the clock on the host's window — freezing the geometry and
    /// re-applying the progress-0 sample on the first arm, re-arming the
    /// display link on a later one. A run that already completed is gone
    /// from the slot and arms nothing.
    fn arm(slot: &Rc<RefCell<Option<Self>>>) {
        let mut borrowed = slot.borrow_mut();
        let Some(run) = borrowed.as_mut() else {
            return;
        };
        if let Some(driver) = &run.driver {
            driver.restart(&run.host.host);
            return;
        }
        run.geometry = host_geometry(&run.host, run.direction);
        let sample = run.state.properties(run.geometry);
        run.host.apply_sample(&sample);
        run.update_blur(&sample);
        let mtm =
            cocoa_ui::MainThreadMarker::new().expect("a transition starts on the main thread");
        let driver = TransitionDriver::start(mtm, &run.host.host, {
            let slot = Rc::downgrade(slot);
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
                        run.update_blur(&sample);
                        // Without the filtered pipeline a pixel insertion
                        // never reaches the run: `start` reported it.
                        #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
                        let pixel_ok = run.update_pixel();
                        #[cfg(not(any(feature = "applied_filter", feature = "view_effect")))]
                        let pixel_ok = true;
                        if pixel_ok {
                            false
                        } else {
                            // A body violation already reported — the
                            // run ends at identity now (§10).
                            run.host.apply_sample(&TransitionProperties::IDENTITY);
                            true
                        }
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
        run.driver = Some(driver);
    }

    /// The lazy blur (§2.6): the first sample whose `blur > 0` creates
    /// the radius binding and a
    /// [`crate::components::filtered::LiveEffect`] over the live
    /// content — captured in place, never reparented. A radius
    /// returning to 0 keeps the effect — one per instance.
    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
    fn update_blur(&mut self, sample: &TransitionProperties) {
        use waterui::reactive::Signal as _;
        if self.blur.is_none() && sample.blur() > 0.0 {
            let radius = waterui::reactive::Binding::container(sample.blur());
            let mut guards = waterui_graphics::filter_view::ParamGuards::default();
            let effect = waterui_graphics::filter_view::AnyEffect::filter(
                waterui_graphics::filtrate::filters::Blur(guards.bind(radius.clone())),
            );
            self.blur = Some((
                radius,
                crate::components::filtered::LiveEffect::attach(
                    vec![(effect, guards)],
                    &self.host.content_view,
                    &self.host.host,
                    &self.env,
                ),
            ));
        }
        if let Some((radius, _effect)) = &self.blur
            && radius.snapshot().to_bits() != sample.blur().to_bits()
        {
            radius.set(sample.blur());
        }
    }

    /// Without the filtered pipeline a nonzero blur radius has no
    /// realization — a user-triggerable failure, reported once per run.
    #[cfg(not(any(feature = "applied_filter", feature = "view_effect")))]
    fn update_blur(&self, sample: &TransitionProperties) {
        if sample.blur() > 0.0 && !self.blur_warned.replace(true) {
            crate::transition::transition_violation!(
                "waterui-apple: a blur transition needs the filtered pipeline \
                 (the view_effect or applied_filter feature)"
            );
        }
    }

    /// The pixel insertion's per-tick work (§2.7): the overlay and its
    /// live-capture effect build on the first tick — never inside the
    /// site's borrow, where a synchronous first capture would re-enter
    /// the site's layout handler — and only once the content has a
    /// nonzero slot to capture. `true` while the run keeps ticking;
    /// `false` only on a body violation the macro already reported.
    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
    fn update_pixel(&mut self) -> bool {
        if self.pixel.is_none() && self.state.captures_content() {
            let bounds = cocoa_ui::view::bounds(&self.host.content_view);
            if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
                return true;
            }
            match crate::transition::pixel::PixelInsertion::start(
                &self.host,
                &self.state,
                self.direction,
                &self.env,
            ) {
                Some(pixel) => self.pixel = Some(pixel),
                None => return false,
            }
        }
        if let Some(pixel) = &self.pixel {
            pixel.update(&self.host, self.direction);
        }
        true
    }
}

impl Drop for InsertionRun {
    /// Dropping the run — at retirement or from the completion task —
    /// stops its clock at the driver's boundary.
    fn drop(&mut self) {
        if let Some(driver) = &self.driver {
            driver.stop();
        }
    }
}
