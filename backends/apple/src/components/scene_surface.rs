//! Retained scene recordings on the native Metal surface host.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, mpsc};

use waterui_core::layout::{ProposalSize, Size, StretchAxis, ViewDimensions};
use waterui_graphics::cherenkov::{DEFAULT_REFRESH, Display, FrameScope, FrameTime, Next, Surface};
use waterui_graphics::cherenkov_gpu::{
    Gpu,
    interop::{OutputAlpha, OutputColor, Presenter, TextureOutput, TextureTarget, shader_delivery},
};
use waterui_graphics::draw::{Content, Draw, kurbo};
use waterui_graphics::gpu::{GpuRuntime, HostedLayerError, RedrawHandle, SharedGpuContext};
use waterui_graphics::input::SurfaceInputEvent;
use waterui_graphics::offscreen::OffscreenSize;
use waterui_graphics::resources::{HeldResources, SceneResources};
use waterui_graphics::scene_view::{
    SceneInvalidator, SceneView, resolve_scene_proposal, scene_stretch_axis,
};
use waterui_graphics::wgpu;

use super::{HostedError, HostedRenderer, HostedView};
use crate::gpu_runtime::{EngineGeneration, SceneEngine, SceneParticipant};

/// Content and its structural invalidation survive ordinary frame submissions.
pub struct Scene {
    view: Rc<RefCell<SceneView>>,
    dirty: Rc<Cell<bool>>,
    /// The batch timestamp this scene's content was last produced into,
    /// shared with its participant like `dirty`. A content that asks for
    /// another frame keeps it — `again` re-dirties the scene to schedule
    /// the next frame without un-producing the one just written; only a
    /// real invalidation clears it, through `mount`'s invalidator.
    produced_for: Rc<Cell<Option<FrameTime>>>,
    engines: Rc<SceneEngine>,
    /// The invalidation callback `mount` installed in the content, kept so an
    /// engine (re)creation can re-install it after `rebuild_for_engine` clears
    /// the content's engine-bound state — watchers included — on the new
    /// generation. `None` while unmounted; a renderer created before `mount`
    /// installs nothing rather than inventing a subscription.
    invalidator: Option<SceneInvalidator>,
    /// The intrinsic size the layout was last notified about: the baseline
    /// `measurement_dependency_invalidated` compares the live
    /// [`SceneContent::intrinsic_size`](waterui_graphics::scene_view::SceneContent::intrinsic_size)
    /// against. It is seeded before the content invalidator is installed, and
    /// it advances only when a measurement invalidation is emitted — ordinary
    /// `measure` calls and placement commits never write it.
    last_published: Cell<Option<Size>>,
}

impl Scene {
    /// Keeps the content instance shared by measurement, input and rendering.
    #[must_use]
    pub fn new(view: SceneView, engines: Rc<SceneEngine>) -> Self {
        Self {
            last_published: Cell::new(view.intrinsic_size()),
            view: Rc::new(RefCell::new(view)),
            dirty: Rc::new(Cell::new(true)),
            produced_for: Rc::new(Cell::new(None)),
            engines,
            invalidator: None,
        }
    }
}

// `invalidator` is a closure: the honest Debug shows the semantic fields and
// stays non-exhaustive.
impl core::fmt::Debug for Scene {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Scene")
            .field("view", &self.view)
            .field("dirty", &self.dirty)
            .field("last_published", &self.last_published)
            .finish_non_exhaustive()
    }
}

