//! General paths: an even-odd star leaves its centre uncovered while the
//! same star filled non-zero covers it, and a dashed circle's gaps stay
//! clear.

use cherenkov::kurbo::{BezPath, Circle, Point};
use cherenkov::{__engine_fn as split_fn, __engine_test as split_test, __engine_wait as wait};
use cherenkov::{Draw, EvenOdd, WorkingColor};
use cherenkov::{Engine, EngineError, Offscreen, OffscreenFormat};
use cherenkov_gpu::{Gpu, GpuConfig};
use cherenkov_oracle::metrics::assert_images_match;

#[path = "support/visual.rs"]
mod visual;
use visual::crop;

const CLEAR: WorkingColor = WorkingColor::new([0.0, 0.0, 0.0, 1.0]);
const RED: WorkingColor = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);

split_test! {
/// Two integer translations admitted before atlas commit share the entire
/// coverage layout, not just the eventual cell addresses.
fn pending_paths_reuse_their_coverage_layout() -> Result<(), Box<dyn std::error::Error>> {
    use cherenkov::kurbo::{Affine, Shape as _};
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((128, 80), OffscreenFormat::LinearF16), || {}))?;
    surface.clear_color(CLEAR);
    let path = Circle::new((400.0, 450.0), 400.0).to_path(0.01);
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            for x in [12.0, 77.0] {
                c.transform(Affine::new([0.052, 0.0, 0.0, -0.052, x, 58.8]), |c| {
                    c.fill(path.clone(), RED);
                });
            }
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(engine.stats().paths_rasterized, 1);
    let pixels = wait!(surface.readback())?;
    // A whole-pixel translation rasterises the same coverage.
    assert_images_match(&crop(&pixels, 0..63, 0..80), &crop(&pixels, 65..128, 0..80));
    Ok(())
}
}

/// A self-intersecting 5-point star over a 64×64 surface, centred.
fn star() -> BezPath {
    let mut path = BezPath::new();
    let c = Point::new(32.0, 32.0);
    for i in 0..5 {
        let angle = f64::from(i).mul_add(144.0, -90.0).to_radians();
        let p = c + cherenkov::kurbo::Vec2::new(angle.cos() * 24.0, angle.sin() * 24.0);
        if i == 0 {
            path.move_to(p);
        } else {
            path.line_to(p);
        }
    }
    path.close_path();
    path
}

split_fn! {
fn render(
    draw: impl FnOnce(&mut cherenkov::Recorder),
) -> Result<Option<cherenkov::Readback>, Box<dyn std::error::Error>> {
    let engine = match wait!(Engine::<Gpu>::new(GpuConfig::default())) {
        Ok(engine) => engine,
        Err(EngineError::Backend(_)) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    surface.clear_color(CLEAR);
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
    });
    surface.update(|tx| {
        tx[&layer].content(surface.record(draw));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    Ok(Some(wait!(surface.readback())?))
}
}

fn px(readback: &cherenkov::Readback, x: u32, y: u32) -> [f32; 4] {
    readback.pixels[(y * readback.width + x) as usize]
}

split_test! {
fn an_even_odd_star_leaves_its_centre_uncovered() -> Result<(), Box<dyn std::error::Error>> {
    let Some(readback) = wait!(render(|c| c.fill(EvenOdd(star()), RED)))? else {
        return Ok(());
    };
    // The pentagonal hole at the centre reads back as the clear colour.
    let [r, g, b, a] = px(&readback, 32, 30);
    assert!(
        r < 0.05 && g < 0.05 && b < 0.05 && a > 0.99,
        "centre: {r} {g} {b} {a}"
    );
    // Deep inside the top arm the star is singly wound, hence covered.
    let [r, ..] = px(&readback, 32, 17);
    assert!(r > 0.9, "arm: {r}");
    Ok(())
}
}

split_test! {
fn a_nonzero_star_covers_its_centre() -> Result<(), Box<dyn std::error::Error>> {
    let Some(readback) = wait!(render(|c| c.fill(star(), RED)))? else {
        return Ok(());
    };
    let [r, ..] = px(&readback, 32, 30);
    assert!(r > 0.9, "centre: {r}");
    Ok(())
}
}

split_test! {
fn a_dashed_circle_stroke_has_gaps() -> Result<(), Box<dyn std::error::Error>> {
    let mut stroke = kurbo::Stroke::new(4.0);
    stroke.dash_pattern.extend([8.0, 8.0]);
    let Some(readback) = wait!(render(move |c| {
        c.stroke(Circle::new((32.0, 32.0), 20.0), stroke, RED);
    }))?
    else {
        return Ok(());
    };
    // Scan the top of the ring (y = 32 - 20): some columns are covered,
    // some fall in dash gaps.
    let mut covered = 0u32;
    let mut gap = 0u32;
    for x in 12..52 {
        let [r, ..] = px(&readback, x, 12);
        if r > 0.5 {
            covered += 1;
        } else {
            gap += 1;
        }
    }
    assert!(covered > 0 && gap > 0, "covered {covered} gap {gap}");
    // Well inside the ring nothing is stroked.
    let [r, ..] = px(&readback, 32, 32);
    assert!(r < 0.05, "interior: {r}");
    Ok(())
}
}

/// A large square hanging off the surface's top-left corner.
fn overhang() -> BezPath {
    let mut path = BezPath::new();
    path.move_to((-100.0, -100.0));
    path.line_to((40.0, -100.0));
    path.line_to((40.0, 40.0));
    path.line_to((-100.0, 40.0));
    path.close_path();
    path
}

split_test! {
/// Coverage clipped by the surface is cached per integer offset: the same
/// path drawn first half off-screen, then shifted, must not replay the
/// clipped coverage.
fn a_clipped_path_is_cached_per_offset() -> Result<(), Box<dyn std::error::Error>> {
    let engine = match wait!(Engine::<Gpu>::new(GpuConfig::default())) {
        Ok(engine) => engine,
        Err(EngineError::Backend(_)) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    surface.clear_color(CLEAR);
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
    });
    // Frame 1: the path hangs off the top-left, so its coverage is clipped.
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| c.fill(overhang(), RED)));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    // Frame 2: the same path at a different integer offset.
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| {
            c.transform(cherenkov::kurbo::Affine::translate((64.0, 64.0)), |c| {
                c.fill(overhang(), RED);
            });
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let shifted = wait!(surface.readback())?;
    // A fresh engine renders the same shifted draw for comparison.
    let Some(fresh) = wait!(render(|c| {
        c.transform(cherenkov::kurbo::Affine::translate((64.0, 64.0)), |c| {
            c.fill(overhang(), RED);
        });
    }))?
    else {
        return Ok(());
    };
    for (i, (a, b)) in shifted.pixels.iter().zip(&fresh.pixels).enumerate() {
        for c in 0..4 {
            assert!(
                (a[c] - b[c]).abs() < 1e-3,
                "pixel {i} channel {c}: {} vs {}",
                a[c],
                b[c]
            );
        }
    }
    Ok(())
}
}

