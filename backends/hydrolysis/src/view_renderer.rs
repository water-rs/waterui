#[cfg(hydrolysis_cherenkov)]
use std::cell::RefCell;
#[cfg(hydrolysis_cherenkov)]
use std::rc::Rc;

#[cfg(hydrolysis_cherenkov)]
use waterui::ContentLedger;
use waterui::view_renderer::{RenderError, RenderRequest, RenderResult, ViewRenderer};
use waterui_core::{Environment, View};
#[cfg(hydrolysis_cherenkov)]
use waterui_graphics::backend::SceneViewMergeToParent;
#[cfg(hydrolysis_cherenkov)]
use waterui_text::FontCollection;

#[cfg(hydrolysis_gpu_painter)]
use crate::platform::{
    GpuGeneration, OffscreenGpuContext, OffscreenSurface, PresentationSurface as _, SurfaceFrame,
};
#[cfg(hydrolysis_gpu_painter)]
use crate::readback::readback_texture_rgba8;
#[cfg(hydrolysis_cherenkov)]
use crate::renderer::{FontFamilyResolution, HydrolysisRenderer};

/// What a capture renders with: the painter of the runtime that supplied
/// the renderer.
#[cfg(hydrolysis_cherenkov)]
enum CaptureTarget {
    /// The GPU painter's offscreen texture, created on the first capture —
    /// on the runtime's device when handed its context, on a device of its
    /// own otherwise.
    #[cfg(hydrolysis_gpu_painter)]
    Gpu {
        context: Option<OffscreenGpuContext>,
        surface: Rc<RefCell<Option<OffscreenSurface>>>,
    },
    /// The CPU painter, on the runtime's engine or opened lazily on first use.
    #[cfg(hydrolysis_cpu_painter)]
    Cpu(RefCell<Option<Rc<crate::engine::RasterEngineState>>>),
}

enum OffscreenState {
    #[cfg(hydrolysis_cherenkov)]
    Cherenkov {
        target: CaptureTarget,
        theme: Rc<dyn crate::theme::WidgetTheme>,
        family_resolution: FontFamilyResolution,
    },
    #[cfg(any(waterui_feature = "capture", not(hydrolysis_cherenkov)))]
    #[cfg_attr(not(waterui_feature = "capture"), allow(dead_code))]
    Unavailable(&'static str),
}

/// [`ViewRenderer`] implementation backed by Hydrolysis offscreen rendering.
///
/// A capture mounts the typed view under a fresh [`ContentLedger`] and keeps
/// the tree mounted while it awaits the content the view owes, re-flushing the
/// retained tree between passes, then renders it once without erasing `V`.
pub struct HydrolysisOffscreenRenderer {
    state: OffscreenState,
}

impl core::fmt::Debug for HydrolysisOffscreenRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydrolysisOffscreenRenderer")
            .finish_non_exhaustive()
    }
}