impl HostedView for Scene {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let resolved = resolve_scene_proposal(self.view.borrow().intrinsic_size(), proposal);
        ViewDimensions::new(waterui_core::layout::Size::new(
            resolved.width.unwrap_or(0.0),
            resolved.height.unwrap_or(0.0),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        scene_stretch_axis(self.view.borrow().intrinsic_size())
    }

    fn accessibility_label(&self) -> Option<String> {
        self.view.borrow().accessibility_label()
    }

    fn accessibility_value(&self) -> Option<String> {
        self.view.borrow().accessibility_value()
    }

    fn mount(&mut self, redraw: &RedrawHandle) {
        let dirty = self.dirty.clone();
        let produced_for = self.produced_for.clone();
        let redraw = redraw.clone();
        let invalidator: SceneInvalidator = Rc::new(move || {
            dirty.set(true);
            produced_for.set(None);
            redraw.request_redraw();
        });
        self.invalidator = Some(Rc::clone(&invalidator));
        self.view
            .borrow_mut()
            .content_mut()
            .set_invalidator(Some(invalidator));
    }

    fn unmount(&mut self) {
        self.invalidator = None;
        self.view.borrow_mut().content_mut().set_invalidator(None);
    }

    /// The mounted invalidator is semantic state — firing it is the
    /// same invalidation a content signal raises: `dirty`,
    /// `produced_for = None`, and a redraw request on the mount's
    /// handle.
    fn invalidate(&self) {
        if let Some(invalidator) = &self.invalidator {
            invalidator();
        }
    }

    fn wants_input_events(&self) -> bool {
        self.view.borrow_mut().content_mut().wants_input_events()
    }

    fn resolved_hdr_preference(&self) -> Option<bool> {
        None
    }

    fn input(&self, event: &SurfaceInputEvent) {
        self.view.borrow_mut().content_mut().input(event);
    }

    fn ime_caret(&self) -> Option<kurbo::Rect> {
        self.view.borrow_mut().content_mut().ime_caret()
    }

    fn before_frame(&self) {}

    /// The scene's measurement contract is its explicit intrinsic size, not
    /// the answer under a delivered proposal: a placement that pinned every
    /// axis — the `Some(0)` slot an image is handed before its decode — keeps
    /// the under-proposal answer identical on both sides of the intrinsic's
    /// arrival, so a response-delta check there can never see it. Comparing
    /// the live `intrinsic_size` with the last published value catches the
    /// real dependency: `None -> Some` on decode, a later aspect change, and
    /// the matching `stretch_axis` `Both -> None` transition all emit exactly
    /// once, and a parent that legitimately re-places the leaf at the same
    /// slot re-publishes the same intrinsic and stabilises instead of
    /// invalidating forever.
    fn measurement_dependency_invalidated(&self) -> Option<bool> {
        let current = self.view.borrow().intrinsic_size();
        if current == self.last_published.get() {
            return Some(false);
        }
        self.last_published.set(current);
        Some(true)
    }

    fn renderer(
        &mut self,
        runtime: &GpuRuntime,
        context: &Arc<SharedGpuContext>,
        redraw: &RedrawHandle,
        size: OffscreenSize,
        failure_sink: &Rc<dyn Fn(Arc<HostedLayerError>)>,
    ) -> Result<Box<dyn HostedRenderer>, HostedError> {
        let generation = self.engines.generation(runtime, context)?;
        let part = ScenePart::new(&generation, redraw, size, self, failure_sink)?;
        let participant: Rc<dyn SceneParticipant> = part.clone();
        generation.mount(&participant);
        Ok(Box::new(SceneRenderer { generation, part }))
    }
}

/// This scene's own share of the generation: its surface, texture target,
/// held registrations, scene resources and native presenter — everything
/// except the shared engine. The generation reaches it weakly through
/// [`SceneParticipant`]; `prepare` runs for it before each batch.
pub struct ScenePart {
    surface: Surface<Gpu>,
    installed: RefCell<HeldResources>,
    resources: SceneResources,
    textures: mpsc::Receiver<wgpu::Texture>,
    source: RefCell<wgpu::Texture>,
    presenter: RefCell<Presenter>,
    view: Rc<RefCell<SceneView>>,
    dirty: Rc<Cell<bool>>,
    redraw: RedrawHandle,
    /// The refresh range this scene's texture target declared.
    refresh: waterui_graphics::cherenkov::RefreshRange,
    /// The geometry the installed recording was made for: the logical size
    /// handed to `build_scene` and the display scale its root transform
    /// carries. A pixel resize that keeps the logical box but changes the
    /// scale rewrites the recording's transform, so the key is points and
    /// scale together; a headroom-only display update shares the key and
    /// re-records nothing.
    recorded_geometry: Cell<Option<(f32, f32, f64)>>,
    /// The frame contract `present` last staged — pixel extent plus the
    /// display update — applied on this surface inside the next batch.
    staged: Cell<(u32, u32, Display)>,
    /// Staged state `prepare` has not applied yet.
    pending: Cell<bool>,
    /// The batch timestamp this scene last produced into — the `Scene`'s
    /// shared cell, so the mounted invalidator un-produces the frame a
    /// content change invalidates, while an `again` request for the next
    /// frame leaves this frame's production standing.
    produced_for: Rc<Cell<Option<FrameTime>>>,
    /// The owner's routed-failure channel: a batch failure the shared
    /// generation routes here — and this scene's own `prepare` error —
    /// moves the owner to `Failed` through the settle the sink enqueues
    /// on the main queue. One record per owner, no mailbox, no polling.
    sink: Rc<dyn Fn(Arc<HostedLayerError>)>,
}

/// The mounted `SceneView`'s handle into an [`EngineGeneration`]: the
/// shared generation plus this scene's own participant state.
struct SceneRenderer {
    generation: Rc<EngineGeneration>,
    part: Rc<ScenePart>,
}

impl ScenePart {
    pub fn new(
        generation: &Rc<EngineGeneration>,
        redraw: &RedrawHandle,
        size: OffscreenSize,
        scene: &Scene,
        failure_sink: &Rc<dyn Fn(Arc<HostedLayerError>)>,
    ) -> Result<Rc<Self>, HostedLayerError> {
        let context = generation.context().clone();
        let (target, textures) = TextureTarget::new((size.width(), size.height()));
        // `TextureTarget::new` always opens on the engine's default range;
        // a `.rate(...)` builder would replace it at construction.
        let refresh = DEFAULT_REFRESH;
        // This scene's queued work, completions and reveal requests wake
        // only its own host, coalesced until the surface next participates
        // in a frame.
        let wake = redraw.clone();
        let surface = generation
            .engine()
            .surface(target, move || wake.request_redraw())?;
        let source = textures
            .try_recv()
            .expect("a TextureTarget publishes its texture when its surface is created");
        let presenter = Presenter::new(
            context.device(),
            shader_delivery(context.adapter().get_info().backend, context.device())?,
        );
        // `rebuild_for_engine` clears the content's engine-bound state,
        // including the watchers its invalidator feeds — so the mounted
        // callback goes back in before the first record on this generation.
        scene.view.borrow_mut().content_mut().rebuild_for_engine();
        scene
            .view
            .borrow_mut()
            .content_mut()
            .set_invalidator(scene.invalidator.clone());
        scene.dirty.set(true);
        Ok(Rc::new(Self {
            surface,
            installed: RefCell::new(HeldResources::empty()),
            resources: SceneResources::with_shaders(
                generation.engine().clone(),
                generation.engine().clone(),
            ),
            textures,
            source: RefCell::new(source),
            presenter: RefCell::new(presenter),
            view: scene.view.clone(),
            dirty: scene.dirty.clone(),
            redraw: redraw.clone(),
            refresh,
            recorded_geometry: Cell::new(None),
            staged: Cell::new((
                size.width(),
                size.height(),
                Display {
                    scale: 1.0,
                    headroom: 1.0,
                },
            )),
            pending: Cell::new(true),
            produced_for: scene.produced_for.clone(),
            sink: failure_sink.clone(),
        }))
    }

    /// Stages the frame contract `present` was called with; `prepare`
    /// applies it inside the batch.
    pub fn stage(&self, pixels: (u32, u32), display: Display) {
        if self.staged.get() != (pixels.0, pixels.1, display) {
            self.staged.set((pixels.0, pixels.1, display));
            self.pending.set(true);
        }
    }