split_test! {
/// A path clip multiplies content by the rasterized star's coverage.
fn a_path_clip_masks_content() -> Result<(), Box<dyn std::error::Error>> {
    let Some(readback) = wait!(render(|c| {
        c.clip(star(), |c| {
            c.fill(cherenkov::kurbo::Rect::new(0.0, 0.0, 64.0, 64.0), RED);
        });
    }))?
    else {
        return Ok(());
    };
    // Deep inside the top arm the star covers.
    let [r, ..] = px(&readback, 32, 17);
    assert!(r > 0.9, "arm: {r}");
    // The concave notch between the top and right arm tips is outside.
    let [r, ..] = px(&readback, 46, 14);
    assert!(r < 0.05, "notch: {r}");
    // Somewhere along an edge a pixel is partially covered.
    let partial = readback.pixels.iter().any(|p| p[0] > 0.1 && p[0] < 0.9);
    assert!(partial, "no partially covered edge pixel");
    Ok(())
}
}

split_test! {
/// A rect drawn as a path hanging off the surface's left edge: the
/// out-of-window edge's winding deposit must still reach column 0, or the
/// interior stays clear.
fn a_fill_overhanging_the_left_edge_paints_its_interior() -> Result<(), Box<dyn std::error::Error>>
{
    let mut left_edge_rect = BezPath::new();
    left_edge_rect.move_to((-40.0, 10.0));
    left_edge_rect.line_to((30.0, 10.0));
    left_edge_rect.line_to((30.0, 50.0));
    left_edge_rect.line_to((-40.0, 50.0));
    left_edge_rect.close_path();
    let Some(readback) = wait!(render(|c| c.fill(left_edge_rect, RED)))? else {
        return Ok(());
    };
    for x in [5, 15, 28] {
        let [r, g, b, a] = px(&readback, x, 20);
        assert!(
            r > 0.9 && g < 0.1 && b < 0.1 && a > 0.9,
            "interior ({x},20): {r} {g} {b} {a}"
        );
    }
    // Right of the fill stays clear.
    let [r, ..] = px(&readback, 40, 20);
    assert!(r < 0.05, "outside: {r}");
    Ok(())
}
}

