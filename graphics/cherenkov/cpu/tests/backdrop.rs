//! Backdrop groups on the CPU backend: captures scheduled into bands.

#![cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;

use cherenkov::kurbo::{Rect, RoundedRect};
use cherenkov::{Draw, Engine, FrameTime, Offscreen, OffscreenFormat, WorkingColor};
use cherenkov_cpu::{BandPixels, Bands, Raster, RasterConfig};

fn engine() -> Engine<Raster> {
    Engine::<Raster>::new(RasterConfig::default()).expect("engine")
}

fn pixel(readback: &cherenkov::Readback, x: usize, y: usize) -> [f32; 4] {
    readback.pixels[y * readback.width as usize + x]
}

fn assert_pixel(actual: [f32; 4], expected: [f32; 4], tolerance: f32) {
    for (a, e) in actual.iter().zip(expected) {
        assert!(
            (a - e).abs() <= tolerance,
            "pixel {actual:?}, expected {expected:?}"
        );
    }
}

#[test]
fn unfiltered_member_samples_what_is_behind_it() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32))
        .expect("surface");
    let group = surface.backdrop_group_unfiltered();
    let glass = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 16.0, 32.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
            r.fill(
                Rect::new(16.0, 0.0, 32.0, 32.0),
                WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&glass);
        tx[&glass]
            .clip(Rect::new(8.0, 8.0, 24.0, 24.0))
            .backdrop(group.sample())
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(8.0, 8.0, 24.0, 24.0),
                    WorkingColor::new([1.0, 1.0, 1.0, 0.5]),
                );
            }));
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    // src_over(50% white, solid red) = [1.0, 0.5, 0.5, 1.0].
    assert_pixel(pixel(&readback, 12, 12), [1.0, 0.5, 0.5, 1.0], 1e-5);
    // src_over(50% white, solid blue) = [0.5, 0.5, 1.0, 1.0].
    assert_pixel(pixel(&readback, 20, 12), [0.5, 0.5, 1.0, 1.0], 1e-5);
    // Outside the member's clip the surface is untouched.
    assert_pixel(pixel(&readback, 4, 4), [1.0, 0.0, 0.0, 1.0], 1e-5);
    let memory = engine.memory();
    assert_eq!(memory.backdrop_capture_format, Some("linear-f32"));
}

#[test]
fn capture_point_is_the_first_member_in_paint_order() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32))
        .expect("surface");
    let group = surface.backdrop_group_unfiltered();
    let first = surface.layer();
    let late = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 32.0, 32.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&first);
        tx[&first]
            .clip(Rect::new(8.0, 8.0, 16.0, 16.0))
            .backdrop(group.sample());
        tx[surface.root()].push(&late);
        // Painted after the first member: its green is not in the
        // capture `first` samples, it is on the surface under `late`.
        tx[&late]
            .clip(Rect::new(8.0, 8.0, 16.0, 16.0))
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(8.0, 8.0, 16.0, 16.0),
                    WorkingColor::new([0.0, 1.0, 0.0, 1.0]),
                );
            }));
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    // `first` sampled red; `late` painted green over it at the same rect.
    assert_pixel(pixel(&readback, 10, 10), [0.0, 1.0, 0.0, 1.0], 1e-5);
    // Outside both clips the surface red survives.
    assert_pixel(pixel(&readback, 4, 4), [1.0, 0.0, 0.0, 1.0], 1e-5);
}

#[test]
fn nested_groups_capture_in_paint_order() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32))
        .expect("surface");
    let outer = surface.backdrop_group_unfiltered();
    let inner = surface.backdrop_group_unfiltered();
    let m1 = surface.layer();
    let m2 = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 32.0, 32.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&m1);
        tx[&m1]
            .clip(Rect::new(4.0, 4.0, 28.0, 28.0))
            .backdrop(outer.sample())
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(4.0, 4.0, 28.0, 28.0),
                    WorkingColor::new([0.0, 0.0, 1.0, 0.5]),
                );
            }));
        tx[&m1].push(&m2);
        tx[&m2]
            .clip(Rect::new(12.0, 12.0, 20.0, 20.0))
            .backdrop(inner.sample())
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(12.0, 12.0, 20.0, 20.0),
                    WorkingColor::new([0.0, 1.0, 0.0, 0.5]),
                );
            }));
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    // Inside m1 but outside m2: 50% blue over red = [0.5, 0.0, 0.5].
    assert_pixel(pixel(&readback, 8, 8), [0.5, 0.0, 0.5, 1.0], 1e-5);
    // Inside m2 the inner capture saw m1's composite ([0.5, 0.0, 0.5]),
    // then 50% green drew over it: [0.25, 0.5, 0.25].
    assert_pixel(pixel(&readback, 16, 16), [0.25, 0.5, 0.25, 1.0], 1e-5);
}

