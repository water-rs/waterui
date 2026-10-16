//! `HydrolysisViewRenderer` captures await the content a view owes: a
//! loader the test resolves by hand stands in for a `Photo` decode, and the
//! render future runs on the pump-driven executor, driven on its wake edge.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use core::time::Duration;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use waterui_core::view_renderer::{
    CustomViewRenderer as _, RenderError, RenderRequest, RenderResult,
};
use waterui_core::{AnyView, ContentLedger, Environment, View, layout::Size};

use super::executor::HeadlessMainThreadExecutor;
use crate::renderer::tests::MinimalTestTheme;
use crate::view_renderer::HydrolysisViewRenderer;

/// How a [`Gate`] resolves.
type Outcome = Result<(), String>;

/// The hand-resolved load: the test opens it, the loader's task wakes.
#[derive(Default)]
struct GateState {
    outcome: Option<Outcome>,
    waker: Option<Waker>,
}

#[derive(Clone, Default)]
struct Gate(Rc<RefCell<GateState>>);

impl Gate {
    fn open(&self, outcome: Outcome) {
        let waker = {
            let mut state = self.0.borrow_mut();
            state.outcome = Some(outcome);
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl Future for Gate {
    type Output = Outcome;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Outcome> {
        let mut state = self.0.borrow_mut();
        if let Some(outcome) = state.outcome.take() {
            return Poll::Ready(outcome);
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// Why the fake load failed.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct LoadError(String);

/// A `Photo`-shaped loader: it owes its content from mount and resolves the
/// entry when its gate opens.
struct FakeLoader(Gate);

impl View for FakeLoader {
    fn body(self, env: &Environment) -> impl View {
        let owed = ContentLedger::owe::<Self>(env, "fake://image");
        let gate = self.0;
        executor_core::spawn_local(async move {
            let outcome = gate.await;
            if let Some(owed) = owed {
                match outcome {
                    Ok(()) => owed.fulfil(),
                    Err(message) => owed.fail(LoadError(message)),
                }
            }
        })
        .detach();
        // Nothing to typeset: the capture is about the content the body
        // owes, not what it draws.
    }
}

/// A capture of `FakeLoader` running on the thread's pump-driven executor.
struct Capture {
    executor: HeadlessMainThreadExecutor,
    result: Rc<RefCell<Option<Result<RenderResult, RenderError>>>>,
}

impl Capture {
    fn start(gate: &Gate, content_deadline: Duration) -> Self {
        let executor = HeadlessMainThreadExecutor::thread_shared();
        let _ = executor_core::try_init_local_executor(executor.clone());
        let renderer = HydrolysisViewRenderer::new(Rc::new(MinimalTestTheme::default()));
        let result: Rc<RefCell<Option<Result<RenderResult, RenderError>>>> = Rc::default();
        executor_core::spawn_local({
            let result = Rc::clone(&result);
            let view = AnyView::new(FakeLoader(gate.clone()));
            async move {
                let capture = renderer
                    .render_to_rgba(
                        view,
                        RenderRequest::new(Size::new(64.0, 32.0), content_deadline),
                    )
                    .await;
                *result.borrow_mut() = Some(capture);
            }
        })
        .detach();
        Self { executor, result }
    }

    /// Runs what is queued; the capture's state after the drain.
    fn drain(&self) -> bool {
        let _ = self.executor.drain();
        self.result.borrow().is_some()
    }

    /// Drives the executor on its wake edge until the capture resolves.
    fn finish(self) -> Result<RenderResult, RenderError> {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.drain() {
            assert!(
                self.executor
                    .wait_queued(deadline.saturating_duration_since(Instant::now())),
                "the capture stalled with nothing re-queued on the executor"
            );
        }
        self.result
            .borrow_mut()
            .take()
            .expect("the drained capture stored its result")
    }
}

#[test]
fn a_capture_awaits_content_a_view_owes() {
    let gate = Gate::default();
    let capture = Capture::start(&gate, Duration::from_secs(30));
    assert!(
        !capture.drain(),
        "the capture must not rasterize while the loader still owes content"
    );
    gate.open(Ok(()));
    let rendered = capture
        .finish()
        .expect("the capture renders once the content lands");
    assert_eq!((rendered.width, rendered.height), (64, 32));
}

#[test]
fn a_capture_names_the_view_still_owing_at_the_deadline() {
    let capture = Capture::start(&Gate::default(), Duration::from_millis(50));
    let Err(error) = capture.finish() else {
        panic!("a load that never lands fails the capture");
    };
    assert!(
        matches!(&error, RenderError::ContentPending { outstanding, .. } if outstanding.len() == 1),
        "the deadline reports the owed entry: {error}"
    );
    assert_eq!(
        error.to_string(),
        "FakeLoader (fake://image) still owes content after 50ms"
    );
}

#[test]
fn a_capture_reports_a_failed_load() {
    let gate = Gate::default();
    let capture = Capture::start(&gate, Duration::from_secs(30));
    let _ = capture.drain();
    gate.open(Err("decode failed".into()));
    let Err(error) = capture.finish() else {
        panic!("a failed load fails the capture");
    };
    assert!(
        matches!(
            &error,
            RenderError::ContentFailed {
                view: "FakeLoader",
                ..
            }
        ),
        "the failure names the view: {error}"
    );
    assert_eq!(
        core::error::Error::source(&error).map(ToString::to_string),
        Some("decode failed".into())
    );
}
