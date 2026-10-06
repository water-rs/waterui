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
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
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
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
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
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let outer = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let inner = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
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
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
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

/// A member layer's own opacity fades the member as a whole, its
/// backdrop sample included: an unfiltered member's sample is its
/// canvas's bottom-most content and attenuates with it; a filtered
/// member's sample stays outside its isolation — the filter never
/// covers it — and still fades by the member's opacity, once (#1974).
#[test]
fn member_sample_is_attenuated_by_layer_opacity() {
    use filtrate::filters::ColorMatrix;

    // A red|blue step under a red↔blue-swapping group: the captured
    // sample differs from the sharp backdrop everywhere — at the clip's
    // antialiased rim too — so attenuating the sample and the member
    // clip's coverage are both observable. `filter` gives the member a
    // red↔blue swap or an identity filter.
    const SWAP: [f32; 12] = [
        0.0, 0.0, 1.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        1.0, 0.0, 0.0, 0.0,
    ];
    fn render(
        opacity: f32,
        filter: Option<[f32; 12]>,
        blend: cherenkov::BlendMode,
    ) -> cherenkov::Readback {
        let engine = engine();
        let surface = engine
            .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
            .expect("surface");
        let group = surface.backdrop_group(ColorMatrix(SWAP), cherenkov::CaptureScale::FULL);
        let member = surface.layer();
        let child = surface.layer();
        let member_filter = filter.map(|matrix| engine.filter(ColorMatrix(matrix)));
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
            tx[surface.root()].push(&member);
            tx[&member]
                .clip(RoundedRect::new(4.0, 4.0, 28.0, 28.0, 6.0))
                .opacity(opacity)
                .blend(blend)
                .backdrop(group.sample());
            if let Some(filter) = &member_filter {
                tx[&member].filter(filter.id());
            }
            tx[&member].push(&child);
            // The member's content reaches past the step at x = 16, so
            // covered pixels see the member's sample too.
            tx[&child].content(surface.record(|r| {
                r.fill(
                    Rect::new(12.0, 4.0, 28.0, 28.0),
                    WorkingColor::new([0.0, 0.7, 0.0, 1.0]),
                );
            }));
        });
        engine.render(FrameTime::now()).expect("render");
        surface.readback().expect("readback")
    }
    const IDENTITY: [f32; 12] = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0,
    ];

    // The member as a whole attenuates by its opacity once: inside its
    // clip, `half` = 0.5·`full` + 0.5·`dst` where `dst` is the sharp
    // backdrop pixel — true on the sample, on member content, and on
    // the clip's antialiased edge alike.
    let dst = |x: usize| {
        if x < 16 {
            [1.0, 0.0, 0.0, 1.0]
        } else {
            [0.0, 0.0, 1.0, 1.0]
        }
    };
    let faded = |full: &cherenkov::Readback, x: usize, y: usize| {
        std::array::from_fn(|c| 0.5f32.mul_add(pixel(full, x, y)[c], 0.5 * dst(x)[c]))
    };
    let normal = cherenkov::BlendMode::Normal;
    let full = render(1.0, None, normal);
    let half = render(0.5, None, normal);
    // Sample-only over red, content-covered at the step, covered
    // content, and the clip's corner edge.
    for &(x, y) in &[(8, 16), (16, 16), (24, 16), (5, 5)] {
        assert_pixel(pixel(&half, x, y), faded(&full, x, y), 1e-5);
    }
    // Outside the clip the surface is untouched.
    assert_pixel(pixel(&half, 1, 16), [1.0, 0.0, 0.0, 1.0], 1e-5);
    // A filtered member at the same pixels: the sample is attenuated —
    // never swapped — and fades with the member's content, so the rule
    // holds at covered pixels too; the member scope compositing sample
    // and content together is what keeps it true where content covers
    // the step.
    let filtered_full = render(1.0, Some(SWAP), normal);
    let filtered_half = render(0.5, Some(SWAP), normal);
    for &(x, y) in &[(8, 16), (16, 16), (24, 16), (5, 5)] {
        assert_pixel(
            pixel(&filtered_half, x, y),
            faded(&filtered_full, x, y),
            1e-5,
        );
    }
    // An identity-filtered member equals the unfiltered member: the
    // member scope's nested scopes change nothing — on the clip's rim,
    // where the swapped sample differs from the sharp backdrop and any
    // extra clip-edge coverage would show, and in the interior, where
    // the assertion also pins that the filter never covers the sample.
    let identity_full = render(1.0, Some(IDENTITY), normal);
    let identity_half = render(0.5, Some(IDENTITY), normal);
    for &(x, y) in &[(6, 5), (5, 5), (8, 16), (16, 16), (24, 16)] {
        assert_pixel(pixel(&identity_full, x, y), pixel(&full, x, y), 1e-5);
        assert_pixel(pixel(&identity_half, x, y), pixel(&half, x, y), 1e-5);
    }
    // A member's non-Normal blend applies to its sample, for both member
    // kinds: `Multiply` blends the whole member — sample and content —
    // against the backdrop, so only channels shared with the backdrop
    // survive and covered content fades to black.
    for filter in [None, Some(SWAP)] {
        let multi = render(1.0, filter, cherenkov::BlendMode::Multiply);
        let base = if filter.is_some() {
            &filtered_full
        } else {
            &full
        };
        // Sample-only pixel over red: Multiply blends S as S·D — the
        // swapped sample is blue over red, so only black survives.
        let s = pixel(base, 8, 16);
        assert_pixel(pixel(&multi, 8, 16), [s[0], 0.0, 0.0, 1.0], 1e-5);
        // Covered content over the step: green × blue = black.
        assert_pixel(pixel(&multi, 24, 16), [0.0, 0.0, 0.0, 1.0], 1e-5);
    }
}