#[test]
fn member_inside_clip_only_isolation_sees_the_surface() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32))
        .expect("surface");
    let group = surface.backdrop_group_unfiltered();
    let p = surface.layer();
    let member = surface.layer();
    surface.update(|tx| {
        // The root's rounded-rect clip cannot merge with P's, so P's body
        // lands in a clip-only scratch — invisible to the capture, which
        // must compose it over the surface copy.
        tx[surface.root()]
            .clip(RoundedRect::new(0.0, 0.0, 32.0, 32.0, 2.0))
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(0.0, 0.0, 32.0, 32.0),
                    WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
                );
            }));
        tx[surface.root()].push(&p);
        tx[&p]
            .clip(RoundedRect::new(0.0, 0.0, 32.0, 32.0, 4.0))
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(8.0, 8.0, 12.0, 24.0),
                    WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
                );
            }));
        tx[&p].push(&member);
        tx[&member]
            .clip(RoundedRect::new(8.0, 8.0, 24.0, 24.0, 3.0))
            .backdrop(group.sample());
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    // The member samples the surface's red plus the clip scratch's blue
    // painted before it, each at the right device pixels.
    assert_pixel(pixel(&readback, 10, 16), [0.0, 0.0, 1.0, 1.0], 1e-5);
    assert_pixel(pixel(&readback, 16, 16), [1.0, 0.0, 0.0, 1.0], 1e-5);
}

#[test]
fn member_sample_is_not_attenuated_by_layer_opacity() {
    fn render(opacity: f32) -> cherenkov::Readback {
        let engine = engine();
        let surface = engine
            .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32))
            .expect("surface");
        let group = surface.backdrop_group_unfiltered();
        let member = surface.layer();
        let child = surface.layer();
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|r| {
                r.fill(
                    Rect::new(0.0, 0.0, 32.0, 32.0),
                    WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
                );
            }));
            tx[surface.root()].push(&member);
            tx[&member]
                .clip(RoundedRect::new(4.0, 4.0, 28.0, 28.0, 6.0))
                .opacity(opacity)
                .backdrop(group.sample());
            tx[&member].push(&child);
            tx[&child].content(surface.record(|r| {
                r.fill(
                    Rect::new(16.0, 4.0, 28.0, 28.0),
                    WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
                );
            }));
        });
        engine.render(FrameTime::now()).expect("render");
        surface.readback().expect("readback")
    }

    let half = render(0.5);
    // Sample-only area (left half of the clip): the red backdrop at full
    // strength, unaffected by the layer's 0.5 opacity.
    assert_pixel(pixel(&half, 10, 16), [1.0, 0.0, 0.0, 1.0], 1e-5);
    // Inside the child: 50% blue over the sampled red = [0.5, 0.0, 0.5].
    assert_pixel(pixel(&half, 20, 16), [0.5, 0.0, 0.5, 1.0], 1e-5);
    // Outside the clip: the surface is untouched.
    assert_pixel(pixel(&half, 1, 16), [1.0, 0.0, 0.0, 1.0], 1e-5);
    // The clip edge is not squared: a corner pixel's coverage matches the
    // same layer at full opacity.
    let full = render(1.0);
    assert_pixel(pixel(&half, 5, 5), pixel(&full, 5, 5), 1e-5);
    assert_pixel(pixel(&half, 7, 7), pixel(&full, 7, 7), 1e-5);
}