    /// Applies the staged contract and any pending content onto this
    /// scene's own surface — `SceneParticipant::prepare`'s body — inside
    /// the frame scope it returns for the batch to hold across its render.
    fn apply_staged(&self) -> Result<FrameScope, HostedLayerError> {
        let frame = self.surface.begin_frame();
        let (width, height, display) = self.staged.get();
        let pixels = (width, height);
        if self.surface.size() != pixels {
            self.surface.resize(pixels)?;
        }
        self.surface.display(display)?;
        let size = (
            f64::from(pixels.0) / display.scale,
            f64::from(pixels.1) / display.scale,
        );
        #[expect(
            clippy::cast_possible_truncation,
            reason = "native texture extents and backing scales become f32 layout points"
        )]
        let size = (size.0 as f32, size.1 as f32);
        let dirty = self.dirty.replace(false);
        let geometry = (size.0, size.1, display.scale);
        if dirty || self.recorded_geometry.get() != Some(geometry) {
            let (recording, held, again) = self.record_content(size.0, size.1, display.scale);
            self.surface.update(|tx| {
                tx[self.surface.root()].content(recording);
            });
            // Replacing the installed layer precedes releasing the old
            // registrations.
            *self.installed.borrow_mut() = held;
            self.recorded_geometry.set(Some(geometry));
            if again {
                self.dirty.set(true);
                self.redraw.request_redraw();
            }
        }
        self.pending.set(false);
        Ok(frame)
    }

    /// Records the content's scene at `width` x `height` logical points
    /// inside a scope applying the display scale. `SceneContent` produces
    /// logical-point geometry while the surface and its recorded output
    /// are physical pixels, so the caller owns the conversion — the same
    /// wrap `Offscreen` puts around its `build_scene` calls. `Display::scale`
    /// in the engine is producer quality/advisory only; it never maps
    /// geometry itself.
    ///
    /// Returns the recording, the resources it names, and the content's
    /// request for another frame; the caller installs the recording and
    /// takes over the held resources so the previous registration set
    /// lives until the new layer is installed.
    fn record_content(
        &self,
        width: f32,
        height: f32,
        scale: f64,
    ) -> (Content, HeldResources, bool) {
        let mut resources = self.resources.recording();
        let mut again = false;
        let recording = self.surface.record(|recorder| {
            recorder.transform(kurbo::Affine::scale(scale), |recorder| {
                again = self.view.borrow_mut().content_mut().build_scene(
                    recorder,
                    &mut resources,
                    width,
                    height,
                );
            });
        });
        (recording, resources.finish(), again)
    }
}

impl SceneParticipant for ScenePart {
    fn prepare(&self, time: FrameTime) -> Result<FrameScope, Arc<HostedLayerError>> {
        match self.apply_staged() {
            Ok(frame) => {
                self.produced_for.set(Some(time));
                Ok(frame)
            }
            Err(error) => {
                // This scene's own error routes through the same channel
                // a batch failure does — the sink enqueues the owner's
                // `settle_failed` on the main queue, where a settle that
                // already landed for this generation is a no-op — and the
                // same carrier goes back to the requester through
                // `produce`, so the caller that failed reports `Err` for
                // a frame it never wrote instead of a produced batch.
                // `produced_for` stays unset: the scene was never
                // produced at `time`.
                let failure = Arc::new(error);
                (self.sink)(failure.clone());
                Err(failure)
            }
        }
    }

    fn produced_at(&self, time: FrameTime) -> bool {
        // `dirty` is deliberately not in the predicate: content that
        // asks for another frame (`again`) re-dirties the scene to
        // schedule it while this frame's production stands — only a real
        // invalidation un-produces it, through `produced_for`.
        self.produced_for.get() == Some(time) && !self.pending.get()
    }

    fn note_failure(&self, failure: Arc<HostedLayerError>) {
        // A batch failure routed to this scene goes straight to the
        // surface that owns it: the sink enqueues that owner's
        // `settle_failed` on the main queue — a settle already landed
        // for this generation is a no-op there. Nothing stores it here.
        (self.sink)(failure);
    }
}

impl HostedRenderer for SceneRenderer {
    fn present(
        &mut self,
        target: &wgpu::Texture,
        display: Display,
        target_time: FrameTime,
    ) -> Result<Next, HostedError> {
        self.part.stage((target.width(), target.height()), display);
        // The first requesting surface at this target timestamp runs the
        // batch: every mounted participant's pending content, geometry and
        // display changes, then one shared engine.render.
        // The requester is this scene: `produce` hands back its own
        // `prepare` failure so `present` answers `Err` for a frame this
        // surface never wrote — a second requester of the same batch is
        // unaffected by another participant's rejection.
        let requester: Rc<dyn SceneParticipant> = self.part.clone();
        self.generation.produce(target_time, &requester)?;
        if !self.part.produced_at(target_time) {
            // Mounted or invalidated after the batch — explicitly owed a
            // later frame, never compositing a stale texture into this
            // timestamp and never reporting Idle.
            return Ok(Next::At {
                time: target_time.0,
                rate: self.part.refresh.clone(),
            });
        }
        for texture in self.part.textures.try_iter() {
            *self.part.source.borrow_mut() = texture;
        }
        let source = self
            .part
            .source
            .borrow()
            .create_view(&wgpu::TextureViewDescriptor::default());
        let context = self.generation.context();
        self.part.presenter.borrow_mut().texture(
            context.device(),
            context.queue(),
            &source,
            TextureOutput {
                texture: target,
                color: if target.format() == wgpu::TextureFormat::Rgba16Float {
                    OutputColor::LinearDisplayP3
                } else {
                    OutputColor::Srgb
                },
                alpha: OutputAlpha::Premultiplied,
                headroom: display.headroom,
            },
        );
        // This surface's own deadline — animation/backend demand that woke
        // it stays routed to it, not to every mounted scene.
        Ok(self.part.surface.next_frame())
    }

