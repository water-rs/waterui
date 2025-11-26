//! GPU renderer built on top of Vello/wgpu.

use std::fmt;

use tracing::error;
use vello::{
    Renderer, RendererError, RendererOptions, Scene as VelloScene, SceneBuilder,
    kurbo::{Affine, Rect as KurboRect},
    peniko::{Brush, Color as PenikoColor, Fill},
};
use waterui_core::Environment;
use wgpu::{Device, Queue, Surface, SurfaceConfiguration, SurfaceError, SurfaceTexture};

use crate::{
    backend::{FrameResult, RenderBackend},
    scene::{DrawCommand, Scene as HydroScene},
    tree::{DirtyReason, LayoutEngine, NodeId, RenderCtx, RenderTree},
};

/// GPU backend that renders Hydrolysis scenes using Vello and wgpu surfaces.
pub struct VelloWgpuBackend<'surface> {
    surface: Surface<'surface>,
    device: Device,
    queue: Queue,
    config: SurfaceConfiguration,
    renderer: Renderer,
}

impl<'surface> fmt::Debug for VelloWgpuBackend<'surface> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VelloWgpuBackend")
            .field("width", &self.config.width)
            .field("height", &self.config.height)
            .finish()
    }
}

impl<'surface> VelloWgpuBackend<'surface> {
    /// Creates a new GPU backend. Callers must create the device/queue/surface externally (e.g. via `winit`).
    pub fn new(
        surface: Surface<'surface>,
        device: Device,
        queue: Queue,
        config: SurfaceConfiguration,
    ) -> Result<Self, RendererError> {
        let renderer = Renderer::new(&device, RendererOptions::default())?;
        let mut backend = Self {
            surface,
            device,
            queue,
            config,
            renderer,
        };
        backend.configure_surface();
        Ok(backend)
    }

    /// Reconfigures the swapchain (call when the window resizes).
    pub fn reconfigure(&mut self, config: SurfaceConfiguration) {
        self.config = config;
        self.configure_surface();
    }

    fn configure_surface(&mut self) {
        self.surface.configure(&self.device, &self.config);
    }
}

fn paint_subtree(tree: &mut RenderTree, ctx: &mut RenderCtx<'_>, id: NodeId) {
    if let Some(node) = tree.node_mut(id) {
        node.paint(ctx);
    }
    let children = tree.children(id).to_vec();
    for child in children {
        paint_subtree(tree, ctx, child);
    }
}

impl<'surface> RenderBackend for VelloWgpuBackend<'surface> {
    fn render(&mut self, tree: &mut RenderTree, env: &Environment) -> FrameResult {
        let mut had_work = false;
        let dirty_nodes: Vec<_> = tree.drain_dirty().collect();
        for dirty in dirty_nodes {
            had_work = true;
            if let Some(node) = tree.node_mut(dirty.id) {
                if matches!(dirty.reason, DirtyReason::Reactive) {
                    node.update_reactive();
                }
            }
        }

        if !had_work {
            return FrameResult::Idle;
        }

        let Some(root_id) = tree.root() else {
            return FrameResult::Idle;
        };

        let surface_texture = match self.surface.get_current_texture() {
            Ok(texture) => texture,
            Err(SurfaceError::Lost) => {
                self.configure_surface();
                return FrameResult::Idle;
            }
            Err(SurfaceError::Outdated) => return FrameResult::Idle,
            Err(error) => {
                error!("failed to acquire GPU surface texture: {error:?}");
                return FrameResult::Idle;
            }
        };

        let mut engine = LayoutEngine::new(tree, env);
        engine.run();

        let mut render_ctx = RenderCtx::new(env);
        paint_subtree(tree, &mut render_ctx, root_id);
        let scene = render_ctx.finish();
        self.present_scene(scene, surface_texture);
        FrameResult::Presented
    }
}

impl<'surface> VelloWgpuBackend<'surface> {
    fn present_scene(&mut self, scene: HydroScene, surface_texture: SurfaceTexture) {
        let vello_scene = self.build_vello_scene(&scene);
        if let Err(error) = self.renderer.render_to_surface(
            &self.device,
            &self.queue,
            &vello_scene,
            &surface_texture,
            &self.config,
        ) {
            error!("vello render_to_surface failed: {error:?}");
        }

        surface_texture.present();
    }

    fn build_vello_scene(&self, scene: &HydroScene) -> VelloScene {
        let mut builder = SceneBuilder::new();
        for command in scene.commands() {
            match command {
                DrawCommand::SolidRect { rect, color } => {
                    let brush = Brush::Solid(PenikoColor::new(
                        color.opacity,
                        color.red,
                        color.green,
                        color.blue,
                    ));
                    let kurbo_rect = KurboRect::new(
                        f64::from(rect.origin.x),
                        f64::from(rect.origin.y),
                        f64::from(rect.origin.x + rect.size.width),
                        f64::from(rect.origin.y + rect.size.height),
                    );
                    builder.fill(Fill::NonZero, Affine::IDENTITY, &brush, None, &kurbo_rect);
                }
                DrawCommand::Text { .. } => {
                    // TODO(text-rendering): convert text commands into Vello glyph runs.
                }
                DrawCommand::Placeholder(_) => {}
            }
        }
        builder.finish()
    }
}