split_test! {
/// A clip path whose left edge sits outside the window still masks its
/// interior: a path clip rasterizes through the same deposit pipeline.
fn a_clip_overhanging_the_left_edge_masks_its_interior() -> Result<(), Box<dyn std::error::Error>> {
    let mut clip = BezPath::new();
    clip.move_to((-40.0, -10.0));
    clip.line_to((32.0, -10.0));
    clip.line_to((32.0, 74.0));
    clip.line_to((-40.0, 74.0));
    clip.close_path();
    let Some(readback) = wait!(render(|c| {
        c.clip(clip, |c| {
            c.fill(cherenkov::kurbo::Rect::new(0.0, 0.0, 64.0, 64.0), RED);
        });
    }))?
    else {
        return Ok(());
    };
    // Inside the clip's right half.
    let [r, ..] = px(&readback, 20, 32);
    assert!(r > 0.9, "clipped interior: {r}");
    // Right of the clip the fill is masked out.
    let [r, ..] = px(&readback, 40, 32);
    assert!(r < 0.05, "outside clip: {r}");
    Ok(())
}
}

/// A self-intersecting 5-point star centred on a 320×320 surface; its
/// coverage bbox exceeds `MASK_TEXTURE_TEXELS`, so the mask lives on a
/// dedicated texture.
fn large_star() -> BezPath {
    let mut path = BezPath::new();
    let c = Point::new(160.0, 160.0);
    for i in 0..5 {
        let angle = f64::from(i).mul_add(144.0, -90.0).to_radians();
        let p = c + cherenkov::kurbo::Vec2::new(angle.cos() * 150.0, angle.sin() * 150.0);
        if i == 0 {
            path.move_to(p);
        } else {
            path.line_to(p);
        }
    }
    path.close_path();
    path
}

