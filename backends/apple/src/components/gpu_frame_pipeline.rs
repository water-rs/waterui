//! A mounted GPU surface's frame cycle against its hosted renderer, apart
//! from any window: the renderer bound to one context generation, its
//! creation, and the one frame it has in flight.
//!
//! Nothing here waits on a render thread. Creating the renderer and
//! producing a frame both resolve through the host's completion waker; the
//! host composites a produced frame into the next target its display link
//! or capture hands it. While creation or a frame is in flight the host
//! asks for no display-link ticks — the completion wake, never a timer or a
//! repeated poll, brings the next step.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::sync::Arc;
use core::task::{Context, Poll};

use futures::future::LocalBoxFuture;
use waterui_graphics::backend::cherenkov::Next;
use waterui_graphics::gpu::{Display, FrameTime, SharedGpuContext, wgpu};
use waterui_graphics::offscreen::OffscreenSize;

use super::{FrameOutcome, HostedError, HostedRenderer};
use crate::gpu_runtime::EngineGeneration;

/// A hosted renderer's creation on one context generation.
pub(super) type RendererCreation =
    LocalBoxFuture<'static, Result<Box<dyn HostedRenderer>, HostedError>>;

/// What a frame is produced for: the target's pixel size, its display and
/// the production target timestamp.
#[derive(Clone, Copy, Debug)]
pub(super) struct FrameRequest {
    pub(super) size: OffscreenSize,
    pub(super) display: Display,
    pub(super) time: FrameTime,
}

/// The renderer bound to the exact context generation that created it,
/// with its creation or its frame — the pair is the binding: a newer
/// context replaces it whole, its in-flight frame included.
pub(super) struct FramePipeline {
    bound: Option<Bound>,
}

struct Bound {
    context: Arc<SharedGpuContext>,
    stage: Stage,
}

enum Stage {
    /// The renderer is being created on the bound context.
    Creating(RendererCreation),
    /// The renderer and the frame it holds.
    Live {
        renderer: Box<dyn HostedRenderer>,
        frame: Frame,
    },
}

enum Frame {
    /// Nothing submitted.
    Idle,
    /// Submitted; the render thread is producing it.
    InFlight,
    /// Produced: composited into the next target, or owed again.
    Done(FrameOutcome),
}

/// What one step left the host to do with its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    /// Creation or a frame is in flight: the target stays unwritten, and
    /// the completion wake asks for the next step.
    Awaiting,
    /// The frame came out without this surface's latest staged state: the
    /// target stays unwritten and a new frame is owed.
    Unproduced,
    /// A produced frame is ready to composite into the target.
    Ready,
}

impl FramePipeline {
    /// A pipeline with no renderer: the first frame creates it.
    pub(super) const fn new() -> Self {
        Self { bound: None }
    }

    /// Binds to `context`. A pair bound to another generation is dropped
    /// whole — renderer or creation, and the frame in flight on it — and
    /// `create` starts a renderer on `context`; a pair already on
    /// `context` is kept.
    pub(super) fn bind(
        &mut self,
        context: &Arc<SharedGpuContext>,
        create: impl FnOnce() -> RendererCreation,
    ) {
        if self
            .bound
            .as_ref()
            .is_some_and(|bound| Arc::ptr_eq(&bound.context, context))
        {
            return;
        }
        self.bound = Some(Bound {
            context: context.clone(),
            stage: Stage::Creating(create()),
        });
    }

    /// The context generation the renderer is bound to.
    pub(super) fn context(&self) -> Option<&Arc<SharedGpuContext>> {
        self.bound.as_ref().map(|bound| &bound.context)
    }

    /// Drops the renderer, its creation and its frame without waiting on
    /// the render thread — the mount's teardown.
    pub(super) fn unbind(&mut self) {
        self.bound = None;
    }

    /// Whether creation or a frame is in flight: the host's completion
    /// waker, not its display link, brings the next step.
    pub(super) fn awaiting(&self) -> bool {
        matches!(
            self.bound.as_ref().map(|bound| &bound.stage),
            Some(
                Stage::Creating(_)
                    | Stage::Live {
                        frame: Frame::InFlight,
                        ..
                    }
            )
        )
    }

    /// Whether a live renderer holds no frame — none in flight, none
    /// produced and waiting for a target.
    #[cfg(all(target_os = "macos", any(test, feature = "native-test")))]
    pub(super) fn idle(&self) -> bool {
        matches!(
            self.bound.as_ref().map(|bound| &bound.stage),
            Some(Stage::Live {
                frame: Frame::Idle,
                ..
            })
        )
    }