    /// The scene's submission evidence is its production generation
    /// itself — retained by the in-flight submission so a late
    /// completion checks that generation's immutable sealed outcome,
    /// never the owner's mutable flag.
    fn submission_evidence(&self) -> Option<Rc<EngineGeneration>> {
        Some(Rc::clone(&self.generation))
    }

    /// `present` composites only when this scene's own staged state made
    /// the produced batch — `produced_at` is the exact predicate its
    /// early `Next::At` return checked, so it is the exact answer to
    /// whether the target carries pixels.
    fn wrote_target(&self, target_time: FrameTime) -> bool {
        self.part.produced_at(target_time)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    use waterui::{Binding, SignalExt, binding};
    use waterui_core::{AnyView, Environment, View};
    use waterui_graphics::draw::{
        Command, Draw, FontId, ImageId, ImageLimits, Paint, Recorder, WorkingColor,
    };
    use waterui_graphics::resources::{Handle, RecordingResources, SceneBackend};
    use waterui_graphics::scene_view::{SceneContent, SceneViewMergeToParent};
    use waterui_graphics::source::{FontSource, ImageData, ResourceError, Rgba8, Rgba16F};
    use waterui_graphics::{Picture, PictureRecording};

    /// Content that counts its recordings and fills a half-extent logical
    /// rect: `build_scene` receives the logical box, so the recorded shape
    /// is in points whatever the surface's device scale is.
    struct CountingContent {
        draws: Rc<Cell<u32>>,
    }

    impl SceneContent for CountingContent {
        fn build_scene(
            &mut self,
            recorder: &mut Recorder,
            _resources: &mut RecordingResources<'_>,
            width: f32,
            height: f32,
        ) -> bool {
            self.draws.set(self.draws.get() + 1);
            recorder.fill(
                kurbo::Rect::new(0.0, 0.0, f64::from(width) / 2.0, f64::from(height) / 2.0),
                WorkingColor::WHITE,
            );
            false
        }

        fn rebuild_for_engine(&mut self) {}
    }

    /// Content that asks for another frame on every record — the
    /// split-component self-animation contract: `again` schedules the
    /// next frame and must not un-produce the one just written.
    struct AnimatingContent;

    impl SceneContent for AnimatingContent {
        fn build_scene(
            &mut self,
            recorder: &mut Recorder,
            _resources: &mut RecordingResources<'_>,
            width: f32,
            height: f32,
        ) -> bool {
            recorder.fill(
                kurbo::Rect::new(0.0, 0.0, f64::from(width) / 2.0, f64::from(height) / 2.0),
                WorkingColor::WHITE,
            );
            true
        }

        fn rebuild_for_engine(&mut self) {}
    }

    /// A routed-failure sink for `ScenePart` construction — these trials
    /// assert preparation and wake behaviour through `redraw`, not the
    /// routed channel, so the sink records nothing.
    fn sink() -> Rc<dyn Fn(Arc<HostedLayerError>)> {
        Rc::new(|_| {})
    }

    /// A `SceneBackend` no trial ever reaches: `SceneResources` built over
    /// it pins no engine, so the participant that swaps it in lets the
    /// generation it came from die on schedule. A scene that got as far
    /// as registering through it has already failed its `display`.
    struct NeverBackend;

    impl SceneBackend for NeverBackend {
        fn register_font(&self, _source: FontSource) -> Result<Handle<FontId>, ResourceError> {
            unreachable!("a dead surface fails before registering a font")
        }
        fn register_rgba8(
            &self,
            _data: ImageData<Rgba8>,
        ) -> Result<Handle<ImageId>, ResourceError> {
            unreachable!("a dead surface fails before registering an image")
        }
        fn register_rgba16f(
            &self,
            _data: ImageData<Rgba16F>,
        ) -> Result<Handle<ImageId>, ResourceError> {
            unreachable!("a dead surface fails before registering an image")
        }
        fn image_limits(&self) -> ImageLimits {
            ImageLimits::UNLIMITED
        }
    }

    /// A scene whose content returns `true` from `build_scene` is
    /// produced at the batch timestamp anyway: `again` only re-dirties
    /// the scene to schedule the next frame — it is not an invalidation —
    /// so `produced_at` stays true for the frame just written and the
    /// redraw request still fires.
    #[test]
    fn an_animating_scene_stays_produced_and_requests_the_next_frame() {
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("a GPU adapter is required on test hardware");
        let context = runtime.context();
        let engines = Rc::new(SceneEngine::new());
        let generation = engines
            .generation(&runtime, &context)
            .expect("scene generation settles");
        let redraws = Arc::new(AtomicU32::new(0));
        let probe = Arc::clone(&redraws);
        let redraw = RedrawHandle::new(move || {
            probe.fetch_add(1, Ordering::Relaxed);
        });
        let scene = Scene::new(SceneView::new(AnimatingContent), engines);
        let size = OffscreenSize::try_from_pixels(20, 20).expect("nonzero size");
        let part = ScenePart::new(&generation, &redraw, size, &scene, &sink())
            .expect("the participant settles");
        let time = FrameTime(std::time::Instant::now());
        part.stage(
            (20, 20),
            Display {
                scale: 1.0,
                headroom: 1.0,
            },
        );
        drop(part.prepare(time).expect("the staged contract applies"));
        assert!(
            part.produced_at(time),
            "`again` schedules the next frame; it does not un-produce this one"
        );
        assert!(
            redraws.load(Ordering::Relaxed) > 0,
            "the animating scene still requests its next frame"
        );
        assert!(
            scene.dirty.get(),
            "the animating scene stays dirty for the next batch"
        );
    }

    /// A real invalidation after `prepare(t)` un-produces the frame: the
    /// mounted invalidator clears `produced_for` while the `again` path
    /// never touches it, so `produced_at(t)` flips false — the frame
    /// must re-render before it composites again.
    #[test]
    fn an_invalidation_after_prepare_unproduces_the_frame() {
        const RED: WorkingColor = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);
        const BLUE: WorkingColor = WorkingColor::new([0.0, 0.0, 1.0, 1.0]);
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("a GPU adapter is required on test hardware");
        let context = runtime.context();
        let engines = Rc::new(SceneEngine::new());
        let generation = engines
            .generation(&runtime, &context)
            .expect("scene generation settles");
        let redraw = RedrawHandle::new(|| {});
        let color = binding(RED);
        let mut scene = picture_scene(&color, &engines);
        scene.mount(&redraw);
        let size = OffscreenSize::try_from_pixels(20, 20).expect("nonzero size");
        let part = ScenePart::new(&generation, &redraw, size, &scene, &sink())
            .expect("the participant settles");
        let time = FrameTime(std::time::Instant::now());
        part.stage(
            (20, 20),
            Display {
                scale: 1.0,
                headroom: 1.0,
            },
        );
        drop(part.prepare(time).expect("the staged contract applies"));
        assert!(part.produced_at(time));
        color.set(BLUE);
        assert!(
            !part.produced_at(time),
            "a real invalidation un-produces the written frame"
        );
        assert!(scene.dirty.get(), "the invalidation also re-dirties");
    }

