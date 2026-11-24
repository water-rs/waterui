//! `ShaderPaintView` rendered as a `Paint::Shader` fill on both Cherenkov
//! painters, and verified pixel-equivalent to its `GpuContent` realization.

use core::time::Duration;
use std::time::Instant;

use waterui::{Binding, ViewExt as _};
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::{FlowingGradient, ShaderPaintView};

use super::{MinimalTestTheme, pumped_test_environment};
use crate::{HeadlessRuntime, OffscreenPainter};

#[cfg(all(hydrolysis_gpu_painter, hydrolysis_cpu_painter))]
const RAMP_SHADER: &str = r"
@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let left = params[0].rgb;
    let right = params[1].rgb;
    let aspect = clamp(uniforms.resolution.x / max(uniforms.resolution.y, 1.0) * 0.25, 0.0, 1.0);
    let color = cherenkov_linear_srgb(mix(left, right, uv.x) * vec3<f32>(1.0, aspect, 1.0));
    return vec4<f32>(color, 1.0);
}
";

const TIME_SHADER: &str = r"
@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let phase = clamp(uniforms.time, 0.0, 1.0);
    let color = cherenkov_linear_srgb(vec3<f32>(phase, uv.y, params[0].x));
    return vec4<f32>(color, 1.0);
}
";

fn capture_at(runtime: &mut HeadlessRuntime, at: Instant) -> Vec<u8> {
    runtime
        .pump_at(true, at)
        .into_snapshot()
        .expect("headless snapshot must be captured")
        .rgba8()
        .to_vec()
}

fn render_view_pixels(painter: OffscreenPainter, view: impl Fn() -> AnyView + 'static) -> Vec<u8> {
    let mut runtime = HeadlessRuntime::new_for_tests_on_context(
        painter,
        pumped_test_environment(),
        AnyViewBuilder::<AnyView>::new(view),
        64,
        32,
        MinimalTestTheme::default(),
    );
    let start = Instant::now();
    for step in 0..4 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(step * 16));
    }
    capture_at(&mut runtime, start + Duration::from_millis(64))
}

fn assert_shader_paint_and_flowing_gradient_render_and_react(
    open_painter: fn() -> OffscreenPainter,
) {
    let uniforms = Binding::container(vec![0.1_f32, 0.0, 0.0, 0.0]);
    let continuous = Binding::container(false);
    let painter = open_painter();
    let uniforms_for_view = uniforms.clone();
    let continuous_for_view = continuous.clone();
    let mut runtime = HeadlessRuntime::new_for_tests_on_context(
        painter,
        pumped_test_environment(),
        AnyViewBuilder::<AnyView>::new(move || {
            AnyView::new(
                ShaderPaintView::new(TIME_SHADER)
                    .uniforms(uniforms_for_view.clone())
                    .continuous(continuous_for_view.clone())
                    .size(48.0, 32.0),
            )
        }),
        48,
        32,
        MinimalTestTheme::default(),
    );

    let t0 = Instant::now();
    let initial = capture_at(&mut runtime, t0);
    // Static shader paint settles without requesting further frames.
    assert!(
        runtime.is_settled(),
        "a static ShaderPaintView must settle after its initial render"
    );

    // Updating `uniforms` invalidates the scene and changes the blue channel.
    uniforms.set(vec![0.9_f32, 0.0, 0.0, 0.0]);
    assert!(
        !runtime.is_settled(),
        "changing `uniforms` must mark the window pending"
    );
    let after_uniforms = capture_at(&mut runtime, t0 + Duration::from_millis(32));
    let center = (16 * 48 + 24) * 4;
    assert!(
        after_uniforms[center + 2] > initial[center + 2].saturating_add(120),
        "uniform update must raise blue channel: before={}, after={}",
        initial[center + 2],
        after_uniforms[center + 2]
    );

    // Enabling `continuous` advances `uniforms.time` across pumped frames and
    // marks perpetual animation demand.
    continuous.set(true);
    let anim_start = capture_at(&mut runtime, t0 + Duration::from_millis(48));
    assert!(
        !runtime.is_settled() && runtime.is_settled_except_perpetual(),
        "an animated ShaderPaintView must report perpetual animation demand"
    );
    let anim_later = capture_at(&mut runtime, t0 + Duration::from_millis(900));
    assert!(
        anim_later[center] > anim_start[center].saturating_add(100),
        "advancing frame time must raise red channel: start={}, later={}",
        anim_start[center],
        anim_later[center]
    );

    // `FlowingGradient` renders non-uniform colour bands through `Paint::Shader`.
    let flowing = render_view_pixels(open_painter(), || {
        AnyView::new(FlowingGradient::new().size(64.0, 32.0))
    });
    let top_left = &flowing[0..4];
    let bottom_right = &flowing[(31 * 64 + 63) * 4..(31 * 64 + 64) * 4];
    assert_eq!(top_left[3], 255);
    assert_ne!(top_left, bottom_right);
}

