use cherenkov::kurbo::{Affine, Circle, Rect, Stroke};
use cherenkov::{__engine_fn as split_fn, __engine_wait as wait};
use cherenkov::{
    Backend, Draw, Engine, FrameTime, Offscreen, OffscreenFormat, Paint, Picture, RadialGradient,
    TransformedPaint, WorkingColor,
};
use nami::Binding;
use std::sync::Arc;

fn gradient() -> Paint {
    RadialGradient::new((32.0, 32.0), 28.0)
        .stop(0.0, WorkingColor::new([1.0, 0.0, 0.0, 1.0]))
        .stop(1.0, WorkingColor::new([0.0, 0.0, 1.0, 1.0]))
        .into()
}

split_fn! {
pub fn retained<B: Backend>(config: B::Config) {
    let engine = wait!(Engine::<B>::new(config)).expect("engine");
    let retained = wait!(engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))
        .expect("surface");
    let full = wait!(engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))
        .expect("surface");
    let stable = Picture::record(|c| c.fill(Rect::new(1.0, 1.0, 4.0, 4.0), WorkingColor::WHITE));
    let source = Arc::new(gradient());
    let transform = Binding::container(Affine::IDENTITY);
    let paint = nami::SignalExt::map(&transform, move |transform| TransformedPaint {
        paint: Arc::clone(&source),
        transform,
    });
    let content = retained.record(|c| {
        c.picture(&stable, Affine::IDENTITY);
        c.stroke(Circle::new((32.0, 32.0), 20.0), Stroke::new(7.0), paint);
    });
    retained.update(|tx| {
        tx[retained.root()].content(content);
    });
    wait!(engine.render(FrameTime::now())).expect("initial");
    assert_eq!(engine.stats().commands_lowered, 2);
    let alpha = wait!(retained
        .readback())
        .expect("readback")
        .pixels
        .into_iter()
        .map(|p| p[3].to_bits())
        .collect::<Vec<_>>();
    for map in [
        Affine::new([1.8, 0.0, 0.3, 0.6, -15.0, 8.0]),
        Affine::new([-1.0, 0.2, 0.0, 1.0, 64.0, 0.0]),
        Affine::rotate_about(0.7, cherenkov::kurbo::Point::new(32.0, 32.0)),
    ] {
        transform.set(map);
        wait!(engine.render(FrameTime::now())).expect("live transform");
        assert_eq!(engine.stats().commands_lowered, 1);
        let a = wait!(retained.readback()).expect("retained pixels");
        assert_eq!(
            alpha,
            a.pixels.iter().map(|p| p[3].to_bits()).collect::<Vec<_>>(),
            "paint changed coverage/stroke width"
        );
        full.update(|tx| {
            tx[full.root()].content(full.record(|c| {
                c.picture(&stable, Affine::IDENTITY);
                c.stroke(
                    Circle::new((32.0, 32.0), 20.0),
                    Stroke::new(7.0),
                    gradient().transformed(map),
                );
            }));
        });
        wait!(engine.render(FrameTime::now())).expect("full lowering");
        let b = wait!(full.readback()).expect("full pixels");
        for (a, b) in a.pixels.iter().zip(b.pixels) {
            assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
        }
        wait!(engine.render(FrameTime::now())).expect("unchanged");
        assert_eq!(engine.stats().commands_lowered, 0);
    }
}
}

split_fn! {
pub fn composition<B: Backend>(config: B::Config) {
    let engine = wait!(Engine::<B>::new(config)).expect("engine");
    let a = wait!(engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))
        .expect("surface");
    let b = wait!(engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))
        .expect("surface");
    let inner = Affine::translate((8.0, -4.0));
    let outer = Affine::scale_non_uniform(1.5, 0.75);
    a.update(|tx| {
        tx[a.root()].content(a.record(|c| {
            c.fill(
                Rect::new(0.0, 0.0, 64.0, 64.0),
                gradient().transformed(inner).transformed(outer),
            );
        }));
    });
    b.update(|tx| {
        tx[b.root()].content(b.record(|c| {
            c.fill(
                Rect::new(0.0, 0.0, 64.0, 64.0),
                gradient().transformed(outer * inner),
            );
        }));
    });
    wait!(engine.render(FrameTime::now())).expect("composition");
    let a = wait!(a.readback()).expect("nested");
    let b = wait!(b.readback()).expect("combined");
    for (a, b) in a.pixels.iter().zip(&b.pixels) {
        assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
    }
    // Analytic radial sample in paint space, independent of the implementation.
    for (x, y) in [(24_u32, 20_u32), (40, 28), (48, 35)] {
        let point = (outer * inner).inverse()
            * cherenkov::kurbo::Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5);
        let t = ((point.x - 32.0).hypot(point.y - 32.0) / 28.0).clamp(0.0, 1.0);
        let pixel = b.pixels[(y * 64 + x) as usize];
        assert!((f64::from(pixel[0]) - (1.0 - t)).abs() < 0.002);
        assert!((f64::from(pixel[2]) - t).abs() < 0.002);
    }
}
}

split_fn! {
pub fn invalid<B: Backend>(config: B::Config) {
    let engine = wait!(Engine::<B>::new(config)).expect("engine");
    let surface = wait!(engine
        .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16)))
        .expect("surface");
    for transform in [
        Affine::scale_non_uniform(0.0, 1.0),
        Affine::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]),
        Affine::scale(f64::INFINITY),
    ] {
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|c| {
                c.fill(
                    Rect::new(0.0, 0.0, 16.0, 16.0),
                    gradient().transformed(transform),
                );
            }));
        });
        assert!(
            wait!(engine.render(FrameTime::now())).is_err(),
            "invalid paint transform was accepted"
        );
    }
}
}

split_fn! {
pub fn identity<B: Backend>(config: B::Config) {
    let engine = wait!(Engine::<B>::new(config)).expect("engine");
    let plain = wait!(engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))
        .expect("surface");
    let wrapped = wait!(engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))
        .expect("surface");
    plain.update(|tx| {
        tx[plain.root()].content(plain.record(|c| {
            c.fill(Rect::new(3.25, 2.5, 61.0, 60.0), gradient());
        }));
    });
    wrapped.update(|tx| {
        tx[wrapped.root()].content(wrapped.record(|c| {
            c.fill(
                Rect::new(3.25, 2.5, 61.0, 60.0),
                TransformedPaint::new(gradient(), Affine::IDENTITY),
            );
        }));
    });
    wait!(engine.render(FrameTime::now())).expect("identity");
    for (a, b) in wait!(plain
        .readback())
        .expect("plain")
        .pixels
        .into_iter()
        .zip(wait!(wrapped.readback()).expect("wrapped").pixels)
    {
        assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
    }
}
}
