//! Blend-mode composites against the W3C formulas on lavapipe.

use cherenkov::kurbo::{Point, Rect};
use cherenkov::{__engine_fn as split_fn, __engine_test as split_test, __engine_wait as wait};
use cherenkov::{
    BlendMode, BlendSpace, Color, ColorStop, Draw, Extend, Group, Interpolation, Paint, Srgb,
    WorkingColor,
};
use cherenkov::{Engine, EngineError, Offscreen, OffscreenFormat};
use cherenkov_gpu::{Gpu, GpuConfig};

const RED: WorkingColor = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);
const BLUE: WorkingColor = WorkingColor::new([0.0, 0.0, 1.0, 1.0]);
const GREEN: WorkingColor = WorkingColor::new([0.0, 1.0, 0.0, 1.0]);
const GREY: WorkingColor = WorkingColor::new([0.5, 0.5, 0.5, 1.0]);
/// HDR primaries so a `PlusLighter` sum exceeds SDR white.
const HDR_RED: WorkingColor = WorkingColor::new([1.5, 0.0, 0.0, 1.0]);
const HDR_BLUE: WorkingColor = WorkingColor::new([0.0, 0.0, 1.25, 1.0]);

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

split_fn! {
/// Renders a red backdrop rect and a blue blend-layer rect; returns the
/// overlap pixel at (24,32) — inside the backdrop only for x < 32 — plus
/// the pass count.
fn render_blend(
    engine: &Engine<Gpu>,
    mode: BlendMode,
) -> Result<([f32; 4], u32), Box<dyn std::error::Error>> {
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(Rect::new(0., 0., 32., 64.), RED);
        }));
    });
    let layer = surface.layer();
    surface.update(|tx| {
        tx[&layer].blend(mode);
        tx[surface.root()].push(&layer);
    });
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| {
            c.fill(Rect::new(16., 0., 64., 64.), BLUE);
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let rb = wait!(surface.readback())?;
    Ok((
        rb.pixels[(32 * rb.width + 24) as usize],
        engine.stats().passes,
    ))
}
}

split_test! {
fn multiply_blends_the_overlap() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let (px, passes) = wait!(render_blend(&engine, BlendMode::Multiply))?;
    // Opaque red × opaque blue = opaque black.
    for c in &px[..3] {
        assert!(c.abs() < 1e-2, "multiply overlap: {px:?}");
    }
    assert!((px[3] - 1.0).abs() < 1e-2, "multiply overlap: {px:?}");
    assert!(passes >= 2, "a blend layer must isolate: passes {passes}");
    Ok(())
}
}

split_test! {
fn screen_blends_the_overlap() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let (px, _) = wait!(render_blend(&engine, BlendMode::Screen))?;
    // screen(red, blue) = 1-(1-r)(1-b) per channel → (1, 0, 1) magenta.
    let [r, g, b, a] = px;
    assert!(
        (r - 1.0).abs() < 1e-2 && g.abs() < 1e-2 && (b - 1.0).abs() < 1e-2 && a > 0.99,
        "screen overlap: {px:?}"
    );
    Ok(())
}
}

split_test! {
/// `DestOut` keeps the backdrop where the source is absent and knocks the
/// overlap out to clear.
fn dest_out_knocks_out_the_overlap() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let (overlap, _) = wait!(render_blend(&engine, BlendMode::DestOut))?;
    for c in overlap {
        assert!(
            c.abs() < 1e-2,
            "dest-out overlap should be clear: {overlap:?}"
        );
    }
    Ok(())
}
}

split_test! {
/// Hue sets the backdrop's luminance on the source's hue and saturation:
/// `SetLum(blue, Lum(red))` = (0.19, 0.19, 1) — the non-separable formula,
/// not a channel-wise blend.
fn hue_is_non_separable() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let (px, _) = wait!(render_blend(&engine, BlendMode::Hue))?;
    let [r, g, b, _a] = px;
    assert!(
        (r - 0.19).abs() < 5e-2 && (g - 0.19).abs() < 5e-2 && (b - 1.0).abs() < 5e-2,
        "hue overlap should be SetLum(blue, 0.3): {px:?}"
    );
    Ok(())
}
}

split_test! {
/// `Extend::None` gradients are transparent outside the range.
fn extend_none_is_transparent_outside_the_range() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                Rect::new(0., 0., 64., 64.),
                Paint::Linear(cherenkov::LinearGradient {
                    start: Point::new(16., 0.),
                    end: Point::new(48., 0.),
                    stops: vec![
                        ColorStop {
                            offset: 0.0,
                            color: RED,
                        },
                        ColorStop {
                            offset: 1.0,
                            color: BLUE,
                        },
                    ],
                    extend: Extend::None,
                    interpolation: Interpolation::Working,
                }),
            );
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let rb = wait!(surface.readback())?;
    let px = |x: u32, y: u32| rb.pixels[(y * rb.width + x) as usize];
    assert_eq!(px(4, 32), [0.0; 4], "left of range must be clear");
    assert_eq!(px(60, 32), [0.0; 4], "right of range must be clear");
    assert!(px(32, 32)[3] > 0.99, "mid-range must be opaque");
    Ok(())
}
}

