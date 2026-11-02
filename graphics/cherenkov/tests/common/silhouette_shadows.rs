//! Retained shadows, viewport contributors and clipping after convolution.
use cherenkov::{__engine_fn as split_fn, __engine_wait as wait};
use cherenkov::{
    Backend, Draw, Engine, FrameTime, Offscreen, OffscreenFormat, Shadow, WorkingColor,
};
use cherenkov_oracle::image::F32Image;
use cherenkov_oracle::metrics::{assert_close, assert_images_match};
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
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))
        .expect("surface");
    let reference = wait!(engine
        .surface(Offscreen::new((128, 128), OffscreenFormat::LinearF16), || {}))
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
        assert!(pixels[24 * 64 + 8][3] > 0.25, "shadow interior must remain visible");
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
        let window = F32Image {
            width: 64,
            height: 64,
            pixels: (0..64)
                .flat_map(|y| (0..64).map(move |x| (x, y)))
                .map(|(x, y)| expected[(y + 32) * 128 + x + 32])
                .collect(),
        };
        assert_images_match(
            &window,
            &F32Image {
                width: 64,
                height: 64,
                pixels: pixels.clone(),
            },
        );
        for y in 0..64 {
            for x in 49..60 {
                // The clip applies after the blur.
                assert_close(&pixels[y * 64 + x], &[0.0; 4]);
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
            .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16), || {}))
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
