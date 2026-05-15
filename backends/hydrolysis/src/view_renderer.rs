use core::future::Future;
use core::pin::Pin;
use std::boxed::Box;
use std::cell::RefCell;
use std::rc::Rc;

use waterui_core::view_renderer::{CustomViewRenderer, RenderResult, RenderSize};
use waterui_core::{AnyView, Environment};
use waterui_graphics::SceneViewMergeToParent;

use crate::platform::{OffscreenSurface, SurfaceProvider};
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
    fn render_to_rgba(
        &self,
        view: AnyView,
        size: RenderSize,
    ) -> Pin<Box<dyn Future<Output = RenderResult> + 'static>> {
        let surface = Rc::clone(&self.surface);
        let configure_environment = Rc::clone(&self.configure_environment);
        Box::pin(async move {
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
                let mut renderer = HydrolysisRenderer::new(device);
                renderer.set_frame_resources(device, queue);
                renderer.reset_scene();
                renderer.begin_rebuild_frame();

                let mut env = Environment::new().extending(SceneViewMergeToParent);
                configure_environment(&mut env);
                let view = crate::renderer::normalize_view_for_render(view, &env);
                let bounds = vello::kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
                renderer.dispatch(view, &env, bounds);
                renderer.finish_rebuild_frame();
                renderer.render_scene_to_texture(crate::renderer::HydrolysisRenderTarget {
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
        })
    }
}

fn readback_texture_rgba8(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    const BYTES_PER_PIXEL: u32 = 4;
    const COPY_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let unpadded_bytes_per_row = width * BYTES_PER_PIXEL;
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(COPY_ALIGNMENT) * COPY_ALIGNMENT;

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hydrolysis_view_renderer_readback"),
        size: u64::from(padded_bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("hydrolysis_view_renderer_readback_encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        sender
            .send(result)
            .expect("hydrolysis view renderer readback callback receiver dropped");
    });

    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    receiver
        .recv()
        .expect("hydrolysis view renderer readback callback dropped")
        .expect("hydrolysis view renderer failed to map readback buffer");

    let mapped = slice.get_mapped_range();
    let mut pixels = vec![0u8; (width * height * BYTES_PER_PIXEL) as usize];
    for row in 0..height as usize {
        let source_start = row * padded_bytes_per_row as usize;
        let source_end = source_start + unpadded_bytes_per_row as usize;
        let destination_start = row * unpadded_bytes_per_row as usize;
        let destination_end = destination_start + unpadded_bytes_per_row as usize;
        pixels[destination_start..destination_end]
            .copy_from_slice(&mapped[source_start..source_end]);
    }
    drop(mapped);
    readback.unmap();
    pixels
}
