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
//!
//! The `captures_content` runtimes live here too (§2.7): [`PixelOverlay`]
//! is the retired, window-filling host a pixel transition draws through,
//! [`PixelRemoval`] drives the frozen input plus effect body for a
//! removal, and [`PixelInsertion`] drives the live capture plus effect
//! body for an insertion. Each publishes its [`TransitionFrame`] on every
//! tick so the effect follows the slot through scroll and resize.

use alloc::rc::{Rc, Weak};
use core::cell::{Cell, RefCell};
use std::sync::Mutex;

use cocoa_ui::capture::{CaptureError, ViewCapture};
use cocoa_ui::{MainThreadMarker, PlatformView, Retained, view};
use objc2::runtime::ProtocolObject;
use objc2_metal::MTLTexture;
use waterui::reactive::{Binding, SignalExt};
use waterui_backend_core::{AnyView, Environment};
use waterui_core::layout::{LayoutDirection, Point, Rect as CoreRect, Size};
use waterui_core::transition::{TransitionFrame, TransitionState};
use waterui_graphics::gpu::GpuRuntime;

use crate::components::filtered::{FrozenEffect, LiveEffect, Sendable};
use crate::transition::ghost::GhostRun;
use crate::transition::host::TransitionHost;
use crate::transition::retire::RetiredVisual;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

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
        if width == 0 || height == 0 {
            // The slot has no pixels — the site's layout may not have
            // realized it yet; the next tick retries. A permanently
            // empty slot keeps deferring until the clock ends the run.
            on_ready(Err(CaptureError::Deferred));
            return Rc::new(Self { capture });
        }
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

/// The window-level overlay a pixel transition draws through (§2.7): a
/// `HostView` created already retired — the effect it holds is pure
/// visuals, never a hit target or an accessibility element — filling its
/// window with flexible autoresizing. `Drop` detaches it.
pub struct PixelOverlay {
    /// The retired host the effect's output mounts into.
    host: Retained<HostView>,
    /// The window the overlay was added to — `sync_window` moves the
    /// overlay when the host's window changes underneath it.
    window: RefCell<Retained<PlatformView>>,
}

impl core::fmt::Debug for PixelOverlay {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PixelOverlay").finish_non_exhaustive()
    }
}

/// The view a pixel overlay mounts into for `within`'s window — the
/// content view on `AppKit`, the `UIWindow` itself on `UIKit`.
fn overlay_parent(within: &PlatformView) -> Retained<PlatformView> {
    let window =
        view::window(within).expect("a pixel transition runs only while its host is windowed");
    #[cfg(target_os = "macos")]
    {
        window
            .contentView()
            .expect("an `NSWindow` with a mounted transition host has a content view")
    }
    #[cfg(target_os = "ios")]
    {
        Retained::into_super(window)
    }
}

impl PixelOverlay {
    /// A fresh retired overlay covering `within`'s window.
    pub fn over(within: &PlatformView) -> Self {
        let mtm = MainThreadMarker::new().expect("a pixel overlay builds on the main thread");
        let parent = overlay_parent(within);
        let host = HostView::new(mtm, view::bounds(&parent));
        host.retire();
        let view: &PlatformView = &host;
        view::add_subview(&parent, view);
        view::set_autoresizing_flexible_size(view);
        Self {
            host,
            window: RefCell::new(parent),
        }
    }

    /// Re-homes the overlay when `within` moved to another window — an
    /// insertion whose host changed windows mid-flight.
    fn sync_window(&self, within: &PlatformView) {
        let current = overlay_parent(within);
        let mut stored = self.window.borrow_mut();
        if !core::ptr::eq(&raw const *current, &raw const **stored) {
            view::remove_from_superview(&self.host);
            view::set_frame(&self.host, view::bounds(&current));
            view::add_subview(&current, &self.host);
            *stored = current;
        }
    }

    /// The overlay's own view — the effect output's parent.
    pub fn view(&self) -> &PlatformView {
        &self.host
    }
}