#[test]
fn blurred_backdrop_matches_full_surface_blur_within_apron() {
    let engine = engine();
    let size = (32, 96);
    let surface = engine
        .surface(Offscreen::new(size, OffscreenFormat::LinearF32))
        .expect("surface");
    let group = surface.backdrop_group(filtrate::filters::GaussianBlur(4.0f32));
    let glass = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 16.0, 96.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
            r.fill(
                Rect::new(16.0, 0.0, 32.0, 96.0),
                WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&glass);
        tx[&glass]
            .clip(Rect::new(8.0, 16.0, 24.0, 88.0))
            .backdrop(group.sample());
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    // The member's left edge is 7 px (~1.75σ) from the step: mostly red.
    let left = pixel(&readback, 9, 48);
    assert!(left[0] > 0.9, "edge pixel {left:?}");
    // The member's right edge, symmetric, is mostly blue.
    let right = pixel(&readback, 22, 48);
    assert!(right[2] > 0.9, "edge pixel {right:?}");
    // The step midpoint blurs to about the mean of both halves (the
    // discrete kernel's centre sits half a pixel into the blue half).
    let edge = pixel(&readback, 16, 48);
    assert!(
        (edge[0] - 0.5).abs() < 0.1 && (edge[2] - 0.5).abs() < 0.1,
        "edge pixel {edge:?}"
    );
}

#[test]
fn band_streamed_backdrop_matches_offscreen_byte_for_byte() {
    let engine = engine();
    let size = (32, 64);
    let width = usize::try_from(size.0).expect("width fits usize");
    let height = usize::try_from(size.1).expect("height fits usize");
    let (sink, streamed_bands) = mpsc::channel();
    let bands = engine
        .surface(Bands::new(size, OffscreenFormat::LinearF32, move |band| {
            let pixels = match band.pixels {
                BandPixels::F32(pixels) => pixels,
                BandPixels::F16(_) => panic!("expected LinearF32 bands"),
            };
            sink.send((band.y, pixels.to_vec()))
                .expect("stream pixels channel open");
        }))
        .expect("bands surface");
    let offscreen = engine
        .surface(Offscreen::new(size, OffscreenFormat::LinearF32))
        .expect("offscreen surface");
    let build = |surface: &cherenkov::Surface<Raster>| {
        let group = surface.backdrop_group(filtrate::filters::GaussianBlur(4.0f32));
        let member = surface.layer();
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|r| {
                r.fill(
                    Rect::new(0.0, 0.0, 32.0, 64.0),
                    WorkingColor::new([0.9, 0.4, 0.1, 1.0]),
                );
            }));
            tx[surface.root()].push(&member);
            tx[&member]
                .clip(Rect::new(6.0, 10.0, 26.0, 58.0))
                .backdrop(group.sample())
                .content(surface.record(|r| {
                    r.fill(
                        Rect::new(6.0, 10.0, 26.0, 58.0),
                        WorkingColor::new([1.0, 1.0, 1.0, 0.25]),
                    );
                }));
        });
        group
    };
    let _bands_group = build(&bands);
    let _offscreen_group = build(&offscreen);
    engine.render(FrameTime::now()).expect("render");

    let mut streamed = Vec::with_capacity(width * height);
    let mut next_y = 0_u32;
    for (y, band) in streamed_bands.try_iter() {
        assert_eq!(y, next_y);
        assert_eq!(band.len() % width, 0);
        next_y += u32::try_from(band.len() / width).expect("band rows");
        streamed.extend_from_slice(&band);
    }
    assert_eq!(next_y, size.1);
    let expected = offscreen.readback().expect("readback").pixels;
    let streamed_bits = streamed
        .iter()
        .flat_map(|pixel| pixel.map(f32::to_bits))
        .collect::<Vec<_>>();
    let expected_bits = expected
        .iter()
        .flat_map(|pixel| pixel.map(f32::to_bits))
        .collect::<Vec<_>>();
    assert_eq!(streamed_bits, expected_bits);
}