split_test! {
/// A full-turn sweep gradient: angle 0 (+x) is the first stop.
fn a_sweep_gradient_resolves_angles() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                Rect::new(0., 0., 64., 64.),
                Paint::Sweep(cherenkov::SweepGradient {
                    center: Point::new(32., 32.),
                    start_angle: 0.0,
                    end_angle: std::f64::consts::TAU,
                    stops: vec![
                        ColorStop {
                            offset: 0.0,
                            color: RED,
                        },
                        ColorStop {
                            offset: 1.0,
                            color: BLUE,
                        },
                    ],
                    extend: Extend::Pad,
                    interpolation: Interpolation::Working,
                }),
            );
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let rb = wait!(surface.readback())?;
    let px = |x: u32, y: u32| rb.pixels[(y * rb.width + x) as usize];
    let right = px(56, 32);
    let top = px(32, 8);
    assert!(
        right[0] > 0.9 && right[2] < 0.1,
        "angle 0 is red: {right:?}"
    );
    // atan2 of (0,−1) ≈ 3π/2 → t ≈ 0.75, mostly blue.
    assert!(top[2] > 0.6 && top[0] < 0.4, "top is blue-ish: {top:?}");
    Ok(())
}
}

split_test! {
/// A `Normal` group flattens unless a descendant group blends; then it
/// must isolate, or the descendant's composite would reach the scene.
fn blended_descendant_isolates_its_normal_group() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(Rect::new(0., 0., 8., 8.), RED);
            c.group(Group::new(), |c| {
                c.fill(Rect::new(0., 0., 4., 8.), BLUE);
                c.group(Group::new().blend(BlendMode::Clear), |c| {
                    c.fill(Rect::new(2., 0., 6., 8.), WorkingColor::WHITE);
                });
            });
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let rb = wait!(surface.readback())?;
    // `Clear` zeroes the inner group's whole raster — but only inside the
    // outer group's offscreen, which then composites `Normal` over the red
    // background. Without outer isolation the `Clear` reached the scene
    // framebuffer and every pixel came out transparent.
    for (i, px) in rb.pixels.iter().enumerate() {
        assert_eq!(*px, [1.0, 0.0, 0.0, 1.0], "pixel {i}");
    }
    Ok(())
}
}

split_test! {
fn tree_layer_isolates_blended_child_layer() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16)))?;
    let background = surface.layer();
    let pass = surface.layer();
    let cutout = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&background).push(&pass);
        tx[&background].content(surface.record(|c| {
            c.fill(Rect::new(0.0, 0.0, 8.0, 8.0), RED);
        }));
        tx[&pass].content(surface.record(|c| {
            c.fill(Rect::new(0.0, 0.0, 4.0, 8.0), BLUE);
        }));
        tx[&pass].push(&cutout);
        tx[&cutout]
            .blend(BlendMode::DestOut)
            .content(surface.record(|c| {
                c.fill(Rect::new(2.0, 0.0, 6.0, 8.0), WorkingColor::WHITE);
            }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let pixels = wait!(surface.readback())?.pixels;
    let pixel = |x: usize, y: usize| pixels[y * 8 + x];
    assert_eq!(
        pixel(1, 3).map(f32::to_bits),
        [0.0_f32, 0.0, 1.0, 1.0].map(f32::to_bits)
    );
    assert_eq!(
        pixel(3, 3).map(f32::to_bits),
        [1.0_f32, 0.0, 0.0, 1.0].map(f32::to_bits)
    );
    Ok(())
}
}

