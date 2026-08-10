use std::cell::Cell;
use std::rc::Rc;

use hydrolysis::HydrolysisExt;
use waterui::Color;
use waterui::View;
use waterui::ViewExt;
use waterui::env::Environment;
use waterui::prelude::zstack;
use waterui::shape::RoundedRectangle;
use waterui_graphics::{
    EffectRenderer, FilterViewExt as _, GpuContext, GpuFrame, GpuRuntime, GpuSurface, GpuView,
    OffscreenRenderConfig, OffscreenSize, ViewEffect, ViewEffectContext, ViewEffectInput,
    ViewEffectOutput,
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
                multiview_mask: None,
            });
        }
        frame.queue.submit([encoder.finish()]);
    }
}

#[derive(Debug)]
struct CountingClearRenderer {
    color: wgpu::Color,
    calls: Rc<Cell<u32>>,
}

impl GpuView for CountingClearRenderer {
    async fn setup(&mut self, _ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {}

    fn render(&mut self, frame: &mut GpuFrame) {
        self.calls.set(
            self.calls
                .get()
                .checked_add(1)
                .expect("counting clear renderer call count overflow"),
        );
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("hydrolysis_ext_counting_gpu_surface_test_encoder"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hydrolysis_ext_counting_gpu_surface_test_pass"),
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

#[derive(Debug, Clone, Copy)]
struct CopyTextureEffect;

impl EffectRenderer for CopyTextureEffect {
    async fn setup(&mut self, _ctx: &ViewEffectContext<'_>) {}

    fn render(&mut self, input: &ViewEffectInput<'_>, output: &ViewEffectOutput<'_>) {
        let mut encoder = input
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("hydrolysis_ext_copy_view_effect_encoder"),
            });
        encoder.copy_texture_to_texture(
            input.texture.as_image_copy(),
            output.texture.as_image_copy(),
            wgpu::Extent3d {
                width: input.width,
                height: input.height,
                depth_or_array_layers: 1,
            },
        );
        input.queue.submit([encoder.finish()]);
    }
}

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
struct GpuSurfaceUnderVelloOverlayView;

impl View for GpuSurfaceUnderVelloOverlayView {
    fn body(self, _env: &Environment) -> impl View {
        zstack((
            GpuSurface::new(SolidClearRenderer {
                color: wgpu::Color {
                    r: 0.9,
                    g: 0.3,
                    b: 0.2,
                    a: 1.0,
                },
            }),
            Color::srgb_hex("#2563EB").size(12.0, 12.0),
        ))
    }
}

#[derive(Clone)]
struct TransparentGpuSurfaceOpacityView {
    calls: Rc<Cell<u32>>,
}

impl View for TransparentGpuSurfaceOpacityView {
    fn body(self, _env: &Environment) -> impl View {
        GpuSurface::new(CountingClearRenderer {
            color: wgpu::Color {
                r: 0.9,
                g: 0.3,
                b: 0.2,
                a: 1.0,
            },
            calls: self.calls,
        })
        .opacity(0.0)
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

#[derive(Clone)]
struct GpuSurfaceViewEffectView;

impl View for GpuSurfaceViewEffectView {
    fn body(self, _env: &Environment) -> impl View {
        ViewEffect::new(
            GpuSurface::new(SolidClearRenderer {
                color: wgpu::Color {
                    r: 0.35,
                    g: 0.55,
                    b: 0.95,
                    a: 1.0,
                },
            }),
            CopyTextureEffect,
        )
    }
}

#[derive(Clone)]
struct GpuSurfaceAppliedFilterView;

impl View for GpuSurfaceAppliedFilterView {
    fn body(self, _env: &Environment) -> impl View {
        GpuSurface::new(SolidClearRenderer {
            color: wgpu::Color {
                r: 0.85,
                g: 0.45,
                b: 0.15,
                a: 1.0,
            },
        })
        .brightness(0.0)
    }
}

/// Compares a blended pixel, allowing the one-unit difference that compositing
/// produces across GPU implementations.
///
/// Solid fills are compared exactly; only blended output needs this. CI
/// rasterizes in software while development machines use a hardware GPU, and
/// the two round the same blend differently.
#[track_caller]
fn assert_pixel_close(actual: &[u8], expected: [u8; 4], message: &str) {
    const TOLERANCE: i16 = 1;
    let close = actual
        .iter()
        .zip(expected)
        .all(|(&got, want)| i16::from(got).abs_diff(i16::from(want)) <= TOLERANCE.unsigned_abs());
    assert!(
        close,
        "{message}\n  actual:   {actual:?}\n  expected: {expected:?} (tolerance {TOLERANCE})"
    );
}

fn gpu_runtime() -> GpuRuntime {
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    waterui_testing::install_test_executor();
    pollster::block_on(GpuRuntime::new())
        .expect("hydrolysis extension tests require a high-performance GPU")
}

#[test]
fn hydrolysis_ext_renders_offscreen() {
    let mut env = Environment::new();
    let view = CloneableRect.hydrolysis();

    let runtime = gpu_runtime();
    let output = pollster::block_on(view.render_offscreen(
        &runtime,
        OffscreenRenderConfig::new(
            OffscreenSize::try_from_pixels(400, 300).expect("static size must be valid"),
        ),
        &mut env,
    ))
    .expect("hydrolysis extension offscreen render failed");

    assert_eq!(output.width, 400);
    assert_eq!(output.height, 300);
    assert_eq!(output.rgba8.len(), 400 * 300 * 4);
    let center =
        ((output.width as usize / 2) + (output.height as usize / 2) * output.width as usize) * 4;
    let pixel = &output.rgba8[center..center + 4];
    assert_eq!(
        pixel,
        [37, 99, 235, 255],
        "expected the solid center pixel to match #2563EB"
    );
}

#[test]
fn hydrolysis_ext_renders_gpu_surface_inside_opacity_layer() {
    let mut env = Environment::new();
    let view = GpuSurfaceOpacityView.hydrolysis();

    let runtime = gpu_runtime();
    let output = pollster::block_on(view.render_offscreen(
        &runtime,
        OffscreenRenderConfig::new(
            OffscreenSize::try_from_pixels(96, 72).expect("static size must be valid"),
        ),
        &mut env,
    ))
    .expect("hydrolysis extension offscreen render failed");

    let center =
        ((output.width as usize / 2) + (output.height as usize / 2) * output.width as usize) * 4;
    let alpha = output.rgba8[center + 3];
    assert!(
        alpha > 90 && alpha < 180,
        "expected partially transparent output alpha, got {alpha}"
    );
}

#[test]
fn hydrolysis_ext_preserves_gpu_surface_under_vello_overlay() {
    let mut env = Environment::new();
    let view = GpuSurfaceUnderVelloOverlayView.hydrolysis();

    let runtime = gpu_runtime();
    let output = pollster::block_on(view.render_offscreen(
        &runtime,
        OffscreenRenderConfig::new(
            OffscreenSize::try_from_pixels(96, 72).expect("static size must be valid"),
        ),
        &mut env,
    ))
    .expect("hydrolysis extension offscreen render failed");

    assert_pixel_close(
        &output.rgba8[..4],
        [243, 149, 124, 255],
        "a later transparent Vello layer must preserve the underlying GPU surface",
    );
}

#[test]
fn hydrolysis_ext_skips_transparent_gpu_surface_inside_opacity_layer() {
    let mut env = Environment::new();
    let calls = Rc::new(Cell::new(0));
    let view = TransparentGpuSurfaceOpacityView {
        calls: Rc::clone(&calls),
    }
    .hydrolysis();

    let runtime = gpu_runtime();
    let output = pollster::block_on(view.render_offscreen(
        &runtime,
        OffscreenRenderConfig::new(
            OffscreenSize::try_from_pixels(96, 72).expect("static size must be valid"),
        ),
        &mut env,
    ))
    .expect("hydrolysis extension offscreen render failed");

    assert_eq!(
        calls.get(),
        0,
        "transparent opacity layer should not render hidden GpuSurface content"
    );
    let center =
        ((output.width as usize / 2) + (output.height as usize / 2) * output.width as usize) * 4;
    let alpha = output.rgba8[center + 3];
    assert_eq!(alpha, 0, "transparent output should keep alpha at zero");
}

#[test]
fn hydrolysis_ext_renders_gpu_surface_inside_clip_shape() {
    let mut env = Environment::new();
    let view = GpuSurfaceClipView.hydrolysis();

    let runtime = gpu_runtime();
    let output = pollster::block_on(view.render_offscreen(
        &runtime,
        OffscreenRenderConfig::new(
            OffscreenSize::try_from_pixels(96, 72).expect("static size must be valid"),
        ),
        &mut env,
    ))
    .expect("hydrolysis extension offscreen render failed");

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

#[test]
fn hydrolysis_ext_captures_gpu_surface_inside_view_effect() {
    let mut env = Environment::new();
    let view = GpuSurfaceViewEffectView.hydrolysis();

    let runtime = gpu_runtime();
    let output = pollster::block_on(view.render_offscreen(
        &runtime,
        OffscreenRenderConfig::new(
            OffscreenSize::try_from_pixels(96, 72).expect("static size must be valid"),
        ),
        &mut env,
    ))
    .expect("hydrolysis extension offscreen render failed");

    let center =
        ((output.width as usize / 2) + (output.height as usize / 2) * output.width as usize) * 4;
    assert!(
        output.rgba8[center + 3] > 200,
        "ViewEffect must capture its nested GpuSurface"
    );
}

#[test]
fn hydrolysis_ext_captures_gpu_surface_inside_applied_filter() {
    let mut env = Environment::new();
    let view = GpuSurfaceAppliedFilterView.hydrolysis();

    let runtime = gpu_runtime();
    let output = pollster::block_on(view.render_offscreen(
        &runtime,
        OffscreenRenderConfig::new(
            OffscreenSize::try_from_pixels(96, 72).expect("static size must be valid"),
        ),
        &mut env,
    ))
    .expect("hydrolysis extension offscreen render failed");

    let center =
        ((output.width as usize / 2) + (output.height as usize / 2) * output.width as usize) * 4;
    assert!(
        output.rgba8[center + 3] > 200,
        "AppliedFilter must capture its nested GpuSurface"
    );
}

/// Offscreen rendering at 2x must allocate twice the pixels without touching
/// the logical layout, so previews are sharp on HiDPI displays.
#[test]
fn offscreen_window_scale_factor_scales_the_surface_only() {
    use hydrolysis::{PlatformWindow as _, SurfaceProvider as _};

    let mut window = hydrolysis::OffscreenWindow::new_for_tests(
        320,
        200,
        wgpu::TextureFormat::Rgba8Unorm,
    );
    assert_eq!(window.surface_ref().size(), (320, 200));
    assert!((window.scale_factor() - 1.0).abs() < f64::EPSILON);

    let window = window.with_scale_factor(2.0);
    assert_eq!(
        window.surface_ref().size(),
        (640, 400),
        "a 2x window allocates twice the physical pixels"
    );
    assert!((window.scale_factor() - 2.0).abs() < f64::EPSILON);
}