    /// Drives creation and the in-flight frame as far as they have come,
    /// registering `cx`'s waker for what is still running: `Pending` while
    /// either runs, otherwise `Ready` with whether this poll settled one —
    /// a renderer now ready to submit with, or a frame to composite or
    /// owe. A wake that finds nothing settled (a frame discarded after its
    /// waker was registered, or one a display-link step already took)
    /// answers `false`, so it asks the host for nothing.
    ///
    /// # Errors
    ///
    /// The creation's or the frame's [`HostedError`]. A failed creation
    /// unbinds, so only a new context generation creates again; a failed
    /// frame leaves the renderer idle.
    pub(super) fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<bool, HostedError>> {
        let Some(bound) = self.bound.as_mut() else {
            return Poll::Ready(Ok(false));
        };
        let mut settled = false;
        if let Stage::Creating(creation) = &mut bound.stage {
            match creation.as_mut().poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => {
                    self.bound = None;
                    return Poll::Ready(Err(error));
                }
                Poll::Ready(Ok(renderer)) => {
                    bound.stage = Stage::Live {
                        renderer,
                        frame: Frame::Idle,
                    };
                    settled = true;
                }
            }
        }
        let Stage::Live { renderer, frame } = &mut bound.stage else {
            unreachable!("a settled creation is live above");
        };
        if matches!(frame, Frame::InFlight) {
            match renderer.poll_frame(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => {
                    *frame = Frame::Idle;
                    return Poll::Ready(Err(error));
                }
                Poll::Ready(Ok(outcome)) => {
                    *frame = Frame::Done(outcome);
                    settled = true;
                }
            }
        }
        Poll::Ready(Ok(settled))
    }

    /// One frame step for a target the host holds now: drives creation and
    /// the frame in flight, submits a frame for `request` when the renderer
    /// is idle — `before` runs just ahead of the submission, where the
    /// frame's UI-side work belongs — and reports what the target gets.
    ///
    /// # Errors
    ///
    /// Creation's, the submission's or the frame's [`HostedError`].
    pub(super) fn step(
        &mut self,
        cx: &mut Context<'_>,
        request: FrameRequest,
        before: impl FnOnce(),
    ) -> Result<Step, HostedError> {
        if self.poll(cx)?.is_pending() {
            return Ok(Step::Awaiting);
        }
        let Some(Bound {
            stage: Stage::Live { frame, .. },
            ..
        }) = self.bound.as_mut()
        else {
            return Ok(Step::Awaiting);
        };
        if matches!(frame, Frame::Idle) {
            self.submit(cx, request, before)?;
        }
        Ok(self.settled_step())
    }

    /// The step the frame's current state gives the host, without
    /// submitting.
    const fn settled_step(&mut self) -> Step {
        let Some(Bound {
            stage: Stage::Live { frame, .. },
            ..
        }) = self.bound.as_mut()
        else {
            return Step::Awaiting;
        };
        match frame {
            Frame::Idle | Frame::InFlight => Step::Awaiting,
            Frame::Done(FrameOutcome::Rendered(_)) => Step::Ready,
            Frame::Done(FrameOutcome::Unproduced) => {
                *frame = Frame::Idle;
                Step::Unproduced
            }
        }
    }

    /// Submits a frame for `request` on an idle renderer — `before` runs
    /// just ahead of it — and polls it once so its completion wakes `cx`.
    /// A renderer still being created, or already holding a frame, submits
    /// nothing.
    ///
    /// # Errors
    ///
    /// The submission's or an already-failed frame's [`HostedError`].
    pub(super) fn submit(
        &mut self,
        cx: &mut Context<'_>,
        request: FrameRequest,
        before: impl FnOnce(),
    ) -> Result<(), HostedError> {
        let Some(Bound {
            stage: Stage::Live { renderer, frame },
            ..
        }) = self.bound.as_mut()
        else {
            return Ok(());
        };
        if !matches!(frame, Frame::Idle) {
            return Ok(());
        }
        before();
        renderer.submit(request.size, request.display, request.time)?;
        *frame = Frame::InFlight;
        match self.poll(cx) {
            Poll::Ready(Err(error)) => Err(error),
            Poll::Ready(Ok(_)) | Poll::Pending => Ok(()),
        }
    }

    /// Composites the produced frame into `target` and answers when the
    /// engine wants the next one, with the generation evidence the
    /// target's completion checks.
    ///
    /// # Panics
    ///
    /// When no produced frame is ready — the host composites only after a
    /// [`Step::Ready`].
    pub(super) fn composite(
        &mut self,
        target: &wgpu::Texture,
        display: Display,
    ) -> (Next, Option<Rc<EngineGeneration>>) {
        let Some(Bound {
            stage: Stage::Live { renderer, frame },
            ..
        }) = self.bound.as_mut()
        else {
            panic!("a GPU surface composites only a produced frame");
        };
        let Frame::Done(FrameOutcome::Rendered(next)) = core::mem::replace(frame, Frame::Idle)
        else {
            panic!("a GPU surface composites only a produced frame");
        };
        renderer.composite(target, display);
        (next, renderer.submission_evidence())
    }

    /// Drops the frame in flight or produced — a detached native surface's
    /// — without waiting on the render thread: the renderer, its producer
    /// and its context stay bound, so the next frame renders on them.
    pub(super) fn discard_frame(&mut self) {
        if let Some(Bound {
            stage: Stage::Live { renderer, frame },
            ..
        }) = self.bound.as_mut()
            && !matches!(frame, Frame::Idle)
        {
            renderer.discard_frame();
            *frame = Frame::Idle;
        }
    }
}

