use std::rc::Rc;

use waterui::View;
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Environment};
use waterui_graphics::{
    DeviceLoss, GpuContext, GpuFrame, GpuSurface, GpuView, SceneViewMergeToParent,
};

use crate::engine::WidgetTheme;
use crate::renderer::HydrolysisRenderer;
use crate::time::Instant;

/// A `GpuView` that renders any cloneable `View` through hydrolysis.
pub struct HydrolysisGpuView<V>
where
    V: View + Clone + 'static,
{
    view: V,
    theme: Rc<dyn WidgetTheme>,
    adapter: Option<wgpu::Adapter>,
    /// Reports this device lost; taken when the device was opened.
    device_loss: Option<DeviceLoss>,
    renderer: Option<HydrolysisRenderer>,
    env: Option<Environment>,
    needs_rebuild: bool,
    /// Arbitrary epoch the host's animation clock is projected onto.
    ///
    /// The embedded renderer samples animations at `Instant`s, but the host
    /// hands this view a monotonically advancing `GpuFrame::elapsed()`; only
    /// the differences matter, so any fixed epoch makes the projection exact.
    animation_epoch: Instant,
}

impl<V> core::fmt::Debug for HydrolysisGpuView<V>
where
    V: View + Clone + 'static,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydrolysisGpuView").finish_non_exhaustive()
    }
}

impl<V> HydrolysisGpuView<V>
where
    V: View + Clone + 'static,
{
    #[must_use]
    pub fn new(view: V, theme: Rc<dyn WidgetTheme>) -> Self {
        Self {
            view,
            theme,
            adapter: None,
            device_loss: None,
            renderer: None,
            env: None,
            needs_rebuild: true,
            animation_epoch: Instant::now(),
        }
    }
}

impl<V> GpuView for HydrolysisGpuView<V>
where
    V: View + Clone + 'static,
{
    async fn setup(&mut self, ctx: &GpuContext<'_>, env: &mut Environment) {
        let scoped_env = env.extending(SceneViewMergeToParent);
        let mut renderer = HydrolysisRenderer::new(ctx.adapter, ctx.device, Rc::clone(&self.theme));
        renderer.set_host_redraw_handle(ctx.redraw_handle.clone());
        renderer.prepare_window_tree(AnyView::new(self.view.clone()), &scoped_env);
        renderer.setup_embedded_gpu_surfaces(ctx).await;
        renderer.setup_embedded_effects(ctx).await;

        self.adapter = Some(ctx.adapter.clone());
        self.device_loss = Some(ctx.device_loss.clone());
        self.renderer = Some(renderer);
        self.env = Some(scoped_env);
    }

    #[allow(clippy::cast_precision_loss)]
    fn render(&mut self, frame: &mut GpuFrame) {
        let adapter = self
            .adapter
            .as_ref()
            .expect("HydrolysisGpuView adapter missing");
        let device_loss = self
            .device_loss
            .as_ref()
            .expect("HydrolysisGpuView device_loss missing");
        let renderer = self
            .renderer
            .as_mut()
            .expect("HydrolysisGpuView used before setup");
        let env = self
            .env
            .as_ref()
            .expect("HydrolysisGpuView environment missing");

        renderer.set_frame_resources(
            adapter,
            frame.device,
            frame.queue,
            device_loss,
            frame.gpu_context_id(),
            &frame.shared_device(),
        );
        renderer.poll_gpu_surface_redraw_handles();

        // Advance the embedded frame clock from the host's animation clock.
        // Without this the renderer samples every animation at its build
        // instant: progress never completes, `animation_dirty` latches, and
        // the surface rebuilds itself every frame forever.
        renderer.set_frame_instant(self.animation_epoch + frame.elapsed());

        let animation_dirty = renderer.advance_animations();
        let rebuild_requested = renderer.take_rebuild_request();
        // A pure reactive value change raises the *patch* trigger (the retained
        // tree's refresh pump), not the rebuild trigger — without consuming it the
        // embedded surface would keep presenting a stale scene until an animation
        // or structural change happened to fire.
        let patch_requested = renderer.take_patch_request();
        let should_rebuild =
            self.needs_rebuild || animation_dirty || rebuild_requested || patch_requested;

        if should_rebuild {
            renderer.reset_scene();
            renderer.begin_rebuild_frame();
            let bounds = kurbo::Rect::new(0.0, 0.0, frame.width as f64, frame.height as f64);
            renderer.capture_window_tree(
                waterui_core::AnyView::new(self.view.clone()),
                env,
                bounds,
                kurbo::Affine::IDENTITY,
                kurbo::Affine::IDENTITY,
            );
            renderer.finish_rebuild_frame();
            self.needs_rebuild = false;
        }

        renderer.render_scene_to_surface(crate::renderer::HydrolysisRenderTarget {
            adapter,
            device: frame.device,
            queue: frame.queue,
            device_loss: device_loss.clone(),
            gpu_context_id: frame.gpu_context_id(),
            shared_device: frame.shared_device(),
            display_scale: frame.display_scale,
            headroom: frame.headroom,
            persistent: true,
            texture: Some(frame.texture),
            view: &frame.view,
            format: frame.format,
            width: frame.width,
            height: frame.height,
            base_color: peniko::Color::TRANSPARENT,
        });
        // Work raised during this render — a structural request or a reactive
        // patch — needs another frame. The patch bit is only peeked (not taken)
        // so the next render's `take_patch_request` still observes it.
        let next_frame = renderer.take_rebuild_request()
            || renderer.has_patch_request()
            || renderer.take_redraw_request();
        renderer.clear_frame_resources();

        if animation_dirty || next_frame {
            frame.request_redraw();
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.view.stretch_axis()
    }
}

/// Extension trait for rendering a view through hydrolysis into a `GpuSurface`.
pub trait HydrolysisExt: View + Clone + Sized + 'static {
    /// Wrap this view in a hydrolysis-powered `GpuSurface`. The embedded
    /// renderer is style-driven like the window runtime: `theme` is the same
    /// `WidgetTheme` the `Style` the runtime was launched with supplies.
    fn hydrolysis(self, theme: Rc<dyn WidgetTheme>) -> GpuSurface {
        GpuSurface::new(HydrolysisGpuView::new(self, theme))
    }
}

impl<V> HydrolysisExt for V where V: View + Clone + Sized + 'static {}
