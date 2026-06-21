use waterui::View;
use waterui_core::Environment;
use waterui_core::layout::StretchAxis;
use waterui_graphics::{GpuContext, GpuFrame, GpuSurface, GpuView, SceneViewMergeToParent};

use crate::renderer::HydrolysisRenderer;

/// A `GpuView` that renders any cloneable `View` through hydrolysis.
#[derive(Debug)]
pub struct HydrolysisGpuView<V>
where
    V: View + Clone + 'static,
{
    view: V,
    renderer: Option<HydrolysisRenderer>,
    env: Option<Environment>,
    needs_rebuild: bool,
}

impl<V> HydrolysisGpuView<V>
where
    V: View + Clone + 'static,
{
    #[must_use]
    pub fn new(view: V) -> Self {
        Self {
            view,
            renderer: None,
            env: None,
            needs_rebuild: true,
        }
    }
}

impl<V> GpuView for HydrolysisGpuView<V>
where
    V: View + Clone + 'static,
{
    async fn setup(&mut self, ctx: &GpuContext<'_>, env: &mut Environment) {
        self.renderer = Some(HydrolysisRenderer::new(ctx.device));
        self.env = Some(env.extending(SceneViewMergeToParent));
    }

    #[allow(clippy::cast_precision_loss)]
    fn render(&mut self, frame: &mut GpuFrame) {
        let renderer = self
            .renderer
            .as_mut()
            .expect("HydrolysisGpuView used before setup");
        let env = self
            .env
            .as_ref()
            .expect("HydrolysisGpuView environment missing");

        renderer.set_frame_resources(frame.device, frame.queue);

        let animation_dirty = renderer.advance_animations();
        let rebuild_requested = renderer.take_rebuild_request();
        let should_rebuild = self.needs_rebuild || animation_dirty || rebuild_requested;

        if should_rebuild {
            renderer.reset_scene();
            renderer.begin_rebuild_frame();
            let bounds = vello::kurbo::Rect::new(0.0, 0.0, frame.width as f64, frame.height as f64);
            renderer.capture_window_tree(
                waterui_core::AnyView::new(self.view.clone()),
                env,
                bounds,
                vello::kurbo::Affine::IDENTITY,
                vello::kurbo::Affine::IDENTITY,
            );
            renderer.finish_rebuild_frame();
            self.needs_rebuild = false;
        }

        renderer.render_scene_to_surface(crate::renderer::HydrolysisRenderTarget {
            device: frame.device,
            queue: frame.queue,
            texture: Some(frame.texture),
            view: &frame.view,
            format: frame.format,
            width: frame.width,
            height: frame.height,
            base_color: vello::peniko::Color::TRANSPARENT,
        });
        let next_rebuild = renderer.take_rebuild_request();
        renderer.clear_frame_resources();

        if animation_dirty || next_rebuild {
            frame.request_redraw();
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.view.stretch_axis()
    }
}

/// Extension trait for rendering a view through hydrolysis into a `GpuSurface`.
pub trait HydrolysisExt: View + Clone + Sized + 'static {
    /// Wrap this view in a hydrolysis-powered `GpuSurface`.
    fn hydrolysis(self) -> GpuSurface {
        GpuSurface::new(HydrolysisGpuView::new(self))
    }
}

impl<V> HydrolysisExt for V where V: View + Clone + Sized + 'static {}