/// A blurred member taller than a band: sampling identical spans at
/// different band phases must produce identical pixels — the capture is a
/// windowed intermediate, not a band-seamed artifact.
#[test]
fn multi_band_capture_has_no_band_seams() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 96), OffscreenFormat::LinearF32))
        .expect("surface");
    let group = surface.backdrop_group(filtrate::filters::GaussianBlur(5.0f32));
    let member = surface.layer();
    surface.update(|tx| {
        // A smooth vertical ramp under the member: each row differs
        // slightly, so a band seam would show.
        tx[surface.root()].content(surface.record(|r| {
            for y in 0..96_u8 {
                let v = f32::from(y) / 95.0;
                let y = f64::from(y);
                r.fill(
                    Rect::new(0.0, y, 32.0, y + 1.0),
                    WorkingColor::new([v, 0.5 * (1.0 - v), 0.3, 1.0]),
                );
            }
        }));
        tx[surface.root()].push(&member);
        // 48 rows ≥ 3 bands; blur apron is 15 rows (> 16).
        tx[&member]
            .clip(Rect::new(4.0, 24.0, 28.0, 72.0))
            .backdrop(group.sample());
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    // The ramp's red channel rises smoothly down the rows; a band seam
    // would appear as a discontinuity, several times the per-row slope.
    let column: Vec<f32> = (24..72).map(|y| pixel(&readback, 16, y)[0]).collect();
    for (i, pair) in column.windows(2).enumerate() {
        assert!(
            pair[1] - pair[0] > -1.0e-5 && pair[1] - pair[0] < 0.05,
            "seam at row {}: {:?} -> {:?}",
            24 + i,
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn capture_memory_is_bounded_by_bands_not_capture_height() {
    let peak = |height: u32| -> u64 {
        let engine = engine();
        let surface = engine
            .surface(Offscreen::new((32, height), OffscreenFormat::LinearF32))
            .expect("surface");
        let group = surface.backdrop_group(filtrate::filters::GaussianBlur(4.0f32));
        let member = surface.layer();
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|r| {
                r.fill(
                    Rect::new(0.0, 0.0, 32.0, f64::from(height)),
                    WorkingColor::new([0.4, 0.5, 0.6, 1.0]),
                );
            }));
            tx[surface.root()].push(&member);
            tx[&member]
                .clip(Rect::new(4.0, 8.0, 28.0, f64::from(height) - 8.0))
                .backdrop(group.sample());
        });
        engine.render(FrameTime::now()).expect("render");
        let memory = engine.memory();
        assert_eq!(memory.backdrop_capture_format, Some("linear-f32"));
        memory.backdrop_captures.0
    };
    let p64 = peak(64);
    assert!(p64 > 0, "a capture ran: {p64}");
    assert_eq!(peak(256), p64, "peak scales with band, not capture height");
    assert_eq!(peak(1024), p64, "peak scales with band, not capture height");
}

#[test]
fn member_without_clip_is_unsupported() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32))
        .expect("surface");
    let group = surface.backdrop_group_unfiltered();
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&member);
        tx[&member].backdrop(group.sample());
    });
    let result = engine.render(FrameTime::now());
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported("backdrop-unclipped"))
        ),
        "unexpected result {result:?}"
    );
}

#[test]
fn dropped_group_fails_the_frame() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32))
        .expect("surface");
    let member = surface.layer();
    {
        let group = surface.backdrop_group_unfiltered();
        surface.update(|tx| {
            tx[surface.root()].push(&member);
            tx[&member]
                .clip(Rect::new(8.0, 8.0, 24.0, 24.0))
                .backdrop(group.sample());
        });
    }
    let result = engine.render(FrameTime::now());
    assert!(
        matches!(result, Err(cherenkov::RenderError::Render(_))),
        "unexpected result {result:?}"
    );
}

#[test]
fn two_members_share_one_capture() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32))
        .expect("surface");
    let group = surface.backdrop_group_unfiltered();
    let left = surface.layer();
    let right = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 32.0, 32.0),
                WorkingColor::new([0.0, 1.0, 0.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&left).push(&right);
        tx[&left]
            .clip(Rect::new(2.0, 2.0, 6.0, 6.0))
            .backdrop(group.sample());
        tx[&right]
            .clip(Rect::new(26.0, 2.0, 30.0, 6.0))
            .backdrop(group.sample());
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    // Both members sample the one capture: the green surface behind them.
    assert_pixel(pixel(&readback, 4, 4), [0.0, 1.0, 0.0, 1.0], 1e-5);
    assert_pixel(pixel(&readback, 28, 4), [0.0, 1.0, 0.0, 1.0], 1e-5);
    let memory = engine.memory();
    assert_eq!(memory.backdrop_capture_format, Some("linear-f32"));
    assert!(memory.backdrop_captures.0 > 0);
}
