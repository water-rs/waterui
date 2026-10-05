//! Retained scene recordings on the native Metal surface host.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, mpsc};

use waterui_core::layout::{ProposalSize, Size, StretchAxis, ViewDimensions};
use waterui_graphics::cherenkov::{DEFAULT_REFRESH, Display, FrameTime, Next, Surface};
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
    pub fn new(view: SceneView) -> Self {
        Self {
            last_published: Cell::new(view.intrinsic_size()),
            view: Rc::new(RefCell::new(view)),
            dirty: Rc::new(Cell::new(true)),
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
        let redraw = redraw.clone();
        let invalidator: SceneInvalidator = Rc::new(move || {
            dirty.set(true);
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
        engines: &Rc<SceneEngine>,
        redraw: &RedrawHandle,
        size: OffscreenSize,
    ) -> Result<Box<dyn HostedRenderer>, HostedError> {
        let generation = engines.generation(runtime, context)?;
        let part = ScenePart::new(&generation, redraw, size, self)?;
        let participant: Rc<dyn SceneParticipant> = part.clone();
        generation.mount(&participant);
        Ok(Box::new(SceneRenderer { generation, part }))
    }
}

/// This scene's own share of the generation: its surface, texture target,
/// held registrations, scene resources and native presenter — everything
/// except the shared engine. The generation reaches it weakly through
/// [`SceneParticipant`]; `prepare` runs for it before each batch.
struct ScenePart {
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
    /// The production timestamp this scene last prepared into.
    prepared: Cell<Option<FrameTime>>,
    /// The failure this scene must report — its own prepare error, or a
    /// batch failure the generation routed to it.
    failure: RefCell<Option<Rc<HostedLayerError>>>,
}

/// The mounted `SceneView`'s handle into an [`EngineGeneration`]: the
/// shared generation plus this scene's own participant state.
struct SceneRenderer {
    generation: Rc<EngineGeneration>,
    part: Rc<ScenePart>,
}

impl ScenePart {
    fn new(
        generation: &Rc<EngineGeneration>,
        redraw: &RedrawHandle,
        size: OffscreenSize,
        scene: &Scene,
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
            prepared: Cell::new(None),
            failure: RefCell::new(None),
        }))
    }

    /// Stages the frame contract `present` was called with; `prepare`
    /// applies it inside the batch.
    fn stage(&self, pixels: (u32, u32), display: Display) {
        if self.staged.get() != (pixels.0, pixels.1, display) {
            self.staged.set((pixels.0, pixels.1, display));
            self.pending.set(true);
        }
    }

    /// Applies the staged contract and any pending content onto this
    /// scene's own surface — `SceneParticipant::prepare`'s body, kept
    /// separate so tests can drive it without a batch.
    fn apply_staged(&self) -> Result<(), HostedLayerError> {
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
        Ok(())
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
    fn prepare(&self, time: FrameTime) {
        match self.apply_staged() {
            Ok(()) => self.prepared.set(Some(time)),
            Err(error) => {
                // A failure already routed stays until its owner settles
                // it — a later shared preparation never overwrites it.
                self.failure
                    .borrow_mut()
                    .get_or_insert_with(|| Rc::new(error));
                // Same contract as `note_failure`: this scene's host has
                // to come back to settle the typed failure.
                self.redraw.request_redraw();
            }
        }
    }

    fn produced_at(&self, time: FrameTime) -> bool {
        self.prepared.get() == Some(time) && !self.pending.get() && !self.dirty.get()
    }

    fn note_failure(&self, failure: Rc<HostedLayerError>) {
        // An unsettled earlier failure is never overwritten — the owner
        // settles the first typed failure it was woken for.
        self.failure.borrow_mut().get_or_insert(failure);
        // Wake this scene's own host once through the owned redraw
        // mechanism: an idle or hidden participant's readiness owner
        // still has to come back to consume `take_failure` and settle —
        // the failed generation is never re-produced for it.
        self.redraw.request_redraw();
    }

    fn take_failure(&self) -> Option<Rc<HostedLayerError>> {
        self.failure.borrow_mut().take()
    }
}

impl SceneRenderer {
    #[cfg(all(test, target_os = "macos"))]
    fn new(
        runtime: &GpuRuntime,
        context: &Arc<SharedGpuContext>,
        engines: &Rc<SceneEngine>,
        redraw: &RedrawHandle,
        size: OffscreenSize,
        scene: &Scene,
    ) -> Result<Self, Rc<HostedLayerError>> {
        let generation = engines.generation(runtime, context)?;
        let part = ScenePart::new(&generation, redraw, size, scene)?;
        let participant: Rc<dyn SceneParticipant> = part.clone();
        generation.mount(&participant);
        Ok(Self { generation, part })
    }

