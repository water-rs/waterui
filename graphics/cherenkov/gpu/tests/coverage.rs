//! Pixel-area coverage on oblique edges (lavapipe).

use cherenkov::kurbo::Rect;
use cherenkov::{__engine_fn as split_fn, __engine_test as split_test, __engine_wait as wait};
use cherenkov::{Draw, Engine, EngineError, Offscreen, OffscreenFormat, WorkingColor};
use cherenkov_gpu::{Gpu, GpuConfig};

split_test! {
/// Opaque interiors occlude earlier translucent paint, while later paint
/// still composites in order. Extended P3 values must survive both phases.
fn opaque_interiors_preserve_translucent_painter_order() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(Rect::new(0.0, 0.0, 32.0, 32.0), WorkingColor::new([1.5, 0.25, 0.0, 1.0]));
            c.fill(Rect::new(0.0, 0.0, 32.0, 32.0), WorkingColor::new([0.0, 0.5, 0.0, 0.5]));
            c.fill(cherenkov::kurbo::Circle::new((16.0, 16.0), 8.0), WorkingColor::new([0.0, 0.0, 2.0, 1.0]));
            c.fill(Rect::new(16.0, 0.0, 32.0, 32.0), WorkingColor::new([1.0, 1.0, 0.0, 0.25]));
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let pixels = wait!(surface.readback())?.pixels;
    for (x, y, expected) in [
        (4, 4, [0.75, 0.375, 0.0, 1.0]),
        (12, 16, [0.0, 0.0, 2.0, 1.0]),
        (20, 16, [0.25, 0.25, 1.5, 1.0]),
        (28, 4, [0.8125, 0.53125, 0.0, 1.0]),
    ] {
        for (actual, expected) in pixels[y * 32 + x].iter().zip(expected) {
            assert!((actual - expected).abs() < 0.001, "pixel {x},{y}: {actual} != {expected}");
        }
    }
    Ok(())
}
}

split_fn! {
/// An engine, or `None` when no adapter exists.
fn engine() -> Option<Engine<Gpu>> {
    match wait!(Engine::<Gpu>::new(GpuConfig::default())) {
        Ok(engine) => Some(engine),
        Err(EngineError::Backend(_)) => None,
        Err(e) => panic!("engine init failed: {e}"),
    }
}
}

/// Supersampled area of `f(x, y) <= 0` over the unit pixel at (x, y).
fn supersample(x: u32, y: u32, f: &dyn Fn(f64, f64) -> f64) -> f64 {
    const N: u32 = 64;
    let mut inside = 0usize;
    for sy in 0..N {
        for sx in 0..N {
            let px = f64::from(x) + (f64::from(sx) + 0.5) / f64::from(N);
            let py = f64::from(y) + (f64::from(sy) + 0.5) / f64::from(N);
            if f(px, py) <= 0.0 {
                inside += 1;
            }
        }
    }
    f64::from(u32::try_from(inside).unwrap()) / f64::from(N * N)
}

