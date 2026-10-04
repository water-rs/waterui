//! Group compositing is clipped as an operation, including destination modes.
#![cfg(not(target_arch = "wasm32"))]
use cherenkov::kurbo::Rect;
use cherenkov::{
    BlendMode, BlendSpace, Color, Draw, Engine, FrameTime, Group, Offscreen, OffscreenFormat, Srgb,
    WorkingColor,
};
use cherenkov_cpu::{Raster, RasterConfig};

const RED: WorkingColor = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);
const BLUE: WorkingColor = WorkingColor::new([0.0, 0.0, 1.0, 1.0]);
const GREEN: WorkingColor = WorkingColor::new([0.0, 1.0, 0.0, 1.0]);

#[test]
fn clear_composite_preserves_destination_outside_clip() {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let surface = engine
        .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF32))
        .expect("surface");
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(Rect::new(0.0, 0.0, 8.0, 8.0), WorkingColor::WHITE);
            c.clip(Rect::new(2.0, 2.0, 6.0, 6.0), |c| {
                c.group(Group::new().blend(BlendMode::Clear), |c| {
                    c.fill(Rect::new(0.0, 0.0, 8.0, 8.0), WorkingColor::WHITE);
                });
            });
        }));
    });
    engine.render(FrameTime::now()).expect("render");
    let pixels = surface.readback().expect("pixels").pixels;
    assert_eq!(pixels[0].map(f32::to_bits), [1.0_f32; 4].map(f32::to_bits));
    assert_eq!(
        pixels[3 * 8 + 3].map(f32::to_bits),
        [0.0_f32; 4].map(f32::to_bits)
    );
}

#[test]
fn blended_descendant_isolates_its_normal_group() {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let surface = engine
        .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF32))
        .expect("surface");
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                Rect::new(0.0, 0.0, 8.0, 8.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
            c.group(Group::new(), |c| {
                c.fill(
                    Rect::new(0.0, 0.0, 4.0, 8.0),
                    WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
                );
                c.group(Group::new().blend(BlendMode::Clear), |c| {
                    c.fill(
                        Rect::new(2.0, 0.0, 6.0, 8.0),
                        WorkingColor::new([1.0, 1.0, 1.0, 1.0]),
                    );
                });
            });
        }));
    });
    engine.render(FrameTime::now()).expect("render");
    let pixels = surface.readback().expect("pixels").pixels;
    // `Clear` zeroes the inner group's whole raster — but only inside the
    // outer group's offscreen, which then composites `Normal` over the red
    // background. Without outer isolation the `Clear` reached the scene
    // framebuffer and every pixel came out transparent.
    let red = [1.0_f32, 0.0, 0.0, 1.0].map(f32::to_bits);
    for (i, px) in pixels.iter().enumerate() {
        assert_eq!(px.map(f32::to_bits), red, "pixel {i}");
    }
}

#[test]
fn tree_layer_isolates_blended_child_layer() {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let surface = engine
        .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF32))
        .expect("surface");
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
    engine.render(FrameTime::now()).expect("render");
    let pixels = surface.readback().expect("pixels").pixels;
    let pixel = |x: usize, y: usize| pixels[y * 8 + x];
    assert_eq!(
        pixel(1, 3).map(f32::to_bits),
        [0.0_f32, 0.0, 1.0, 1.0].map(f32::to_bits)
    );
    assert_eq!(
        pixel(3, 3).map(f32::to_bits),
        [1.0_f32, 0.0, 0.0, 1.0].map(f32::to_bits)
    );
}

#[test]
fn tree_layer_isolates_blended_content_group() {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let surface = engine
        .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF32))
        .expect("surface");
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
    engine.render(FrameTime::now()).expect("render");
    let pixels = surface.readback().expect("pixels").pixels;
    let pixel = |x: usize, y: usize| pixels[y * 8 + x];
    assert_eq!(
        pixel(1, 3).map(f32::to_bits),
        [0.0_f32, 0.0, 1.0, 1.0].map(f32::to_bits)
    );
    assert_eq!(
        pixel(3, 3).map(f32::to_bits),
        [1.0_f32, 0.0, 0.0, 1.0].map(f32::to_bits)
    );
}

