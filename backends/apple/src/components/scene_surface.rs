//! Retained scene recordings on the native Metal surface host.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, mpsc};

use waterui_core::layout::{ProposalSize, StretchAxis, ViewDimensions};
use waterui_graphics::cherenkov::{
    Content, Display, Draw, Engine, FrameTime, Next, Surface, kurbo,
};
use waterui_graphics::cherenkov_gpu::{
    Gpu,
    interop::{OutputAlpha, OutputColor, Presenter, TextureOutput, TextureTarget, shader_delivery},
};
use waterui_graphics::gpu::{GpuRuntime, RedrawHandle, SharedGpuContext};
use waterui_graphics::input::SurfaceInputEvent;
use waterui_graphics::offscreen::OffscreenSize;
use waterui_graphics::resources::{HeldResources, SceneResources};
use waterui_graphics::scene_view::{SceneView, resolve_scene_proposal, scene_stretch_axis};
use waterui_graphics::wgpu;

use super::{HostedRenderer, HostedView};

/// Content and its structural invalidation survive ordinary frame submissions.
#[derive(Debug)]
pub struct Scene {
    view: Rc<RefCell<SceneView>>,
    dirty: Rc<Cell<bool>>,
}

impl Scene {
    /// Keeps the content instance shared by measurement, input and rendering.
    #[must_use]
    pub fn new(view: SceneView) -> Self {
        Self {
            view: Rc::new(RefCell::new(view)),
            dirty: Rc::new(Cell::new(true)),
        }
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
        self.view
            .borrow_mut()
            .content_mut()
            .set_invalidator(Some(Rc::new(move || {
                dirty.set(true);
                redraw.request_redraw();
            })));
    }

    fn unmount(&mut self) {
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

    fn renderer(
        &mut self,
        runtime: &GpuRuntime,
        context: &Arc<SharedGpuContext>,
        redraw: &RedrawHandle,
        size: OffscreenSize,
    ) -> Box<dyn HostedRenderer> {
        Box::new(SceneRenderer::new(runtime, context, redraw, size, self))
    }
}

/// One engine generation owns the recording and every resource that it names.
struct SceneRenderer {
    surface: Surface<Gpu>,
    installed: HeldResources,
    resources: SceneResources,
    engine: Rc<Engine<Gpu>>,
    context: Arc<SharedGpuContext>,
    textures: mpsc::Receiver<wgpu::Texture>,
    source: wgpu::Texture,
    presenter: Presenter,
    view: Rc<RefCell<SceneView>>,
    dirty: Rc<Cell<bool>>,
    redraw: RedrawHandle,
    /// The geometry the installed recording was made for: the logical size
    /// handed to `build_scene` and the display scale its root transform
    /// carries. A pixel resize that keeps the logical box but changes the
    /// scale rewrites the recording's transform, so the key is points and
    /// scale together; a headroom-only display update shares the key and
    /// re-records nothing.
    recorded_geometry: Option<(f32, f32, f64)>,
}

impl SceneRenderer {
    fn new(
        runtime: &GpuRuntime,
        context: &Arc<SharedGpuContext>,
        redraw: &RedrawHandle,
        size: OffscreenSize,
        scene: &Scene,
    ) -> Self {
        let engine = Rc::new(
            runtime
                .engine_on(context)
                .expect("scene engine creation failed"),
        );
        let wake = redraw.clone();
        // Bound recorder operands wake the engine without rebuilding the scene.
        engine.set_waker(move || wake.request_redraw());
        let resources = SceneResources::new(engine.clone());
        let (target, textures) = TextureTarget::new((size.width(), size.height()));
        let surface = engine
            .surface(target)
            .expect("scene surface creation failed");
        let source = textures
            .try_recv()
            .expect("scene surface publishes its texture");
        let presenter = Presenter::new(
            context.device(),
            shader_delivery(context.adapter().get_info().backend, context.device())
                .expect("scene presentation shaders failed"),
        );
        scene.view.borrow_mut().content_mut().rebuild_for_engine();
        scene.dirty.set(true);
        Self {
            surface,
            installed: HeldResources::empty(),
            resources,
            engine,
            context: context.clone(),
            textures,
            source,
            presenter,
            view: scene.view.clone(),
            dirty: scene.dirty.clone(),
            redraw: redraw.clone(),
            recorded_geometry: None,
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "native texture extents and backing scales become f32 layout points"
    )]
    fn record_if_needed(&mut self, target: &wgpu::Texture, display: Display) {
        let pixels = (target.width(), target.height());
        if self.surface.size() != pixels {
            self.surface
                .resize(pixels)
                .expect("scene surface resize failed");
        }
        let size = (
            f64::from(pixels.0) / display.scale,
            f64::from(pixels.1) / display.scale,
        );
        let size = (size.0 as f32, size.1 as f32);
        let dirty = self.dirty.replace(false);
        let geometry = (size.0, size.1, display.scale);
        if !dirty && self.recorded_geometry == Some(geometry) {
            return;
        }
        let (recording, held, again) = self.record_content(size.0, size.1, display.scale);
        self.surface.update(|tx| {
            tx[self.surface.root()].content(recording);
        });
        // Replacing the installed layer precedes releasing the old registrations.
        self.installed = held;
        self.recorded_geometry = Some(geometry);
        if again {
            self.dirty.set(true);
            self.redraw.request_redraw();
        }
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

impl HostedRenderer for SceneRenderer {
    fn generation(&self) -> u64 {
        self.context.generation()
    }

    fn present(&mut self, target: &wgpu::Texture, display: Display) -> Next {
        self.record_if_needed(target, display);
        self.surface
            .display(display)
            .expect("scene display configuration failed");
        let next = self
            .engine
            .render(FrameTime::now())
            .expect("scene rendering failed");
        for texture in self.textures.try_iter() {
            self.source = texture;
        }
        let source = self
            .source
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.presenter.texture(
            self.context.device(),
            self.context.queue(),
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
        next
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use waterui_graphics::cherenkov::{Command, Draw, Recorder, WorkingColor};
    use waterui_graphics::resources::RecordingResources;
    use waterui_graphics::scene_view::SceneContent;

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
        let redraw = RedrawHandle::new(|| {});
        let scene = Scene::new(SceneView::new(CountingContent {
            draws: draws.clone(),
        }));
        let size = OffscreenSize::try_from_pixels(512, 512).expect("nonzero size");
        let renderer = SceneRenderer::new(&runtime, &context, &redraw, size, &scene);
        (renderer, draws)
    }

    fn target(renderer: &SceneRenderer, pixels: u32) -> wgpu::Texture {
        renderer
            .context
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

    /// The recording wraps the logical-point content in exactly one
    /// display-scale transform — the conversion `Offscreen` applies —
    /// and it is applied at every scale, exactly once. One renderer
    /// suffices: each `record_content` is an independent recording.
    #[test]
    fn scene_recording_wraps_logical_content_in_the_display_scale() {
        let (renderer, _) = renderer();
        for scale in [1.0f64, 2.0, 3.0] {
            let (mut content, _held, _again) = renderer.record_content(512.0, 512.0, scale);
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
            renderer.record_if_needed(&target, Display { scale, headroom });
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
}