/// The frame cycle a mounted surface drives, without a window: the renderer
/// behind the pipeline is a probe whose creation and frame resolve only when
/// a trial releases them, standing in for the render thread's reply.
#[cfg(all(test, target_os = "macos"))]
mod tests {
    use alloc::boxed::Box;
    use alloc::rc::Rc;
    use alloc::sync::Arc;
    use core::cell::{Cell, RefCell};
    use core::task::{Context, Poll, Waker};
    use std::sync::atomic::{AtomicU32, Ordering};

    use waterui_graphics::backend::cherenkov::Next;
    use waterui_graphics::gpu::{Display, FrameTime, GpuRuntime, SharedGpuContext, wgpu};
    use waterui_graphics::offscreen::OffscreenSize;

    use super::{FramePipeline, FrameRequest, RendererCreation, Step};
    use crate::components::gpu_surface::{FrameOutcome, HostedError, HostedRenderer};

    /// What the probe renderer saw, and the frame the trial resolves.
    #[derive(Default)]
    struct Probe {
        created: Cell<u32>,
        submitted: Cell<u32>,
        composited: Cell<u32>,
        discarded: Cell<u32>,
        dropped: Cell<u32>,
        outcome: RefCell<Option<Result<FrameOutcome, HostedError>>>,
        waker: RefCell<Option<Waker>>,
    }

    impl Probe {
        /// The render thread's reply: the frame resolves and wakes its
        /// poller.
        fn resolve(&self, outcome: FrameOutcome) {
            *self.outcome.borrow_mut() = Some(Ok(outcome));
            if let Some(waker) = self.waker.borrow_mut().take() {
                waker.wake();
            }
        }
    }

    struct ProbeRenderer(Rc<Probe>);

    impl HostedRenderer for ProbeRenderer {
        fn submit(
            &mut self,
            _size: OffscreenSize,
            _display: Display,
            _target_time: FrameTime,
        ) -> Result<(), HostedError> {
            self.0.submitted.set(self.0.submitted.get() + 1);
            Ok(())
        }

        fn poll_frame(&mut self, cx: &mut Context<'_>) -> Poll<Result<FrameOutcome, HostedError>> {
            if let Some(outcome) = self.0.outcome.borrow_mut().take() {
                return Poll::Ready(outcome);
            }
            *self.0.waker.borrow_mut() = Some(cx.waker().clone());
            Poll::Pending
        }

        fn composite(&mut self, _target: &wgpu::Texture, _display: Display) {
            self.0.composited.set(self.0.composited.get() + 1);
        }

        fn discard_frame(&mut self) {
            self.0.discarded.set(self.0.discarded.get() + 1);
            self.0.waker.borrow_mut().take();
        }
    }

    impl Drop for ProbeRenderer {
        fn drop(&mut self) {
            self.0.dropped.set(self.0.dropped.get() + 1);
        }
    }

    /// A creation the trial opens: pending until `open`, then the probe
    /// renderer.
    #[derive(Default)]
    struct Gate {
        open: Cell<bool>,
        waker: RefCell<Option<Waker>>,
    }

    impl Gate {
        fn open(&self) {
            self.open.set(true);
            if let Some(waker) = self.waker.borrow_mut().take() {
                waker.wake();
            }
        }
    }

