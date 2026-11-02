use std::cell::RefCell;
use std::rc::Rc;

use waterui_core::view_renderer::{CustomViewRenderer, RenderError, RenderRequest, RenderResult};
use waterui_core::{AnyView, ContentLedger, Environment};
use waterui_graphics::scene_view::SceneViewMergeToParent;
use waterui_text::FontCollection;

use crate::platform::{DeviceLoss, OffscreenSurface, PresentationSurface as _, SurfaceFrame};
use crate::readback::readback_texture_rgba8;
use crate::renderer::{FontFamilyResolution, HydrolysisRenderer};

/// `ViewRenderer` implementation backed by Hydrolysis offscreen rendering.
///
/// A capture mounts the view under a fresh [`ContentLedger`] and keeps the
/// tree mounted while it awaits the content the view owes, re-flushing the
/// retained tree between passes, then renders it once.
pub struct HydrolysisViewRenderer {
    surface: Rc<RefCell<Option<OffscreenSurface>>>,
    theme: Rc<dyn crate::theme::WidgetTheme>,
    /// The environment the runner that installed this renderer had fully
    /// assembled — theme tokens, hooks and the shared font collection a
    /// capture's text engine shapes from. A capture mounts under a snapshot
    /// of it, so the captured subtree resolves exactly what it would mounted
    /// in one of the runtime's windows.
    env: Environment,
    /// The named-family resolution mode the runtime's own renderers shape
    /// under; the capture's text engine resolves the same way.
    family_resolution: FontFamilyResolution,
}

impl core::fmt::Debug for HydrolysisViewRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydrolysisViewRenderer")
            .finish_non_exhaustive()
    }
}

impl HydrolysisViewRenderer {
    #[must_use]
    /// Creates a renderer drawing with `theme` under `env` — the environment
    /// as the runner finished assembling it — resolving named font families
    /// as the runtime's own renderers do.
    ///
    /// `env` is captured as a snapshot: install this renderer once the
    /// environment carries everything a subtree can read (theme tokens,
    /// hooks, the [`FontCollection`]), or the capture resolves against
    /// entries the runtime itself never sees.
    pub fn new(
        theme: Rc<dyn crate::theme::WidgetTheme>,
        env: Environment,
        family_resolution: FontFamilyResolution,
    ) -> Self {
        Self {
            surface: Rc::new(RefCell::new(None)),
            theme,
            env,
            family_resolution,
        }
    }

    /// Ensures the offscreen surface exists at `width`×`height`, acquires
    /// its next frame and returns the GPU handles the capture renders
    /// through.
    #[expect(
        clippy::future_not_send,
        reason = "view rendering runs on the main thread; the future borrows non-Send GPU state"
    )]
    async fn acquire_frame(
        &self,
        width: u32,
        height: u32,
    ) -> (
        SurfaceFrame,
        wgpu::Adapter,
        wgpu::Device,
        wgpu::Queue,
        DeviceLoss,
        wgpu::TextureFormat,
    ) {
        let cell = &self.surface;
        if cell.borrow().is_none() {
            // Test builds of this renderer — the crate's own `cfg(test)`
            // targets and `testing`-feature hosts — open their surface
            // through the test adapter selection so a CI runner whose only
            // device is a software rasterizer (Windows WARP, llvmpipe) still
            // answers (#2732). The production build keeps the production
            // selection.
            #[cfg(any(test, feature = "testing"))]
            let offscreen =
                OffscreenSurface::new_for_tests(width, height, wgpu::TextureFormat::Rgba8Unorm)
                    .await;
            #[cfg(not(any(test, feature = "testing")))]
            let offscreen =
                OffscreenSurface::new(width, height, wgpu::TextureFormat::Rgba8Unorm).await;
            *cell.borrow_mut() = Some(offscreen);
        }

        // The surface borrow ends before this returns: a suspended frame
        // cannot hold a `RefCell` borrow another task might need while the
        // browser device is awaited.
        let mut borrowed = cell.borrow_mut();
        let surface = borrowed
            .as_mut()
            .expect("hydrolysis view renderer surface must initialize before rendering");
        surface.resize(width, height);
        let frame = surface.acquire();
        (
            frame,
            surface.adapter().clone(),
            surface.device().clone(),
            surface.queue().clone(),
            surface.device_loss().clone(),
            surface.format(),
        )
    }
}

