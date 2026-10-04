//! Retained shadows, viewport contributors and clipping after convolution.
use cherenkov::{__engine_fn as split_fn, __engine_wait as wait};
use cherenkov::{
    Backend, Draw, Engine, FrameTime, Offscreen, OffscreenFormat, Shadow, WorkingColor,
};
use kurbo::{Affine, BezPath, Rect};

fn silhouette() -> BezPath {
    let mut p = BezPath::new();
    p.move_to((-8.0, 10.0));
    p.line_to((23.0, 3.0));
    p.line_to((16.0, 23.0));
    p.line_to((38.0, 40.0));
    p.line_to((-8.0, 36.0));
    p.close_path();
    p
}

split_fn! {
pub fn retained_and_padded<B: Backend>(config: B::Config) {
    let engine = wait!(Engine::<B>::new(config)).expect("backend");
    let actual = wait!(engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))
        .expect("surface");
    let reference = wait!(engine
        .surface(Offscreen::new((128, 128), OffscreenFormat::LinearF16)))
        .expect("reference");
    let color = WorkingColor::new([0.2, 0.5, 1.0, 0.75]);
    let shadow = nami::Binding::container(Shadow::new(3.0, color));
    actual.update(|tx| {
        tx[actual.root()].content(actual.record(|r| {
            r.fill(Rect::new(60.0, 60.0, 64.0, 64.0), WorkingColor::WHITE);
            r.clip(Rect::new(0.0, 0.0, 48.5, 60.0), |r| {
                r.shadow(silhouette(), shadow.clone());
            });
        }));
    });
    wait!(engine.render(FrameTime::now())).expect("initial");
    for spread in [2.0, -2.0, 0.0] {
        let spec = Shadow::new(3.0, color).spread(spread);
        shadow.set(spec);
        wait!(engine.render(FrameTime::now())).expect("live shadow");
        assert_eq!(engine.stats().commands_lowered, 1);
        let pixels = wait!(actual.readback()).expect("pixels").pixels;
        reference.update(|tx| {
            tx[reference.root()].content(reference.record(|r| {
                r.transform(Affine::translate((32.0, 32.0)), |r| {
                    r.fill(Rect::new(60.0, 60.0, 64.0, 64.0), WorkingColor::WHITE);
                    r.clip(Rect::new(0.0, 0.0, 48.5, 60.0), |r| {
                        r.shadow(silhouette(), spec);
                    });
                });
            }));
        });
        wait!(engine.render(FrameTime::now())).expect("padded reference");
        let expected = wait!(reference.readback()).expect("reference pixels").pixels;
        for y in 0..64 {
            for x in 0..64 {
                let a = pixels[y * 64 + x];
                let b = expected[(y + 32) * 128 + x + 32];
                for (a, b) in a.into_iter().zip(b) {
                    assert!(
                        (a - b).abs() < 0.001,
                        "viewport changes shadow at {x},{y}: {a} != {b}"
                    );
                }
                if (49..60).contains(&x) {
                    assert_eq!(
                        a.map(f32::to_bits),
                        [0.0_f32; 4].map(f32::to_bits),
                        "clip applied after blur"
                    );
                }
            }
        }
        wait!(engine.render(FrameTime::now())).expect("idle");
        assert_eq!(engine.stats().commands_lowered, 0);
    }
}
}

split_fn! {
/// Invalid placements and parameters are errors, not empty silhouettes.
pub fn invalid<B: Backend>(mut config: impl FnMut() -> B::Config) {
    // One engine for all cases: each case gets a fresh surface, and the
    // validation errors asserted below leave the engine usable.
    let engine = wait!(Engine::<B>::new(config())).expect("engine");
    for (transform, spec) in [
        (Affine::scale(0.0), Shadow::new(3.0, WorkingColor::BLACK)),
        (
            Affine::translate((f64::INFINITY, 0.0)),
            Shadow::new(3.0, WorkingColor::BLACK),
        ),
        (Affine::IDENTITY, Shadow::new(f64::NAN, WorkingColor::BLACK)),
        (
            Affine::IDENTITY,
            Shadow::new(3.0, WorkingColor::BLACK).offset((f64::NAN, 0.0)),
        ),
    ] {
        let surface = wait!(engine
            .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16)))
            .expect("surface");
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|r| {
                r.transform(transform, |r| {
                    r.shadow(silhouette(), spec);
                });
            }));
        });
        assert!(
            wait!(engine.render(FrameTime::now())).is_err(),
            "invalid silhouette input must fail"
        );
    }
}
}