#[cfg(any(waterui_feature = "capture", not(hydrolysis_cherenkov)))]
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct OffscreenUnavailableError(&'static str);

impl HydrolysisOffscreenRenderer {
    /// Returns a renderer that fails captures with `reason` (used by
    /// `SemanticRuntime` and targets without Cherenkov offscreen support).
    #[cfg(waterui_feature = "capture")]
    #[must_use]
    pub(crate) const fn unavailable(reason: &'static str) -> Self {
        Self {
            state: OffscreenState::Unavailable(reason),
        }
    }

    /// Creates a renderer drawing with `theme` and `family_resolution`,
    /// lazily opening a GPU offscreen surface (or CPU engine in CPU-only
    /// builds) on first capture.
    #[cfg(hydrolysis_cherenkov)]
    #[must_use]
    pub fn new(
        theme: Rc<dyn crate::theme::WidgetTheme>,
        family_resolution: FontFamilyResolution,
    ) -> Self {
        #[cfg(hydrolysis_gpu_painter)]
        let target = CaptureTarget::Gpu {
            context: None,
            surface: Rc::new(RefCell::new(None)),
        };
        #[cfg(all(hydrolysis_cpu_painter, not(hydrolysis_gpu_painter)))]
        let target = CaptureTarget::Cpu(RefCell::new(None));
        Self::on_target(target, theme, family_resolution)
    }

    /// `Self::new` on the offscreen runtime's own `painter`: a capture
    /// renders on its device or CPU engine instead of opening another.
    #[cfg(all(not(target_arch = "wasm32"), hydrolysis_cherenkov))]
    #[must_use]
    pub fn on_painter(
        painter: &crate::platform::OffscreenPainter,
        theme: Rc<dyn crate::theme::WidgetTheme>,
        family_resolution: FontFamilyResolution,
    ) -> Self {
        let target = match painter {
            #[cfg(hydrolysis_gpu_painter)]
            crate::platform::OffscreenPainter::Gpu(gpu) => CaptureTarget::Gpu {
                context: Some(gpu.clone()),
                surface: Rc::new(RefCell::new(None)),
            },
            #[cfg(hydrolysis_cpu_painter)]
            crate::platform::OffscreenPainter::Cpu(state) => {
                CaptureTarget::Cpu(RefCell::new(Some(Rc::clone(state))))
            }
        };
        Self::on_target(target, theme, family_resolution)
    }

    #[cfg(hydrolysis_cherenkov)]
    const fn on_target(
        target: CaptureTarget,
        theme: Rc<dyn crate::theme::WidgetTheme>,
        family_resolution: FontFamilyResolution,
    ) -> Self {
        Self {
            state: OffscreenState::Cherenkov {
                target,
                theme,
                family_resolution,
            },
        }
    }

    #[cfg(all(waterui_feature = "capture", hydrolysis_cherenkov))]
    pub(crate) const fn set_family_resolution(&mut self, resolution: FontFamilyResolution) {
        if let OffscreenState::Cherenkov {
            family_resolution, ..
        } = &mut self.state
        {
            *family_resolution = resolution;
        }
    }

    /// Returns a fresh renderer sharing the underlying device/engine configuration
    /// without sharing the per-renderer surface borrow.
    #[cfg(all(waterui_feature = "capture", hydrolysis_cherenkov))]
    #[must_use]
    pub(crate) fn clone_for_dispatch(&self) -> Self {
        match &self.state {
            OffscreenState::Cherenkov {
                target,
                theme,
                family_resolution,
            } => {
                let target = match target {
                    #[cfg(hydrolysis_gpu_painter)]
                    CaptureTarget::Gpu { context, .. } => CaptureTarget::Gpu {
                        context: context.clone(),
                        surface: Rc::new(RefCell::new(None)),
                    },
                    #[cfg(hydrolysis_cpu_painter)]
                    CaptureTarget::Cpu(state) => {
                        CaptureTarget::Cpu(RefCell::new(state.borrow().clone()))
                    }
                };
                Self::on_target(target, Rc::clone(theme), *family_resolution)
            }
            OffscreenState::Unavailable(reason) => Self::unavailable(reason),
        }
    }

    /// Returns a fresh renderer sharing the underlying configuration.
    #[cfg(all(waterui_feature = "capture", not(hydrolysis_cherenkov)))]
    #[must_use]
    pub(crate) const fn clone_for_dispatch(&self) -> Self {
        let OffscreenState::Unavailable(reason) = self.state;
        Self::unavailable(reason)
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

#[cfg(hydrolysis_gpu_painter)]
#[expect(
    clippy::future_not_send,
    reason = "view rendering runs on the main thread; the future borrows non-Send GPU and renderer state"
)]
async fn rasterize_gpu(
    context: Option<&OffscreenGpuContext>,
    surface: &RefCell<Option<OffscreenSurface>>,
    renderer: &mut HydrolysisRenderer<crate::text::ParleyEngine>,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, RenderError> {
    let (frame, generation, format) = acquire_frame(context, surface, width, height).await?;
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
    Ok(rgba_data)
}

#[cfg(hydrolysis_cpu_painter)]
fn rasterize_cpu(
    state_cell: &RefCell<Option<Rc<crate::engine::RasterEngineState>>>,
    renderer: &mut HydrolysisRenderer<crate::text::ParleyEngine>,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, RenderError> {
    if state_cell.borrow().is_none() {
        let created = crate::engine::RasterEngineState::new().map_err(render_failure)?;
        *state_cell.borrow_mut() = Some(created);
    }
    let state = state_cell
        .borrow()
        .as_ref()
        .expect("cpu capture engine initialized")
        .clone();
    Ok(renderer
        .render_raster_frame(
            &state,
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
        .expect("a capture reads back the frame it renders"))
}

#[cfg(hydrolysis_cherenkov)]
impl ViewRenderer for HydrolysisOffscreenRenderer {
    #[expect(
        clippy::future_not_send,
        reason = "view rendering runs on the main thread; the future borrows non-Send GPU and Environment state"
    )]
    async fn render_to_rgba<V: View>(
        &self,
        view: V,
        env: &Environment,
        request: RenderRequest,
    ) -> Result<RenderResult, RenderError> {
        match &self.state {
            #[cfg(waterui_feature = "capture")]
            OffscreenState::Unavailable(reason) => Err(RenderError::Capture(Box::new(
                OffscreenUnavailableError(reason),
            ))),
            OffscreenState::Cherenkov {
                target,
                theme,
                family_resolution,
            } => {
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

                // The capture's text engine shapes against the collection the
                // caller's environment carries — `FontCollection::from_env`
                // names the host's obligation when the environment carries none.
                let mut renderer = HydrolysisRenderer::with_fonts(
                    Rc::clone(theme),
                    &FontCollection::from_env(env),
                    *family_resolution,
                );
                renderer.reset_scene();
                renderer.begin_rebuild_frame();

                let ledger = ContentLedger::new();
                let render_env = env
                    .extending(SceneViewMergeToParent)
                    .extending(ledger.clone());
                let bounds = kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
                renderer.capture_window_tree(
                    view,
                    &render_env,
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
                                &render_env,
                                bounds,
                                kurbo::Affine::IDENTITY,
                                kurbo::Affine::IDENTITY,
                            );
                            waterui::task::yield_now()
                        },
                    )
                    .await?;

                let rgba_data = match target {
                    #[cfg(hydrolysis_gpu_painter)]
                    CaptureTarget::Gpu { context, surface } => {
                        rasterize_gpu(context.as_ref(), surface, &mut renderer, width, height)
                            .await?
                    }
                    #[cfg(hydrolysis_cpu_painter)]
                    CaptureTarget::Cpu(state_cell) => {
                        rasterize_cpu(state_cell, &mut renderer, width, height)?
                    }
                };
                Ok(RenderResult::new(rgba_data, width, height))
            }
        }
    }
}

#[cfg(not(hydrolysis_cherenkov))]
impl ViewRenderer for HydrolysisOffscreenRenderer {
    fn render_to_rgba<V: View>(
        &self,
        _view: V,
        _env: &Environment,
        _request: RenderRequest,
    ) -> impl core::future::Future<Output = Result<RenderResult, RenderError>> {
        let OffscreenState::Unavailable(reason) = self.state;
        core::future::ready(Err(RenderError::Capture(Box::new(
            OffscreenUnavailableError(reason),
        ))))
    }
}

/// Wraps a Hydrolysis render failure as the [`RenderError`] callers receive.
#[cfg(hydrolysis_cherenkov)]
fn render_failure(error: impl core::error::Error + Send + Sync + 'static) -> RenderError {
    RenderError::Gpu(Box::new(error))
}