    /// Mounts the shared generation from the environment's engine owner,
    /// the scene's own `ScenePart` on it — a mounted participant built the
    /// way `Scene::renderer` builds it. Answers the context, the
    /// participant and the content's draw counter.
    fn part() -> (Arc<SharedGpuContext>, Rc<ScenePart>, Rc<Cell<u32>>) {
        let draws = Rc::new(Cell::new(0));
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("a GPU adapter is required on test hardware");
        let context = runtime.context();
        let engines = Rc::new(SceneEngine::new());
        let redraw = RedrawHandle::new(|| {});
        let scene = Scene::new(
            SceneView::new(CountingContent {
                draws: draws.clone(),
            }),
            engines.clone(),
        );
        let size = OffscreenSize::try_from_pixels(512, 512).expect("nonzero size");
        let generation = engines
            .generation(&runtime, &context)
            .expect("scene generation settles");
        let part = ScenePart::new(&generation, &redraw, size, &scene, &sink())
            .expect("the participant settles");
        let participant: Rc<dyn SceneParticipant> = part.clone();
        generation.mount(&participant);
        (context, part, draws)
    }

    fn target(context: &SharedGpuContext, pixels: u32) -> wgpu::Texture {
        context.device().create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: pixels,
                height: pixels,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    }

    /// Content whose intrinsic size resolves asynchronously — the same shape
    /// as an image that only learns its pixels after decode.
    struct AsyncIntrinsic {
        size: Rc<Cell<Option<Size>>>,
    }

    impl SceneContent for AsyncIntrinsic {
        fn build_scene(
            &mut self,
            _recorder: &mut Recorder,
            _resources: &mut RecordingResources<'_>,
            _width: f32,
            _height: f32,
        ) -> bool {
            false
        }

        fn intrinsic_size(&self) -> Option<Size> {
            self.size.get()
        }

        fn rebuild_for_engine(&mut self) {}
    }

    /// The measurement dependency is the intrinsic itself: a leaf handed a
    /// `Some(0)` slot before its intrinsic arrives still invalidates when it
    /// lands, probe-only measures never consume the pending change, the
    /// emission fires once, and a parent that keeps the leaf on a dead axis
    /// re-publishes the same intrinsic and stabilises.
    #[test]
    fn intrinsic_arrival_invalidates_once_regardless_of_pinned_placement() {
        let size = Rc::new(Cell::new(None));
        let scene = Scene::new(
            SceneView::new(AsyncIntrinsic { size: size.clone() }),
            Rc::new(SceneEngine::new()),
        );
        // The initial zero-height placement committed: the baseline must not
        // fire for the unchanged intrinsic.
        assert!(
            scene.measurement_dependency_invalidated() == Some(false),
            "unchanged intrinsic stays quiet even on a pinned Some(0) slot"
        );

        // The intrinsic resolves, then probe-only measures run before the
        // invalidation check does: a baseline a measure could overwrite
        // would lose the pending change here.
        size.set(Some(Size::new(2249.0, 1500.0)));
        assert_eq!(
            scene.measure(ProposalSize::new(304.0, 0.0)).size,
            Size::new(304.0, 0.0),
            "the pinned slot still answers zero after the decode"
        );
        assert_ne!(
            scene.measure(ProposalSize::new(304.0, None)).size,
            Size::new(304.0, 0.0),
            "an open proposal now sees the new intrinsic"
        );
        assert_eq!(
            scene.measurement_dependency_invalidated(),
            Some(true),
            "the intervening measures must not consume the change"
        );
        let _ = scene.measure(ProposalSize::new(304.0, None));
        assert_eq!(
            scene.measurement_dependency_invalidated(),
            Some(false),
            "the emitted intrinsic is the new baseline: re-checks stay quiet"
        );

        // A legitimately fixed slot that differs from the intrinsic still
        // does not thrash: the dependency did not change again.
        assert_eq!(scene.measurement_dependency_invalidated(), Some(false));
    }