split_test! {
/// A circle's oblique rim pixels get their exact pixel area, not the
/// linear ramp (which saturates at |d| = 0.5 instead of the true
//  0.707 at 45°); axis-aligned edges keep the ramp.
fn oblique_rim_pixels_get_their_exact_area() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                cherenkov::kurbo::Circle::new((16.0, 16.0), 14.0),
                WorkingColor::WHITE,
            );
            c.fill(Rect::new(4.25, 4.5, 31.75, 8.5), WorkingColor::WHITE);
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let pixels = wait!(surface.readback())?.pixels;
    let alpha = |x: u32, y: u32| f64::from(pixels[(y * 32 + x) as usize][3]);
    // Every pixel in the rim band compares against the 64×64
    // supersampled exact circle area (the oracle's model).
    let mut max_err = 0.0_f64;
    for y in 0..32u32 {
        for x in 0..32u32 {
            let dx = f64::from(x) + 0.5 - 16.0;
            let dy = f64::from(y) + 0.5 - 16.0;
            let r = dx.hypot(dy);
            if !(12.0..=16.0).contains(&r) {
                continue;
            }
            // The white rect overpaints the circle inside it; skip it.
            if x >= 4 && (4..=8).contains(&y) {
                continue;
            }
            let exact = supersample(x, y, &|px, py| (px - 16.0).hypot(py - 16.0) - 14.0);
            max_err = max_err.max((alpha(x, y) - exact).abs());
        }
    }
    eprintln!("max_err circle r=14: {max_err}");
    assert!(max_err < 0.01, "max |Δalpha| on the r=14 rim: {max_err}");

    // A small circle (centre (8.5, 8.5), r = 3): curvature is large
    // enough that the tangent half-plane needs its κw³/24 correction.
    let small = wait!(engine.surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16)))?;
    small.update(|tx| {
        tx[small.root()].content(small.record(|c| {
            c.fill(
                cherenkov::kurbo::Circle::new((8.5, 8.5), 3.0),
                WorkingColor::WHITE,
            );
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let pixels = wait!(small.readback())?.pixels;
    let alpha = |x: u32, y: u32| f64::from(pixels[(y * 16 + x) as usize][3]);
    let mut max_err = 0.0_f64;
    for y in 0..16u32 {
        for x in 0..16u32 {
            let dx = f64::from(x) + 0.5 - 8.5;
            let dy = f64::from(y) + 0.5 - 8.5;
            let r = dx.hypot(dy);
            if !(1.0..=5.0).contains(&r) {
                continue;
            }
            let exact = supersample(x, y, &|px, py| (px - 8.5).hypot(py - 8.5) - 3.0);
            max_err = max_err.max((alpha(x, y) - exact).abs());
        }
    }
    eprintln!("max_err circle r=3: {max_err}");
    assert!(max_err < 0.025, "max |Δalpha| on the r=3 rim: {max_err}");
    Ok(())
}
}

split_test! {
/// Axis-aligned rect coverage stays the linear ramp — the
/// quarter-covered edge columns read 0.75, the interior 1.
fn axis_aligned_edges_keep_the_ramp() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(Rect::new(4.25, 4.5, 31.75, 8.5), WorkingColor::WHITE);
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let pixels = wait!(surface.readback())?.pixels;
    let alpha = |x: u32, y: u32| f64::from(pixels[(y * 32 + x) as usize][3]);
    assert!((alpha(4, 5) - 0.75).abs() < 1e-3, "x=4: {}", alpha(4, 5));
    assert!((alpha(31, 5) - 0.75).abs() < 1e-3, "x=31: {}", alpha(31, 5));
    assert!(
        (alpha(16, 5) - 1.0).abs() < 1e-3,
        "interior: {}",
        alpha(16, 5)
    );
    Ok(())
}
}

split_test! {
/// Elliptical and Lamé corner rims get their exact pixel area now that
/// their distance is second-order (`lame_corner`), the same class of
/// accuracy the circular arcs reached with the κw³/24 term.
fn elliptical_and_lame_rims_get_their_exact_area() -> Result<(), Box<dyn std::error::Error>> {
    use cherenkov::{ContinuousRect, kurbo::Ellipse};
    // (shape name, draw fn, inside indicator).
    type Case = (
        &'static str,
        fn(&mut cherenkov::Recorder),
        Box<dyn Fn(f64, f64) -> f64>,
    );
    fn draw_ellipse(c: &mut cherenkov::Recorder) {
        c.fill(
            Ellipse::new((16.0, 16.0), (14.0, 6.0), 0.0),
            WorkingColor::WHITE,
        );
    }
    fn inside_ellipse(px: f64, py: f64) -> f64 {
        let ux = (px - 16.0) / 14.0;
        let uy = (py - 16.0) / 6.0;
        ux.mul_add(ux, uy * uy) - 1.0
    }
    fn inside_continuous(n: i32) -> impl Fn(f64, f64) -> f64 {
        move |px, py| {
            let qx = (px - 16.0).abs() - 2.0;
            let qy = (py - 16.0).abs() - 2.0;
            if qx <= 0.0 || qy <= 0.0 {
                (px - 16.0).abs().max((py - 16.0).abs()) - 14.0
            } else {
                (qx / 12.0).powi(n) + (qy / 12.0).powi(n) - 1.0
            }
        }
    }
    fn draw_lame(c: &mut cherenkov::Recorder, smoothing: f64) {
        c.fill(
            ContinuousRect::new(Rect::new(2.0, 2.0, 30.0, 30.0), 12.0).with_smoothing(smoothing),
            WorkingColor::WHITE,
        );
    }
    let cases: Vec<Case> = vec![
        ("ellipse 14x6", draw_ellipse, Box::new(inside_ellipse)),
        (
            "continuous n=3 r=12",
            |c| draw_lame(c, 0.5),
            Box::new(inside_continuous(3)),
        ),
        (
            "continuous n=4 r=12",
            |c| draw_lame(c, 1.0),
            Box::new(inside_continuous(4)),
        ),
    ];
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    for (name, draw, inside) in cases {
        let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|c| {
                draw(c);
            }));
        });
        wait!(engine.render(cherenkov::FrameTime::now()))?;
        let pixels = wait!(surface.readback())?.pixels;
        let alpha = |x: u32, y: u32| f64::from(pixels[(y * 32 + x) as usize][3]);
        let mut max_err = 0.0_f64;
        let mut worst = (0u32, 0u32);
        for y in 0..32u32 {
            for x in 0..32u32 {
                // Rim band around the corner arcs: a ring in max-norm
                // coordinates that catches every partially-covered pixel.
                let dx = (f64::from(x) + 0.5 - 16.0).abs();
                let dy = (f64::from(y) + 0.5 - 16.0).abs();
                if !(dx.max(dy) >= 5.0 && dx.max(dy) <= 15.5) {
                    continue;
                }
                let exact = supersample(x, y, inside.as_ref());
                let err = (alpha(x, y) - exact).abs();
                if err > max_err {
                    max_err = err;
                    worst = (x, y);
                }
            }
        }
        eprintln!("max_err {name}: {max_err} at {worst:?}");
        assert!(
            max_err < 0.004,
            "{name}: max |Δalpha| {max_err} at {worst:?}"
        );
    }
    Ok(())
}
}

