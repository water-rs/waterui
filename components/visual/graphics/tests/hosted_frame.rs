//! A hosted renderer's frame is submitted without the host waiting on the
//! engine's render thread: the frame resolves on the poller once the render
//! thread replies, a host that detaches drops only the pending frame, and
//! the frame keeps the context generation it was submitted under.
//!
//! The render worker is paused deliberately: the content's `render` runs on
//! the render thread and waits on a gate the test alone releases.
#![cfg(all(feature = "gpu", not(target_arch = "wasm32")))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Waker};

use cherenkov::{Display, FrameTime, Next};
use waterui_graphics::gpu::interop::GpuContentBox;
use waterui_graphics::gpu::{
    Context, Frame, GpuContent, GpuContentRenderer, GpuContentView, GpuRuntime, HostedFrame,
    RedrawHandle, wgpu,
};
use waterui_graphics::offscreen::OffscreenSize;

const SDR: Display = Display {
    scale: 1.0,
    headroom: 1.0,
};

fn runtime() -> GpuRuntime {
    pollster::block_on(GpuRuntime::new()).expect("a GPU adapter is required on test hardware")
}

const fn size() -> OffscreenSize {
    OffscreenSize::try_from_pixels(16, 16).expect("a nonempty size")
}

/// What the content's instance has seen: its setups and renders. A content
/// recreated by its host would start these over.
#[derive(Default)]
struct Seen {
    setups: AtomicUsize,
    renders: AtomicUsize,
}

/// A paused render: the worker announces it entered the content's render
/// on `entered`, then waits until the test releases `gate`.
struct Gate {
    entered: Sender<()>,
    gate: Receiver<()>,
}

/// Content whose `render` — on the render thread — stops at each gate the
/// test queues, so the render worker is paused for as long as the test
/// holds the gate's sender. A paused render asks for the next frame, so
/// the frame after it renders the content again; an unpaused one asks for
/// nothing.
struct Gated {
    seen: Arc<Seen>,
    gates: Arc<Mutex<Vec<Gate>>>,
}

impl GpuContent for Gated {
    fn setup(&mut self, _gpu: &Context<'_>) {
        self.seen.setups.fetch_add(1, Ordering::SeqCst);
    }

    fn render(&mut self, frame: &mut Frame<'_>) {
        self.seen.renders.fetch_add(1, Ordering::SeqCst);
        let gate = self.gates.lock().unwrap().pop();
        if let Some(Gate { entered, gate }) = gate {
            let _ = entered.send(());
            let _ = gate.recv();
            frame.request_redraw();
        }
    }
}

/// The test's hold on a paused render.
struct Paused {
    entered: Receiver<()>,
    release: Sender<()>,
}

impl Paused {
    /// Waits, bounded, until the worker is inside the content's render.
    fn entered(&self) {
        self.entered
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the render worker reached the content");
    }

    fn release(self) {
        self.release.send(()).expect("the worker waits on the gate");
    }
}

struct Fixture {
    runtime: GpuRuntime,
    renderer: GpuContentRenderer,
    seen: Arc<Seen>,
    gates: Arc<Mutex<Vec<Gate>>>,
    redraws: Arc<AtomicUsize>,
}

impl Fixture {
    fn new() -> Self {
        let runtime = runtime();
        let seen = Arc::new(Seen::default());
        let gates = Arc::new(Mutex::new(Vec::new()));
        let redraws = Arc::new(AtomicUsize::new(0));
        let content = GpuContentBox::new(
            GpuContentView::new(Gated {
                seen: Arc::clone(&seen),
                gates: Arc::clone(&gates),
            })
            .engine_content(),
        );
        let counted = Arc::clone(&redraws);
        let renderer = pollster::block_on(GpuContentRenderer::new(
            &runtime,
            runtime.context(),
            content,
            size(),
            RedrawHandle::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            }),
        ))
        .expect("the hosted renderer is created");
        Self {
            runtime,
            renderer,
            seen,
            gates,
            redraws,
        }
    }

    /// Pauses the render worker inside the next content render.
    fn pause(&self) -> Paused {
        let (release, gate) = channel();
        let (announce, entered) = channel();
        self.gates.lock().unwrap().push(Gate {
            entered: announce,
            gate,
        });
        Paused { entered, release }
    }

    fn submit(&mut self) -> HostedFrame {
        self.renderer
            .submit(size(), SDR, FrameTime::now())
            .expect("the frame is submitted")
    }

    /// A host texture on the renderer's own context generation.
    fn target(&self) -> wgpu::Texture {
        self.runtime
            .context()
            .device()
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("hosted frame test target"),
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
}

