//! The removal ghost (§1.4/§1.5): the site's handle on an outgoing
//! instance — the retired visuals, the sampling run, and the completion
//! sequence that releases the slot and detaches the views.

use alloc::boxed::Box;
use alloc::rc::{Rc, Weak};
use core::cell::{Cell, RefCell};

use cocoa_ui::{MainThreadMarker, PlatformView, view};
use waterui_backend_core::Environment;
use waterui_core::transition::{GhostSlot, TransitionState};

use crate::transition::clock::TransitionDriver;
use crate::transition::property::PropertyTarget;
use crate::transition::retire::RetiredVisual;

/// The site's handle on a removal transition in flight. The site stores
/// one per ghosted member and drops it when `on_complete` fires — or
/// when the site itself leaves, which cancels the run.
pub struct Ghost(Rc<GhostRun>);

impl core::fmt::Debug for Ghost {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ghost").finish_non_exhaustive()
    }
}

/// The ghost's realization — the property-sample target; the lazy blur
/// and pixel runtimes join it in stages 8 and 9.
pub struct GhostRealization {
    /// Host alpha plus the retired root's transform.
    target: PropertyTarget,
    /// The lazy removal blur — created on the first sample whose
    /// `blur > 0`, kept when the radius returns to 0.
    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
    blur: Option<Rc<RemovalBlur>>,
    /// The `captures_content` removal runtime — the window overlay and
    /// its frozen-input effect (§2.7), built with the run.
    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
    pixel: Option<Rc<crate::transition::pixel::PixelRemoval>>,
}

/// The removal's lazy blur runtime (§2.6): the radius binding the
/// `FrozenEffect` samples, the in-flight frozen capture, and the landed
/// effect — all `None` until the first nonzero radius.
#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
struct RemovalBlur {
    /// The binding `FrozenEffect`'s blur radius samples — written on
    /// every tick.
    radius: waterui::reactive::Binding<f32>,
    /// The site's environment — `FrozenEffect::blur` reads its GPU
    /// services when the capture lands.
    env: Environment,
    /// A frozen capture is in flight — the next tick waits for it.
    pending: Cell<bool>,
    /// The in-flight capture — kept alive until it settles.
    input: RefCell<Option<Rc<crate::transition::pixel::FrozenInput>>>,
    /// The landed effect — `None` until the capture delivers.
    effect: RefCell<Option<crate::components::filtered::FrozenEffect>>,
}

/// The run behind [`Ghost`]: everything the tick and the completion
/// sequence need.
pub struct GhostRun {
    /// The main-thread marker completion tasks enqueue under.
    mtm: MainThreadMarker,
    /// The retired host and root — detaches when the run dies.
    pub(crate) visual: RetiredVisual,
    /// The removal clock.
    state: RefCell<TransitionState>,
    /// The frozen layout slot — collection sites only (§4).
    slot: Option<GhostSlot>,
    /// The sample target — `None` once completion drops it.
    realization: RefCell<Option<GhostRealization>>,
    /// The display driver — `None` once completion takes it off the tick.
    driver: RefCell<Option<TransitionDriver>>,
    /// The deliberate self-cycle keeping the run alive until completion
    /// or cancel, whichever comes first.
    keep: RefCell<Option<Rc<Self>>>,
    /// The site's slot release — fires once, inside `complete`.
    on_complete: RefCell<Option<Box<dyn FnOnce()>>>,
    /// The completion sequence ran — guards it to once.
    completed: Cell<bool>,
    /// The site's environment — the lazy blur effect reads its GPU
    /// services from it.
    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
    env: Environment,
    /// Set once a nonzero radius could not be realized — one violation
    /// report per run.
    #[cfg(not(any(feature = "applied_filter", feature = "view_effect")))]
    blur_warned: Cell<bool>,
}

impl core::fmt::Debug for GhostRun {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GhostRun").finish_non_exhaustive()
    }
}