    /// The recording wraps the logical-point content in exactly one
    /// display-scale transform — the conversion `Offscreen` applies —
    /// and it is applied at every scale, exactly once. One participant
    /// suffices: each `record_content` is an independent recording.
    #[test]
    fn scene_recording_wraps_logical_content_in_the_display_scale() {
        let (_context, part, _) = part();
        for scale in [1.0f64, 2.0, 3.0] {
            let (mut content, _held, _again) = part.record_content(512.0, 512.0, scale);
            let commands = content.snapshot().commands();
            let transforms = commands
                .iter()
                .filter(|command| matches!(command, Command::BeginTransform { .. }))
                .count();
            assert_eq!(transforms, 1, "one transform scope, applied once");
            let Command::BeginTransform { transform, .. } = &commands[0] else {
                panic!("the scale scope wraps the recorded content");
            };
            assert_eq!(*transform, kurbo::Affine::scale(scale));
            let fill = commands
                .iter()
                .find_map(|command| match command {
                    Command::Fill { shape, .. } => Some(shape.bounds()),
                    _ => None,
                })
                .expect("the rect fill is recorded");
            assert_eq!(
                fill,
                kurbo::Rect::new(0.0, 0.0, 256.0, 256.0),
                "content records in logical points inside the scaled scope"
            );
        }
    }

    /// The geometry key is logical size and display scale together: a
    /// headroom-only `display` update re-records nothing, while the same
    /// logical box at a new pixel extent and scale must re-record — the
    /// recording's root transform changed. Driven through
    /// `SceneParticipant::prepare` — the call `produce` issues on every
    /// mounted participant inside a batch.
    #[test]
    fn geometry_key_tracks_scale_and_skips_headroom_only_updates() {
        let (_context, part, draws) = part();
        let record = |part: &Rc<ScenePart>, pixels: u32, scale: f64, headroom: f32| {
            part.stage((pixels, pixels), Display { scale, headroom });
            drop(
                part.prepare(FrameTime(std::time::Instant::now()))
                    .expect("the staged contract applies"),
            );
        };

        record(&part, 512, 1.0, 1.0);
        assert_eq!(draws.get(), 1);
        record(&part, 512, 1.0, 4.0);
        assert_eq!(draws.get(), 1, "a headroom-only update re-records nothing");
        record(&part, 1024, 2.0, 1.0);
        assert_eq!(
            draws.get(),
            2,
            "same logical box at a new scale rewrites the transform"
        );
        record(&part, 1024, 2.0, 1.0);
        assert_eq!(draws.get(), 2, "an unchanged geometry stays cached");
        record(&part, 256, 2.0, 1.0);
        assert_eq!(draws.get(), 3, "a new logical size re-records");
    }

    fn square(color: WorkingColor) -> PictureRecording {
        Picture::record(|scene| {
            scene.fill(kurbo::Rect::new(0.0, 0.0, 10.0, 10.0), color);
        })
    }

    /// A `Scene` over a real reactive `Picture`: its content watches the
    /// recording signal through whichever invalidator the host installs.
    fn picture_scene(recording: &Binding<WorkingColor>, engines: &Rc<SceneEngine>) -> Scene {
        let picture = Picture::new(Size::new(10.0, 10.0), recording.map(square));
        let scene_view =
            AnyView::new(picture.body(&Environment::new().extending(SceneViewMergeToParent)))
                .downcast::<SceneView>()
                .unwrap_or_else(|_| panic!("a merged picture is a SceneView"));
        Scene::new(*scene_view, engines.clone())
    }

    /// The colour the content's last recording draws, read out of the
    /// picture's own display list — the observable output a reactive
    /// `Picture` changes when its recording signal lands.
    fn recorded_color(part: &ScenePart) -> WorkingColor {
        let (mut content, _held, _again) = part.record_content(10.0, 10.0, 1.0);
        let picture_command = content
            .snapshot()
            .commands()
            .iter()
            .find_map(|command| match command {
                Command::Picture { picture, .. } => Some(picture),
                _ => None,
            })
            .expect("the scene draws the picture, not inline content");
        picture_command
            .display_list()
            .commands()
            .iter()
            .find_map(|command| match command {
                Command::Fill {
                    paint: Paint::Solid(color),
                    ..
                } => Some(*color),
                _ => None,
            })
            .expect("the picture's square fill is recorded")
    }

    /// `mount`'s invalidator is semantic state; `rebuild_for_engine` clears
    /// the watchers it feeds along with every other engine-bound value, so
    /// every renderer creation must put it back — on the first creation and
    /// on every rebuild — or fine-grained invalidation dies with the old
    /// engine-bound state. `ScenePart::new` runs that re-install
    /// unconditionally, so a second participant creation on the same
    /// generation exercises the same code a context replacement would.
    /// An unmounted scene gains no subscription.
    #[test]
    fn renderer_creation_preserves_the_mounted_invalidator() {
        const RED: WorkingColor = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);
        const BLUE: WorkingColor = WorkingColor::new([0.0, 0.0, 1.0, 1.0]);
        const GREEN: WorkingColor = WorkingColor::new([0.0, 1.0, 0.0, 1.0]);

        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("a GPU adapter is required on test hardware");
        let context = runtime.context();
        let engines = Rc::new(SceneEngine::new());
        let generation = engines
            .generation(&runtime, &context)
            .expect("scene generation settles");
        let redraws = Arc::new(AtomicU32::new(0));
        let redraws_probe = Arc::clone(&redraws);
        let redraw = RedrawHandle::new(move || {
            redraws_probe.fetch_add(1, Ordering::Relaxed);
        });
        let color = binding(RED);
        let mut scene = picture_scene(&color, &engines);
        let size = OffscreenSize::try_from_pixels(20, 20).expect("nonzero size");
        let display = Display {
            scale: 1.0,
            headroom: 1.0,
        };
        let record = |part: &Rc<ScenePart>| {
            part.stage((20, 20), display);
            drop(
                part.prepare(FrameTime(std::time::Instant::now()))
                    .expect("the staged contract applies"),
            );
        };

