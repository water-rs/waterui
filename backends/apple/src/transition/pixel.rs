//! The frozen exit capture (§2.3): one `ViewCapture` of the retired
//! root into a private texture — the input a removal's
//! [`crate::components::filtered::FrozenEffect`] blurs. The capture is
//! owned by the ghost: the retired root's `FrozenCapturable`s answer
//! blits of their last presented frames inside it, so the frame the
//! user last saw is exactly what the blur reads.
//!
//! `Deferred` is re-issued by the caller on the next tick; a `Failed`
//! outcome is the contract's error — the caller logs it and ends the
//! transition (§10).

use alloc::rc::Rc;
use std::sync::Mutex;

use cocoa_ui::capture::{CaptureError, ViewCapture};
use cocoa_ui::{MainThreadMarker, Retained, view};
use objc2::runtime::ProtocolObject;
use objc2_metal::MTLTexture;
use waterui_graphics::gpu::GpuRuntime;

use crate::components::filtered::Sendable;
use crate::transition::retire::RetiredVisual;

type MetalTexture = ProtocolObject<dyn MTLTexture>;

/// One `ViewCapture` over the retired root — kept alive until the
/// capture settles or the ghost ends (§2.3).
pub struct FrozenInput {
    /// The capture pipeline — owns the in-flight raster, composition
    /// and surface registrations.
    capture: Rc<ViewCapture>,
}

impl core::fmt::Debug for FrozenInput {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FrozenInput").finish_non_exhaustive()
    }
}

impl FrozenInput {
    /// Starts one capture of `visual.root` into a private target in the
    /// filtered presentation format, sized to the slot's pixels at the
    /// window's scale. `on_ready` runs on the main thread:
    /// `Ok(texture)` lands the frame, `Err(Deferred)` asks the caller to
    /// retry on the next tick, `Err(Failed)` is the contract's error.
    ///
    /// The capture's context is the runtime's current one — the frozen
    /// surfaces blit on their own recorded queues at the Metal level,
    /// so only the target's device matters, and a dead context defers
    /// rather than allocates on it.
    pub fn start(
        visual: &RetiredVisual,
        runtime: &GpuRuntime,
        on_ready: impl Fn(Result<Retained<MetalTexture>, CaptureError>) + 'static,
    ) -> Rc<Self> {
        let mtm = MainThreadMarker::new().expect("a frozen capture starts on the main thread");
        let capture = Rc::new(ViewCapture::new(mtm, visual.root.clone()));
        // Frozen producers never redraw — the slots answer blits, not
        // redraw requests.
        capture.set_on_redraw(|| {});
        let context = runtime.context();
        if context.device_lost_reason().is_some() {
            // Nothing allocates on a dead context — the next tick
            // retries on the rebuilt one.
            on_ready(Err(CaptureError::Deferred));
            return Rc::new(Self { capture });
        }
        let window =
            view::window(&visual.host).expect("a ghost runs only while its host is windowed");
        #[cfg(target_os = "macos")]
        let scale = window.backingScaleFactor();
        #[cfg(target_os = "ios")]
        let scale = window.screen().scale();
        let (width, height) = crate::components::filtered::pixel_size(&visual.root, scale);
        let target = crate::components::filtered::new_capture_input(&context, width, height);
        // The completion is `Fn` and `Send`-bound — the texture and the
        // non-`Send` callback cross it inside slots, and `ViewCapture`
        // lands them on the main queue.
        let target_slot = Mutex::new(Some(Sendable(target.clone())));
        let ready_slot = Mutex::new(Some(Sendable(on_ready)));
        capture.capture(&target, context.generation(), move |outcome| {
            let ready = ready_slot
                .lock()
                .expect("the capture completion fires once")
                .take();
            let Some(Sendable(on_ready)) = ready else {
                return;
            };
            match outcome {
                Ok(()) => {
                    let target = target_slot
                        .lock()
                        .expect("the capture completion fires once")
                        .take();
                    if let Some(Sendable(target)) = target {
                        on_ready(Ok(target));
                    }
                }
                Err(error) => on_ready(Err(error)),
            }
        });
        Rc::new(Self { capture })
    }
}

impl Drop for FrozenInput {
    /// Ends the capture's surface registrations and releases its GPU
    /// state — the `FilteredGuard` teardown's mirror for a capture with
    /// no filtered state behind it.
    fn drop(&mut self) {
        self.capture.shutdown();
    }
}