split_fn! {
fn engine() -> Result<Option<Engine<Gpu>>, Box<dyn std::error::Error>> {
    match wait!(Engine::<Gpu>::new(GpuConfig::default())) {
        Ok(engine) => Ok(Some(engine)),
        Err(EngineError::Backend(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
}

split_fn! {
fn big_surface(
    engine: &Engine<Gpu>,
) -> Result<(cherenkov::Surface<Gpu>, cherenkov::Layer), Box<dyn std::error::Error>> {
    wait!(sized_surface(engine, (320, 320)))
}
}

split_fn! {
fn sized_surface(
    engine: &Engine<Gpu>,
    size: (u32, u32),
) -> Result<(cherenkov::Surface<Gpu>, cherenkov::Layer), Box<dyn std::error::Error>> {
    let surface = wait!(engine.surface(Offscreen::new(size, OffscreenFormat::LinearF16), || {}))?;
    surface.clear_color(CLEAR);
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
    });
    Ok((surface, layer))
}
}

split_test! {
/// A clip mask too big for the atlas rasterizes to its own texture and
/// still multiplies every instance under it.
fn a_large_path_clip_masks_content() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine())? else {
        return Ok(());
    };
    let (surface, layer) = wait!(big_surface(&engine))?;
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| {
            c.fill(cherenkov::kurbo::Rect::new(0.0, 0.0, 320.0, 320.0), RED);
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let unmasked = wait!(engine.memory()).gpu.0;
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| {
            c.clip(large_star(), |c| {
                c.fill(cherenkov::kurbo::Rect::new(0.0, 0.0, 320.0, 320.0), RED);
            });
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(engine.stats().paths_rasterized, 1);
    // The ~285×272 mask (over 65536 texels) lives on its own texture.
    assert!(
        wait!(engine.memory()).gpu.0 >= unmasked + 70000,
        "gpu {} vs {}",
        wait!(engine.memory()).gpu.0,
        unmasked
    );
    let readback = wait!(surface.readback())?;
    // Deep inside the top arm the star covers.
    let [r, ..] = px(&readback, 160, 60);
    assert!(r > 0.9, "arm: {r}");
    // The corner is outside the star.
    let [r, g, b, a] = px(&readback, 5, 5);
    assert!(
        r < 0.05 && g < 0.05 && b < 0.05 && a > 0.99,
        "corner: {r} {g} {b} {a}"
    );
    // Along an edge a pixel is partially covered.
    let partial = readback.pixels.iter().any(|p| p[0] > 0.1 && p[0] < 0.9);
    assert!(partial, "no partially covered edge pixel");
    Ok(())
}
}

split_test! {
/// The texture mask is cached: an identical second frame rasterizes
/// nothing.
fn a_large_path_clip_is_cached_across_frames() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine())? else {
        return Ok(());
    };
    let (surface, layer) = wait!(big_surface(&engine))?;
    let draw = |c: &mut cherenkov::Recorder| {
        c.clip(large_star(), |c| {
            c.fill(cherenkov::kurbo::Rect::new(0.0, 0.0, 320.0, 320.0), RED);
        });
    };
    surface.update(|tx| {
        tx[&layer].content(surface.record(draw));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(engine.stats().paths_rasterized, 1);
    surface.update(|tx| {
        tx[&layer].content(surface.record(draw));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(
        engine.stats().paths_rasterized,
        0,
        "mask texture not cached"
    );
    let readback = wait!(surface.readback())?;
    let [r, ..] = px(&readback, 160, 60);
    assert!(r > 0.9, "arm: {r}");
    Ok(())
}
}

split_test! {
/// Vertices far outside the surface still rasterize a valid mask.
fn a_large_path_clip_larger_than_the_surface() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine())? else {
        return Ok(());
    };
    let (surface, layer) = wait!(big_surface(&engine))?;
    let mut triangle = BezPath::new();
    triangle.move_to((-200.0, -200.0));
    triangle.line_to((520.0, -200.0));
    triangle.line_to((-200.0, 520.0));
    triangle.close_path();
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| {
            c.clip(triangle, |c| {
                c.fill(cherenkov::kurbo::Rect::new(0.0, 0.0, 320.0, 320.0), RED);
            });
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    let [r, ..] = px(&readback, 60, 60);
    assert!(r > 0.9, "interior: {r}");
    // Below the hypotenuse is outside the polygon.
    let [r, ..] = px(&readback, 300, 300);
    assert!(r < 0.05, "outside: {r}");
    Ok(())
}
}

split_test! {
/// Two nested large path clips compose: a pixel must be inside both to
/// show the fill.
fn nested_large_path_clips_intersect() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine())? else {
        return Ok(());
    };
    let (surface, layer) = wait!(big_surface(&engine))?;
    // Outer: a rectangle covering the left ~270 columns.
    let mut outer = BezPath::new();
    outer.move_to((-50.0, -50.0));
    outer.line_to((270.0, -50.0));
    outer.line_to((270.0, 370.0));
    outer.line_to((-50.0, 370.0));
    outer.close_path();
    // Inner: a diamond centred on the surface.
    let mut inner = BezPath::new();
    inner.move_to((160.0, 10.0));
    inner.line_to((310.0, 160.0));
    inner.line_to((160.0, 310.0));
    inner.line_to((10.0, 160.0));
    inner.close_path();
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| {
            c.clip(outer, |c| {
                c.clip(inner, |c| {
                    c.fill(cherenkov::kurbo::Rect::new(0.0, 0.0, 320.0, 320.0), RED);
                });
            });
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // Inside both.
    let [r, ..] = px(&readback, 160, 160);
    assert!(r > 0.9, "intersection: {r}");
    // Inside the diamond but right of the outer rect.
    let [r, ..] = px(&readback, 290, 160);
    assert!(r < 0.05, "diamond only: {r}");
    // Inside the outer rect but below the diamond.
    let [r, ..] = px(&readback, 200, 300);
    assert!(r < 0.05, "outer only: {r}");
    Ok(())
}
}

split_test! {
/// A transformed layer's large clip still addresses the texture mask in
/// device space.
fn a_large_path_clip_under_a_rotation() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine())? else {
        return Ok(());
    };
    let (surface, layer) = wait!(big_surface(&engine))?;
    let center = Point::new(160.0, 160.0);
    let transform = cherenkov::kurbo::Affine::rotate_about(0.5, center);
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| {
            c.transform(transform, |c| {
                c.clip(large_star(), |c| {
                    c.fill(cherenkov::kurbo::Rect::new(0.0, 0.0, 320.0, 320.0), RED);
                });
            });
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // (160, 60) — inside the top arm — rotates to ~(208, 72).
    let [r, ..] = px(&readback, 208, 72);
    assert!(r > 0.9, "rotated arm: {r}");
    // (248, 39) — between two arms — rotates to ~(296, 96).
    let [r, ..] = px(&readback, 296, 96);
    assert!(r < 0.05, "rotated gap: {r}");
    Ok(())
}
}

split_test! {
/// Mask textures over the budget are evicted unless a retained frame
/// still references them; an evicted mask re-rasterizes.
fn mask_textures_are_evicted_over_budget() -> Result<(), Box<dyn std::error::Error>> {
    // One 320×320 mask fits `gpu / 16`; two do not.
    let config = GpuConfig {
        budget: cherenkov::Budget {
            gpu: cherenkov::Bytes::mib(2),
            ..cherenkov::Budget::default()
        },
        ..GpuConfig::default()
    };
    let engine = match wait!(Engine::<Gpu>::new(config)) {
        Ok(engine) => engine,
        Err(EngineError::Backend(_)) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let (surface, layer) = wait!(big_surface(&engine))?;
    let clip_a = large_star();
    // A different geometry: same star turned half a point.
    let mut clip_b = BezPath::new();
    let c = Point::new(160.0, 160.0);
    for i in 0..5 {
        let angle = f64::from(i).mul_add(144.0, -54.0).to_radians();
        let p = c + cherenkov::kurbo::Vec2::new(angle.cos() * 150.0, angle.sin() * 150.0);
        if i == 0 {
            clip_b.move_to(p);
        } else {
            clip_b.line_to(p);
        }
    }
    clip_b.close_path();
    let draw = move |clip: BezPath| {
        move |c: &mut cherenkov::Recorder| {
            c.clip(clip, |c| {
                c.fill(cherenkov::kurbo::Rect::new(0.0, 0.0, 320.0, 320.0), RED);
            });
        }
    };
    surface.update(|tx| {
        tx[&layer].content(surface.record(draw(clip_a.clone())));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let one_mask = wait!(engine.memory()).gpu.0;
    surface.update(|tx| {
        tx[&layer].content(surface.record(draw(clip_b)));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    // Over budget after storing B, so the unreferenced A is evicted.
    assert!(wait!(engine.memory()).gpu.0 <= one_mask, "A not evicted");
    let rb = wait!(surface.readback())?;
    let [r, ..] = px(&rb, 160, 232);
    assert!(r > 0.9, "frame 2: {r}");
    surface.update(|tx| {
        tx[&layer].content(surface.record(draw(clip_a)));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    // A was evicted, so it rasterizes again — and renders correctly.
    assert_eq!(engine.stats().paths_rasterized, 1);
    assert!(wait!(engine.memory()).gpu.0 <= one_mask);
    let rb = wait!(surface.readback())?;
    let [r, ..] = px(&rb, 160, 60);
    assert!(r > 0.9, "frame 3: {r}");
    Ok(())
}
}

split_test! {
/// A rect clip merged with a path clip: the mask still applies inside the
/// intersected rectangle.
fn a_rect_clip_merges_with_a_path_clip() -> Result<(), Box<dyn std::error::Error>> {
    let Some(readback) = wait!(render(|c| {
        c.clip(cherenkov::kurbo::Rect::new(8.0, 8.0, 56.0, 56.0), |c| {
            c.clip(EvenOdd(star()), |c| {
                c.fill(cherenkov::kurbo::Rect::new(0.0, 0.0, 64.0, 64.0), RED);
            });
        });
    }))?
    else {
        return Ok(());
    };
    // Inside the rect and the star arm.
    let [r, ..] = px(&readback, 32, 17);
    assert!(r > 0.9, "arm: {r}");
    // Inside the rect but in the even-odd hole.
    let [r, ..] = px(&readback, 32, 30);
    assert!(r < 0.05, "hole: {r}");
    // Outside the rect even where the star would cover.
    let [r, ..] = px(&readback, 4, 32);
    assert!(r < 0.05, "outside rect: {r}");
    Ok(())
}
}

split_test! {
/// A clip wider than the atlas cap can't take a cell at all; it gets a
/// dedicated texture.
fn a_path_clip_wider_than_the_atlas_cap() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine())? else {
        return Ok(());
    };
    let Ok((surface, layer)) = wait!(sized_surface(&engine, (4200, 96))) else {
        // The device can't hold a 4200-wide surface; nothing to test.
        return Ok(());
    };
    // An elongated hexagonal band covering the middle rows.
    let mut band = BezPath::new();
    band.move_to((20.0, 48.0));
    band.line_to((40.0, 10.0));
    band.line_to((4160.0, 10.0));
    band.line_to((4180.0, 48.0));
    band.line_to((4160.0, 86.0));
    band.line_to((40.0, 86.0));
    band.close_path();
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| {
            c.fill(cherenkov::kurbo::Rect::new(0.0, 0.0, 4200.0, 96.0), RED);
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let unmasked = wait!(engine.memory()).gpu.0;
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| {
            c.clip(band, |c| {
                c.fill(cherenkov::kurbo::Rect::new(0.0, 0.0, 4200.0, 96.0), RED);
            });
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(engine.stats().paths_rasterized, 1);
    // ~4160×76 mask texels on a dedicated texture.
    assert!(
        wait!(engine.memory()).gpu.0 >= unmasked + 300_000,
        "gpu {} vs {}",
        wait!(engine.memory()).gpu.0,
        unmasked
    );
    let readback = wait!(surface.readback())?;
    let [r, ..] = px(&readback, 2100, 48);
    assert!(r > 0.9, "interior: {r}");
    // A corner sits outside the band.
    let [r, g, b, a] = px(&readback, 5, 5);
    assert!(
        r < 0.05 && g < 0.05 && b < 0.05 && a > 0.99,
        "corner: {r} {g} {b} {a}"
    );
    Ok(())
}
}

split_test! {
/// A fractional translation is rasterized exactly, not snapped to a
/// quarter-pixel grid: the fill's left edge at x=10.2 deposits 0.8
/// coverage in column 10.
fn a_path_fill_keeps_its_subpixel_translation() -> Result<(), Box<dyn std::error::Error>> {
    let mut rect = BezPath::new();
    rect.move_to((10.0, 10.0));
    rect.line_to((40.0, 10.0));
    rect.line_to((40.0, 40.0));
    rect.line_to((10.0, 40.0));
    rect.close_path();
    let Some(readback) = wait!(render(|c| {
        c.transform(cherenkov::kurbo::Affine::translate((0.2, 0.0)), |c| {
            c.fill(rect, RED);
        });
    }))?
    else {
        return Ok(());
    };
    let [r, ..] = px(&readback, 10, 25);
    assert!((r - 0.8).abs() < 0.03, "edge pixel: {r}");
    let [r, ..] = px(&readback, 9, 25);
    assert!(r < 0.01, "outside pixel: {r}");
    let [r, ..] = px(&readback, 25, 25);
    assert!(r > 0.99, "interior pixel: {r}");
    Ok(())
}
}

split_test! {
/// While a layer's transform animates, its content's device translation
/// is placed on the quarter-pixel grid, so a cached emission is reused
/// across frames; the settled frame is placed exactly.
fn an_animating_layer_places_paths_on_the_quarter_pixel_grid()
-> Result<(), Box<dyn std::error::Error>> {
    use cherenkov::kurbo::Affine;
    use nami::SignalExt as _;
    use std::time::Duration;
use cherenkov::Instant;

    let engine = match wait!(Engine::<Gpu>::new(GpuConfig::default())) {
        Ok(engine) => engine,
        Err(EngineError::Backend(_)) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    surface.clear_color(CLEAR);
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
    });
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| {
            let mut rect = BezPath::new();
            rect.move_to((10.0, 10.0));
            rect.line_to((40.0, 10.0));
            rect.line_to((40.0, 40.0));
            rect.line_to((10.0, 40.0));
            rect.close_path();
            c.fill(rect, RED);
        }));
    });
    let translate = nami::binding(Affine::IDENTITY);
    surface.update(|tx| {
        tx[&layer].transform(translate.clone().with(cherenkov::Animation::from(
            cherenkov::Curve::linear(Duration::from_secs(1)),
        )));
    });
    let start = Instant::now();
    translate.set(Affine::translate((1.2, 0.0)));
    // The track starts at the first sampled frame.
    wait!(engine.render(cherenkov::FrameTime::at(start)))?;
    // Mid-animation at e = 0.3: snapped to 0.25, the edge lands at 10.25.
    let next = wait!(engine.render(cherenkov::FrameTime::at(start + Duration::from_millis(250))))?;
    assert!(matches!(next, cherenkov::Next::At { .. }));
    let readback = wait!(surface.readback())?;
    let [r, ..] = px(&readback, 10, 25);
    assert!((r - 0.75).abs() < 0.03, "snapped edge: {r}");
    // Settled at e = 1.2 exactly: the edge lands at 11.2.
    let next = wait!(engine.render(cherenkov::FrameTime::at(start + Duration::from_secs(2))))?;
    assert!(matches!(next, cherenkov::Next::Idle));
    let readback = wait!(surface.readback())?;
    let [r, ..] = px(&readback, 11, 25);
    assert!((r - 0.8).abs() < 0.03, "settled edge: {r}");
    Ok(())
}
}

split_test! {
fn overlapping_fills_cover_their_union_not_their_winding() -> Result<(), Box<dyn std::error::Error>>
{
    // Two squares [0,2.5]² and [0.5,3]² in one non-zero fill. Pixel
    // (2,0) is half-covered by each square with a 0.25 overlap: union
    // coverage 0.75, not the accumulator's clamped 1.0.
    let Some(readback) = wait!(render(|c| {
        let mut path = BezPath::new();
        for (x0, y0, x1, y1) in [(0.0, 0.0, 2.5, 2.5), (0.5, 0.5, 3.0, 3.0)] {
            path.move_to((x0, y0));
            path.line_to((x1, y0));
            path.line_to((x1, y1));
            path.line_to((x0, y1));
            path.close_path();
        }
        c.fill(path, RED);
    }))?
    else {
        return Ok(());
    };
    let [r, ..] = px(&readback, 2, 0);
    assert!((r - 0.75).abs() < 0.01, "union coverage: {r}");
    let [r, ..] = px(&readback, 2, 2);
    assert!(r > 0.99, "interior: {r}");
    Ok(())
}
}

/// Quarter-covered dots 10 px apart in every 4-row coverage strip: each
/// dot is its own atlas cell, 51 × 64 = 3264 cells in one path.
fn sparse_dots() -> BezPath {
    let mut path = BezPath::new();
    for strip in 0..64_u32 {
        for column in 0..51_u32 {
            let x = f64::from(column * 10) + 1.25;
            let y = f64::from(strip * 4) + 1.25;
            path.move_to((x, y));
            path.line_to((x + 0.5, y));
            path.line_to((x + 0.5, y + 0.5));
            path.line_to((x, y + 0.5));
            path.close_path();
        }
    }
    path
}

split_test! {
/// A leaf whose path emits more cells than 11 bits count keeps every
/// one: replayed from retained storage while a sibling layer changes,
/// it draws the same pixels as the frame that produced it
/// (water-rs/waterui#2778).
fn a_leaf_with_thousands_of_cells_replays_every_cell() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine())? else {
        return Ok(());
    };
    let (surface, dots) = wait!(sized_surface(&engine, (512, 272)))?;
    let marker = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&marker);
        tx[&dots].content(surface.record(|c| c.fill(sparse_dots(), RED)));
        tx[&marker].content(surface.record(|c| {
            c.fill(cherenkov::kurbo::Rect::new(0.0, 260.0, 8.0, 268.0), RED);
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(engine.stats().paths_rasterized, 1);
    let produced = wait!(surface.readback())?;
    let [r, ..] = px(&produced, 1, 1);
    assert!((r - 0.25).abs() < 0.01, "a dot is quarter covered: {r}");
    surface.update(|tx| {
        tx[&marker].content(surface.record(|c| {
            c.fill(cherenkov::kurbo::Rect::new(0.0, 260.0, 8.0, 268.0), CLEAR);
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(engine.stats().paths_rasterized, 0, "the dots replay");
    let replayed = wait!(surface.readback())?;
    assert_images_match(&crop(&replayed, 0..512, 0..256), &crop(&produced, 0..512, 0..256));
    Ok(())
}
}

/// 0.5 px dots at 1 px pitch in a 4-row band — a second qualifying
/// path whose coverage stays disjoint from `sparse_dots`' rows.
fn dense_band() -> BezPath {
    let mut path = BezPath::new();
    for row in 0..4_u32 {
        for column in 0..512_u32 {
            let x = f64::from(column);
            let y = 266.0 + f64::from(row);
            path.move_to((x + 0.25, y + 0.25));
            path.line_to((x + 0.75, y + 0.25));
            path.line_to((x + 0.75, y + 0.75));
            path.line_to((x + 0.25, y + 0.75));
            path.close_path();
        }
    }
    path
}

split_test! {
/// An integer layer translation replays the retained coverage entry —
/// the block shifts per draw and nothing regenerates; a fractional
/// translation changes the placement key and regenerates
/// (water-rs/waterui#3081).
fn a_gpu_path_replays_under_integer_translation_only() -> Result<(), Box<dyn std::error::Error>> {
    use cherenkov::kurbo::Affine;
    let Some(engine) = wait!(engine())? else {
        return Ok(());
    };
    let (surface, dots) = wait!(sized_surface(&engine, (512, 272)))?;
    surface.update(|tx| {
        tx[&dots].content(surface.record(|c| c.fill(sparse_dots(), RED)));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(engine.stats().paths_rasterized, 1);
    let produced = wait!(surface.readback())?;
    // +3, +2 keeps the hull on the surface: same key, a per-draw shift.
    surface.update(|tx| {
        tx[&dots].transform(Affine::translate((3.0, 2.0)));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(engine.stats().paths_rasterized, 0, "an integer move replays");
    let moved = wait!(surface.readback())?;
    assert_images_match(&crop(&moved, 3..512, 2..258), &crop(&produced, 0..509, 0..256));
    // A half-pixel translation changes the key: the entry regenerates.
    surface.update(|tx| {
        tx[&dots].transform(Affine::translate((3.5, 2.0)));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(engine.stats().paths_rasterized, 1, "a fractional move regenerates");
    Ok(())
}
}

split_test! {
/// One retained entry serves two integer-offset draws of the same
/// path — frame 1 generates once per distinct key — and changing only
/// the other path's content regenerates it alone (water-rs/waterui#3081).
fn a_retained_gpu_entry_serves_two_offsets_and_survives_a_sibling_change(
) -> Result<(), Box<dyn std::error::Error>> {
    use cherenkov::kurbo::Affine;
    let Some(engine) = wait!(engine())? else {
        return Ok(());
    };
    let (surface, layer) = wait!(sized_surface(&engine, (512, 272)))?;
    let draw = |surface: &cherenkov::Surface<Gpu>, band: BezPath| {
        surface.record(move |c| {
            c.fill(sparse_dots(), RED);
            c.transform(Affine::translate((3.0, 7.0)), |c| {
                c.fill(sparse_dots(), RED);
            });
            c.fill(band, RED);
        })
    };
    surface.update(|tx| {
        tx[&layer].content(draw(&surface, dense_band()));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(
        engine.stats().paths_rasterized,
        2,
        "one generation per distinct key"
    );
    let produced = wait!(surface.readback())?;
    // A band of 1 px squares on a shifted pitch — a new content hash.
    let mut changed = BezPath::new();
    for row in 0..4_u32 {
        for column in 0..340_u32 {
            let x = f64::from(column) * 1.5;
            let y = 266.0 + f64::from(row);
            changed.move_to((x + 0.3, y + 0.3));
            changed.line_to((x + 0.8, y + 0.3));
            changed.line_to((x + 0.8, y + 0.8));
            changed.line_to((x + 0.3, y + 0.8));
            changed.close_path();
        }
    }
    surface.update(|tx| {
        tx[&layer].content(draw(&surface, changed));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(
        engine.stats().paths_rasterized,
        1,
        "only the changed path regenerates"
    );
    let replayed = wait!(surface.readback())?;
    assert_images_match(&crop(&replayed, 0..506, 0..264), &crop(&produced, 0..506, 0..264));
    Ok(())
}
}

/// Where the cut test's content lies in its local space, as in a document
/// scrolled this far: tens of thousands of units from the origin.
const CUT_SCROLL: (f64, f64) = (40_000.0, 24_000.0);
const CUT_SIZE: (u32, u32) = (96, 64);

/// The transform both renders of the cut test draw under: the scroll, then
/// a rotation with a non-uniform scale, so the local point of a device
/// pixel is inexact.
fn cut_transform() -> cherenkov::kurbo::Affine {
    use cherenkov::kurbo::{Affine, Vec2};
    Affine::new([0.93, 0.37, -0.41, 0.89, 21.3, 3.7]) * Affine::translate(-Vec2::from(CUT_SCROLL))
}

split_fn! {
/// `path` filled under [`cut_transform`] with a repeating three-stop
/// linear gradient a few pixels long, starting at the scrolled origin.
fn render_cut(
    engine: &Engine<Gpu>,
    path: BezPath,
) -> Result<cherenkov::Readback, Box<dyn std::error::Error>> {
    let surface = wait!(engine.surface(Offscreen::new(CUT_SIZE, OffscreenFormat::LinearF16), || {}))?;
    surface.clear_color(CLEAR);
    let (x, y) = CUT_SCROLL;
    let paint = cherenkov::LinearGradient::new((x, y), (x + 6.1, y + 2.7))
        .extend(cherenkov::Extend::Repeat)
        .stop(0.0, WorkingColor::new([0.93, 0.11, 0.27, 1.0]))
        .stop(0.43, WorkingColor::new([0.17, 0.81, 0.39, 1.0]))
        .stop(1.0, WorkingColor::new([0.21, 0.29, 1.37, 1.0]));
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.transform(cut_transform(), |c| c.fill(path, paint));
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    Ok(wait!(surface.readback())?)
}
}

split_test! {
/// A device-space cell paints its pixel, not a point interpolated across
/// the cell (#2835). A gradient-filled triangle renders twice: alone, and
/// with a hairline a few pixels to its right in the same path. The
/// hairline moves the right edge of every strip cell of the triangle —
/// its gap is cheaper to bridge than a second cell — but not one covered
/// pixel, so every pixel away from the hairline looks the same.
///
/// An interpolated local point rounds by an amount that grows with the
/// local coordinates at the cell's corners. The content lies far from its
/// local origin and the gradient repeats every few pixels, so that
/// rounding moves the gradient by more than the half-float surface
/// resolves.
fn a_cell_paints_the_same_however_its_path_is_cut() -> Result<(), Box<dyn std::error::Error>> {
    use cherenkov::kurbo::{Rect, Shape as _, Vec2};
    let Some(engine) = wait!(engine())? else {
        return Ok(());
    };
    let transform = cut_transform();
    let mut triangle = BezPath::new();
    triangle.move_to((1.3, 2.1));
    triangle.line_to((47.9, 5.3));
    triangle.line_to((9.7, 51.6));
    triangle.close_path();
    let triangle = cherenkov::kurbo::Affine::translate(Vec2::from(CUT_SCROLL)) * triangle;
    let device = (transform * triangle.clone()).bounding_box();
    // Twelve device columns right of the triangle, inside its rows.
    let hairline = Rect::new(device.x1 + 12.2, device.y0 + 1.0, device.x1 + 13.8, device.y1 - 1.0);
    let mut with_hairline = triangle.clone();
    with_hairline.extend((transform.inverse() * hairline.to_path(0.01)).iter());
    let alone = wait!(render_cut(&engine, triangle))?;
    let cut = wait!(render_cut(&engine, with_hairline))?;
    let away = hairline.inflate(2.0, 2.0);
    let (width, height) = (CUT_SIZE.0 as usize, CUT_SIZE.1 as usize);
    let reference = crop(&alone, 0..width, 0..height);
    let mut test = crop(&cut, 0..width, 0..height);
    let mut painted = 0;
    for y in 0..CUT_SIZE.1 {
        for x in 0..CUT_SIZE.0 {
            let i = y as usize * width + x as usize;
            if away.contains((f64::from(x) + 0.5, f64::from(y) + 0.5)) {
                // The hairline's own pixels are out of the comparison.
                test.pixels[i] = reference.pixels[i];
                continue;
            }
            painted += usize::from(reference.pixels[i] != CLEAR.components);
        }
    }
    assert_images_match(&reference, &test);
    assert!(painted > 900, "the triangle paints {painted} pixels");
    Ok(())
}
}
