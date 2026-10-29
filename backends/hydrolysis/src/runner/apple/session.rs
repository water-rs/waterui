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
use waterui::window::WindowState;
use waterui_core::Environment;

use super::accessibility::AccessibilityBridge;
use super::surface::AppleViewSurface;
use super::text_input::ImeShared;
use crate::platform::GpuSurfaceWindow;
use crate::runner::host_executor::HostMainThreadExecutor;
use crate::runner::window::{
    FrameHost, FrameOutcome, FrameTransaction, RuntimeWindow, render_window, reports_ui_idle,
    run_frame_transaction,
};
use crate::text::ParleyEngine;

/// The process-wide executor every Apple scene drains.
pub(super) type AppleExecutor = HostMainThreadExecutor<MainQueueWake>;

/// What a host window supplies the session beyond the platform contract.
pub(super) trait AppleHostWindow: GpuSurfaceWindow<Presentation = AppleViewSurface> {
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
    ime: Rc<ImeShared>,
    accessibility: AccessibilityBridge,
    /// Windows the app asked for while mounted; a host that cannot open
    /// them asserts this queue stays empty.
    pending_window_queue: Rc<RefCell<Vec<waterui::window::Window>>>,
    /// A frame reached the surface at least once; with an idle
    /// `wants_next_frame` it arms the one-shot readiness log.
    presented_once: Cell<bool>,
    ready_logged: Cell<bool>,
}

impl<P: AppleHostWindow> AppleSession<P> {
    pub(super) const fn new(
        env: Environment,
        runtime: RuntimeWindow<P, ParleyEngine>,
        executor: AppleExecutor,
        ime: Rc<ImeShared>,
        accessibility: AccessibilityBridge,
        pending_window_queue: Rc<RefCell<Vec<waterui::window::Window>>>,
    ) -> Self {
        Self {
            env,
            runtime,
            executor,
            ime,
            accessibility,
            pending_window_queue,
            presented_once: Cell::new(false),
            ready_logged: Cell::new(false),
        }
    }

    /// The one coordinated frame transaction — the display link's tick lands
    /// here. The sequence is the shared [`run_frame_transaction`]; this host
    /// supplies the steps around it.
    pub(super) fn on_frame(&mut self) -> FrameOutcome {
        let outcome = run_frame_transaction(self);
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
        self.ime.flush(&mut self.runtime.renderer);
        self.accessibility
            .perform_actions(&mut self.runtime, &self.env);
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
        render_window(&mut self.runtime, &self.env, &mut || executor.drain())
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
        self.accessibility.publish(&mut self.runtime);
        // The renderer's authoritative editor state reaches the input
        // method, and the keyboard follows focus through the platform's
        // text-input state, which `render_window` already synced.
        let snapshot = self.runtime.renderer.focused_editor_snapshot();
        self.ime.sync_from_renderer(snapshot);
        assert!(
            self.pending_window_queue.borrow().is_empty(),
            "hydrolysis UIKit: secondary windows are unsupported — a scene mounts exactly one window"
        );
    }

    fn finished(&mut self, wants_next_frame: bool) {
        if reports_ui_idle(
            self.presented_once.get(),
            wants_next_frame
                || matches!(self.runtime.engine_next, crate::renderer::Next::At { .. }),
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
