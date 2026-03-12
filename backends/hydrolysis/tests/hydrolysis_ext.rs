use hydrolysis::HydrolysisExt;
use waterui::Color;
use waterui::View;
use waterui::ViewExt;
use waterui::env::Environment;
use waterui::shape::RoundedRectangle;
use waterui_graphics::{
    GpuContext, GpuFrame, GpuSurface, GpuView, OffscreenRenderConfig, OffscreenSize, wgpu,
};

#[derive(Clone)]
struct CloneableRect;

impl View for CloneableRect {
    fn body(self, _env: &Environment) -> impl View {
        Color::srgb_hex("#2563EB")
    }
}

#[derive(Debug, Clone, Copy)]
struct SolidClearRenderer {
    color: wgpu::Color,
}

impl GpuView for SolidClearRenderer {
    async fn setup(&mut self, _ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {}

    fn render(&mut self, frame: &mut GpuFrame) {
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("hydrolysis_ext_gpu_surface_test_encoder"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hydrolysis_ext_gpu_surface_test_pass"),
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
            });
        }
        frame.queue.submit([encoder.finish()]);
    }
}

waterui_graphics::impl_gpu_subview!(SolidClearRenderer);

#[derive(Clone)]
struct GpuSurfaceOpacityView;

impl View for GpuSurfaceOpacityView {
    fn body(self, _env: &Environment) -> impl View {
        GpuSurface::new(SolidClearRenderer {
            color: wgpu::Color {
                r: 0.9,
                g: 0.3,
                b: 0.2,
                a: 1.0,
            },
        })
        .opacity(0.5)
    }
}

#[derive(Clone)]
struct GpuSurfaceClipView;

impl View for GpuSurfaceClipView {
    fn body(self, _env: &Environment) -> impl View {
        GpuSurface::new(SolidClearRenderer {
            color: wgpu::Color {
                r: 0.2,
                g: 0.8,
                b: 0.4,
                a: 1.0,
            },
        })
        .clip(RoundedRectangle::new(0.25))
    }
}

#[test]
fn hydrolysis_ext_renders_offscreen() {
    let mut env = Environment::new();
    let view = CloneableRect.hydrolysis();

    let output = view
        .render_offscreen(
            OffscreenRenderConfig::new(
                OffscreenSize::try_from_pixels(400, 300).expect("static size must be valid"),
            ),
            &mut env,
        )
        .expect("hydrolysis extension offscreen render failed");

    assert_eq!(output.width, 400);
    assert_eq!(output.height, 300);
    assert_eq!(output.rgba8.len(), 400 * 300 * 4);

    let opaque_pixels = output.rgba8.chunks_exact(4).filter(|px| px[3] > 0).count();
    assert!(opaque_pixels > 4096, "rendered surface appears empty");
}

#[test]
fn hydrolysis_ext_renders_gpu_surface_inside_opacity_layer() {
    let mut env = Environment::new();
    let view = GpuSurfaceOpacityView.hydrolysis();

    let output = view
        .render_offscreen(
            OffscreenRenderConfig::new(
                OffscreenSize::try_from_pixels(96, 72).expect("static size must be valid"),
            ),
            &mut env,
        )
        .expect("hydrolysis extension opacity-wrapped gpu surface render failed");

    let center =
        ((output.width as usize / 2) + (output.height as usize / 2) * output.width as usize) * 4;
    let alpha = output.rgba8[center + 3];
    assert!(
        alpha > 90 && alpha < 180,
        "expected partially transparent output alpha, got {alpha}"
    );
}

#[test]
fn hydrolysis_ext_renders_gpu_surface_inside_clip_shape() {
    let mut env = Environment::new();
    let view = GpuSurfaceClipView.hydrolysis();

    let output = view
        .render_offscreen(
            OffscreenRenderConfig::new(
                OffscreenSize::try_from_pixels(96, 72).expect("static size must be valid"),
            ),
            &mut env,
        )
        .expect("hydrolysis extension clip-wrapped gpu surface render failed");

    let center =
        ((output.width as usize / 2) + (output.height as usize / 2) * output.width as usize) * 4;
    let center_alpha = output.rgba8[center + 3];
    let corner = 0usize;
    let corner_alpha = output.rgba8[corner + 3];
    assert!(
        center_alpha > 200,
        "center should remain visible, got alpha={center_alpha}"
    );
    assert!(
        corner_alpha < 40,
        "corner should be clipped, got alpha={corner_alpha}"
    );
}