impl Drop for PixelOverlay {
    /// The overlay leaves the window with the instance that owned it.
    fn drop(&mut self) {
        view::remove_from_superview(&self.host);
    }
}

/// The `TransitionFrame` the slot reports right now: `source` is the
/// host's bounds in window space in top-left coordinates, `window` the
/// overlay's full size, `scale_factor` the backing scale, `direction`
/// the claim's resolved direction (§2.7).
#[expect(
    clippy::cast_possible_truncation,
    reason = "the layout contract is f32; kit geometry is f64"
)]
fn frame_now(
    overlay: &PlatformView,
    host: &PlatformView,
    direction: LayoutDirection,
) -> TransitionFrame {
    let source = view::bounds_in_window(host);
    let overlay_bounds = view::bounds(overlay);
    // `bounds_in_window` answers `AppKit`'s bottom-left window space;
    // the contract's `source` is top-left, so it mirrors about the
    // overlay's height. `UIKit` window space is already top-left.
    #[cfg(target_os = "macos")]
    let top = overlay_bounds.size.height - source.origin.y - source.size.height;
    #[cfg(target_os = "ios")]
    let top = source.origin.y;
    TransitionFrame::new(
        CoreRect::new(
            Point::new(source.origin.x as f32, top as f32),
            Size::new(source.size.width as f32, source.size.height as f32),
        ),
        Size::new(
            overlay_bounds.size.width as f32,
            overlay_bounds.size.height as f32,
        ),
        view::backing_scale_factor(overlay) as f32,
        direction,
    )
}

/// The `captures_content` removal runtime (§2.7): the window overlay,
/// the frame binding the effect body reads, the in-flight frozen
/// capture and the landed `FrozenEffect`. The ghost's tick republishes
/// the frame and retries a `Deferred` capture; a `Failed` one ends the
/// run.
pub struct PixelRemoval {
    /// The retired overlay the effect presents into.
    overlay: PixelOverlay,
    /// The published frame — written on every tick.
    frame: Binding<TransitionFrame>,
    /// The transition's effect body — consumed when the capture lands.
    body: RefCell<Option<AnyView>>,
    /// The site's environment — `FrozenEffect::from_body` reads its GPU
    /// services from it.
    env: Environment,
    /// A frozen capture is in flight — the next tick waits for it.
    pending: Cell<bool>,
    /// The in-flight capture — kept alive until it settles.
    input: RefCell<Option<Rc<FrozenInput>>>,
    /// The landed effect — `None` until the capture delivers.
    effect: RefCell<Option<FrozenEffect>>,
}

impl core::fmt::Debug for PixelRemoval {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PixelRemoval").finish_non_exhaustive()
    }
}

impl PixelRemoval {
    /// Builds the removal's overlay, frame binding and effect body. The
    /// first frozen capture waits for the first [`update`] — the tick
    /// runs after the site's borrow released, where the capture's
    /// synchronous layout preparation cannot re-enter the site's own
    /// layout handler.
    pub fn start(visual: &RetiredVisual, state: &TransitionState, env: &Environment) -> Rc<Self> {
        let overlay = PixelOverlay::over(&visual.host);
        let frame = Binding::container(frame_now(
            overlay.view(),
            &visual.host,
            visual.geometry.direction(),
        ));
        let body = state.body(AnyView::new(()), frame.computed());
        Rc::new(Self {
            overlay,
            frame,
            body: RefCell::new(Some(body)),
            env: env.clone(),
            pending: Cell::new(false),
            input: RefCell::new(None),
            effect: RefCell::new(None),
        })
    }

    /// The tick's frame republish plus the `Deferred` capture retry —
    /// called on every advancing frame after the site placed the host.
    pub fn update(this: &Rc<Self>, run: &Rc<GhostRun>) {
        this.overlay.sync_window(&run.visual.host);
        this.frame.set(frame_now(
            this.overlay.view(),
            &run.visual.host,
            run.visual.geometry.direction(),
        ));
        if !this.pending.get() && this.effect.borrow().is_none() {
            Self::attempt(this, &run.visual, Rc::downgrade(run));
        }
    }