impl Ghost {
    /// Starts the removal run over `visual`: the progress-0 sample applies
    /// synchronously, the driver attaches to the retired host's window,
    /// and the self-cycle arms so the run outlives the site's drop.
    pub fn start(
        mtm: MainThreadMarker,
        visual: RetiredVisual,
        state: TransitionState,
        env: &Environment,
        slot: Option<GhostSlot>,
        on_complete: Box<dyn FnOnce()>,
    ) -> Self {
        #[cfg(not(any(feature = "applied_filter", feature = "view_effect")))]
        let _ = env; // no filtered pipeline, no GPU services to keep
        let target = PropertyTarget::new(visual.host.clone(), visual.root.clone());
        let run = Rc::new_cyclic(|_weak: &Weak<GhostRun>| {
            // The `captures_content` realization (§2.7): the retired
            // window overlay plus the frozen-input effect body — its
            // first capture issued on the run's first tick.
            #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
            let pixel = state
                .captures_content()
                .then(|| crate::transition::pixel::PixelRemoval::start(&visual, &state, env));
            GhostRun {
                mtm,
                visual,
                state: RefCell::new(state),
                slot,
                realization: RefCell::new(Some(GhostRealization {
                    target,
                    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
                    blur: None,
                    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
                    pixel,
                })),
                driver: RefCell::new(None),
                keep: RefCell::new(None),
                on_complete: RefCell::new(Some(on_complete)),
                completed: Cell::new(false),
                #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
                env: env.clone(),
                #[cfg(not(any(feature = "applied_filter", feature = "view_effect")))]
                blur_warned: Cell::new(false),
            }
        });
        // The progress-0 sample lands before the first presented frame.
        let sample = run.state.borrow().properties(run.visual.geometry);
        if let Some(realization) = run.realization.borrow().as_ref() {
            realization.target.apply(&sample);
        }
        run.update_blur(sample.blur);
        // Losing the window is the native signal that the ghost left the
        // screen — an enclosing ghost completed and took this one with it —
        // so the windowless `FrameClock` never leaks the run.
        run.visual.host.set_window_handler({
            let weak = Rc::downgrade(&run);
            move |host| {
                if !view::has_window(host)
                    && let Some(run) = weak.upgrade()
                {
                    run.complete();
                }
            }
        });
        let driver = TransitionDriver::start(mtm, &run.visual.host, {
            let weak = Rc::downgrade(&run);
            move |delta| {
                let Some(run) = weak.upgrade() else {
                    return;
                };
                if run.completed.get() {
                    return;
                }
                if run.state.borrow_mut().advance(delta) {
                    let sample = run.state.borrow().properties(run.visual.geometry);
                    if let Some(realization) = run.realization.borrow().as_ref() {
                        realization.target.apply(&sample);
                    }
                    run.update_blur(sample.blur);
                    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
                    run.update_pixel();
                } else {
                    run.complete();
                }
            }
        });
        *run.driver.borrow_mut() = Some(driver);
        *run.keep.borrow_mut() = Some(Rc::clone(&run));
        Self(run)
    }

    /// The retired host — the ghost's placed view.
    pub fn host(&self) -> &PlatformView {
        &self.0.visual.host
    }

    /// The frozen layout slot a collection site holds — `None` for a
    /// conditional's ghost.
    pub fn slot(&self) -> Option<&GhostSlot> {
        self.0.slot.as_ref()
    }

    /// Places the retired host at the frame the collection's layout
    /// computed for the ghost's slot (§4.3), pixel-aligned under the
    /// container's `offset`. Only the host moves — its frozen size means
    /// the content inside never re-lays out, so nothing is delivered to it,
    /// and the pass's `frame.size` is the slot's own answer, so a mismatch
    /// is a container bug, not an event.
    pub fn place(&self, frame: cocoa_ui::Rect, offset: cocoa_ui::view::SubpixelOffset) {
        let slot = self
            .0
            .slot
            .as_ref()
            .expect("a collection ghost holds a slot");
        let expected =
            cocoa_ui::Size::new(f64::from(slot.size().width), f64::from(slot.size().height));
        assert!(
            frame.size.width.to_bits() == expected.width.to_bits()
                && frame.size.height.to_bits() == expected.height.to_bits(),
            "container placed a ghost at {frame:?}, not its frozen slot size {expected:?}"
        );
        view::set_frame_pixel_aligned(&self.0.visual.host, frame, offset);
    }
}