split_test! {
/// A sharp corner inside a pixel: the exact area inside both folded
/// half-planes, not either one's linear ramp. The rect corner at the
/// centre of pixel (14, 14) covers one quarter of it (0.25); the same
/// corner as a stroke's inner hole covers three quarters (0.75).
fn a_sharp_corner_inside_a_pixel_gets_its_exact_area() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let tol = 2.0 / 255.0;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(Rect::new(14.5, 14.5, 30.5, 30.5), WorkingColor::WHITE);
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let pixels = wait!(surface.readback())?.pixels;
    let alpha = |x: u32, y: u32| f64::from(pixels[(y * 32 + x) as usize][3]);
    assert!(
        (alpha(14, 14) - 0.25).abs() < tol,
        "corner pixel (14,14): {}",
        alpha(14, 14)
    );

    let stroke = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
    stroke.update(|tx| {
        tx[stroke.root()].content(stroke.record(|c| {
            c.stroke(
                Rect::new(11.5, 11.5, 22.5, 22.5),
                kurbo::Stroke {
                    width: 2.0,
                    join: kurbo::Join::Round,
                    ..kurbo::Stroke::default()
                },
                WorkingColor::WHITE,
            );
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let pixels = wait!(stroke.readback())?.pixels;
    let alpha = |x: u32, y: u32| f64::from(pixels[(y * 32 + x) as usize][3]);
    assert!(
        (alpha(12, 12) - 0.75).abs() < tol,
        "stroke inner corner pixel (12,12): {}",
        alpha(12, 12)
    );
    Ok(())
}
}