    /// Issues one frozen capture of the retired subtree — the effect
    /// installs on its texture, a `Deferred` outcome waits a tick and a
    /// `Failed` one ends the run (§10).
    fn attempt(this: &Rc<Self>, visual: &RetiredVisual, weak_run: Weak<GhostRun>) {
        this.pending.set(true);
        let runtime = crate::gpu_runtime::runtime(&this.env);
        let weak_this = Rc::downgrade(this);
        *this.input.borrow_mut() = Some(FrozenInput::start(visual, &runtime, move |outcome| {
            let Some(this) = weak_this.upgrade() else {
                return;
            };
            this.pending.set(false);
            match outcome {
                Ok(texture) => {
                    let Some(run) = weak_run.upgrade() else {
                        return;
                    };
                    let body = this
                        .body
                        .borrow_mut()
                        .take()
                        .expect("the effect body is consumed exactly once");
                    match FrozenEffect::from_body(body, &this.env, texture) {
                        Some(effect) => {
                            view::add_subview(this.overlay.view(), effect.view());
                            view::set_frame(effect.view(), view::bounds(this.overlay.view()));
                            view::set_autoresizing_flexible_size(effect.view());
                            // The root hides only once the effect's first
                            // frame is on screen — never before (§2.7).
                            let root = run.visual.root.clone();
                            effect.on_first_present(move || {
                                view::set_hidden(&root, true);
                            });
                            effect.request_render();
                            *this.effect.borrow_mut() = Some(effect);
                        }
                        // The body violation already reported — the
                        // transition ends at once.
                        None => run.complete(),
                    }
                }
                // Not presented yet — the next tick retries the capture.
                Err(CaptureError::Deferred) => {}
                Err(CaptureError::Failed(error)) => {
                    tracing::error!("a pixel removal's frozen capture failed: {error}");
                    if let Some(run) = weak_run.upgrade() {
                        run.complete();
                    }
                }
            }
        }));
    }
}

/// The `captures_content` insertion runtime (§2.7): the window overlay
/// and the `LiveEffect` drawing the still-live — and hidden-in-place —
/// content through the body's chain. The run's tick republishes the
/// frame after the site placed the host.
pub struct PixelInsertion {
    /// The retired overlay the effect presents into.
    overlay: PixelOverlay,
    /// The published frame — written on every tick.
    frame: Binding<TransitionFrame>,
    /// The live-capture effect — held for its lifetime: its drop
    /// un-hides the content and detaches the output.
    _effect: LiveEffect,
}

impl core::fmt::Debug for PixelInsertion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PixelInsertion").finish_non_exhaustive()
    }
}

impl PixelInsertion {
    /// Builds the insertion's overlay, frame binding and live-capture
    /// effect over `host`'s content. `None` only when the body is not a
    /// `FilteredView` chain — a user-triggerable violation the macro
    /// already reported, so the caller ends the transition.
    pub fn start(
        host: &Rc<TransitionHost>,
        state: &TransitionState,
        direction: LayoutDirection,
        env: &Environment,
    ) -> Option<Self> {
        let overlay = PixelOverlay::over(&host.host);
        let frame = Binding::container(frame_now(overlay.view(), &host.host, direction));
        let body = state.body(AnyView::new(()), frame.computed());
        let effect = LiveEffect::from_body(body, &host.content_view, overlay.view(), env)?;
        Some(Self {
            overlay,
            frame,
            _effect: effect,
        })
    }

    /// The tick's frame republish — called on every advancing frame
    /// after the site placed the host.
    pub fn update(&self, host: &Rc<TransitionHost>, direction: LayoutDirection) {
        let host_view: &PlatformView = &host.host;
        self.overlay.sync_window(host_view);
        self.frame
            .set(frame_now(self.overlay.view(), host_view, direction));
    }
}
