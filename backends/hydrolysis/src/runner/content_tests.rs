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
use crate::renderer::FontFamilyResolution;
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

/// The environment a capture mounts under: the assembled runner environment
/// a [`HydrolysisViewRenderer`] is installed with — theme tokens, so `text`
/// resolves its `Body` slot, and the shared font collection its text engine
/// shapes from.
fn capture_environment() -> Environment {
    capture_environment_with_fonts(waterui_text::FontCollection::from_parley(
        parley::FontContext::new(),
    ))
}

/// [`capture_environment`] with `collection` installed as the shared font
/// collection.
fn capture_environment_with_fonts(collection: waterui_text::FontCollection) -> Environment {
    let mut env = crate::renderer::tests::pumped_test_environment();
    collection.install(&mut env);
    env
}

/// A `FontCollection` carrying exactly the faces `blobs` register: built
/// with system discovery off, so what it holds is what was registered into
/// it and nothing of the host's.
fn isolated_collection(blobs: &[&[u8]]) -> waterui_text::FontCollection {
    let mut font_cx = parley::FontContext {
        collection: parley::fontique::Collection::new(parley::fontique::CollectionOptions {
            system_fonts: false,
            ..parley::fontique::CollectionOptions::default()
        }),
        source_cache: parley::fontique::SourceCache::default(),
    };
    for blob in blobs {
        font_cx.collection.register_fonts(
            parley::fontique::Blob::new(std::sync::Arc::new(blob.to_vec())),
            None,
        );
    }
    waterui_text::FontCollection::from_parley(font_cx)
}

impl Capture {
    fn start(view: AnyView, content_deadline: Duration) -> Self {
        Self::start_in(
            view,
            content_deadline,
            capture_environment(),
            FontFamilyResolution::Lenient,
        )
    }

    fn start_in(
        view: AnyView,
        content_deadline: Duration,
        env: Environment,
        family_resolution: FontFamilyResolution,
    ) -> Self {
        let executor = HeadlessMainThreadExecutor::thread_shared();
        let _ = executor_core::try_init_local_executor(executor.clone());
        let renderer = HydrolysisViewRenderer::new(
            Rc::new(MinimalTestTheme::default()),
            env,
            family_resolution,
        );
        let result: Rc<RefCell<Option<Result<RenderResult, RenderError>>>> = Rc::default();
        executor_core::spawn_local({
            let result = Rc::clone(&result);
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

    fn start_loader(gate: &Gate, content_deadline: Duration) -> Self {
        Self::start(AnyView::new(FakeLoader(gate.clone())), content_deadline)
    }

    /// Renders `view` to completion, driving the executor on its wake edges.
    fn render(view: AnyView) -> Result<RenderResult, RenderError> {
        Self::start(view, Duration::from_secs(30)).finish()
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

/// A `text` leaf resolves its `Body` font token out of the environment, so
/// a capture mounted under the assembled runner environment renders it
/// instead of panicking on the missing token (water-rs/waterui#2703).
#[test]
fn a_capture_renders_text_under_the_installed_environment() {
    use waterui::prelude::text;

    let rendered = Capture::render(AnyView::new(text("capture me")))
        .expect("a text leaf captures under the installed environment");
    assert_eq!((rendered.width(), rendered.height()), (64, 32));
}

/// A face registered into the installed collection answers a named family
/// inside a capture exactly as it does in a window: the collection built
/// here carries only the registered Roboto — no system faces — so "Roboto"
/// resolving proves the capture's engine shapes from the environment's
/// collection rather than one it built for itself
/// (water-rs/waterui#1963).
#[test]
fn a_capture_resolves_a_family_the_installed_collection_carries() {
    use waterui::prelude::text;
    use waterui_text::font::Font;

    let roboto = crate::text::fonts::installed_font_bytes("Roboto");
    let env = capture_environment_with_fonts(isolated_collection(&[&roboto]));

    let view = text("probe").font(Font::default().family("Roboto"));
    Capture::start_in(
        AnyView::new(view),
        Duration::from_secs(30),
        env,
        FontFamilyResolution::Strict,
    )
    .finish()
    .expect("a capture resolves a family the installed collection carries");
}

/// The installed collection is authoritative: a family the host's system
/// collection answers but the installed one does not fails strict
/// resolution inside the capture. Only an engine built from the installed
/// collection behaves that way — one shaped against a fresh system context,
/// as the capture's was before water-rs/waterui#1963, resolves the family
/// and renders instead.
#[test]
#[should_panic(expected = "is neither installed on this host nor provided by a declared font file")]
fn a_capture_shapes_only_against_the_installed_collection() {
    use waterui::prelude::text;
    use waterui_text::font::Font;

    let roboto = crate::text::fonts::installed_font_bytes("Roboto");
    let env = capture_environment_with_fonts(isolated_collection(&[&roboto]));
    // Any family the host resolves besides the one the collection carries:
    // `installed_font_bytes` already guarantees "Roboto" itself installed.
    let host_family = parley::FontContext::new()
        .collection
        .family_names()
        .find(|name| !name.eq_ignore_ascii_case("Roboto"))
        .expect("the probe needs a second host-installed family")
        .to_owned();

    let view = text("probe").font(Font::default().family(host_family));
    let _ = Capture::start_in(
        AnyView::new(view),
        Duration::from_secs(30),
        env,
        FontFamilyResolution::Strict,
    )
    .finish();
}

#[test]
fn a_capture_awaits_content_a_view_owes() {
    let gate = Gate::default();
    let capture = Capture::start_loader(&gate, Duration::from_secs(30));
    assert!(
        !capture.drain(),
        "the capture must not rasterize while the loader still owes content"
    );
    gate.open(Ok(()));
    let rendered = capture
        .finish()
        .expect("the capture renders once the content lands");
    assert_eq!((rendered.width(), rendered.height()), (64, 32));
}

#[test]
fn a_capture_names_the_view_still_owing_at_the_deadline() {
    let capture = Capture::start_loader(&Gate::default(), Duration::from_millis(50));
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
    let capture = Capture::start_loader(&gate, Duration::from_secs(30));
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
