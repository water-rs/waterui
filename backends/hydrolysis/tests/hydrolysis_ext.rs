//! End-to-end checks for the public `hydrolysis` API surface consumed
//! through the `waterui` facade: GPU content views, environments, and frames.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use waterui::Color;
use waterui::View;
use waterui::env::Environment;
use waterui::graphics::gpu::{Context as GpuContext, Frame as GpuFrame};
use waterui::graphics::{GpuContent, GpuContentView};
use waterui::prelude::zstack;
use waterui::shape::RoundedRectangle;
use waterui::{FilterViewExt, ViewExt};
use waterui_graphics::FilteredView;
use waterui_graphics::filtrate::{
    Effect, EffectContext, EffectInput, EffectOutput, EffectRenderResult, EffectSetupResult,
};
use waterui_testing::TestHost;

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

impl GpuContent for SolidClearRenderer {
    fn setup(&mut self, _gpu: &GpuContext<'_>) {}

    fn render(&mut self, frame: &mut GpuFrame<'_>) {
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("hydrolysis_ext_gpu_surface_test_encoder"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hydrolysis_ext_gpu_surface_test_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame.view,
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
    calls: Arc<AtomicU32>,
}

impl GpuContent for CountingClearRenderer {
    fn setup(&mut self, _gpu: &GpuContext<'_>) {}

    fn render(&mut self, frame: &mut GpuFrame<'_>) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("hydrolysis_ext_counting_gpu_surface_test_encoder"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hydrolysis_ext_counting_gpu_surface_test_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame.view,
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

/// A pass-through filter: copies its captured input texture to the output
/// unchanged. The point of the fixture is the capture path, not the filter.
#[derive(Debug, Clone, Copy)]
struct CopyTextureEffect;

impl Effect for CopyTextureEffect {
    fn setup(
        &mut self,
        _ctx: &EffectContext<'_>,
    ) -> impl std::future::Future<Output = EffectSetupResult> {
        std::future::ready(Ok(()))
    }

    fn encode_render(
        &mut self,
        input: &EffectInput<'_>,
        output: &EffectOutput<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult {
        encoder.copy_texture_to_texture(
            input.texture.as_image_copy(),
            output.texture.as_image_copy(),
            wgpu::Extent3d {
                width: input.width,
                height: input.height,
                depth_or_array_layers: 1,
            },
        );
        Ok(false)
    }
}

#[derive(Clone)]
struct GpuSurfaceOpacityView;

impl View for GpuSurfaceOpacityView {
    fn body(self, _env: &Environment) -> impl View {
        GpuContentView::new(SolidClearRenderer {
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
struct GpuSurfaceUnderOverlayView;

impl View for GpuSurfaceUnderOverlayView {
    fn body(self, _env: &Environment) -> impl View {
        zstack((
            GpuContentView::new(SolidClearRenderer {
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
    calls: Arc<AtomicU32>,
}

impl View for TransparentGpuSurfaceOpacityView {
    fn body(self, _env: &Environment) -> impl View {
        GpuContentView::new(CountingClearRenderer {
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
        GpuContentView::new(SolidClearRenderer {
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
struct GpuSurfaceFilteredView;

impl View for GpuSurfaceFilteredView {
    fn body(self, _env: &Environment) -> impl View {
        FilteredView::new(
            GpuContentView::new(SolidClearRenderer {
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
        GpuContentView::new(SolidClearRenderer {
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

/// An offscreen host at pixel size `width`x`height` under the Material3 style —
/// the render harness `render_offscreen` used to drive by hand.
fn host(width: u32, height: u32) -> TestHost {
    TestHost::new(
        Environment::new(),
        width,
        height,
        hydrolysis_m3::Material3::defaults(),
    )
}

fn center_pixel(rgba8: &[u8], width: u32, height: u32) -> &[u8] {
    let center = ((width as usize / 2) + (height as usize / 2) * width as usize) * 4;
    &rgba8[center..center + 4]
}

#[test]
fn hydrolysis_ext_renders_offscreen() {
    let host = host(400, 300);
    let output = host.render(CloneableRect);

    assert_eq!(output.width, 400);
    assert_eq!(output.height, 300);
    assert_eq!(output.rgba8.len(), 400 * 300 * 4);
    let pixel = center_pixel(&output.rgba8, output.width, output.height);
    assert_eq!(
        pixel,
        [37, 99, 235, 255],
        "expected the solid center pixel to match #2563EB"
    );
}

#[test]
fn hydrolysis_ext_renders_gpu_surface_inside_opacity_layer() {
    let host = host(96, 72);
    let output = host.render(GpuSurfaceOpacityView);

    let center =
        ((output.width as usize / 2) + (output.height as usize / 2) * output.width as usize) * 4;
    let alpha = output.rgba8[center + 3];
    assert!(
        alpha > 90 && alpha < 180,
        "expected partially transparent output alpha, got {alpha}"
    );
}

#[test]
fn hydrolysis_ext_preserves_gpu_surface_under_overlay() {
    let host = host(96, 72);
    let output = host.render(GpuSurfaceUnderOverlayView);

    // The probe clears (0.9, 0.3, 0.2) in linear Display P3; presented back to
    // sRGB that is [255, 143, 117] — the fill survives, gamut-mapped.
    assert_pixel_close(
        &output.rgba8[..4],
        [255, 143, 117, 255],
        "a later transparent overlay layer must preserve the underlying GPU surface",
    );
}

#[test]
fn hydrolysis_ext_skips_transparent_gpu_surface_inside_opacity_layer() {
    let calls = Arc::new(AtomicU32::new(0));
    let host = host(96, 72);
    let output = host.render(TransparentGpuSurfaceOpacityView {
        calls: Arc::clone(&calls),
    });

    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "transparent opacity layer should not render hidden GpuContentView content"
    );
    let center =
        ((output.width as usize / 2) + (output.height as usize / 2) * output.width as usize) * 4;
    let alpha = output.rgba8[center + 3];
    assert_eq!(alpha, 0, "transparent output should keep alpha at zero");
}

#[test]
fn hydrolysis_ext_renders_gpu_surface_inside_clip_shape() {
    let host = host(96, 72);
    let output = host.render(GpuSurfaceClipView);

    let center_alpha = center_pixel(&output.rgba8, output.width, output.height)[3];
    let corner_alpha = output.rgba8[3];
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
fn hydrolysis_ext_captures_gpu_surface_inside_filtered_view() {
    let host = host(96, 72);
    let output = host.render(GpuSurfaceFilteredView);

    let center_alpha = center_pixel(&output.rgba8, output.width, output.height)[3];
    assert!(
        center_alpha > 200,
        "FilteredView must capture its nested GpuContentView"
    );
}

#[test]
fn hydrolysis_ext_captures_gpu_surface_inside_applied_filter() {
    let host = host(96, 72);
    let output = host.render(GpuSurfaceAppliedFilterView);

    let center_alpha = center_pixel(&output.rgba8, output.width, output.height)[3];
    assert!(
        center_alpha > 200,
        "AppliedFilter must capture its nested GpuContentView"
    );
}

/// Offscreen rendering at 2x must allocate twice the pixels without touching
/// the logical layout, so previews are sharp on `HiDPI` displays.
#[test]
fn offscreen_window_scale_factor_scales_the_surface_only() {
    use hydrolysis::PlatformWindow as _;

    let window =
        hydrolysis::OffscreenWindow::new_for_tests(320, 200, wgpu::TextureFormat::Rgba8Unorm);
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
