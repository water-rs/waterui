//! Known mesh weights and retained mode changes, shared by GPU and CPU tests.
use cherenkov::kurbo::{Point, Rect};
use cherenkov::{__engine_fn as split_fn, __engine_wait as wait};
use cherenkov::{
    Backend, Draw, Engine, FrameTime, MeshColorInterpolation, MeshGradient, Offscreen,
    OffscreenFormat, WorkingColor,
};
use cherenkov_oracle::metrics::assert_f16_close;

split_fn! {
fn mesh(mode: MeshColorInterpolation) -> MeshGradient {
    MeshGradient::new(
        1,
        1,
        vec![
            Point::new(0.5, 0.5),
            Point::new(16.5, 0.5),
            Point::new(0.5, 16.5),
            Point::new(16.5, 16.5),
        ],
        vec![
            WorkingColor::BLACK,
            WorkingColor::WHITE,
            WorkingColor::BLACK,
            WorkingColor::WHITE,
        ],
    )
    .interpolation(mode)
}
}

split_fn! {
/// Changing only the weight rule patches exactly its recorded command.
pub fn interpolation<B: Backend>(config: B::Config) {
    let engine = wait!(Engine::<B>::new(config)).expect("backend");
    let surface = wait!(engine
        .surface(Offscreen::new((24, 20), OffscreenFormat::LinearF16), || {}))
        .expect("surface");
    let value = nami::Binding::container(wait!(mesh(MeshColorInterpolation::Linear)));
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(Rect::new(0., 0., 18., 20.), value.clone());
            r.fill(
                Rect::new(20., 0., 24., 20.),
                WorkingColor::new([1., 0., 0., 1.]),
            );
        }));
    });
    wait!(engine.render(FrameTime::now())).expect("initial");
    assert_eq!(engine.stats().commands_lowered, 2);
    for (mode, expected) in [
        (MeshColorInterpolation::Smoothstep, 0.15625),
        (MeshColorInterpolation::Linear, 0.25),
    ] {
        value.set(wait!(mesh(mode)));
        wait!(engine.render(FrameTime::now())).expect("change rule");
        assert_eq!(engine.stats().commands_lowered, 1);
        let pixels = wait!(surface.readback()).expect("pixels").pixels;
        assert_f16_close(&pixels[8 * 24 + 4][..3], &[expected; 3], 1.0);
        assert!(
            (pixels[8 * 24 + 22][0] - 1.).abs() < 0.0001,
            "unrelated command remains"
        );
        wait!(engine.render(FrameTime::now())).expect("idle");
        assert_eq!(engine.stats().commands_lowered, 0);
    }
}
}