/// Polls the frame once; whether it is still pending.
fn pending(frame: &mut HostedFrame) -> bool {
    std::future::Future::poll(
        std::pin::Pin::new(frame),
        &mut TaskContext::from_waker(Waker::noop()),
    )
    .is_pending()
}

/// `submit` returns while the render worker is paused inside the content,
/// the frame stays pending until the worker finishes, and the resolved
/// frame composites into the host's texture.
#[test]
fn submission_returns_while_the_render_worker_is_paused() {
    let mut fixture = Fixture::new();
    let paused = fixture.pause();
    let mut frame = fixture.submit();
    paused.entered();
    assert!(
        pending(&mut frame),
        "the frame stays pending while the worker is paused"
    );
    paused.release();
    let rendered = pollster::block_on(frame).expect("the released frame resolves");
    let target = fixture.target();
    fixture.renderer.composite(&rendered, &target, SDR);
}

/// A host whose native surface detaches drops the pending frame while the
/// worker is still inside it, then reattaches and submits again: the same
/// producer — set up once, its state carried on — renders the next frame,
/// and nothing waits on the worker or presents the dropped frame.
#[test]
fn detaching_mid_frame_keeps_the_producer() {
    let mut fixture = Fixture::new();
    let paused = fixture.pause();
    let detached = fixture.submit();
    paused.entered();
    drop(detached);
    paused.release();

    let rendered = pollster::block_on(fixture.submit()).expect("the reattached frame resolves");
    let target = fixture.target();
    fixture.renderer.composite(&rendered, &target, SDR);
    assert_eq!(
        fixture.seen.setups.load(Ordering::SeqCst),
        1,
        "the producer is never recreated"
    );
    assert_eq!(
        fixture.seen.renders.load(Ordering::SeqCst),
        2,
        "the same producer renders the dropped frame and the next one"
    );
}

/// The device is reported lost while a frame is in flight, and the runtime
/// publishes a rebuilt context before the frame resolves: the completion
/// still answers the generation it was submitted under, which is the
/// renderer's — never the newer publication's.
#[test]
fn a_device_loss_completion_keeps_its_generation() {
    let mut fixture = Fixture::new();
    let submitted = fixture.renderer.generation();
    let paused = fixture.pause();
    let frame = fixture.submit();
    paused.entered();
    fixture
        .runtime
        .context()
        .mark_device_lost_for_testing("test device loss");
    let fresh = pollster::block_on(fixture.runtime.context_after(submitted))
        .expect("a lost context within the rebuild budget recovers");
    assert!(fresh.generation() > submitted);
    assert_eq!(frame.generation(), submitted);
    paused.release();

    let rendered = pollster::block_on(frame).expect("the frame resolves");
    assert_eq!(rendered.generation(), submitted);
    assert_eq!(fixture.renderer.generation(), submitted);
}

/// Content that asks for nothing settles: its frames resolve `Idle`, and a
/// later frame — whose reply proves the render thread finished everything
/// queued before it — finds no frame request delivered to the host since.
#[test]
fn an_idle_host_receives_no_frame_requests() {
    let mut fixture = Fixture::new();
    let first = pollster::block_on(fixture.submit()).expect("the first frame resolves");
    assert_eq!(*first.next(), Next::Idle);
    let settled = fixture.redraws.load(Ordering::SeqCst);
    let second = pollster::block_on(fixture.submit()).expect("the second frame resolves");
    assert_eq!(*second.next(), Next::Idle);
    assert_eq!(
        fixture.redraws.load(Ordering::SeqCst),
        settled,
        "an idle renderer asks its host for no frame"
    );
}
