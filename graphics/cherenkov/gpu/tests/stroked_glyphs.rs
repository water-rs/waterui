//! Stroke cache identity and retained updates.
use cherenkov::kurbo::{Affine, Cap, Join, Rect, Stroke};
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
use cherenkov::{
    Draw, Engine, FontSource, FrameTime, Glyph, GlyphRun, GlyphStyle, Offscreen, OffscreenFormat,
    Picture, Pressure, WorkingColor,
};
use cherenkov_gpu::{Gpu, GpuConfig};
use nami::Binding;

split_test! {
fn live_stroke_changes_match_cold_coverage_without_relowering_static_content() {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default())).expect("GPU engine");
    let font = engine
        .font(FontSource::bytes(
            std::fs::read("../scenes/fonts/NotoSans.ttf").expect("font"),
        ))
        .expect("registered font");
    let surface = wait!(engine
        .surface(Offscreen::new((96, 72), OffscreenFormat::LinearF16)))
        .expect("surface");
    let mut run = GlyphRun {
        font: font.id(),
        size: 38.0,
        coords: Vec::new().into(),
        glyphs: vec![
            Glyph {
                id: 36,
                x: 12.25,
                y: 52.5,
                transform: None,
            },
            Glyph {
                id: 37,
                x: 45.5,
                y: 52.5,
                transform: None,
            },
        ]
        .into(),
        style: GlyphStyle::Stroke(Stroke::new(1.0)),
    };
    let value = Binding::container(run.clone());
    let fixed = Picture::record(|c| {
        c.fill(Rect::new(1.0, 1.0, 5.0, 5.0), WorkingColor::WHITE);
    });
    let content = surface.record(|c| {
        c.picture(&fixed, Affine::IDENTITY);
        c.glyphs(value.clone(), WorkingColor::WHITE);
    });
    surface.update(|tx| {
        tx[surface.root()].content(content);
    });
    wait!(engine.render(FrameTime::now())).expect("initial stroke");
    assert_eq!(engine.stats().commands_lowered, 2);
    for style in [
        Stroke::new(3.0).with_join(Join::Round),
        Stroke::new(2.0)
            .with_dashes(0.5, [2.0, 1.0])
            .with_caps(Cap::Round),
        Stroke::new(2.0)
            .with_dashes(1.5, [2.0, 1.0])
            .with_caps(Cap::Square),
        Stroke::new(1.0)
            .with_join(Join::Miter)
            .with_miter_limit(2.0),
    ] {
        run.style = GlyphStyle::Stroke(style);
        value.set(run.clone());
        wait!(engine.render(FrameTime::now())).expect("stroke edit");
        assert_eq!(engine.stats().commands_lowered, 1);
        let warm = wait!(surface.readback()).expect("warm pixels");
        engine.trim(Pressure::Critical);
        // Force the frame after trim without replacing retained commands.
        surface.update(|tx| {
            tx[surface.root()].opacity(0.999_f32);
        });
        wait!(engine.render(FrameTime::now())).expect("trimmed frame");
        surface.update(|tx| {
            tx[surface.root()].opacity(1.0_f32);
        });
        wait!(engine.render(FrameTime::now())).expect("cold coverage");
        let cold = wait!(surface.readback()).expect("cold pixels");
        for (warm, cold) in warm.pixels.iter().zip(cold.pixels) {
            assert_eq!(
                warm.map(f32::to_bits),
                cold.map(f32::to_bits),
                "stroke cache alias"
            );
        }
        wait!(engine.render(FrameTime::now())).expect("idle");
        assert_eq!(engine.stats().commands_lowered, 0);
    }
}
}
