#[cfg(hydrolysis_gpu_painter)]
use std::cell::RefCell;
use std::rc::Rc;

use waterui_core::view_renderer::{CustomViewRenderer, RenderError, RenderRequest, RenderResult};
use waterui_core::{AnyView, ContentLedger, Environment};
use waterui_graphics::backend::SceneViewMergeToParent;
use waterui_text::FontCollection;

#[cfg(hydrolysis_gpu_painter)]
use crate::platform::{
    GpuGeneration, OffscreenGpuContext, OffscreenSurface, PresentationSurface as _, SurfaceFrame,
};
#[cfg(hydrolysis_gpu_painter)]
use crate::readback::readback_texture_rgba8;
use crate::renderer::{FontFamilyResolution, HydrolysisRenderer};

/// What a capture renders with: the painter of the runtime that installed
/// the service.
enum CaptureTarget {
    /// The GPU painter's offscreen texture, created on the first capture —
    /// on the runtime's device when the service was handed its context,
    /// on a device of its own otherwise.
    #[cfg(hydrolysis_gpu_painter)]
    Gpu {
        context: Option<OffscreenGpuContext>,
        surface: Rc<RefCell<Option<OffscreenSurface>>>,
    },
    /// The CPU painter, on the runtime's engine.
    #[cfg(hydrolysis_cpu_painter)]
    Cpu(Rc<crate::engine::RasterEngineState>),
}

/// `ViewRenderer` implementation backed by Hydrolysis offscreen rendering.
///
/// A capture mounts the view under a fresh [`ContentLedger`] and keeps the
/// tree mounted while it awaits the content the view owes, re-flushing the
/// retained tree between passes, then renders it once.
pub struct HydrolysisViewRenderer {
    target: CaptureTarget,
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
    /// Creates a renderer drawing on a GPU device of its own with `theme`
    /// under `env` — the environment as the runner finished assembling it —
    /// resolving named font families as the runtime's own renderers do.
    ///
    /// `env` is captured as a snapshot: install this renderer once the
    /// environment carries everything a subtree can read (theme tokens,
    /// hooks, the [`FontCollection`]), or the capture resolves against
    /// entries the runtime itself never sees.
    #[cfg(hydrolysis_gpu_painter)]
    pub fn new(
        theme: Rc<dyn crate::theme::WidgetTheme>,
        env: Environment,
        family_resolution: FontFamilyResolution,
    ) -> Self {
        Self::on_target(
            CaptureTarget::Gpu {
                context: None,
                surface: Rc::new(RefCell::new(None)),
            },
            theme,
            env,
            family_resolution,
        )
    }

    /// `Self::new` on the offscreen runtime's own `painter`: a capture
    /// renders on its device or CPU engine instead of opening another.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn on_painter(
        painter: &crate::platform::OffscreenPainter,
        theme: Rc<dyn crate::theme::WidgetTheme>,
        env: Environment,
        family_resolution: FontFamilyResolution,
    ) -> Self {
        let target = match painter {
            #[cfg(hydrolysis_gpu_painter)]
            crate::platform::OffscreenPainter::Gpu(gpu) => CaptureTarget::Gpu {
                context: Some(gpu.clone()),
                surface: Rc::new(RefCell::new(None)),
            },
            #[cfg(hydrolysis_cpu_painter)]
            crate::platform::OffscreenPainter::Cpu(state) => CaptureTarget::Cpu(Rc::clone(state)),
        };
        Self::on_target(target, theme, env, family_resolution)
    }

    const fn on_target(
        target: CaptureTarget,
        theme: Rc<dyn crate::theme::WidgetTheme>,
        env: Environment,
        family_resolution: FontFamilyResolution,
    ) -> Self {
        Self {
            target,
            theme,
            env,
            family_resolution,
        }
    }
}

/// Ensures `cell`'s offscreen surface exists at `width`×`height` — on
/// `context` when given, on a device of its own otherwise — acquires its
/// next frame and returns the GPU handles the capture renders through.
///
/// A host without a usable GPU or a device lost beyond recovery fails the
/// capture with [`RenderError::Gpu`].
#[cfg(hydrolysis_gpu_painter)]
#[expect(
    clippy::future_not_send,
    reason = "view rendering runs on the main thread; the future borrows non-Send GPU state"
)]
async fn acquire_frame(
    context: Option<&OffscreenGpuContext>,
    cell: &RefCell<Option<OffscreenSurface>>,
    width: u32,
    height: u32,
) -> Result<AcquiredFrame, RenderError> {
    if cell.borrow().is_none() {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let offscreen = match context {
            Some(gpu) => OffscreenSurface::on_context(gpu.clone(), width, height, format),
            None => OffscreenSurface::new(width, height, format)
                .await
                .map_err(render_failure)?,
        };
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
    let frame = surface.acquire().map_err(render_failure)?;
    Ok((frame, surface.generation().clone(), surface.format()))
}

/// An acquired capture frame, the device generation it renders through and
/// the frame's format.
#[cfg(hydrolysis_gpu_painter)]
type AcquiredFrame = (SurfaceFrame, GpuGeneration, wgpu::TextureFormat);

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

        let rgba_data = match &self.target {
            #[cfg(hydrolysis_gpu_painter)]
            CaptureTarget::Gpu { context, surface } => {
                let (frame, generation, format) =
                    acquire_frame(context.as_ref(), surface, width, height).await?;
                crate::engine::engine_await!(renderer.render_generation_to_texture(
                    &generation,
                    frame.texture(),
                    format,
                    (width, height),
                    cherenkov_record::WorkingColor::TRANSPARENT,
                ))
                .map_err(render_failure)?;
                renderer.frame_work_counters_mut().gpu_submissions += 1;

                let mut borrowed = surface.borrow_mut();
                let surface = borrowed
                    .as_mut()
                    .expect("hydrolysis view renderer surface lost its surface");
                let rgba_data = readback_texture_rgba8(&*surface, frame.texture(), width, height)
                    .map_err(render_failure)?;
                surface.present(frame);
                rgba_data
            }
            #[cfg(hydrolysis_cpu_painter)]
            CaptureTarget::Cpu(state) => renderer
                .render_raster_frame(
                    state,
                    crate::renderer::FrameGeometry {
                        display_scale: 1.0,
                        width,
                        height,
                        base_color: cherenkov_record::WorkingColor::TRANSPARENT,
                    },
                    // The GPU capture's convention: the texture is presented
                    // in straight alpha.
                    Some(cherenkov::OutputAlpha::Straight),
                )
                .map_err(render_failure)?
                .expect("a capture reads back the frame it renders"),
        };

        Ok(RenderResult::new(rgba_data, width, height))
    }
}

/// Wraps a Hydrolysis render failure as the [`RenderError`] callers receive.
fn render_failure(error: impl core::error::Error + Send + Sync + 'static) -> RenderError {
    RenderError::Gpu(Box::new(error))
}