    /// The context generation this renderer's generation is bound to.
    #[cfg(all(test, target_os = "macos"))]
    fn context(&self) -> &Arc<SharedGpuContext> {
        self.generation.context()
    }

    /// Applies the staged frame contract now — the test driver for what
    /// `produce` does inside a batch.
    #[cfg(all(test, target_os = "macos"))]
    fn record_if_needed(
        &self,
        target: &wgpu::Texture,
        display: Display,
    ) -> Result<(), HostedLayerError> {
        self.part.stage((target.width(), target.height()), display);
        self.part.apply_staged()
    }
}

impl HostedRenderer for SceneRenderer {
    fn generation(&self) -> u64 {
        self.generation.context().generation()
    }

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
        self.generation.produce(target_time)?;
        if let Some(failure) = self.part.take_failure() {
            return Err(HostedError::Scene(failure));
        }
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

    /// Drains a failure the shared generation routed to this scene —
    /// the owner's next wake settles it before any frame gate, without
    /// re-producing the failed generation.
    fn take_failure(&mut self) -> Option<HostedError> {
        self.part.take_failure().map(HostedError::Scene)
    }

    /// The scene's submission evidence is its production generation
    /// itself — retained by the in-flight submission so a late
    /// completion checks that generation's immutable sealed outcome,
    /// never the owner's mutable flag.
    fn submission_evidence(&self) -> Option<Rc<EngineGeneration>> {
        Some(Rc::clone(&self.generation))
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    use waterui::{Binding, SignalExt, binding};
    use waterui_core::{AnyView, Environment, View};
    use waterui_graphics::cherenkov::RenderError;
    use waterui_graphics::draw::{Command, Draw, Paint, Recorder, WorkingColor};
    use waterui_graphics::resources::RecordingResources;
    use waterui_graphics::scene_view::{SceneContent, SceneViewMergeToParent};
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

    fn renderer() -> (SceneRenderer, Rc<Cell<u32>>) {
        let draws = Rc::new(Cell::new(0));
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("a GPU adapter is required on test hardware");
        let context = runtime.context();
        let engines = Rc::new(SceneEngine::new());
        let redraw = RedrawHandle::new(|| {});
        let scene = Scene::new(SceneView::new(CountingContent {
            draws: draws.clone(),
        }));
        let size = OffscreenSize::try_from_pixels(512, 512).expect("nonzero size");
        let renderer = SceneRenderer::new(&runtime, &context, &engines, &redraw, size, &scene)
            .expect("scene generation settles");
        (renderer, draws)
    }

    fn target(renderer: &SceneRenderer, pixels: u32) -> wgpu::Texture {
        renderer
            .context()
            .device()
            .create_texture(&wgpu::TextureDescriptor {
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
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
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
        let scene = Scene::new(SceneView::new(AsyncIntrinsic { size: size.clone() }));
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
    /// and it is applied at every scale, exactly once. One renderer
    /// suffices: each `record_content` is an independent recording.
    #[test]
    fn scene_recording_wraps_logical_content_in_the_display_scale() {
        let (renderer, _) = renderer();
        for scale in [1.0f64, 2.0, 3.0] {
            let (mut content, _held, _again) = renderer.part.record_content(512.0, 512.0, scale);
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
    /// recording's root transform changed.
    #[test]
    fn geometry_key_tracks_scale_and_skips_headroom_only_updates() {
        let (mut renderer, draws) = renderer();
        let record = |renderer: &mut SceneRenderer, pixels: u32, scale: f64, headroom: f32| {
            let target = target(renderer, pixels);
            renderer
                .record_if_needed(&target, Display { scale, headroom })
                .expect("staged record applies");
        };

        record(&mut renderer, 512, 1.0, 1.0);
        assert_eq!(draws.get(), 1);
        record(&mut renderer, 512, 1.0, 4.0);
        assert_eq!(draws.get(), 1, "a headroom-only update re-records nothing");
        record(&mut renderer, 1024, 2.0, 1.0);
        assert_eq!(
            draws.get(),
            2,
            "same logical box at a new scale rewrites the transform"
        );
        record(&mut renderer, 1024, 2.0, 1.0);
        assert_eq!(draws.get(), 2, "an unchanged geometry stays cached");
        record(&mut renderer, 256, 2.0, 1.0);
        assert_eq!(draws.get(), 3, "a new logical size re-records");
    }

    fn square(color: WorkingColor) -> PictureRecording {
        Picture::record(|scene| {
            scene.fill(kurbo::Rect::new(0.0, 0.0, 10.0, 10.0), color);
        })
    }

    /// A `Scene` over a real reactive `Picture`: its content watches the
    /// recording signal through whichever invalidator the host installs.
    fn picture_scene(recording: &Binding<WorkingColor>) -> Scene {
        let picture = Picture::new(Size::new(10.0, 10.0), recording.map(square));
        let scene_view =
            AnyView::new(picture.body(&Environment::new().extending(SceneViewMergeToParent)))
                .downcast::<SceneView>()
                .unwrap_or_else(|_| panic!("a merged picture is a SceneView"));
        Scene::new(*scene_view)
    }

    /// The colour the content's last recording draws, read out of the
    /// picture's own display list — the observable output a reactive
    /// `Picture` changes when its recording signal lands.
    fn recorded_color(renderer: &SceneRenderer) -> WorkingColor {
        let (mut content, _held, _again) = renderer.part.record_content(10.0, 10.0, 1.0);
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
    /// the watchers it feeds along with every other engine-bound value, so a
    /// renderer created on a fresh generation must put it back — on the first
    /// creation and on every replacement — or fine-grained invalidation dies
    /// with the old engine. An unmounted scene gains no subscription.
    #[expect(
        clippy::too_many_lines,
        reason = "the mount, rebuild, rebind and unmount lifecycle is one narrative contract"
    )]
    #[test]
    fn engine_recreation_preserves_the_mounted_invalidator() {
        const RED: WorkingColor = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);
        const BLUE: WorkingColor = WorkingColor::new([0.0, 0.0, 1.0, 1.0]);
        const GREEN: WorkingColor = WorkingColor::new([0.0, 1.0, 0.0, 1.0]);

        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("a GPU adapter is required on test hardware");
        let context = runtime.context();
        let engines = Rc::new(SceneEngine::new());
        let redraws = Arc::new(AtomicU32::new(0));
        let redraws_probe = Arc::clone(&redraws);
        let redraw = RedrawHandle::new(move || {
            redraws_probe.fetch_add(1, Ordering::Relaxed);
        });
        let color = binding(RED);
        let mut scene = picture_scene(&color);
        let size = OffscreenSize::try_from_pixels(20, 20).expect("nonzero size");

        // An unmounted scene installs nothing: the renderer's rebuild leaves
        // the content without a watcher rather than inventing one, so a model
        // update on it never dirties or wakes.
        let quiet_color = binding(RED);
        let scene_unmounted = picture_scene(&quiet_color);
        let unmounted_renderer = SceneRenderer::new(
            &runtime,
            &context,
            &engines,
            &redraw,
            size,
            &scene_unmounted,
        )
        .expect("scene generation settles");
        unmounted_renderer
            .record_if_needed(
                &target(&unmounted_renderer, 20),
                Display {
                    scale: 1.0,
                    headroom: 1.0,
                },
            )
            .expect("staged record applies");
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
        let renderer = SceneRenderer::new(&runtime, &context, &engines, &redraw, size, &scene)
            .expect("scene generation settles");

        // The first record clears the seeded dirty bit. A fresh install on an
        // idle engine may itself ask for a frame through the surface's wake;
        // the invalidator's contribution is the delta each `set` adds.
        renderer
            .record_if_needed(
                &target(&renderer, 20),
                Display {
                    scale: 1.0,
                    headroom: 1.0,
                },
            )
            .expect("staged record applies");
        assert!(!scene.dirty.get(), "an ordinary record clears dirty");
        assert_eq!(recorded_color(&renderer), RED);

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
        renderer
            .record_if_needed(
                &target(&renderer, 20),
                Display {
                    scale: 1.0,
                    headroom: 1.0,
                },
            )
            .expect("staged record applies");
        assert_eq!(recorded_color(&renderer), BLUE);

        // A real context replacement keeps the semantic state — the picture,
        // the subscription — while the engine is rebuilt.
        context.mark_device_lost_for_testing("test device loss");
        let fresh = pollster::block_on(runtime.context_after(context.generation()));
        assert!(fresh.generation() > context.generation());
        let renderer = SceneRenderer::new(&runtime, &fresh, &engines, &redraw, size, &scene)
            .expect("the new context generation settles");
        assert_eq!(recorded_color(&renderer), BLUE);
        renderer
            .record_if_needed(
                &target(&renderer, 20),
                Display {
                    scale: 1.0,
                    headroom: 1.0,
                },
            )
            .expect("staged record applies");
        assert!(!scene.dirty.get());
        let wakes = redraws.load(Ordering::Relaxed);
        color.set(GREEN);
        assert!(
            scene.dirty.get(),
            "invalidation survives the generation change"
        );
        assert_eq!(redraws.load(Ordering::Relaxed), wakes + 1);
        renderer
            .record_if_needed(
                &target(&renderer, 20),
                Display {
                    scale: 1.0,
                    headroom: 1.0,
                },
            )
            .expect("staged record applies");
        assert_eq!(recorded_color(&renderer), GREEN);

        // Unmount cancels the watcher: a later model update neither dirties
        // nor wakes.
        scene.unmount();
        let wakes = redraws.load(Ordering::Relaxed);
        color.set(RED);
        assert!(!scene.dirty.get());
        assert_eq!(redraws.load(Ordering::Relaxed), wakes);
    }

    /// A failed shared batch reaches every mounted scene's owner through
    /// its own redraw handle: `note_failure` wakes an idle participant
    /// exactly once, and the woken owner's next `present` consumes the
    /// routed failure through `take_failure` as the typed scene error.
    /// The failed generation itself is never re-produced for any of them.
    ///
    /// What this test does not cover: the host's `handle_redraw_request`
    /// drain, `complete_ready` and `arm_context_watch` — those live on
    /// `SurfaceState`/`SurfaceView` and need a real view; they are
    /// covered by integration/physical runs only.
    #[test]
    fn a_failed_batch_wakes_each_mounted_owner_with_the_typed_failure() {
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("a GPU adapter is required on test hardware");
        let context = runtime.context();
        let engines = Rc::new(SceneEngine::new());
        let size = OffscreenSize::try_from_pixels(20, 20).expect("nonzero size");

        let wakes_a = Arc::new(AtomicU32::new(0));
        let wakes_b = Arc::new(AtomicU32::new(0));
        let probe_a = Arc::clone(&wakes_a);
        let probe_b = Arc::clone(&wakes_b);
        let redraw_a = RedrawHandle::new(move || {
            probe_a.fetch_add(1, Ordering::Relaxed);
        });
        let redraw_b = RedrawHandle::new(move || {
            probe_b.fetch_add(1, Ordering::Relaxed);
        });
        let scene_a = Scene::new(SceneView::new(CountingContent {
            draws: Rc::new(Cell::new(0)),
        }));
        let scene_b = Scene::new(SceneView::new(CountingContent {
            draws: Rc::new(Cell::new(0)),
        }));
        let mut renderer_a =
            SceneRenderer::new(&runtime, &context, &engines, &redraw_a, size, &scene_a)
                .expect("scene generation settles");
        let mut renderer_b =
            SceneRenderer::new(&runtime, &context, &engines, &redraw_b, size, &scene_b)
                .expect("scene generation settles");
        let base_a = wakes_a.load(Ordering::Relaxed);
        let base_b = wakes_b.load(Ordering::Relaxed);

        // The batch fails on the shared generation: the real settle path
        // retains the failure and routes it to every live participant.
        let generation = engines
            .generation(&runtime, &context)
            .expect("the shared generation");
        generation.fail_for_testing(HostedLayerError::Render(RenderError::DeviceLost));

        // Each affected owner was summoned through its own redraw handle.
        assert_eq!(
            wakes_a.load(Ordering::Relaxed),
            base_a + 1,
            "the failure wakes owner A exactly once"
        );
        assert_eq!(
            wakes_b.load(Ordering::Relaxed),
            base_b + 1,
            "the failure wakes owner B exactly once"
        );

        // The woken owner's own present consumes the routed failure — no
        // produce, no retry, no stale composite.
        let display = Display {
            scale: 1.0,
            headroom: 1.0,
        };
        let failure_a = renderer_a
            .present(
                &target(&renderer_a, 20),
                display,
                FrameTime(std::time::Instant::now()),
            )
            .expect_err("owner A reads the routed failure");
        assert!(matches!(failure_a, HostedError::Scene(_)));
        let failure_b = renderer_b
            .present(
                &target(&renderer_b, 20),
                display,
                FrameTime(std::time::Instant::now()),
            )
            .expect_err("owner B reads the routed failure");
        assert!(matches!(failure_b, HostedError::Scene(_)));

        // The retained generation stays failed — neither owner's present
        // re-produced it.
        assert!(
            generation
                .produce(FrameTime(std::time::Instant::now()))
                .is_err_and(|error| matches!(
                    *error,
                    HostedLayerError::Render(RenderError::DeviceLost)
                )),
            "the retained failed generation never retries"
        );
    }
}
