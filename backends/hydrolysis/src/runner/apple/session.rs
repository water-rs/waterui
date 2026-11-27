//! One window's frame transaction and its platform publishes.
//!
//! [`AppleSession`] is the mounted window: the runtime window, the process
//! environment, the executor and the editing and accessibility publishes,
//! driven by one frame transaction per display-link tick. The host window it
//! wraps supplies what differs between the `UIKit` and `AppKit` hosts — where
//! input and metrics come from, how a redraw is requested and how occlusion
//! is read — through [`AppleHostWindow`].

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cocoa_ui::main_queue::MainQueueWake;
use nami::Signal as _;
use waterui_backend_core::window::WindowState;
use waterui_core::{Environment, contract_violation};

#[cfg(feature = "accessibility")]
use super::accessibility::AccessibilityBridge;
use super::surface::AppleViewSurface;
#[cfg(any(feature = "text-field", feature = "secure-field"))]
use super::text_input::ImeShared;
use crate::platform::PaintedWindow;
use crate::runner::host_executor::HostMainThreadExecutor;
use crate::runner::window::{
    FrameHost, FrameOutcome, FrameTransaction, RootWindow, RuntimeWindow, WindowFrame,
    render_window, reports_ui_idle, run_frame_transaction,
};
use crate::text::ParleyEngine;

/// The process-wide executor every Apple scene drains.
pub(super) type AppleExecutor = HostMainThreadExecutor<MainQueueWake>;

/// What a host window supplies the session beyond the platform contract.
pub(super) trait AppleHostWindow: PaintedWindow<Presentation = AppleViewSurface> {
    /// The window's frame transaction, which gates its frame wake.
    fn frame_transaction(&self) -> &FrameTransaction;

    /// Whether a drawn frame can reach the user: the view is on a screen.
    fn presentation_attached(&self) -> bool;

    /// Consumes the latched redraw request.
    fn take_redraw_pending(&self) -> bool;

    /// Applies what native callbacks stashed since the last frame — metrics,
    /// insets, appearance — at the head of the transaction, before input is
    /// dispatched.
    fn begin_frame(&mut self, env: &Environment);

    /// The window's content asked to close.
    fn close_requested(&mut self);
}

/// A mounted window.
pub(super) struct AppleSession<P: AppleHostWindow> {
    pub(super) env: Environment,
    pub(super) runtime: RuntimeWindow<P, ParleyEngine>,
    executor: AppleExecutor,
    /// The document the platform's text system edits.
    #[cfg(any(feature = "text-field", feature = "secure-field"))]
    ime: Rc<ImeShared>,
    #[cfg(feature = "accessibility")]
    accessibility: AccessibilityBridge,
    /// Windows the app asked for while mounted — popups among them; a scene
    /// mounts exactly one window, so they are reported and dropped.
    pending_window_queue: Rc<RefCell<Vec<RootWindow<ParleyEngine>>>>,
    /// A frame reached the surface at least once; with an idle
    /// `wants_next_frame` it arms the one-shot readiness log.
    presented_once: Cell<bool>,
    ready_logged: Cell<bool>,
    /// The surface failed for good: the window draws no further frame.
    rendering_failed: bool,
}

impl<P: AppleHostWindow> AppleSession<P> {
    pub(super) const fn new(
        env: Environment,
        runtime: RuntimeWindow<P, ParleyEngine>,
        executor: AppleExecutor,
        #[cfg(any(feature = "text-field", feature = "secure-field"))] ime: Rc<ImeShared>,
        #[cfg(feature = "accessibility")] accessibility: AccessibilityBridge,
        pending_window_queue: Rc<RefCell<Vec<RootWindow<ParleyEngine>>>>,
    ) -> Self {
        Self {
            env,
            runtime,
            executor,
            #[cfg(any(feature = "text-field", feature = "secure-field"))]
            ime,
            #[cfg(feature = "accessibility")]
            accessibility,
            pending_window_queue,
            presented_once: Cell::new(false),
            ready_logged: Cell::new(false),
            rendering_failed: false,
        }
    }