        // An unmounted scene installs nothing: the participant's rebuild
        // leaves the content without a watcher rather than inventing one,
        // so a model update on it never dirties or wakes.
        let quiet_color = binding(RED);
        let scene_unmounted = picture_scene(&quiet_color, &engines);
        let unmounted_part = ScenePart::new(&generation, &redraw, size, &scene_unmounted, &sink())
            .expect("the participant settles");
        record(&unmounted_part);
        let wakes = redraws.load(Ordering::Relaxed);
        quiet_color.set(GREEN);
        assert!(
            !scene_unmounted.dirty.get(),
            "no subscription is invented for an unmounted scene"
        );
        assert_eq!(
            redraws.load(Ordering::Relaxed),
            wakes,
            "the unmounted scene's update wakes nobody"
        );

        scene.mount(&redraw);
        let part = ScenePart::new(&generation, &redraw, size, &scene, &sink())
            .expect("the participant settles");

        // The first record clears the seeded dirty bit. A fresh install on an
        // idle engine may itself ask for a frame through the surface's wake;
        // the invalidator's contribution is the delta each `set` adds.
        record(&part);
        assert!(!scene.dirty.get(), "an ordinary record clears dirty");
        assert_eq!(recorded_color(&part), RED);

        // The invalidator the rebuild re-installed is live: a fine-grained
        // signal update dirties the scene and requests a frame, and the next
        // record draws the new recording.
        let wakes = redraws.load(Ordering::Relaxed);
        color.set(BLUE);
        assert!(
            scene.dirty.get(),
            "the reinstalled watcher marks the scene dirty"
        );
        assert_eq!(
            redraws.load(Ordering::Relaxed),
            wakes + 1,
            "the reinstalled watcher requests a redraw"
        );
        record(&part);
        assert_eq!(recorded_color(&part), BLUE);

        // Creating the scene's participant again — the same `ScenePart::new`
        // a rebuilt engine generation runs — re-installs the mounted
        // invalidator on the rebuilt engine-bound state.
        let part = ScenePart::new(&generation, &redraw, size, &scene, &sink())
            .expect("the replacement participant settles");
        assert_eq!(recorded_color(&part), BLUE);
        record(&part);
        assert!(!scene.dirty.get());
        let wakes = redraws.load(Ordering::Relaxed);
        color.set(GREEN);
        assert!(
            scene.dirty.get(),
            "invalidation survives the engine rebuild"
        );
        assert_eq!(redraws.load(Ordering::Relaxed), wakes + 1);
        record(&part);
        assert_eq!(recorded_color(&part), GREEN);

