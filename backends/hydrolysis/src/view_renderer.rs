use std::cell::RefCell;
use std::rc::Rc;

use waterui_core::view_renderer::{CustomViewRenderer, RenderResult, RenderSize};
use waterui_core::{AnyView, Environment};
use waterui_graphics::SceneViewMergeToParent;

use crate::platform::{OffscreenSurface, SurfaceProvider};
use crate::readback::readback_texture_rgba8;
use crate::renderer::HydrolysisRenderer;

/// `ViewRenderer` implementation backed by Hydrolysis offscreen rendering.
pub struct HydrolysisViewRenderer {
    surface: Rc<RefCell<Option<OffscreenSurface>>>,
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
    pub fn new() -> Self {
        Self {
            surface: Rc::new(RefCell::new(None)),
            configure_environment: Rc::new(|_env| {}),
        }
    }

    #[must_use]
    pub fn with_environment(configure_environment: impl Fn(&mut Environment) + 'static) -> Self {
        Self {
            surface: Rc::new(RefCell::new(None)),
            configure_environment: Rc::new(configure_environment),
        }
    }
}

impl Default for HydrolysisViewRenderer {
    fn default() -> Self {
        Self::new()
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

            let mut surface = surface.borrow_mut();
            let surface = surface
                .as_mut()
                .expect("hydrolysis view renderer surface must initialize before rendering");
            surface.resize(width, height);
            let frame = surface
                .acquire()
                .expect("hydrolysis view renderer failed to acquire offscreen frame");

            let rgba_data = {
                let device = surface.device();
                let queue = surface.queue();
                let mut renderer = HydrolysisRenderer::new(surface.adapter(), device);
                renderer.set_frame_resources(surface.adapter(), device, queue);
                renderer.reset_scene();
                renderer.begin_rebuild_frame();

                let mut env = Environment::new().extending(SceneViewMergeToParent);
                configure_environment(&mut env);
                let view = crate::renderer::normalize_view_for_render(view, &env);
                let bounds = vello::kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
                renderer.capture_window_tree(
                    view,
                    &env,
                    bounds,
                    vello::kurbo::Affine::IDENTITY,
                    vello::kurbo::Affine::IDENTITY,
                );
                renderer.finish_rebuild_frame();
                renderer.render_scene_to_texture(crate::renderer::HydrolysisRenderTarget {
                    adapter: surface.adapter(),
                    device,
                    queue,
                    texture: Some(frame.texture()),
                    view: frame.view(),
                    format: surface.format(),
                    width,
                    height,
                    base_color: vello::peniko::Color::TRANSPARENT,
                });
                let rgba_data =
                    readback_texture_rgba8(device, queue, frame.texture(), width, height);
                renderer.clear_frame_resources();
                rgba_data
            };

            surface.present(frame);

            RenderResult {
                rgba_data,
                width,
                height,
            }
        }
    }
}
