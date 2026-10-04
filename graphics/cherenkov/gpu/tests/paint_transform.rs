//! Independent paint coordinates on the gpu backend.
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
#[path = "../../tests/support/paint_transform.rs"]
mod common;

split_test! {
fn live_paint_transforms_preserve_geometry_and_retained_output() {
    wait!(common::retained::<cherenkov_gpu::Gpu>(cherenkov_gpu::GpuConfig::default()));
}
}
split_test! {
fn nested_paint_transforms_compose_and_sample_analytically() {
    wait!(common::composition::<cherenkov_gpu::Gpu>(cherenkov_gpu::GpuConfig::default()));
}
}
split_test! {
fn singular_and_non_finite_paint_transforms_fail() {
    wait!(common::invalid::<cherenkov_gpu::Gpu>(cherenkov_gpu::GpuConfig::default()));
}
}

split_test! {
fn shader_paint_transform_changes_sampling_without_moving_geometry()
-> Result<(), Box<dyn std::error::Error>> {
    use cherenkov::kurbo::{Affine, Rect};
    use cherenkov::{
        Draw, Engine, FrameTime, Offscreen, OffscreenFormat, Paint, ShaderPaint, ShaderSource,
    };
    use cherenkov_gpu::{Gpu, GpuConfig};
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader = engine.shader(ShaderSource::wgsl(include_str!("shaders/paint.wgsl")))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                Rect::new(8.0, 8.0, 24.0, 24.0),
                Paint::from(ShaderPaint {
                    shader: shader.id(),
                    uniforms: vec![0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0],
                })
                .transformed(Affine::translate((4.0, 0.0))),
            );
        }));
    });
    wait!(engine.render(FrameTime::now()))?;
    let pixels = wait!(surface.readback())?.pixels;
    let sample = pixels[18 * 32 + 18];
    assert!(
        (sample[1] - 0.40625).abs() < 0.002,
        "translated shader x: {sample:?}"
    );
    assert!(
        (sample[2] - 0.65625).abs() < 0.002,
        "shader y unchanged: {sample:?}"
    );
    assert_eq!(
        pixels[16 * 32 + 27][3].to_bits(),
        0.0_f32.to_bits(),
        "paint moved coverage"
    );
    Ok(())
}
}

split_test! {
fn image_paint_transform_composes_before_pattern_transform() {
    use cherenkov::kurbo::{Affine, Rect};
    use cherenkov::{
        Draw, Engine, Extend, FrameTime, ImageData, ImagePattern, Offscreen, OffscreenFormat,
        Paint, Rgba8, Sampling,
    };
    let engine =
        wait!(Engine::<cherenkov_gpu::Gpu>::new(cherenkov_gpu::GpuConfig::default())).expect("GPU");
    let image = engine
        .image(
            ImageData::<Rgba8>::new(
                2,
                2,
                vec![
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
                ],
            )
            .expect("image data"),
        )
        .expect("image");
    let wrapped = wait!(engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))
        .expect("surface");
    let combined = wait!(engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))
        .expect("surface");
    let pattern = Affine::translate((2.0, 3.0)) * Affine::scale_non_uniform(5.0, 7.0);
    let paint_map = Affine::new([-1.0, 0.25, 0.5, 1.0, 28.0, -3.0]);
    let make = |transform| {
        Paint::Image(ImagePattern {
            image: image.id(),
            transform,
            extend_x: Extend::Repeat,
            extend_y: Extend::Reflect,
            sampling: Sampling::Nearest,
        })
    };
    wrapped.update(|tx| {
        tx[wrapped.root()].content(wrapped.record(|c| {
            c.fill(
                Rect::new(1.0, 1.0, 31.0, 31.0),
                make(pattern).transformed(paint_map),
            );
        }));
    });
    combined.update(|tx| {
        tx[combined.root()].content(
            combined.record(|c| c.fill(Rect::new(1.0, 1.0, 31.0, 31.0), make(paint_map * pattern))),
        );
    });
    wait!(engine.render(FrameTime::now())).expect("render");
    let actual = wait!(wrapped.readback()).expect("wrapped");
    let expected = wait!(combined.readback()).expect("combined");
    for (a, b) in actual.pixels.iter().zip(expected.pixels) {
        assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
    }
}
}

split_test! {
fn explicit_identity_mapping_preserves_pixels_exactly() {
    wait!(common::identity::<cherenkov_gpu::Gpu>(cherenkov_gpu::GpuConfig::default()));
}
}
