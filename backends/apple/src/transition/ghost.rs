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
}

/// The run behind [`Ghost`]: everything the tick and the completion
/// sequence need.
struct GhostRun {
    /// The main-thread marker completion tasks enqueue under.
    mtm: MainThreadMarker,
    /// The retired host and root — detaches when the run dies.
    visual: RetiredVisual,
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
        let _ = env; // GPU services land with the pixel removal (stage 9)
        let target = PropertyTarget::new(visual.host.clone(), visual.root.clone());
        let run = Rc::new_cyclic(|_weak: &Weak<GhostRun>| GhostRun {
            mtm,
            visual,
            state: RefCell::new(state),
            slot,
            realization: RefCell::new(Some(GhostRealization { target })),
            driver: RefCell::new(None),
            keep: RefCell::new(None),
            on_complete: RefCell::new(Some(on_complete)),
            completed: Cell::new(false),
        });
        // The progress-0 sample lands before the first presented frame.
        let sample = run.state.borrow().properties(run.visual.geometry);
        if let Some(realization) = run.realization.borrow().as_ref() {
            realization.target.apply(&sample);
        }
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
}

impl GhostRun {
    /// The completion sequence (§1.5): the site's slot release, the
    /// realization's drop, then the driver's drop off the tick — a
    /// `FrameClock` is never dropped inside its own callback, so the
    /// teardown lands on the main queue.
    fn complete(&self) {
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
