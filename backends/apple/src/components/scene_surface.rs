//! Retained scene recordings on the native Metal surface host.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, mpsc};

use waterui_core::layout::{ProposalSize, StretchAxis, ViewDimensions};
use waterui_graphics::cherenkov::{Display, Engine, FrameTime, Next, Surface, kurbo};
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
    logical_size: Option<(f32, f32)>,
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
            logical_size: None,
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
        if !dirty && self.logical_size == Some(size) {
            return;
        }
        let mut resources = self.resources.recording();
        let mut again = false;
        let recording = self.surface.record(|recorder| {
            again = self.view.borrow_mut().content_mut().build_scene(
                recorder,
                &mut resources,
                size.0,
                size.1,
            );
        });
        let held = resources.finish();
        self.surface.update(|tx| {
            tx[self.surface.root()].content(recording);
        });
        // Replacing the installed layer precedes releasing the old registrations.
        self.installed = held;
        self.logical_size = Some(size);
        if again {
            self.dirty.set(true);
            self.redraw.request_redraw();
        }
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