#[test]
fn blurred_backdrop_matches_full_surface_blur_within_apron() {
    let engine = engine();
    let size = (32, 96);
    let surface = engine
        .surface(Offscreen::new(size, OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group(
        filtrate::filters::GaussianBlur::new(4.0f32),
        cherenkov::CaptureScale::FULL,
    );
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
        .surface(
            Bands::new(size, OffscreenFormat::LinearF32, move |band| {
                let pixels = match band.pixels {
                    BandPixels::F32(pixels) => pixels,
                    BandPixels::F16(_) => panic!("expected LinearF32 bands"),
                };
                sink.send((band.y, pixels.to_vec()))
                    .expect("stream pixels channel open");
            }),
            || {},
        )
        .expect("bands surface");
    let offscreen = engine
        .surface(Offscreen::new(size, OffscreenFormat::LinearF32), || {})
        .expect("offscreen surface");
    let build = |surface: &cherenkov::Surface<Raster>| {
        let group = surface.backdrop_group(
            filtrate::filters::GaussianBlur::new(4.0f32),
            cherenkov::CaptureScale::FULL,
        );
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
        .surface(Offscreen::new((32, 96), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group(
        filtrate::filters::GaussianBlur::new(5.0f32),
        cherenkov::CaptureScale::FULL,
    );
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
            .surface(
                Offscreen::new((32, height), OffscreenFormat::LinearF32),
                || {},
            )
            .expect("surface");
        let group = surface.backdrop_group(
            filtrate::filters::GaussianBlur::new(4.0f32),
            cherenkov::CaptureScale::FULL,
        );
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
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
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
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let member = surface.layer();
    {
        let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
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
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
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

#[test]
fn reduced_capture_resolves_and_samples_bilinearly() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let quarter = cherenkov::CaptureScale::new(0.25).expect("in range");
    let group = surface.backdrop_group_unfiltered(quarter);
    let glass = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 8.0, 32.0),
                WorkingColor::new([0.0, 1.0, 0.0, 1.0]),
            );
            r.fill(
                Rect::new(8.0, 0.0, 16.0, 32.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
            r.fill(
                Rect::new(16.0, 0.0, 24.0, 32.0),
                WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            );
            r.fill(
                Rect::new(24.0, 0.0, 32.0, 32.0),
                WorkingColor::new([0.0, 1.0, 0.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&glass);
        tx[&glass]
            .clip(Rect::new(8.0, 8.0, 24.0, 24.0))
            .backdrop(group.sample());
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    // Texel 3 covers device [12, 16) — red — and texel 4 [16, 20) — blue.
    // Pixel 15's centre lands at 15.5 / 4 = 3.875, 0.375 past texel 3's
    // centre; pixel 17's at 4.375, 0.875 past it.
    assert_pixel(pixel(&readback, 15, 16), [0.625, 0.0, 0.375, 1.0], 1e-5);
    assert_pixel(pixel(&readback, 17, 16), [0.125, 0.0, 0.875, 1.0], 1e-5);
    // The member's edge pixel 8 lands at 8.5 / 4 = 2.125, 0.375 short of
    // texel 2's centre: its taps are texel 1 — green [4, 8), outside the
    // member — and texel 2 — red [8, 12).
    assert_pixel(pixel(&readback, 8, 16), [0.625, 0.375, 0.0, 1.0], 1e-5);
    assert_pixel(pixel(&readback, 11, 16), [1.0, 0.0, 0.0, 1.0], 1e-5);
    // The opposite edge pixel 23 lands at 23.5 / 4 = 5.875, 0.375 past
    // texel 5's centre: its taps are texel 5 — blue [20, 24) — and
    // texel 6 — green [24, 28), outside the member.
    assert_pixel(pixel(&readback, 23, 16), [0.0, 0.375, 0.625, 1.0], 1e-5);
}

#[test]
fn reduced_capture_composes_clip_only_levels_before_resolving() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let half = cherenkov::CaptureScale::new(0.5).expect("in range");
    let group = surface.backdrop_group_unfiltered(half);
    let p = surface.layer();
    let member = surface.layer();
    surface.update(|tx| {
        // P's body is a clip-only level the capture flattens over the
        // surface at device resolution before the resolve.
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
    // Texels 4 and 5 cover the blue [8, 12); texel 6 [12, 14) is red.
    assert_pixel(pixel(&readback, 10, 16), [0.0, 0.0, 1.0, 1.0], 1e-5);
    assert_pixel(pixel(&readback, 12, 16), [0.75, 0.0, 0.25, 1.0], 1e-5);
    assert_pixel(pixel(&readback, 18, 16), [1.0, 0.0, 0.0, 1.0], 1e-5);
}

/// A reduced blurred member taller than several bands, over a ramp that
/// differs on every row: each band resolves the device rows under its
/// own texel window, and a band seam would show as a step.
#[test]
fn reduced_multi_band_capture_has_no_band_seams() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 96), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let quarter = cherenkov::CaptureScale::new(0.25).expect("in range");
    let group = surface.backdrop_group(filtrate::filters::GaussianBlur::new(1.5f32), quarter);
    let member = surface.layer();
    surface.update(|tx| {
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
        tx[&member]
            .clip(Rect::new(4.0, 22.0, 28.0, 74.0))
            .backdrop(group.sample());
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    let column: Vec<f32> = (22..74).map(|y| pixel(&readback, 16, y)[0]).collect();
    for (i, pair) in column.windows(2).enumerate() {
        assert!(
            pair[1] - pair[0] > -1.0e-5 && pair[1] - pair[0] < 0.05,
            "seam at row {}: {:?} -> {:?}",
            22 + i,
            pair[0],
            pair[1]
        );
    }
    // A linear ramp survives the box resolve, the symmetric blur and the
    // bilinear sample away from the region's clamped edges.
    let mid = pixel(&readback, 16, 48)[0];
    assert!((mid - 48.5 / 95.0 + 0.5 / 95.0).abs() < 0.01, "ramp {mid}");
}

#[test]
fn reduced_refraction_samples_the_displaced_point_on_the_capture_grid() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let half = cherenkov::CaptureScale::new(0.5).expect("in range");
    let group = surface.backdrop_group_unfiltered(half);
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 52.0, 64.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
            r.fill(
                Rect::new(52.0, 0.0, 64.0, 64.0),
                WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&member);
        tx[&member]
            .clip(Rect::new(8.0, 8.0, 56.0, 56.0))
            .backdrop(group.sample_with(cherenkov::Refraction {
                depth: 8.0,
                strength: 4.0,
            }));
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    // Pixel 54's centre is 1.5 inside the right edge: t = 1 − 1.5/8 =
    // 0.8125, so q = 54.5 − 4·t² = 51.859375 device pixels, 25.9296875 on
    // the grid — 0.4296875 past texel 25's centre, between texel 25 (red,
    // device [50, 52)) and texel 26 (blue, [52, 54)). The undisplaced
    // point would read only blue texels.
    assert_pixel(
        pixel(&readback, 54, 32),
        [0.570_312_5, 0.0, 0.429_687_5, 1.0],
        1e-5,
    );
    // The centre is past `depth` from every edge: q = p, at 16.25 on the
    // grid, between two red texels.
    assert_pixel(pixel(&readback, 32, 32), [1.0, 0.0, 0.0, 1.0], 1e-5);
}

#[test]
fn reduced_rim_lights_the_bilinear_sample_on_the_capture_grid() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let half = cherenkov::CaptureScale::new(0.5).expect("in range");
    let group = surface.backdrop_group_unfiltered(half);
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 54.0, 64.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
            r.fill(
                Rect::new(54.0, 0.0, 64.0, 64.0),
                WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&member);
        tx[&member]
            .clip(Rect::new(8.0, 8.0, 56.0, 56.0))
            .backdrop(group.sample_with(cherenkov::Rim {
                width: 4.0,
                color: [0.0, 1.0, 0.0, 1.0],
                gain: 2.0,
            }));
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    // Pixel 54's centre lands at 27.25 on the grid, 0.75 past texel 26's
    // centre: a quarter of texel 26 (red, device [52, 54)) and three
    // quarters of texel 27 (blue, [54, 56)). It is 1.5 inside the right
    // edge, so the rim adds 1 · 2 · (1 − 1.5/4)² = 0.78125 of green.
    assert_pixel(pixel(&readback, 54, 32), [0.25, 0.781_25, 0.75, 1.0], 1e-5);
    // Past the rim's width the sample is unlit.
    assert_pixel(pixel(&readback, 32, 32), [1.0, 0.0, 0.0, 1.0], 1e-5);
}
