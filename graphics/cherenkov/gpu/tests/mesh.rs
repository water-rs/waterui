//! Analytic mesh sampling and retained operands.
use cherenkov::kurbo::{Affine, Point, Rect};
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
use cherenkov::{
    Draw, Engine, FrameTime, MeshGradient, Offscreen, OffscreenFormat, Picture, WorkingColor,
};
use cherenkov_gpu::{Gpu, GpuConfig};
use nami::Binding;

fn mesh(alpha: f32) -> MeshGradient {
    MeshGradient::new(
        1,
        1,
        vec![
            Point::new(8.0, 8.0),
            Point::new(24.0, 8.0),
            Point::new(8.0, 24.0),
            Point::new(24.0, 24.0),
        ],
        vec![
            WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            WorkingColor::new([0.0, 1.0, 0.0, alpha]),
            WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            WorkingColor::new([1.0, 1.0, 1.0, alpha]),
        ],
    )
}

split_test! {
fn mesh_interpolates_premultiplied_color_and_updates_one_retained_command() {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default())).expect("GPU");
    let surface = wait!(engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))
        .expect("surface");
    let value = Binding::container(mesh(0.25));
    let fixed = Picture::record(|c| {
        c.fill(Rect::new(0.0, 0.0, 2.0, 2.0), WorkingColor::WHITE);
    });
    let content = surface.record(|c| {
        c.picture(&fixed, Affine::IDENTITY);
        c.fill(Rect::new(4.0, 4.0, 28.0, 28.0), value.clone());
    });
    surface.update(|tx| {
        tx[surface.root()].content(content);
    });
    wait!(engine.render(FrameTime::now())).expect("initial mesh");
    assert_eq!(engine.stats().commands_lowered, 2);
    for alpha in [0.25, 0.75, 1.0] {
        value.set(mesh(alpha));
        wait!(engine.render(FrameTime::now())).expect("mesh edit");
        assert_eq!(engine.stats().commands_lowered, 1);
        let pixels = wait!(surface.readback()).expect("pixels").pixels;
        let u = 8.5_f32 / 16.0;
        let v = 10.5_f32 / 16.0;
        let expected = [
            (alpha * u).mul_add(v, (1.0 - u) * (1.0 - v)),
            alpha * u,
            (alpha * u).mul_add(v, (1.0 - u) * v),
            alpha.mul_add(u, 1.0 - u),
        ];
        for (got, want) in pixels[18 * 32 + 16].iter().zip(expected) {
            assert!((got - want).abs() < 0.002, "{got} != {want}");
        }
        assert_eq!(
            pixels[5 * 32 + 5].map(f32::to_bits),
            [0; 4],
            "outside the mesh is transparent"
        );
        wait!(engine.render(FrameTime::now())).expect("idle");
        assert_eq!(engine.stats().commands_lowered, 0);
    }
}
}