impl CustomViewRenderer for HydrolysisViewRenderer {
    #[expect(
        clippy::future_not_send,
        reason = "view rendering runs on the main thread; the future borrows non-Send GPU and Environment state"
    )]
    async fn render_to_rgba(
        &self,
        view: AnyView,
        request: RenderRequest,
    ) -> Result<RenderResult, RenderError> {
        let surface = Rc::clone(&self.surface);
        #[expect(
            clippy::cast_sign_loss,
            clippy::cast_possible_truncation,
            reason = "the proposal's width is clamped to at least 1.0 and a display size always fits u32"
        )]
        let width = request.size().width.max(1.0).round() as u32;
        #[expect(
            clippy::cast_sign_loss,
            clippy::cast_possible_truncation,
            reason = "the proposal's height is clamped to at least 1.0 and a display size always fits u32"
        )]
        let height = request.size().height.max(1.0).round() as u32;

        let (frame, adapter, device, queue, device_loss, format) =
            self.acquire_frame(width, height).await;

        // The capture's text engine shapes against the collection the runner
        // installed — `FontCollection::from_env` names the host's obligation
        // when the environment carries none.
        let mut renderer = HydrolysisRenderer::with_fonts(
            Rc::clone(&self.theme),
            &FontCollection::from_env(&self.env),
            self.family_resolution,
        );
        renderer.reset_scene();
        renderer.begin_rebuild_frame();

        let ledger = ContentLedger::new();
        let env = self
            .env
            .extending(SceneViewMergeToParent)
            .extending(ledger.clone());
        let view = crate::renderer::normalize_view_for_render(view, &env);
        let bounds = kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        renderer.capture_window_tree(
            view,
            &env,
            bounds,
            kurbo::Affine::IDENTITY,
            kurbo::Affine::IDENTITY,
        );
        renderer.finish_rebuild_frame();
        // The tree stays mounted while its content arrives: each pass
        // flushes what resolved content changed into the retained tree —
        // a decoded photo taking its size — and yields so the tasks the
        // flush spawned run before the ledger is checked again.
        ledger
            .wait(
                request.content_deadline(),
                waterui::task::sleep(request.content_deadline()),
                || {
                    renderer.drain_producer_wakes();
                    let _ = renderer.flush_window_tree(
                        &env,
                        bounds,
                        kurbo::Affine::IDENTITY,
                        kurbo::Affine::IDENTITY,
                    );
                    waterui::task::yield_now()
                },
            )
            .await?;
        crate::engine::engine_await!(renderer.render_scene_to_texture(
            crate::renderer::HydrolysisRenderTarget {
                adapter: &adapter,
                device: &device,
                queue: &queue,
                device_loss,
                texture: frame.texture(),
                format,
                width,
                height,
                base_color: waterui_graphics::draw::WorkingColor::TRANSPARENT,
            }
        ))
        .map_err(gpu_failure)?;
        renderer.frame_work_counters_mut().gpu_submissions += 1;

        let mut borrowed = surface.borrow_mut();
        let surface = borrowed
            .as_mut()
            .expect("hydrolysis view renderer surface lost its surface");
        let rgba_data = readback_texture_rgba8(&*surface, frame.texture(), width, height)
            .map_err(gpu_failure)?;
        surface.present(frame);

        Ok(RenderResult::new(rgba_data, width, height))
    }
}

/// Wraps a Hydrolysis GPU failure as the [`RenderError`] callers receive.
fn gpu_failure(error: impl core::error::Error + Send + Sync + 'static) -> RenderError {
    RenderError::Gpu(Box::new(error))
}
