use std::path::Path;

use hydrolysis::{HydrolysisRenderer, OffscreenWindow, PlatformWindow};
use waterui::graphics::SceneViewMergeToParent;
use waterui_core::{AnyView, Environment, View};

use crate::artifacts::{CapturedSnapshot, TestArtifacts};

/// RGBA8 frame captured from a headless hydrolysis render pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Pixel data in RGBA8 row-major order.
    pub rgba8: Vec<u8>,
}

impl Snapshot {
    /// Saves the snapshot to a PNG file.
    ///
    /// # Errors
    ///
    /// Returns image I/O or encoding errors from the underlying PNG writer.
    ///
    /// # Panics
    ///
    /// Panics if the stored RGBA buffer length does not match the snapshot dimensions.
    pub fn save_png(&self, path: impl AsRef<Path>) -> image::ImageResult<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(image::ImageError::IoError)?;
        }
        let image = image::RgbaImage::from_raw(self.width, self.height, self.rgba8.clone())
            .expect("Snapshot::save_png: rgba buffer shape must match dimensions");
        image.save(path)
    }
}

/// Headless host that renders `WaterUI` views into an offscreen texture.
#[derive(Debug)]
pub struct TestHost {
    env: Environment,
    width: u32,
    height: u32,
}

impl TestHost {
    /// Creates a test host with a fixed render size.
    #[must_use]
    pub const fn new(env: Environment, width: u32, height: u32) -> Self {
        Self { env, width, height }
    }

    /// Renders a view and returns the captured RGBA8 snapshot.
    ///
    /// # Panics
    ///
    /// Panics if the offscreen Hydrolysis surface cannot acquire a frame.
    pub fn render<V: View>(&self, view: V) -> Snapshot {
        let mut platform = OffscreenWindow::new_for_tests(
            self.width.max(1),
            self.height.max(1),
            wgpu::TextureFormat::Rgba8Unorm,
        );
        let mut renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };
        let bounds = vello::kurbo::Rect::new(
            0.0,
            0.0,
            f64::from(self.width.max(1)),
            f64::from(self.height.max(1)),
        );

        let surface = platform.surface();
        renderer.set_frame_resources(surface.adapter(), surface.device(), surface.queue());
        renderer.reset_scene();
        renderer.begin_rebuild_frame();
        let env = self.env.clone().extending(SceneViewMergeToParent);
        renderer.capture_window_tree(
            AnyView::new(view),
            &env,
            bounds,
            vello::kurbo::Affine::IDENTITY,
            vello::kurbo::Affine::IDENTITY,
        );
        renderer.finish_rebuild_frame();

        let frame = surface
            .acquire()
            .expect("waterui-testing failed to acquire offscreen frame");
        renderer.render_scene_to_texture(hydrolysis::HydrolysisRenderTarget {
            adapter: surface.adapter(),
            device: surface.device(),
            queue: surface.queue(),
            texture: Some(frame.texture()),
            view: frame.view(),
            format: surface.format(),
            width: self.width.max(1),
            height: self.height.max(1),
            base_color: vello::peniko::Color::TRANSPARENT,
        });
        let rgba8 = readback_texture_rgba8(
            surface.device(),
            surface.queue(),
            frame.texture(),
            self.width.max(1),
            self.height.max(1),
        );
        renderer.clear_frame_resources();
        surface.present(frame);

        Snapshot {
            width: self.width.max(1),
            height: self.height.max(1),
            rgba8,
        }
    }

    /// Renders a view, stores the PNG in the canonical artifact layout, and returns both.
    pub fn capture_snapshot_with<V: View>(
        &self,
        view: V,
        artifacts: &TestArtifacts,
        case: impl AsRef<str>,
        stage: impl AsRef<str>,
    ) -> CapturedSnapshot {
        artifacts.capture_snapshot(case, stage, self.render(view))
    }
    /// Creates a canonical artifact helper rooted at the provided suite.
    #[must_use]
    pub fn artifacts(&self, suite: impl AsRef<str>) -> TestArtifacts {
        TestArtifacts::new(suite.as_ref())
    }

    /// Renders a view and stores the resulting snapshot in `WaterUI`'s canonical artifact layout.
    pub fn capture_snapshot<V: View>(
        &self,
        view: V,
        suite: impl AsRef<str>,
        case: impl AsRef<str>,
        stage: impl AsRef<str>,
    ) -> CapturedSnapshot {
        let artifacts = self.artifacts(suite);
        artifacts.capture_snapshot(case, stage, self.render(view))
    }
}

pub fn readback_texture_rgba8(
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
        label: Some("waterui-testing-readback"),
        size: u64::from(padded_bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("waterui-testing-readback-encoder"),
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
            .expect("waterui-testing readback channel receiver dropped");
    });

    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    receiver
        .recv()
        .expect("waterui-testing readback callback dropped")
        .expect("waterui-testing failed to map readback buffer");

    let mapped = slice.get_mapped_range();
    let mut pixels = vec![0_u8; (width * height * BYTES_PER_PIXEL) as usize];
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