impl GhostRun {
    /// The lazy removal blur (§2.6): the first sample whose `blur > 0`
    /// creates the radius binding and starts the frozen exit capture of
    /// the retired subtree; the landed texture becomes a
    /// `FrozenEffect` over the retired root, which hides the root on
    /// its first present — never before, so no frame shows a gap. A
    /// radius returning to 0 keeps the effect — one per instance.
    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
    fn update_blur(self: &Rc<Self>, radius: f32) {
        use waterui::reactive::Signal as _;
        let blur = {
            let mut realization = self.realization.borrow_mut();
            let Some(realization) = realization.as_mut() else {
                return;
            };
            if realization.blur.is_none() && radius > 0.0 {
                realization.blur = Some(Rc::new(RemovalBlur {
                    radius: waterui::reactive::Binding::container(radius),
                    env: self.env.clone(),
                    pending: Cell::new(false),
                    input: RefCell::new(None),
                    effect: RefCell::new(None),
                }));
            }
            let Some(blur) = realization.blur.as_ref().map(Rc::clone) else {
                return;
            };
            if blur.radius.snapshot().to_bits() != radius.to_bits() {
                blur.radius.set(radius);
            }
            blur
        };
        if blur.pending.get() || blur.effect.borrow().is_some() {
            return;
        }
        blur.pending.set(true);
        let weak_blur = Rc::downgrade(&blur);
        let weak_run = Rc::downgrade(self);
        let runtime = crate::gpu_runtime::runtime(&blur.env);
        *blur.input.borrow_mut() = Some(crate::transition::pixel::FrozenInput::start(
            &self.visual,
            &runtime,
            move |outcome| {
                let Some(blur) = weak_blur.upgrade() else {
                    return;
                };
                blur.pending.set(false);
                match outcome {
                    Ok(texture) => {
                        let effect = crate::components::filtered::FrozenEffect::blur(
                            blur.radius.clone(),
                            &blur.env,
                            texture,
                        );
                        if let Some(run) = weak_run.upgrade() {
                            let root = run.visual.root.clone();
                            view::add_subview(&run.visual.host, effect.view());
                            view::set_frame(effect.view(), view::bounds(&run.visual.host));
                            view::set_autoresizing_flexible_size(effect.view());
                            // The root hides only once the blurred frame is
                            // on screen — never before, so no frame shows
                            // a gap (§2.6).
                            effect.on_first_present(move || {
                                view::set_hidden(&root, true);
                            });
                            effect.request_render();
                        }
                        *blur.effect.borrow_mut() = Some(effect);
                    }
                    // Not presented yet — the next tick retries the
                    // capture (§2.3).
                    Err(cocoa_ui::capture::CaptureError::Deferred) => {}
                    Err(cocoa_ui::capture::CaptureError::Failed(error)) => {
                        tracing::error!("a blur removal's frozen capture failed: {error}");
                        if let Some(run) = weak_run.upgrade() {
                            run.complete();
                        }
                    }
                }
            },
        ));
    }

    /// Without the filtered pipeline a nonzero blur radius has no
    /// realization — a user-triggerable failure, reported once per run.
    #[cfg(not(any(feature = "applied_filter", feature = "view_effect")))]
    fn update_blur(&self, radius: f32) {
        if radius > 0.0 && !self.blur_warned.replace(true) {
            crate::transition::transition_violation!(
                "waterui-apple: a blur transition needs the filtered pipeline \
                 (the view_effect or applied_filter feature)"
            );
        }
    }

    /// The pixel removal's per-tick work (§2.7): the published
    /// `TransitionFrame` follows the slot and a `Deferred` capture is
    /// re-issued.
    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
    fn update_pixel(self: &Rc<Self>) {
        let pixel = {
            let realization = self.realization.borrow();
            let Some(realization) = realization.as_ref() else {
                return;
            };
            let Some(pixel) = realization.pixel.as_ref().map(Rc::clone) else {
                return;
            };
            pixel
        };
        crate::transition::pixel::PixelRemoval::update(&pixel, self);
    }

    /// The completion sequence (§1.5): the site's slot release, the
    /// realization's drop, then the driver's drop off the tick — a
    /// `FrameClock` is never dropped inside its own callback, so the
    /// teardown lands on the main queue.
    pub(crate) fn complete(&self) {
        if self.completed.replace(true) {
            return;
        }
        if let Some(complete) = self.on_complete.borrow_mut().take() {
            complete();
        }
        self.realization.borrow_mut().take();
        let driver = self.driver.borrow_mut().take();
        let keep = self.keep.borrow_mut().take();
        cocoa_ui::main_queue::enqueue_local(self.mtm, move |_mtm| {
            drop(driver);
            drop(keep);
        });
    }
}

impl Drop for Ghost {
    /// Inside an enclosing ghost the run completes by itself — the
    /// window handler fires when the visuals leave the screen. Otherwise
    /// this is a cancel: the site's ordinary removal takes the ghost
    /// with it.
    fn drop(&mut self) {
        if !view::has_retired_ancestor(&self.0.visual.host) {
            self.0.complete();
        }
    }
}