split_test! {
fn tree_layer_isolates_blended_content_group() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16)))?;
    let background = surface.layer();
    let pass = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&background).push(&pass);
        tx[&background].content(surface.record(|c| {
            c.fill(Rect::new(0.0, 0.0, 8.0, 8.0), RED);
        }));
        tx[&pass].content(surface.record(|c| {
            c.fill(Rect::new(0.0, 0.0, 4.0, 8.0), BLUE);
            c.group(Group::new().blend(BlendMode::DestOut), |c| {
                c.fill(Rect::new(2.0, 0.0, 6.0, 8.0), WorkingColor::WHITE);
            });
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let pixels = wait!(surface.readback())?.pixels;
    let pixel = |x: usize, y: usize| pixels[y * 8 + x];
    assert_eq!(
        pixel(1, 3).map(f32::to_bits),
        [0.0_f32, 0.0, 1.0, 1.0].map(f32::to_bits)
    );
    assert_eq!(
        pixel(3, 3).map(f32::to_bits),
        [1.0_f32, 0.0, 0.0, 1.0].map(f32::to_bits)
    );
    Ok(())
}
}

split_test! {
fn nested_tree_layers_isolate_at_the_blending_parent() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16)))?;
    let background = surface.layer();
    let outer = surface.layer();
    let inner = surface.layer();
    let cutout = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&background).push(&outer);
        tx[&outer].push(&inner);
        tx[&inner].push(&cutout);
        tx[&background].content(surface.record(|c| {
            c.fill(Rect::new(0.0, 0.0, 8.0, 8.0), RED);
        }));
        tx[&outer].content(surface.record(|c| {
            c.fill(Rect::new(0.0, 0.0, 8.0, 8.0), BLUE);
        }));
        tx[&inner].content(surface.record(|c| {
            c.fill(Rect::new(0.0, 0.0, 4.0, 8.0), GREEN);
        }));
        tx[&cutout]
            .blend(BlendMode::DestOut)
            .content(surface.record(|c| {
                c.fill(Rect::new(2.0, 0.0, 6.0, 8.0), WorkingColor::WHITE);
            }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let pixels = wait!(surface.readback())?.pixels;
    let pixel = |x: usize, y: usize| pixels[y * 8 + x];
    assert_eq!(
        pixel(1, 3).map(f32::to_bits),
        [0.0_f32, 1.0, 0.0, 1.0].map(f32::to_bits)
    );
    assert_eq!(
        pixel(3, 3).map(f32::to_bits),
        [0.0_f32, 0.0, 1.0, 1.0].map(f32::to_bits)
    );
    Ok(())
}
}

split_test! {
/// Two overlapping opaque `PlusLighter` layers: coverage saturates at 1,
/// summed light exceeds 1 and survives in the extended working space (#126).
fn plus_lighter_saturates_alpha_not_colour() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(Rect::new(0., 0., 32., 64.), HDR_RED);
        }));
    });
    let layer = surface.layer();
    surface.update(|tx| {
        tx[&layer].blend(BlendMode::PlusLighter);
        tx[surface.root()].push(&layer);
    });
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| {
            c.fill(Rect::new(16., 0., 64., 64.), HDR_BLUE);
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let px = wait!(surface.readback())?.pixels[(32 * 64 + 24) as usize];
    assert!(
        px[3] <= 1.0 + 1e-3,
        "plus-lighter alpha must saturate: {px:?}"
    );
    assert!(
        (px[0] - 1.5).abs() < 1e-2 && (px[2] - 1.25).abs() < 1e-2,
        "plus-lighter colour adds unclamped: {px:?}"
    );
    Ok(())
}
}

split_test! {
/// A clipped blended layer's clip coverage must scale the source once,
/// not again at the composite: Screen over 0.5 grey at a half-covered
/// edge is `B(cb, 0.5·cs) = 0.75`, not `c²` = 0.4375. Clear keeps the
/// lerp-by-clip bound.
fn clipped_blend_layer_scales_source_by_clip_coverage() -> Result<(), Box<dyn std::error::Error>> {
    const GREY: WorkingColor = WorkingColor::new([0.5, 0.5, 0.5, 1.0]);
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((12, 12), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(Rect::new(0.0, 0.0, 12.0, 12.0), GREY);
        }));
    });
    // Screen layer clipped to the top half, Clear to the bottom half;
    // both clips share the half-covered left edge at x = 4.5. The layer
    // handles must outlive the render or the layer detaches.
    let mut layers = Vec::new();
    for (blend, y0, y1) in [(BlendMode::Screen, 0.0, 6.0), (BlendMode::Clear, 6.0, 12.0)] {
        let layer = surface.layer();
        surface.update(|tx| {
            tx[&layer].blend(blend);
            tx[&layer].clip(Rect::new(4.5, y0, 8.0, y1));
            tx[surface.root()].push(&layer);
        });
        surface.update(|tx| {
            tx[&layer].content(surface.record(|c| {
                c.fill(Rect::new(0.0, 0.0, 12.0, 12.0), WorkingColor::WHITE);
            }));
        });
        layers.push(layer);
    }
    assert_eq!(layers.len(), 2);
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let pixels = wait!(surface.readback())?.pixels;
    let px = |x: usize, y: usize| pixels[y * 12 + x];
    let approx = |got: [f32; 4], want: [f32; 4], what: &str| {
        for i in 0..4 {
            assert!(
                (got[i] - want[i]).abs() < 0.02,
                "{what}: got {got:?}, want {want:?}"
            );
        }
    };
    // Screen rows (3): untouched outside, screen(0.5, 1) = 1 inside,
    // B(0.5, 0.5·white) = 0.75 at the half-covered edge.
    approx(px(2, 3), [0.5, 0.5, 0.5, 1.0], "screen outside clip");
    approx(
        px(4, 3),
        [0.75, 0.75, 0.75, 1.0],
        "screen at half-covered edge",
    );
    approx(px(6, 3), [1.0; 4], "screen inside clip");
    // Clear rows (9): the destructive path lerps by the clip coverage.
    approx(px(2, 9), [0.5, 0.5, 0.5, 1.0], "clear outside clip");
    approx(
        px(4, 9),
        [0.25, 0.25, 0.25, 0.5],
        "clear at half-covered edge",
    );
    approx(px(6, 9), [0.0; 4], "clear inside clip");
    Ok(())
}
}

