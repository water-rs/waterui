//! Offscreen rendering regression test for `GpuSurface`.

use waterui_graphics::{
    GpuContext, GpuFrame, GpuRuntime, GpuSurface, GpuView, OffscreenRenderConfig, OffscreenSize,
};

const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

#[derive(Debug, Clone, Copy)]
struct SolidClearRenderer {
    color: wgpu::Color,
}

impl GpuView for SolidClearRenderer {
    #[expect(
        clippy::future_not_send,
        reason = "GpuView setup runs on the UI-local GPU executor with a non-Send Environment"
    )]
    async fn setup(&mut self, _ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {}

    fn render(&mut self, frame: &mut GpuFrame) {
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("offscreen_test_encoder"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("offscreen_test_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        frame.queue.submit([encoder.finish()]);
    }
}

#[test]
fn render_offscreen_returns_expected_rgba_and_png() {
    let size = OffscreenSize::try_from_pixels(16, 12).expect("test size must be valid");
    let config = OffscreenRenderConfig::new(size).format(wgpu::TextureFormat::Rgba8Unorm);
    let renderer = SolidClearRenderer {
        color: wgpu::Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        },
    };
    let runtime = pollster::block_on(GpuRuntime::new())
        .expect("offscreen regression test requires a working GPU runtime");
    let mut env = waterui_core::Environment::new();

    let output =
        pollster::block_on(GpuSurface::new(renderer).render_offscreen(&runtime, config, &mut env))
            .expect("offscreen render should succeed");

    assert_eq!(output.width, 16);
    assert_eq!(output.height, 12);
    assert_eq!(output.rgba8.len(), 16 * 12 * 4);
    for px in output.rgba8.chunks_exact(4) {
        assert_eq!(px, [255, 0, 0, 255], "pixel should match clear color");
    }

    let png = output.to_png().expect("png encoding should succeed");
    assert!(png.len() >= PNG_SIGNATURE.len());
    assert_eq!(&png[..PNG_SIGNATURE.len()], &PNG_SIGNATURE);
}