    /// The one coordinated frame transaction — the display link's tick lands
    /// here. The sequence is the shared [`run_frame_transaction`]; this host
    /// supplies the steps around it. The engine renders each frame at the
    /// instant it starts.
    pub(super) fn on_frame(&mut self) -> FrameOutcome {
        let outcome = run_frame_transaction(self, None);
        tracing::debug!(
            target: "waterui::hydrolysis::apple",
            wants_next_frame = outcome.wants_next_frame,
            has_deadline = outcome.deadline.is_some(),
            should_close = outcome.should_close,
            "frame outcome"
        );
        outcome
    }

    /// Whether the window's content asked the host to close.
    pub(super) fn closed(&self) -> bool {
        self.runtime.window.state.snapshot() == WindowState::Closed
    }
}

impl<P: AppleHostWindow> FrameHost for AppleSession<P> {
    type Platform = P;
    type Engine = ParleyEngine;

    fn parts(&mut self) -> (&mut RuntimeWindow<P, ParleyEngine>, &Environment) {
        (&mut self.runtime, &self.env)
    }

    fn frame_transaction(&self) -> &FrameTransaction {
        self.runtime.platform.frame_transaction()
    }

    fn drain_executor(&self) {
        self.executor.drain();
    }

    fn before_input(&mut self) {
        self.runtime.platform.begin_frame(&self.env);
        // The view may have arrived on or left a screen since the last frame.
        self.runtime.sync_occlusion_and_post_restore();
        // Edits the input method made since the last frame reach the focused
        // model before this frame dispatches input and lays out.
        #[cfg(any(feature = "text-field", feature = "secure-field"))]
        self.ime.flush(&mut self.runtime.renderer);
        #[cfg(feature = "accessibility")]
        self.accessibility
            .perform_actions(&mut self.runtime, &self.env);
    }

    fn rendering_ended(&self) -> bool {
        self.rendering_failed
    }

    fn should_close(&self) -> bool {
        self.closed()
    }

    fn presentation_attached(&self) -> bool {
        self.runtime.platform.presentation_attached()
    }

    fn take_redraw_pending(platform: &P) -> bool {
        platform.take_redraw_pending()
    }

    fn render(&mut self) -> bool {
        let executor = self.executor.clone();
        match render_window(&mut self.runtime, &self.env, &mut || executor.drain()) {
            WindowFrame::Failed(error) => {
                tracing::error!(
                    target: "waterui::hydrolysis::apple",
                    "{:#}; the window cannot render and draws no further frame",
                    waterui_core::Error::from(error)
                );
                self.rendering_failed = true;
                false
            }
            frame => frame.presented(),
        }
    }

    fn after_render(&mut self, presented: bool) {
        if presented {
            self.presented_once.set(true);
        }
    }

    fn notify_close(&mut self) {
        self.runtime.platform.close_requested();
    }

    fn publish(&mut self, _flushed: bool) {
        #[cfg(feature = "accessibility")]
        self.accessibility.publish(&mut self.runtime);
        // The renderer's authoritative editor state reaches the input
        // method, and the keyboard follows focus through the platform's
        // text-input state, which `render_window` already synced.
        #[cfg(any(feature = "text-field", feature = "secure-field"))]
        {
            let snapshot = self.runtime.renderer.focused_editor_snapshot();
            self.ime.sync_from_renderer(snapshot);
        }
        let secondary_windows = self.pending_window_queue.borrow().len();
        if secondary_windows != 0 {
            // The windows are dropped unmounted so the violation is not
            // reported again by every later frame.
            self.pending_window_queue.borrow_mut().clear();
            contract_violation!(
                "hydrolysis apple: secondary windows are unsupported — a scene mounts exactly one window; {secondary_windows} window(s) dropped"
            );
        }
    }

    fn finished(&mut self, pending: bool) {
        if reports_ui_idle(
            self.presented_once.get(),
            pending || matches!(self.runtime.engine_next, crate::renderer::Next::At { .. }),
            self.runtime.is_hidden(),
        ) && !self.ready_logged.get()
        {
            tracing::info!(
                target: "waterui::hydrolysis::apple",
                "hydrolysis apple: first frame presented; ui idle"
            );
            self.ready_logged.set(true);
        }
    }
}
