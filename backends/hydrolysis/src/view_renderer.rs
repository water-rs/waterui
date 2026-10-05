use std::cell::RefCell;
use std::rc::Rc;

use waterui_core::view_renderer::{CustomViewRenderer, RenderResult, RenderSize};
use waterui_core::{AnyView, Environment};
use waterui_graphics::scene_view::SceneViewMergeToParent;

use crate::platform::{OffscreenSurface, SurfaceProvider};
use crate::readback::readback_texture_rgba8;
use crate::renderer::{FontFamilyResolution, HydrolysisRenderer};

/// `ViewRenderer` implementation backed by Hydrolysis offscreen rendering.
pub struct HydrolysisViewRenderer {
    surface: Rc<RefCell<Option<OffscreenSurface>>>,
    theme: Rc<dyn crate::engine::WidgetTheme>,
    configure_environment: Rc<dyn Fn(&mut Environment)>,
}

impl core::fmt::Debug for HydrolysisViewRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydrolysisViewRenderer")
            .finish_non_exhaustive()
    }
}

impl HydrolysisViewRenderer {
    #[must_use]
    /// Creates a renderer drawing with `theme`.
    pub fn new(theme: Rc<dyn crate::engine::WidgetTheme>) -> Self {
        Self {
            surface: Rc::new(RefCell::new(None)),
            theme,
            configure_environment: Rc::new(|_env| {}),
        }
    }

    #[must_use]
    /// Creates a renderer whose environment is first configured by `configure_environment`.
    pub fn with_environment(
        theme: Rc<dyn crate::engine::WidgetTheme>,
        configure_environment: impl Fn(&mut Environment) + 'static,
    ) -> Self {
        Self {
            surface: Rc::new(RefCell::new(None)),
            theme,
            configure_environment: Rc::new(configure_environment),
        }
    }
}

impl CustomViewRenderer for HydrolysisViewRenderer {
    #[expect(
        clippy::future_not_send,
        reason = "view rendering runs on the main thread; the future borrows non-Send GPU and Environment state"
    )]
    async fn render_to_rgba(&self, view: AnyView, size: RenderSize) -> RenderResult {
        let surface = Rc::clone(&self.surface);
        let configure_environment = Rc::clone(&self.configure_environment);
        {
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let width = size.width.max(1.0).round() as u32;
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let height = size.height.max(1.0).round() as u32;

            if surface.borrow().is_none() {
                let offscreen =
                    OffscreenSurface::new(width, height, wgpu::TextureFormat::Rgba8Unorm).await;
                *surface.borrow_mut() = Some(offscreen);
            }

            // The surface borrow ends before the render future is awaited:
            // a suspended frame cannot hold a `RefCell` borrow another task
            // might need while the browser device is awaited.
            let (frame, adapter, device, queue, device_loss, format) = {
                let mut borrowed = surface.borrow_mut();
                let surface = borrowed
                    .as_mut()
                    .expect("hydrolysis view renderer surface must initialize before rendering");
                surface.resize(width, height);
                let frame = surface
                    .acquire()
                    .expect("hydrolysis view renderer failed to acquire offscreen frame");
                (
                    frame,
                    surface.adapter().clone(),
                    surface.device().clone(),
                    surface.queue().clone(),
                    surface.device_loss().clone(),
                    surface.format(),
                )
            };

            let rgba_data = {
                let device = &device;
                let queue = &queue;
                let device_loss = device_loss;
                let mut renderer =
                    HydrolysisRenderer::new(Rc::clone(&self.theme), FontFamilyResolution::Lenient);
                renderer.reset_scene();
                renderer.begin_rebuild_frame();

                let mut env = Environment::new().extending(SceneViewMergeToParent);
                configure_environment(&mut env);
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
                crate::engine::engine_await!(renderer.render_scene_to_texture(
                    crate::renderer::HydrolysisRenderTarget {
                        adapter: &adapter,
                        device,
                        queue,
                        device_loss,
                        texture: Some(frame.texture()),
                        format,
                        width,
                        height,
                        base_color: waterui_graphics::draw::WorkingColor::TRANSPARENT,
                    }
                ));
                renderer.frame_work_counters_mut().gpu_submissions += 1;
                readback_texture_rgba8(device, queue, frame.texture(), width, height)
            };

            surface
                .borrow_mut()
                .as_mut()
                .expect("hydrolysis view renderer surface lost its surface")
                .present(frame);

            RenderResult {
                rgba_data,
                width,
                height,
            }
        }
    }
}