#[cfg(hydrolysis_gpu_painter)]
#[test]
fn shader_paint_and_flowing_gradient_render_and_react_on_gpu_painter() {
    assert_shader_paint_and_flowing_gradient_render_and_react(|| {
        OffscreenPainter::select_with(false).expect("GPU painter opens on test hardware")
    });
}

#[cfg(hydrolysis_cpu_painter)]
#[test]
fn shader_paint_and_flowing_gradient_render_and_react_on_cpu_painter() {
    assert_shader_paint_and_flowing_gradient_render_and_react(|| {
        OffscreenPainter::open_cpu().expect("CPU painter opens")
    });
}

#[cfg(all(hydrolysis_gpu_painter, hydrolysis_cpu_painter))]
#[test]
fn shader_paint_is_visually_equivalent_across_gpu_cpu_and_gpu_content() {
    let params = || vec![0.9_f32, 0.2, 0.1, 1.0, 0.1, 0.4, 0.95, 1.0];
    let cpu_pixels = render_view_pixels(
        OffscreenPainter::open_cpu().expect("CPU painter opens"),
        move || {
            AnyView::new(
                ShaderPaintView::new(RAMP_SHADER)
                    .uniforms(params())
                    .size(64.0, 32.0),
            )
        },
    );
    let gpu_paint_pixels = render_view_pixels(
        OffscreenPainter::select_with(false).expect("GPU painter opens on test hardware"),
        move || {
            AnyView::new(
                ShaderPaintView::new(RAMP_SHADER)
                    .uniforms(params())
                    .size(64.0, 32.0),
            )
        },
    );
    let gpu_content_pixels = render_view_pixels(
        OffscreenPainter::select_with(false).expect("GPU painter opens on test hardware"),
        move || {
            AnyView::new(
                ShaderPaintView::new(RAMP_SHADER)
                    .uniforms(params())
                    .into_gpu_view()
                    .size(64.0, 32.0),
            )
        },
    );

    assert_eq!(cpu_pixels.len(), gpu_paint_pixels.len());
    assert_eq!(gpu_paint_pixels.len(), gpu_content_pixels.len());
    let max_cpu_vs_gpu = cpu_pixels
        .iter()
        .zip(&gpu_paint_pixels)
        .map(|(&a, &b)| a.abs_diff(b))
        .max()
        .unwrap_or(0);
    let max_paint_vs_content = gpu_paint_pixels
        .iter()
        .zip(&gpu_content_pixels)
        .map(|(&a, &b)| a.abs_diff(b))
        .max()
        .unwrap_or(0);
    assert!(
        max_cpu_vs_gpu <= 2,
        "CPU Paint::Shader vs GPU Paint::Shader max channel delta {max_cpu_vs_gpu} > 2"
    );
    assert!(
        max_paint_vs_content <= 2,
        "GPU Paint::Shader vs GpuContent max channel delta {max_paint_vs_content} > 2"
    );
}