    fn creation(probe: &Rc<Probe>, gate: Option<Rc<Gate>>) -> RendererCreation {
        let probe = probe.clone();
        Box::pin(async move {
            if let Some(gate) = gate {
                core::future::poll_fn(|cx| {
                    if gate.open.get() {
                        Poll::Ready(())
                    } else {
                        *gate.waker.borrow_mut() = Some(cx.waker().clone());
                        Poll::Pending
                    }
                })
                .await;
            }
            probe.created.set(probe.created.get() + 1);
            Ok(Box::new(ProbeRenderer(probe)) as Box<dyn HostedRenderer>)
        })
    }

    /// The completion waker the host wires: counts its wakes.
    struct Wakes(AtomicU32);

    impl std::task::Wake for Wakes {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn wakes() -> (Arc<Wakes>, Waker) {
        let wakes = Arc::new(Wakes(AtomicU32::new(0)));
        (wakes.clone(), Waker::from(wakes))
    }

    fn context() -> Arc<SharedGpuContext> {
        pollster::block_on(GpuRuntime::new())
            .expect("a GPU adapter is required on test hardware")
            .context()
    }

    fn request() -> FrameRequest {
        FrameRequest {
            size: OffscreenSize::try_from_pixels(16, 16).expect("a nonempty size"),
            display: Display::default(),
            time: FrameTime::now(),
        }
    }

    fn target(context: &SharedGpuContext) -> wgpu::Texture {
        context.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("frame pipeline test target"),
            size: wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
    }

    /// A step submits and returns while the frame is in flight; the
    /// frame's completion wakes the host, which composites it on the next
    /// step — nothing waits on the render thread in between.
    #[test]
    fn a_step_submits_without_waiting_and_composites_once_the_frame_resolves() {
        let context = context();
        let probe = Rc::new(Probe::default());
        let (woken, waker) = wakes();
        let mut cx = Context::from_waker(&waker);
        let mut pipeline = FramePipeline::new();
        pipeline.bind(&context, || creation(&probe, None));
        let mut begun = 0;
        let step = pipeline
            .step(&mut cx, request(), || begun += 1)
            .expect("the frame is submitted");
        assert_eq!(step, Step::Awaiting, "the frame is still being produced");
        assert_eq!(probe.submitted.get(), 1);
        assert_eq!(begun, 1, "the frame's UI side ran ahead of its submission");
        assert!(pipeline.awaiting(), "the host asks the link for no ticks");

        probe.resolve(FrameOutcome::Rendered(Next::Idle));
        assert_eq!(
            woken.0.load(Ordering::SeqCst),
            1,
            "the reply wakes the host"
        );
        assert!(matches!(pipeline.poll(&mut cx), Poll::Ready(Ok(true))));
        assert!(!pipeline.awaiting());
        assert_eq!(
            pipeline
                .step(&mut cx, request(), || begun += 1)
                .expect("the produced frame is ready"),
            Step::Ready
        );
        let (next, evidence) = pipeline.composite(&target(&context), Display::default());
        assert_eq!(next, Next::Idle);
        assert!(evidence.is_none());
        assert_eq!(probe.composited.get(), 1);
        assert_eq!(
            probe.submitted.get(),
            1,
            "a produced frame is not resubmitted"
        );
    }

    /// Creation resolves through the same waker: a step while it runs
    /// writes nothing and submits nothing.
    #[test]
    fn creation_resolves_through_the_completion_waker() {
        let context = context();
        let probe = Rc::new(Probe::default());
        let gate = Rc::new(Gate::default());
        let (woken, waker) = wakes();
        let mut cx = Context::from_waker(&waker);
        let mut pipeline = FramePipeline::new();
        pipeline.bind(&context, || creation(&probe, Some(gate.clone())));
        assert_eq!(
            pipeline
                .step(&mut cx, request(), || {})
                .expect("creation runs"),
            Step::Awaiting
        );
        assert!(pipeline.awaiting());
        assert_eq!(probe.submitted.get(), 0);

        gate.open();
        assert_eq!(woken.0.load(Ordering::SeqCst), 1);
        assert!(matches!(pipeline.poll(&mut cx), Poll::Ready(Ok(true))));
        assert_eq!(probe.created.get(), 1);
        assert_eq!(
            pipeline
                .step(&mut cx, request(), || {})
                .expect("the frame is submitted"),
            Step::Awaiting
        );
        assert_eq!(probe.submitted.get(), 1);
    }