        // Unmount cancels the watcher: a later model update neither dirties
        // nor wakes.
        scene.unmount();
        let wakes = redraws.load(Ordering::Relaxed);
        color.set(RED);
        assert!(!scene.dirty.get());
        assert_eq!(redraws.load(Ordering::Relaxed), wakes);
    }

    /// A routed-failure channel that counts its deliveries — when
    /// `typed`, each delivery also asserts the carried error is the
    /// `HostedLayerError::Surface` a resize rejection produces.
    fn counting_sink(routes: &Rc<Cell<u32>>, typed: bool) -> Rc<dyn Fn(Arc<HostedLayerError>)> {
        let routes = routes.clone();
        Rc::new(move |failure| {
            if typed {
                assert!(
                    matches!(&*failure, HostedLayerError::Surface(_)),
                    "a resize rejection routes as the typed surface error"
                );
            }
            routes.set(routes.get() + 1);
        })
    }

    /// One routed-failure owner under test: its redraw-wake probe, the
    /// sink's delivery counter, its mounted participant and the renderer
    /// `Scene::renderer` hands the surface.
    struct RoutedOwner {
        wakes: Arc<AtomicU32>,
        routes: Rc<Cell<u32>>,
        part: Rc<ScenePart>,
        renderer: SceneRenderer,
    }

    fn routed_owner(
        generation: &Rc<EngineGeneration>,
        engines: &Rc<SceneEngine>,
        size: OffscreenSize,
        typed: bool,
    ) -> RoutedOwner {
        let wakes = Arc::new(AtomicU32::new(0));
        let probe = Arc::clone(&wakes);
        let redraw = RedrawHandle::new(move || {
            probe.fetch_add(1, Ordering::Relaxed);
        });
        let scene = Scene::new(
            SceneView::new(CountingContent {
                draws: Rc::new(Cell::new(0)),
            }),
            engines.clone(),
        );
        let routes = Rc::new(Cell::new(0_u32));
        let sink = counting_sink(&routes, typed);
        let part = ScenePart::new(generation, &redraw, size, &scene, &sink)
            .expect("the participant settles");
        // The production mount `Scene::renderer` runs — the generation
        // holds each participant weakly for its batch `prepare`.
        let participant: Rc<dyn SceneParticipant> = part.clone();
        generation.mount(&participant);
        let renderer = SceneRenderer {
            generation: generation.clone(),
            part: part.clone(),
        };
        RoutedOwner {
            wakes,
            routes,
            part,
            renderer,
        }
    }

    /// A participant's own `prepare` failure routes to its owner through
    /// the sink exactly once per preparation — the channel a
    /// batch-routed `note_failure` also uses. A staged target extent no
    /// surface can take is the real failure: `resize` answers
    /// `SurfaceError::TooLarge`, carried as `HostedLayerError::Surface`.
    /// Each failing prepare delivers the failure once; the batch itself
    /// still produces.
    #[test]
    fn a_failed_prepare_routes_the_typed_failure_to_its_owner_once() {
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("a GPU adapter is required on test hardware");
        let context = runtime.context();
        let engines = Rc::new(SceneEngine::new());
        let size = OffscreenSize::try_from_pixels(20, 20).expect("nonzero size");
        let generation = engines
            .generation(&runtime, &context)
            .expect("the shared generation");

        let mut owner_a = routed_owner(&generation, &engines, size, true);
        let mut owner_b = routed_owner(&generation, &engines, size, false);
        let base_a = owner_a.wakes.load(Ordering::Relaxed);
        let base_b = owner_b.wakes.load(Ordering::Relaxed);

        // Stage a target no surface can take, then drive the real
        // `prepare` `produce` issues inside the batch: the resize error
        // reaches each owner through its sink exactly once.
        let display = Display {
            scale: 1.0,
            headroom: 1.0,
        };
        owner_a.part.stage((u32::MAX, u32::MAX), display);
        owner_b.part.stage((u32::MAX, u32::MAX), display);
        let time = FrameTime(std::time::Instant::now());
        assert!(
            owner_a.part.prepare(time).is_err(),
            "the rejected extent fails A's prepare"
        );
        assert!(
            owner_b.part.prepare(time).is_err(),
            "the rejected extent fails B's prepare"
        );

        assert_eq!(owner_a.routes.get(), 1, "the failure reaches A once");
        assert_eq!(owner_b.routes.get(), 1, "the failure reaches B once");
        assert_eq!(
            owner_a.wakes.load(Ordering::Relaxed),
            base_a,
            "the failure routes through the sink, never a redraw wake"
        );
        assert_eq!(owner_b.wakes.load(Ordering::Relaxed), base_b);

        // Owner A's `present` restages a takeable target and produces —
        // its earlier failure already settled through the sink. The batch
        // re-prepares every live participant, so B's still-bad stage
        // routes its failure a second time.
        owner_a
            .renderer
            .present(
                &target(&context, 20),
                display,
                FrameTime(std::time::Instant::now()),
            )
            .expect("owner A's next present produces");
        assert_eq!(owner_a.routes.get(), 1, "no failure routes again for A");
        assert_eq!(
            owner_b.routes.get(),
            2,
            "B's still-rejected stage routes once more inside A's batch"
        );

        // B restages and produces the same way.
        owner_b
            .renderer
            .present(
                &target(&context, 20),
                display,
                FrameTime(std::time::Instant::now()),
            )
            .expect("owner B's next present produces");
        assert_eq!(owner_b.routes.get(), 2, "B's repaired stage routes nothing");

        // A scene's own failure never seals the shared generation.
        let requester: Rc<dyn SceneParticipant> = owner_a.part.clone();
        assert!(
            generation
                .produce(FrameTime(std::time::Instant::now()), &requester)
                .is_ok(),
            "a scene's own prepare failure never seals the shared generation"
        );
    }

    /// The requester's own `prepare` failure answers `present` with `Err`
    /// synchronously — `produce` hands the caller its own outcome, so the
    /// capture path's completion contract never receives `Ok` for a frame
    /// the requester never wrote. The same carrier still routes once to
    /// the owner's sink — the `settle_failed` the routed copy enqueues
    /// dedupes on the generation — and the shared batch itself produces
    /// for the other participant.
    ///
    /// The failure is the real `apply_staged` trigger: the participant's
    /// surface belongs to a dropped engine generation, so its render
    /// thread is gone and `surface.display` answers `SurfaceError::Lost`.
    /// The cross-generation mount — the dead participant registered on
    /// the live generation — is a topology built for the test: the
    /// generation holds every participant weakly, wherever its surface
    /// came from, so registering it exercises the production path
    /// without staging the mount itself.
    #[test]
    fn a_present_reports_its_own_prepare_failure() {
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("a GPU adapter is required on test hardware");
        let context = runtime.context();
        let engines = Rc::new(SceneEngine::new());
        let size = OffscreenSize::try_from_pixels(20, 20).expect("nonzero size");
        let generation = engines
            .generation(&runtime, &context)
            .expect("the shared generation");
        let healthy = routed_owner(&generation, &engines, size, false);
        let display = Display {
            scale: 1.0,
            headroom: 1.0,
        };

        // Build the requester on a second engine, then drop the whole
        // generation: its render thread ends, and the participant's
        // surface channel to it closes — the real `SurfaceError::Lost`
        // `apply_staged` can answer.
        let dead_routes = Rc::new(Cell::new(0_u32));
        let dead_scene = Scene::new(
            SceneView::new(CountingContent {
                draws: Rc::new(Cell::new(0)),
            }),
            engines,
        );
        let dead_redraw = RedrawHandle::new(|| {});
        let dead_part = {
            let dead_generation = Rc::new(SceneEngine::new())
                .generation(&runtime, &context)
                .expect("the doomed generation settles");
            let mut part = ScenePart::new(
                &dead_generation,
                &dead_redraw,
                size,
                &dead_scene,
                &counting_sink(&dead_routes, true),
            )
            .expect("the participant settles");
            // `ScenePart::new` pins the engine twice through
            // `SceneResources::with_shaders` — by design, so a mounted
            // scene's engine outlives its generation handle. This trial
            // owns the engine's end instead: a registration table over a
            // backend that pins nothing lets the generation's drop end
            // the render thread, and the surface's `display` then answers
            // the production `SurfaceError::Lost`.
            Rc::get_mut(&mut part)
                .expect("the fresh participant is exclusively owned")
                .resources = SceneResources::new(Rc::new(NeverBackend));
            drop(dead_generation);
            assert!(
                matches!(
                    part.surface.display(display),
                    Err(waterui_graphics::cherenkov::SurfaceError::Lost)
                ),
                "the dropped engine's surface channel is closed"
            );
            part
        };
        let requester: Rc<dyn SceneParticipant> = dead_part.clone();
        generation.mount(&requester);
        let mut renderer = SceneRenderer {
            generation,
            part: dead_part,
        };

        let texture = target(&context, 20);
        let time = FrameTime(std::time::Instant::now());
        let outcome = renderer.present(&texture, display, time);
        assert!(
            matches!(outcome, Err(HostedError::Scene(_))),
            "present reports the requester's own prepare failure as Err"
        );
        assert_eq!(
            dead_routes.get(),
            1,
            "the routed copy still reaches the owner's sink once"
        );
        assert!(
            healthy.part.produced_at(time),
            "the batch still produced the healthy participant"
        );
    }
}