split_test! {
/// A destructive child of the surface root composites against the surface
/// clear colour — the scene root is not isolated, as in the oracle. `DestIn`
/// keeps the backdrop where the source covers it and zeroes the rest
/// (transparent, not the clear colour); Clear empties the surface; Src
/// writes the source verbatim.
fn destructive_child_of_root_clears_the_surface_clear_colour()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    for (mode, left, right) in [
        (BlendMode::DestIn, RED, WorkingColor::new([0.0; 4])),
        (
            BlendMode::Clear,
            WorkingColor::new([0.0; 4]),
            WorkingColor::new([0.0; 4]),
        ),
        (BlendMode::Src, BLUE, WorkingColor::new([0.0; 4])),
    ] {
        let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16)))?;
        surface.clear_color(GREY);
        let cutout = surface.layer();
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|c| {
                c.fill(Rect::new(0.0, 0.0, 8.0, 8.0), RED);
            }));
            tx[surface.root()].push(&cutout);
            tx[&cutout].blend(mode).content(surface.record(|c| {
                c.fill(Rect::new(0.0, 0.0, 4.0, 8.0), BLUE);
            }));
        });
        wait!(engine.render(cherenkov::FrameTime::now()))?;
        let pixels = wait!(surface.readback())?.pixels;
        let pixel = |x: usize, y: usize| pixels[y * 8 + x];
        for (x, expected) in [(1usize, left), (6usize, right)] {
            let px = pixel(x, 3);
            for (got, want) in px.iter().zip(expected.components) {
                assert!(
                    (*got - want).abs() < 0.02,
                    "{mode:?} x={x}: {px:?} != {expected:?}"
                );
            }
        }
    }
    Ok(())
}
}

split_test! {
/// The same destructive child under a real intermediate layer stays
/// isolated: the `DestIn` cuts the pass's own content, and the cleared
/// region reads back as the surface clear colour through the composite.
fn destructive_child_of_an_intermediate_layer_stays_isolated()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16)))?;
    surface.clear_color(GREY);
    let pass = surface.layer();
    let cutout = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&pass);
        tx[&pass].content(surface.record(|c| {
            c.fill(Rect::new(0.0, 0.0, 8.0, 8.0), RED);
        }));
        tx[&pass].push(&cutout);
        tx[&cutout]
            .blend(BlendMode::DestIn)
            .content(surface.record(|c| {
                c.fill(Rect::new(0.0, 0.0, 4.0, 8.0), WorkingColor::WHITE);
            }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let pixels = wait!(surface.readback())?.pixels;
    let pixel = |x: usize, y: usize| pixels[y * 8 + x];
    for (x, expected) in [(1usize, RED), (6usize, GREY)] {
        let px = pixel(x, 3);
        for (got, want) in px.iter().zip(expected.components) {
            assert!((*got - want).abs() < 0.02, "x={x}: {px:?} != {expected:?}");
        }
    }
    Ok(())
}
}

split_test! {
/// 50% sRGB blue over sRGB red inside an sRGB-encoded group composites in
/// the encoded space: the readback is the encoded mix [0.5, 0, 0.5], not
/// the linear-space one (~[0.735, 0, 0.735]).
fn srgb_encoded_members_composite_in_the_encoded_space() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16)))?;
    let red = Color::<Srgb>::new([1.0, 0.0, 0.0, 1.0]).to_working();
    let blue = Color::<Srgb>::new([0.0, 0.0, 1.0, 0.5]).to_working();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.group(Group::new().blend_space(BlendSpace::SrgbEncoded), |c| {
                c.fill(Rect::new(0.0, 0.0, 8.0, 8.0), red);
                c.fill(Rect::new(0.0, 0.0, 8.0, 8.0), blue);
            });
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let pixels = wait!(surface.readback())?.pixels;
    let expected = Color::<Srgb>::new([0.5, 0.0, 0.5, 1.0]).to_working();
    for (got, want) in pixels[0].iter().zip(expected.components) {
        assert!(
            (*got - want).abs() < 0.02,
            "{:?} != {expected:?}",
            pixels[0]
        );
    }
    Ok(())
}
}