    /// A detached surface drops only its pending frame: the renderer — and
    /// the producer behind it — stays bound, and the reattached surface's
    /// next frame is submitted on it. A wake the dropped frame left behind
    /// asks for nothing.
    #[test]
    fn detaching_drops_only_the_pending_frame() {
        let context = context();
        let probe = Rc::new(Probe::default());
        let (_wakes, waker) = wakes();
        let mut cx = Context::from_waker(&waker);
        let mut pipeline = FramePipeline::new();
        pipeline.bind(&context, || creation(&probe, None));
        pipeline
            .step(&mut cx, request(), || {})
            .expect("the frame is submitted");
        assert!(pipeline.awaiting());

        pipeline.discard_frame();
        assert_eq!(probe.discarded.get(), 1);
        assert_eq!(probe.dropped.get(), 0, "the renderer stays bound");
        assert!(!pipeline.awaiting());
        assert!(matches!(pipeline.poll(&mut cx), Poll::Ready(Ok(false))));

        pipeline.bind(&context, || creation(&probe, None));
        pipeline
            .step(&mut cx, request(), || {})
            .expect("the reattached frame is submitted");
        assert_eq!(probe.created.get(), 1, "the renderer is never recreated");
        assert_eq!(probe.submitted.get(), 2);
    }

    /// A new context generation replaces the renderer whole: its pending
    /// frame drops with it, never waited on, and the replacement is
    /// created on the new context.
    #[test]
    fn a_context_replacement_drops_the_pending_frame_with_its_context() {
        let first = context();
        let second = context();
        let old = Rc::new(Probe::default());
        let new = Rc::new(Probe::default());
        let (_wakes, waker) = wakes();
        let mut cx = Context::from_waker(&waker);
        let mut pipeline = FramePipeline::new();
        pipeline.bind(&first, || creation(&old, None));
        pipeline
            .step(&mut cx, request(), || {})
            .expect("the frame is submitted");

        pipeline.bind(&second, || creation(&new, None));
        assert_eq!(
            old.dropped.get(),
            1,
            "the old renderer drops with its frame"
        );
        assert!(
            pipeline
                .context()
                .is_some_and(|bound| Arc::ptr_eq(bound, &second)),
            "the pipeline is bound to the new generation"
        );
        pipeline
            .step(&mut cx, request(), || {})
            .expect("the new renderer submits");
        assert_eq!(new.created.get(), 1);
        assert_eq!(new.submitted.get(), 1);
    }

    /// A composited frame whose engine asks for nothing leaves the pipeline
    /// idle: no frame in flight, no wake owed, nothing for the link.
    #[test]
    fn an_idle_pipeline_requests_no_frame() {
        let context = context();
        let probe = Rc::new(Probe::default());
        let (woken, waker) = wakes();
        let mut cx = Context::from_waker(&waker);
        let mut pipeline = FramePipeline::new();
        pipeline.bind(&context, || creation(&probe, None));
        pipeline
            .step(&mut cx, request(), || {})
            .expect("the frame is submitted");
        probe.resolve(FrameOutcome::Rendered(Next::Idle));
        assert_eq!(
            pipeline.step(&mut cx, request(), || {}).expect("ready"),
            Step::Ready
        );
        let (next, _) = pipeline.composite(&target(&context), Display::default());
        assert_eq!(next, Next::Idle);
        let settled = woken.0.load(Ordering::SeqCst);
        assert!(!pipeline.awaiting());
        assert!(pipeline.idle());
        assert!(matches!(pipeline.poll(&mut cx), Poll::Ready(Ok(false))));
        assert_eq!(
            woken.0.load(Ordering::SeqCst),
            settled,
            "nothing is owed a wake"
        );
        assert_eq!(probe.submitted.get(), 1);
    }

    /// A frame that came out without this surface's content reports
    /// unproduced once and leaves the renderer free for the owed frame.
    #[test]
    fn an_unproduced_frame_is_owed_again() {
        let context = context();
        let probe = Rc::new(Probe::default());
        let (_wakes, waker) = wakes();
        let mut cx = Context::from_waker(&waker);
        let mut pipeline = FramePipeline::new();
        pipeline.bind(&context, || creation(&probe, None));
        pipeline
            .step(&mut cx, request(), || {})
            .expect("the frame is submitted");
        probe.resolve(FrameOutcome::Unproduced);
        assert_eq!(
            pipeline.step(&mut cx, request(), || {}).expect("settled"),
            Step::Unproduced
        );
        assert!(pipeline.idle());
        pipeline
            .step(&mut cx, request(), || {})
            .expect("the owed frame is submitted");
        assert_eq!(probe.submitted.get(), 2);
    }
}