#[test]
fn nested_tree_layers_isolate_at_the_blending_parent() {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let surface = engine
        .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF32))
        .expect("surface");
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
    engine.render(FrameTime::now()).expect("render");
    let pixels = surface.readback().expect("pixels").pixels;
    let pixel = |x: usize, y: usize| pixels[y * 8 + x];
    assert_eq!(
        pixel(1, 3).map(f32::to_bits),
        [0.0_f32, 1.0, 0.0, 1.0].map(f32::to_bits)
    );
    assert_eq!(
        pixel(3, 3).map(f32::to_bits),
        [0.0_f32, 0.0, 1.0, 1.0].map(f32::to_bits)
    );
}

#[test]
fn encoded_group_composites_in_encoded_space() {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let surface = engine
        .surface(Offscreen::new((4, 4), OffscreenFormat::LinearF32))
        .expect("surface");
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                Rect::new(0.0, 0.0, 4.0, 4.0),
                WorkingColor::new([0.2, 0.2, 0.2, 1.0]),
            );
            c.group(
                Group::new()
                    .blend_space(BlendSpace::SrgbEncoded)
                    .opacity(0.5),
                |c| {
                    c.fill(
                        Rect::new(0.0, 0.0, 4.0, 4.0),
                        WorkingColor::new([0.8, 0.8, 0.8, 1.0]),
                    );
                },
            );
        }));
    });
    engine.render(FrameTime::now()).expect("render");
    let pixel = surface.readback().expect("pixels").pixels[5];
    let encoded = 0.2_f64.powf(1.0 / 2.4).midpoint(0.8_f64.powf(1.0 / 2.4));
    let expected = encoded.powf(2.4);
    for channel in &pixel[..3] {
        assert!((f64::from(*channel) - expected).abs() < 1e-5, "{pixel:?}");
    }
    assert_eq!(pixel[3].to_bits(), 1.0_f32.to_bits());
}

/// A non-destructive blended layer under a clip scales the source by the
/// clip coverage, not the whole composite: Screen over 0.5 grey at a
/// half-covered edge is `B(cb, 0.5·cs) = 0.75`, not `lerp(cb, B, 0.5) =
/// 0.625` (nor `c² = 0.4375`). Destructive modes keep the lerp-by-clip
/// bound.
#[test]
fn clipped_blend_layer_scales_source_by_clip_coverage() {
    const GREY: WorkingColor = WorkingColor::new([0.5, 0.5, 0.5, 1.0]);
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let surface = engine
        .surface(Offscreen::new((12, 12), OffscreenFormat::LinearF32))
        .expect("surface");
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
    engine.render(FrameTime::now()).expect("render");
    let pixels = surface.readback().expect("pixels").pixels;
    let w = 12;
    // Screen rows (3): outside the clip the backdrop is untouched;
    // fully inside, screen(0.5, 1) = 1; at the half-covered edge the
    // coverage scales the source — B(0.5, 0.5·white) = 0.75 premultiplied.
    assert_eq!(
        pixels[3 * w + 2].map(f32::to_bits),
        [0.5_f32, 0.5, 0.5, 1.0].map(f32::to_bits),
        "screen outside clip"
    );
    assert_eq!(
        pixels[3 * w + 4].map(f32::to_bits),
        [0.75_f32, 0.75, 0.75, 1.0].map(f32::to_bits),
        "screen at half-covered clip edge: {:?}",
        pixels[3 * w + 4]
    );
    assert_eq!(
        pixels[3 * w + 6].map(f32::to_bits),
        [1.0_f32; 4].map(f32::to_bits),
        "screen inside clip"
    );
    // Clear rows (9): the destructive path lerps by the clip coverage —
    // untouched outside, cleared inside, halfway at the edge.
    assert_eq!(
        pixels[9 * w + 2].map(f32::to_bits),
        [0.5_f32, 0.5, 0.5, 1.0].map(f32::to_bits),
        "clear outside clip"
    );
    assert_eq!(
        pixels[9 * w + 4].map(f32::to_bits),
        [0.25_f32, 0.25, 0.25, 0.5].map(f32::to_bits),
        "clear at half-covered clip edge: {:?}",
        pixels[9 * w + 4]
    );
    assert_eq!(
        pixels[9 * w + 6].map(f32::to_bits),
        [0.0_f32; 4].map(f32::to_bits),
        "clear inside clip"
    );
}

/// Two overlapping opaque `PlusLighter` layers: coverage saturates at 1,
/// summed light exceeds 1 and survives in the extended working space (#126).
#[test]
fn plus_lighter_saturates_alpha_not_colour() {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let surface = engine
        .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF32))
        .expect("surface");
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                Rect::new(0.0, 0.0, 4.0, 8.0),
                WorkingColor::new([1.5, 0.0, 0.0, 1.0]),
            );
            c.group(Group::new().blend(BlendMode::PlusLighter), |c| {
                c.fill(
                    Rect::new(2.0, 0.0, 8.0, 8.0),
                    WorkingColor::new([0.0, 0.0, 1.25, 1.0]),
                );
            });
        }));
    });
    engine.render(FrameTime::now()).expect("render");
    let pixel = surface.readback().expect("pixels").pixels[3];
    assert!(pixel[3] <= 1.0 + 1e-6, "alpha must saturate: {pixel:?}");
    assert!(
        (pixel[0] - 1.5).abs() < 1e-5 && (pixel[2] - 1.25).abs() < 1e-5,
        "colour adds unclamped: {pixel:?}"
    );
}

/// A destructive child of the surface root composites against the surface
/// clear colour — the scene root is not isolated, as in the oracle. `DestIn`
/// keeps the backdrop where the source covers it and zeroes the rest
/// (transparent, not the clear colour); Clear empties the surface; Src
/// writes the source verbatim.
#[test]
fn destructive_child_of_root_clears_the_surface_clear_colour() {
    const GREY: WorkingColor = WorkingColor::new([0.5, 0.5, 0.5, 1.0]);
    for (mode, left, right) in [
        (BlendMode::DestIn, RED, WorkingColor::new([0.0; 4])),
        (
            BlendMode::Clear,
            WorkingColor::new([0.0; 4]),
            WorkingColor::new([0.0; 4]),
        ),
        (BlendMode::Src, BLUE, WorkingColor::new([0.0; 4])),
    ] {
        let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
        let surface = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF32))
            .expect("surface");
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
        engine.render(FrameTime::now()).expect("render");
        let pixels = surface.readback().expect("pixels").pixels;
        let pixel = |x: usize, y: usize| pixels[y * 8 + x];
        for (x, expected) in [(1usize, left), (6usize, right)] {
            let px = pixel(x, 3);
            for (got, want) in px.iter().zip(expected.components) {
                assert!(
                    (*got - want).abs() < 1e-5,
                    "{mode:?} x={x}: {px:?} != {expected:?}"
                );
            }
        }
    }
}

/// The same destructive child under a real intermediate layer stays
/// isolated: the `DestIn` cuts the pass's own content, and the cleared
/// region reads back as the surface clear colour through the composite.
#[test]
fn destructive_child_of_an_intermediate_layer_stays_isolated() {
    const GREY: WorkingColor = WorkingColor::new([0.5, 0.5, 0.5, 1.0]);
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let surface = engine
        .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF32))
        .expect("surface");
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
    engine.render(FrameTime::now()).expect("render");
    let pixels = surface.readback().expect("pixels").pixels;
    let pixel = |x: usize, y: usize| pixels[y * 8 + x];
    for (x, expected) in [(1usize, RED), (6usize, GREY)] {
        let px = pixel(x, 3);
        for (got, want) in px.iter().zip(expected.components) {
            assert!((*got - want).abs() < 1e-5, "x={x}: {px:?} != {expected:?}");
        }
    }
}

/// 50% sRGB blue over sRGB red inside an sRGB-encoded group composites in
/// the encoded space: the readback is the encoded mix [0.5, 0, 0.5], not
/// the linear-space one (~[0.735, 0, 0.735]).
#[test]
fn srgb_encoded_members_composite_in_the_encoded_space() {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let surface = engine
        .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF32))
        .expect("surface");
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
    engine.render(FrameTime::now()).expect("render");
    let pixels = surface.readback().expect("pixels").pixels;
    let expected = Color::<Srgb>::new([0.5, 0.0, 0.5, 1.0]).to_working();
    for (got, want) in pixels[0].iter().zip(expected.components) {
        assert!(
            (*got - want).abs() < 1e-3,
            "{:?} != {expected:?}",
            pixels[0]
        );
    }
}
