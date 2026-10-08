//! Backdrop groups on the CPU backend: captures scheduled into bands.

#![cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;

use cherenkov::kurbo::{Circle, Ellipse, Rect, RoundedRect};
use cherenkov::{BackdropOuter, Draw, Engine, FrameTime, Offscreen, OffscreenFormat, WorkingColor};
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

/// A 33×33 surface, column 32 blue and the rest red, captured at full
/// scale into 2 levels and read through a `LevelRamp` at `level`.
fn odd_grid(level: f32) -> cherenkov::Readback {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((33, 33), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let spec = cherenkov::BackdropSpec::new(
        cherenkov::CaptureScale::FULL,
        cherenkov::CaptureLevels::new(2).expect("in range"),
    );
    let group = surface.backdrop_group_unfiltered(spec);
    let ramp = cherenkov::LevelRamp::new(1.0, level, level).expect("valid ramp");
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 32.0, 33.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
            r.fill(
                Rect::new(32.0, 0.0, 33.0, 33.0),
                WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&member);
        tx[&member]
            .clip(Rect::new(0.0, 0.0, 33.0, 33.0))
            .backdrop(group.sample_with(cherenkov::BackdropEffect::Level(ramp)));
    });
    engine.render(FrameTime::now()).expect("render");
    surface.readback().expect("readback")
}

/// The pyramid's odd-grid partial box as a pixel read: level 1 of the
/// 33-texel capture is 17 texels wide, its last a partial box over blue
/// column 32 alone, so pixel 31 (level-1 coordinate 15.25) mixes red
/// texel 15 and blue texel 16 at 0.25. A level below 0 reads level 0,
/// one above `n − 1` level `n − 1` (#1786).
#[test]
fn level_ramp_reads_odd_grid_partial_boxes_and_clamps_levels() {
    for level in [1.0, 7.0] {
        let readback = odd_grid(level);
        assert_pixel(pixel(&readback, 31, 16), [0.75, 0.0, 0.25, 1.0], 1e-5);
        // Pixel 32 lands at 15.75: three quarters of the blue texel.
        assert_pixel(pixel(&readback, 32, 16), [0.25, 0.0, 0.75, 1.0], 1e-5);
        // The bottom row's partial boxes and the corner hold the texels
        // present too.
        assert_pixel(pixel(&readback, 31, 32), [0.75, 0.0, 0.25, 1.0], 1e-5);
        assert_pixel(pixel(&readback, 32, 32), [0.25, 0.0, 0.75, 1.0], 1e-5);
        assert_pixel(pixel(&readback, 8, 32), [1.0, 0.0, 0.0, 1.0], 1e-5);
    }
    let below = odd_grid(-3.0);
    assert_pixel(pixel(&below, 31, 16), [1.0, 0.0, 0.0, 1.0], 1e-5);
    assert_pixel(pixel(&below, 32, 16), [0.0, 0.0, 1.0, 1.0], 1e-5);
}

/// Levels on a reduced capture: at scale 0.5 a 2-level pyramid's level
/// 1 texel covers device columns `[4j, 4j + 4)` — red below 16 — and
/// pixel `x` reads it at `(x + 0.5) / 4 − 0.5` (#1786).
#[test]
fn level_ramp_reads_levels_of_a_reduced_capture() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let spec = cherenkov::BackdropSpec::new(
        cherenkov::CaptureScale::new(0.5).expect("in range"),
        cherenkov::CaptureLevels::new(2).expect("in range"),
    );
    let group = surface.backdrop_group_unfiltered(spec);
    let ramp = cherenkov::LevelRamp::new(1.0, 1.0, 1.0).expect("valid ramp");
    let member = surface.layer();
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
            .clip(Rect::new(0.0, 0.0, 32.0, 32.0))
            .backdrop(group.sample_with(cherenkov::BackdropEffect::Level(ramp)));
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    // Pixel 13 lands at 2.875 and pixel 18 at 4.125: wholly red, wholly
    // blue texels. Pixel 15 at 3.375 and pixel 17 at 3.875 mix red
    // texel 3 and blue texel 4.
    assert_pixel(pixel(&readback, 13, 16), [1.0, 0.0, 0.0, 1.0], 1e-5);
    assert_pixel(pixel(&readback, 15, 16), [0.625, 0.0, 0.375, 1.0], 1e-5);
    assert_pixel(pixel(&readback, 17, 16), [0.125, 0.0, 0.875, 1.0], 1e-5);
    assert_pixel(pixel(&readback, 18, 16), [0.0, 0.0, 1.0, 1.0], 1e-5);
}

/// The stripes surface with the levelled member at x 600..840, alone —
/// its region then starts at capture texel 112 (device x 448) — or with
/// an adjacent plain member at x 0..596 that pulls the region to texel 0.
fn deep_level_member(with_plain: bool) -> cherenkov::Readback {
    let engine = engine();
    let surface = engine
        .surface(
            Offscreen::new((1024, 64), OffscreenFormat::LinearF32),
            || {},
        )
        .expect("surface");
    let spec = cherenkov::BackdropSpec::new(
        cherenkov::CaptureScale::new(0.25).expect("in range"),
        cherenkov::CaptureLevels::new(5).expect("in range"),
    );
    let group = surface.backdrop_group(filtrate::filters::GaussianBlur::new(2.0f32), spec);
    let ramp = cherenkov::LevelRamp::new(1.0, 4.0, 4.0).expect("valid ramp");
    let member = surface.layer();
    let plain = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 1024.0, 64.0),
                WorkingColor::new([0.0, 0.0, 0.0, 1.0]),
            );
            for i in (0..64u16).step_by(2) {
                let x = f64::from(i * 16);
                r.fill(
                    Rect::new(x, 0.0, x + 16.0, 64.0),
                    WorkingColor::new([1.0, 1.0, 1.0, 1.0]),
                );
            }
        }));
        tx[surface.root()].push(&member);
        tx[&member]
            .clip(Rect::new(600.0, 8.0, 840.0, 56.0))
            .backdrop(group.sample_with(cherenkov::BackdropEffect::Level(ramp)));
        if with_plain {
            tx[surface.root()].push(&plain);
            tx[&plain]
                .clip(Rect::new(0.0, 8.0, 596.0, 56.0))
                .backdrop(group.sample());
        }
    });
    engine.render(FrameTime::now()).expect("render");
    surface.readback().expect("readback")
}

/// A member's deep-level read does not depend on the other members: a
/// σ = 2 blur at scale 0.25 into 5 levels, read at level 4 over 16-px
/// stripes, renders the same in its own region away from the origin and
/// in the region a plain member pulls to the origin (#1786).
#[test]
fn deep_level_reads_are_independent_of_the_other_members() {
    let alone = deep_level_member(false);
    let shared = deep_level_member(true);
    for y in 8..56 {
        for x in 600..840 {
            assert_eq!(
                pixel(&alone, x, y),
                pixel(&shared, x, y),
                "member pixel ({x}, {y}) depends on the other member"
            );
        }
    }
}

/// Two groups anchored at the same layer sample one frozen copy taken at
/// the anchor's paint position: neither group sees what the other paints,
/// and each group runs its own chain (#2097).
#[test]
fn anchored_groups_sample_the_anchors_frozen_copy() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let anchor = surface.layer();
    let spec = || {
        cherenkov::BackdropSpec::new(cherenkov::CaptureScale::FULL, cherenkov::CaptureLevels::ONE)
            .anchor(anchor.id())
    };
    let group_a = surface.backdrop_group_unfiltered(spec());
    let group_b = surface.backdrop_group_unfiltered(spec());
    let a1 = surface.layer();
    let b1 = surface.layer();
    let a2 = surface.layer();
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
        // The anchor layer paints before every member, in the same canvas.
        tx[surface.root()].push(&anchor);
        tx[surface.root()].push(&a1).push(&b1).push(&a2);
        tx[&a1]
            .clip(Rect::new(4.0, 4.0, 20.0, 28.0))
            .backdrop(group_a.sample())
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(4.0, 4.0, 20.0, 28.0),
                    WorkingColor::new([0.0, 1.0, 0.0, 0.5]),
                );
            }));
        tx[&b1]
            .clip(Rect::new(12.0, 4.0, 28.0, 28.0))
            .backdrop(group_b.sample())
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(12.0, 4.0, 28.0, 28.0),
                    WorkingColor::new([1.0, 1.0, 1.0, 0.5]),
                );
            }));
        tx[&a2]
            .clip(Rect::new(20.0, 20.0, 28.0, 28.0))
            .backdrop(group_a.sample())
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(20.0, 20.0, 28.0, 28.0),
                    WorkingColor::new([0.0, 1.0, 0.0, 0.5]),
                );
            }));
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    // Inside A1 only: 50% green over the frozen red.
    assert_pixel(pixel(&readback, 6, 6), [0.5, 0.5, 0.0, 1.0], 1e-5);
    // Inside A1 and B1: B1's capture is the anchor's frozen blue — not
    // A1's composite — then 50% white over it. A first-member capture
    // would hold A1's [0.0, 0.5, 0.5] here and give [0.5, 0.75, 0.75].
    assert_pixel(pixel(&readback, 16, 16), [0.5, 0.5, 1.0, 1.0], 1e-5);
    // Inside B1 and A2: A2 samples the same frozen blue — not B1's
    // [0.5, 0.5, 1.0] composite — then 50% green over it.
    assert_pixel(pixel(&readback, 24, 24), [0.0, 0.5, 0.5, 1.0], 1e-5);
    let memory = engine.memory();
    assert_eq!(memory.backdrop_capture_format, Some("linear-f32"));
    assert!(memory.backdrop_captures.0 > 0, "the anchored capture ran");
}

/// A member that paints before its anchor fails the frame with the named
/// error — the anchored rule never falls back to the first-member capture.
#[test]
fn a_member_before_the_anchor_is_unsupported() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let anchor = surface.layer();
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::new(cherenkov::CaptureScale::FULL, cherenkov::CaptureLevels::ONE)
            .anchor(anchor.id()),
    );
    let member = surface.layer();
    surface.update(|tx| {
        // The member pushes first, the anchor second: the member paints
        // before the anchor in the same canvas.
        tx[surface.root()].push(&member);
        tx[&member]
            .clip(Rect::new(8.0, 8.0, 24.0, 24.0))
            .backdrop(group.sample());
        tx[surface.root()].push(&anchor);
    });
    let result = engine.render(FrameTime::now());
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(
                "backdrop-member-before-anchor"
            ))
        ),
        "unexpected result {result:?}"
    );
}

/// A member inside a compositing canvas of its own — a filtered layer's
/// subtree — fails the frame with the named error: it is outside the
/// anchor's canvas even though it paints after the anchor in the tree.
#[test]
fn a_member_outside_the_anchors_canvas_is_unsupported() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let anchor = surface.layer();
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::new(cherenkov::CaptureScale::FULL, cherenkov::CaptureLevels::ONE)
            .anchor(anchor.id()),
    );
    let filtered = surface.layer();
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&anchor);
        // A filter on `filtered` makes its subtree its own compositing
        // canvas — the member inside it is not in the anchor's canvas.
        tx[surface.root()].push(&filtered);
        let blur = engine.filter(filtrate::filters::GaussianBlur::new(1.0f32));
        tx[&filtered].filter(blur.id());
        tx[&filtered].push(&member);
        tx[&member]
            .clip(Rect::new(8.0, 8.0, 24.0, 24.0))
            .backdrop(group.sample());
    });
    let result = engine.render(FrameTime::now());
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(
                "backdrop-member-outside-anchor-canvas"
            ))
        ),
        "unexpected result {result:?}"
    );
}

/// The same layout without an anchor keeps the first-member rule: group
/// B's capture is taken at its own member's position and holds what A's
/// member already painted.
#[test]
fn unanchored_groups_keep_the_first_member_rule() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group_a = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let group_b = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let a1 = surface.layer();
    let b1 = surface.layer();
    let a2 = surface.layer();
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
        tx[surface.root()].push(&a1).push(&b1).push(&a2);
        tx[&a1]
            .clip(Rect::new(4.0, 4.0, 20.0, 28.0))
            .backdrop(group_a.sample())
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(4.0, 4.0, 20.0, 28.0),
                    WorkingColor::new([0.0, 1.0, 0.0, 0.5]),
                );
            }));
        tx[&b1]
            .clip(Rect::new(12.0, 4.0, 28.0, 28.0))
            .backdrop(group_b.sample())
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(12.0, 4.0, 28.0, 28.0),
                    WorkingColor::new([1.0, 1.0, 1.0, 0.5]),
                );
            }));
        tx[&a2]
            .clip(Rect::new(20.0, 20.0, 28.0, 28.0))
            .backdrop(group_a.sample())
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(20.0, 20.0, 28.0, 28.0),
                    WorkingColor::new([0.0, 1.0, 0.0, 0.5]),
                );
            }));
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    // Inside A1 and B1: B1 captured at its own paint position — A1's
    // [0.0, 0.5, 0.5] composite — then 50% white over it.
    assert_pixel(pixel(&readback, 16, 16), [0.5, 0.75, 0.75, 1.0], 1e-5);
    // Inside B1 and A2: A2 still samples group A's frozen copy from its
    // own first member — the blue — unchanged by anchoring absence.
    assert_pixel(pixel(&readback, 24, 24), [0.0, 0.5, 0.5, 1.0], 1e-5);
}

// `smin`, `rect_sdf`, `cover`, `circle_field` and `ownership` live in
// `cherenkov::testing`, shared with the GPU suite. `union_bridge` stays
// per-suite: its engine call shape differs (a fresh sync
// `Engine<Raster>` here vs `wait!`-driven `&Engine<Gpu>` in the GPU
// file).
use cherenkov::testing::{circle_field, cover, ownership, rect_field, rect_sdf, smin};

/// Two bridge members 6 px apart on the shared field: `a` owns the left
/// half, `b` the right (`k = 20` joins a gap below `k/2`). Returns the
/// readback for the given member opacities and member order.
fn union_bridge(a_opacity: f32, b_opacity: f32, swap: bool) -> cherenkov::Readback {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::FULL.union(cherenkov::BackdropUnion::new(20.0).expect("union")),
    );
    let a = surface.layer();
    let b = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 48.0, 32.0),
                WorkingColor::new([1.0, 0.0, 0.0, 0.5]),
            );
        }));
        let (first, second) = if swap { (&b, &a) } else { (&a, &b) };
        tx[surface.root()].push(first);
        tx[surface.root()].push(second);
        tx[&a]
            .clip(Rect::new(4.0, 8.0, 20.0, 24.0))
            .backdrop(group.sample())
            .opacity(a_opacity);
        tx[&b]
            .clip(Rect::new(26.0, 8.0, 42.0, 24.0))
            .backdrop(group.sample())
            .opacity(b_opacity);
    });
    engine.render(FrameTime::now()).expect("render");
    surface.readback().expect("readback")
}

#[test]
fn union_members_composite_identically_in_any_order() {
    let ab = union_bridge(0.5, 0.5, false);
    let ba = union_bridge(0.5, 0.5, true);
    assert_eq!(
        ab.pixels, ba.pixels,
        "the union composite depends on member order"
    );
}

#[test]
fn union_members_partition_the_bridge_alpha() {
    // Each member at opacity 0.5 over a 0.5-alpha capture: member i's
    // deposit above the root is `w_i · c · 0.5 · 0.5`. The ownership
    // weights sum to 1, so the two deposits sum to `0.25 · c` — exactly
    // what one member covering the field deposits.
    let only_a = union_bridge(0.5, 0.0, false);
    let only_b = union_bridge(0.0, 0.5, false);
    let both = union_bridge(0.5, 0.5, false);
    let a_clip = Rect::new(4.0, 8.0, 20.0, 24.0);
    let b_clip = Rect::new(26.0, 8.0, 42.0, 24.0);
    let bg = 0.5;
    for row in 10..22usize {
        for col in 18..30usize {
            let (px, py) = (
                f32::from(u16::try_from(col).unwrap()) + 0.5,
                f32::from(u16::try_from(row).unwrap()) + 0.5,
            );
            let dist_a = rect_sdf(px, py, a_clip);
            let dist_b = rect_sdf(px, py, b_clip);
            let field = smin(dist_a.min(dist_b), dist_a.max(dist_b), 20.0);
            let cov = cover(field);
            let weight_a = (0.5 + (dist_b - dist_a) / 2.0).clamp(0.0, 1.0);
            let weight_b = 1.0 - weight_a;
            let single = 0.125 * cov;
            let deposit_a = pixel(&only_a, col, row)[3] - bg;
            let deposit_b = pixel(&only_b, col, row)[3] - bg;
            assert!(
                single.mul_add(-weight_a, deposit_a).abs() <= 2e-3,
                "pixel ({col}, {row}): member a's deposit {deposit_a} != {single}·w_a"
            );
            assert!(
                single.mul_add(-weight_b, deposit_b).abs() <= 2e-3,
                "pixel ({col}, {row}): member b's deposit {deposit_b} != {single}·w_b"
            );
            assert!(
                (deposit_a + deposit_b - single).abs() <= 2e-3,
                "pixel ({col}, {row}): deposits {deposit_a} + {deposit_b} != a single member's {single}"
            );
            // Never drawn twice: both members together deposit no more
            // than a single member; never left open: no less than the
            // larger share.
            let both_a = pixel(&both, col, row)[3];
            assert!(
                both_a <= bg + single + 2e-3,
                "pixel ({col}, {row}): alpha {both_a} above a single member's {single}"
            );
            assert!(
                both_a >= single.mul_add(weight_a.max(weight_b), bg) - 2e-3,
                "pixel ({col}, {row}): alpha {both_a} below the seam's larger share"
            );
        }
    }
}

/// Circle members of a union group at the given opacities — `circles` is
/// `(cx, cy, r)` in paint order. The `union_bridge` counterpart for the
/// ownership-boundary tests, where non-parallel member normals give the
/// seam a real slope.
fn union_circles(circles: &[(f64, f64, f64)], opacities: &[f32]) -> cherenkov::Readback {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((48, 40), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::FULL.union(cherenkov::BackdropUnion::new(20.0).expect("union")),
    );
    let members: Vec<_> = circles.iter().map(|_| surface.layer()).collect();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 48.0, 40.0),
                WorkingColor::new([1.0, 0.0, 0.0, 0.5]),
            );
        }));
        for (member, (&(cx, cy, radius), &opacity)) in
            members.iter().zip(circles.iter().zip(opacities.iter()))
        {
            tx[surface.root()].push(member);
            tx[member]
                .clip(Circle::new((cx, cy), radius))
                .backdrop(group.sample())
                .opacity(opacity);
        }
    });
    engine.render(FrameTime::now()).expect("render");
    surface.readback().expect("readback")
}

#[test]
fn the_seam_weights_ramp_across_one_pixel() {
    const CIRCLES: &[(f64, f64, f64)] = &[(12.3, 16.0, 16.0), (32.3, 16.0, 16.0)];
    let only_a = union_circles(CIRCLES, &[0.5, 0.0]);
    let only_b = union_circles(CIRCLES, &[0.0, 0.5]);
    #[expect(clippy::cast_precision_loss, reason = "pixel coords stay small")]
    let fields = |col: usize, row: usize| {
        let px = col as f32 + 0.5;
        let py = row as f32 + 0.5;
        let (da, ga) = circle_field(px, py, 12.3, 16.0, 16.0);
        let (db, gb) = circle_field(px, py, 32.3, 16.0, 16.0);
        [(da, ga), (db, gb)]
    };
    for row in 8..24usize {
        for col in 20..25usize {
            let members = fields(col, row);
            let field = smin(
                members[0].0.min(members[1].0),
                members[0].0.max(members[1].0),
                20.0,
            );
            let cov = cover(field);
            let wa = ownership(&members, 0);
            let wb = ownership(&members, 1);
            let single = 0.125 * cov;
            let deposit_a = pixel(&only_a, col, row)[3] - 0.5;
            let deposit_b = pixel(&only_b, col, row)[3] - 0.5;
            assert!(
                single.mul_add(-wa, deposit_a).abs() <= 2e-3,
                "pixel ({col}, {row}): member a's deposit {deposit_a} != {single}·w_a={wa}"
            );
            assert!(
                single.mul_add(-wb, deposit_b).abs() <= 2e-3,
                "pixel ({col}, {row}): member b's deposit {deposit_b} != {single}·w_b={wb}"
            );
            assert!(
                (deposit_a + deposit_b - single).abs() <= 2e-3,
                "pixel ({col}, {row}): deposits {deposit_a} + {deposit_b} != {single}"
            );
        }
    }
    // The ramp is one pixel wide: column 21 saturates to member a and
    // column 23 to member b; the straddling column 22 is fractional.
    // Read the rendered weight — member a's deposit share of the
    // coverage — not the helper's.
    let wa = |col: usize| {
        let members = fields(col, 16);
        let field = smin(
            members[0].0.min(members[1].0),
            members[0].0.max(members[1].0),
            20.0,
        );
        let single = 0.125 * cover(field);
        (pixel(&only_a, col, 16)[3] - 0.5) / single
    };
    assert!(wa(21) > 0.95, "w_a at column 21: {}", wa(21));
    assert!(wa(23) < 0.05, "w_a at column 23: {}", wa(23));
    let mid = wa(22);
    assert!(
        (0.05..0.95).contains(&mid),
        "the seam column carries a fractional weight: {mid}"
    );
}

#[test]
fn the_three_weights_sum_to_one_near_a_triple_point() {
    // Centres on a ~equilateral triangle, r = 12: the circumcentre at
    // about (24, 19.7) sits inside all three members, the pixel
    // neighbourhood where all three weights are of order 1/3.
    const CIRCLES: &[(f64, f64, f64)] =
        &[(24.0, 13.04, 12.0), (29.0, 22.5, 12.0), (19.0, 22.5, 12.0)];
    let mut alones = Vec::new();
    for i in 0..3 {
        let mut opacities = [0.0f32; 3];
        opacities[i] = 0.5;
        alones.push(union_circles(CIRCLES, &opacities));
    }
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "pixel coords and circle dims stay small"
    )]
    for row in 14..26usize {
        for col in 20..29usize {
            let px = col as f32 + 0.5;
            let py = row as f32 + 0.5;
            let members: [(f32, [f32; 2]); 3] = std::array::from_fn(|i| {
                let (cx, cy, r) = CIRCLES[i];
                circle_field(px, py, cx as f32, cy as f32, r as f32)
            });
            let mut ds = members.map(|(d, _)| d);
            ds.sort_by(f32::total_cmp);
            let field = smin(smin(ds[0], ds[1], 20.0), ds[2], 20.0);
            let cov = cover(field);
            let single = 0.125 * cov;
            let mut dsum = 0.0f32;
            for (i, alone) in alones.iter().enumerate() {
                let w = ownership(&members, i);
                let deposit = pixel(alone, col, row)[3] - 0.5;
                dsum += deposit;
                assert!(
                    single.mul_add(-w, deposit).abs() <= 3e-3,
                    "pixel ({col}, {row}): member {i}'s deposit {deposit} != {single}·w (w = {w})"
                );
            }
            assert!(
                (dsum - single).abs() <= 3e-3,
                "deposits at ({col}, {row}) sum to {dsum} != {single}"
            );
        }
    }
}

#[test]
fn outer_extent_draws_exactly_the_band_without_a_union() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 48.0, 32.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&member);
        // A solid-white effect makes the composite's coverage readable
        // as the pixel's whiteness: clip plus a 4.5 px band.
        tx[&member].clip(Rect::new(16.0, 8.0, 32.0, 24.0)).backdrop(
            group
                .sample_with(cherenkov::ColorMatrix([
                    0.0, 0.0, 0.0, 1.0, //
                    0.0, 0.0, 0.0, 1.0, //
                    0.0, 0.0, 0.0, 1.0,
                ]))
                .outer(BackdropOuter::new(4.5).expect("valid")),
        );
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    // Inside the clip and inside the band the white is full.
    assert_pixel(pixel(&readback, 24, 16), [1.0, 1.0, 1.0, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 34, 16), [1.0, 1.0, 1.0, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 35, 16), [1.0, 1.0, 1.0, 1.0], 1e-3);
    // At the band's outer edge (`d = outer` at pixel centre) the
    // coverage is half — half white over the red.
    assert_pixel(pixel(&readback, 36, 16), [1.0, 0.5, 0.5, 1.0], 1e-3);
    // Beyond `outer` the root's red survives untouched.
    assert_pixel(pixel(&readback, 37, 16), [1.0, 0.0, 0.0, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 8, 16), [1.0, 0.0, 0.0, 1.0], 1e-3);
}

#[test]
fn union_member_with_a_path_clip_is_unsupported() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::FULL.union(cherenkov::BackdropUnion::new(20.0).expect("union")),
    );
    let rect = surface.layer();
    let path = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&rect);
        tx[&rect]
            .clip(Rect::new(4.0, 8.0, 20.0, 24.0))
            .backdrop(group.sample());
        tx[surface.root()].push(&path);
        tx[&path]
            .clip(cherenkov::ShapeData::Path {
                elements: vec![
                    cherenkov::kurbo::PathEl::MoveTo((26.0, 8.0).into()),
                    cherenkov::kurbo::PathEl::LineTo((42.0, 8.0).into()),
                    cherenkov::kurbo::PathEl::LineTo((42.0, 24.0).into()),
                    cherenkov::kurbo::PathEl::ClosePath,
                ]
                .into(),
                rule: cherenkov::FillRule::NonZero,
            })
            .backdrop(group.sample());
    });
    let result = engine.render(FrameTime::now());
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(name)) if name == "backdrop-effect-sdf-path"
        ),
        "unexpected result {result:?}"
    );
}

/// Behavioural: a member's shape change re-renders the neighbour's
/// bridge pixels across frames — a changed surface lowers again in
/// full (the engine has no damage rects for this to leak through).
#[test]
fn a_member_shape_change_redraws_the_neighbours_bridge() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::FULL.union(cherenkov::BackdropUnion::new(20.0).expect("union")),
    );
    let a = surface.layer();
    let b = surface.layer();
    let scene = |a_right: f64| {
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|r| {
                r.fill(
                    Rect::new(0.0, 0.0, 48.0, 32.0),
                    WorkingColor::new([0.25, 0.0, 0.0, 1.0]),
                );
            }));
            tx[surface.root()].push(&a);
            tx[&a]
                .clip(Rect::new(4.0, 8.0, a_right, 24.0))
                .backdrop(group.sample_with(cherenkov::Rim {
                    width: 4.0,
                    color: [0.0, 1.0, 0.0, 1.0],
                    gain: 2.0,
                }));
            tx[surface.root()].push(&b);
            // `b` reads the union field: a highlight just inside its
            // edge, and the bridge coverage, move when `a` moves — the
            // whole surface re-renders, not a damage rect.
            tx[&b]
                .clip(Rect::new(26.0, 8.0, 42.0, 24.0))
                .backdrop(group.sample_with(cherenkov::Rim {
                    width: 4.0,
                    color: [0.0, 1.0, 0.0, 1.0],
                    gain: 2.0,
                }));
        });
    };
    scene(14.0);
    engine.render(FrameTime::now()).expect("render");
    let first = surface.readback().expect("readback");
    scene(20.0);
    engine.render(FrameTime::now()).expect("second render");
    let second = surface.readback().expect("readback");
    assert_ne!(
        first.pixels, second.pixels,
        "the neighbour's bridge pixels stayed stale across frames"
    );
}

/// `union_circles` with a per-member blend mode — an isolating member
/// must still reach the union zone outside its own clip.
fn union_circles_blended(
    circles: &[(f64, f64, f64)],
    opacities: &[f32],
    blends: &[cherenkov::BlendMode],
) -> cherenkov::Readback {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((48, 40), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::FULL.union(cherenkov::BackdropUnion::new(20.0).expect("union")),
    );
    let members: Vec<_> = circles.iter().map(|_| surface.layer()).collect();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 48.0, 40.0),
                WorkingColor::new([1.0, 0.0, 0.0, 0.5]),
            );
        }));
        for (member, ((&(cx, cy, radius), &opacity), &blend)) in members
            .iter()
            .zip(circles.iter().zip(opacities.iter()).zip(blends.iter()))
        {
            tx[surface.root()].push(member);
            tx[member]
                .clip(Circle::new((cx, cy), radius))
                .backdrop(group.sample())
                .blend(blend)
                .opacity(opacity);
        }
    });
    engine.render(FrameTime::now()).expect("render");
    surface.readback().expect("readback")
}

/// An isolating union member — here blending `Screen` — must still
/// composite into the union zone outside its own clip: the bridge and
/// the band belong to the field, not to the member shape.
#[test]
fn a_screen_member_bridges_outside_its_clip() {
    const CIRCLES: &[(f64, f64, f64)] = &[(12.3, 16.0, 16.0), (32.3, 16.0, 16.0)];
    let only_a = union_circles_blended(
        CIRCLES,
        &[0.5, 0.0],
        &[cherenkov::BlendMode::Screen, cherenkov::BlendMode::Normal],
    );
    #[expect(clippy::cast_precision_loss, reason = "pixel coords stay small")]
    let members_at = |col: usize, row: usize| {
        let px = col as f32 + 0.5;
        let py = row as f32 + 0.5;
        [
            circle_field(px, py, 12.3, 16.0, 16.0),
            circle_field(px, py, 32.3, 16.0, 16.0),
        ]
    };
    // Screen composites with SrcOver's alpha, so a's alpha deposit is
    // `0.125·w_a·cov` as under a normal blend — including where the
    // union zone lies outside a's clip (the fan below the overlap). The
    // assert region is the fully-covered zone outside a's clip, where
    // `field < -0.5` keeps the coverage model exact: a member isolated
    // under its own clip would deposit 0 there.
    let mut outside_clip = false;
    for row in 20..38usize {
        for col in 14..30usize {
            let members = members_at(col, row);
            let field = smin(
                members[0].0.min(members[1].0),
                members[0].0.max(members[1].0),
                20.0,
            );
            if !(members[0].0 > 0.5 && field < -0.5) {
                continue;
            }
            outside_clip = true;
            let wa = ownership(&members, 0);
            let want = 0.125 * wa;
            let got = pixel(&only_a, col, row)[3] - 0.5;
            assert!(
                (got - want).abs() <= 3e-3,
                "pixel ({col}, {row}): deposit {got} != {want} (w_a = {wa})"
            );
        }
    }
    assert!(
        outside_clip,
        "no pixel outside member a's clip carried a's deposit"
    );
}

/// In a backdrop group with no union field, a member carrying an outer
/// band and a plain member mix: the band member draws its band and the
/// plain member is not inflated.
#[test]
fn a_plain_member_in_a_mixed_group_is_not_inflated() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let banded = surface.layer();
    let plain = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 48.0, 32.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&banded);
        tx[surface.root()].push(&plain);
        // Solid-white effects make coverage readable as whiteness.
        tx[&banded].clip(Rect::new(4.0, 8.0, 20.0, 24.0)).backdrop(
            group
                .sample_with(cherenkov::ColorMatrix([
                    0.0, 0.0, 0.0, 1.0, //
                    0.0, 0.0, 0.0, 1.0, //
                    0.0, 0.0, 0.0, 1.0,
                ]))
                .outer(BackdropOuter::new(4.5).expect("valid")),
        );
        tx[&plain]
            .clip(Rect::new(26.0, 8.0, 42.0, 24.0))
            .backdrop(group.sample_with(cherenkov::ColorMatrix([
                0.0, 0.0, 0.0, 1.0, //
                0.0, 0.0, 0.0, 1.0, //
                0.0, 0.0, 0.0, 1.0,
            ])));
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    // The banded member fills its band (clip x1 = 20, band to 24.5).
    assert_pixel(pixel(&readback, 22, 16), [1.0, 1.0, 1.0, 1.0], 1e-3);
    // The plain member draws only its clip — no band at (44, 16).
    assert_pixel(pixel(&readback, 40, 16), [1.0, 1.0, 1.0, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 44, 16), [1.0, 0.0, 0.0, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 46, 16), [1.0, 0.0, 0.0, 1.0], 1e-3);
}

/// Union members carrying an outer band keep ownership weights summing
/// to one at the field's outer antialiased edge. The members are rects:
/// a second member offset so their top corners nearly tie keeps band
/// coverage and a shared ownership weight on pixels out to the field's
/// coverage wall — bound-distance past `r(n) + outer − 0.5` — so a
/// draw bound more than half a pixel short would clip the outermost
/// sampled pixels a member still owns. (No owned pixel can reach the
/// pad's last pixel: the field's coverage dies at `r(n) + outer + 0.5`,
/// before the `+1.5` bound's margin ends.)
#[test]
fn the_seam_weights_sum_to_one_at_the_outer_edge() {
    const RECTS: [Rect; 2] = [
        Rect::new(4.0, 14.0, 10.0, 28.0),
        Rect::new(16.0, 14.4, 24.0, 28.4),
    ];
    const OUTER: f32 = 4.0;
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((48, 40), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::FULL.union(cherenkov::BackdropUnion::new(20.0).expect("union")),
    );
    let members: Vec<_> = RECTS.iter().map(|_| surface.layer()).collect();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 48.0, 40.0),
                WorkingColor::new([1.0, 0.0, 0.0, 0.5]),
            );
        }));
        for (member, rect) in members.iter().zip(RECTS.iter()) {
            tx[surface.root()].push(member);
            tx[member].clip(*rect).backdrop(
                group
                    .sample()
                    .outer(BackdropOuter::new(OUTER).expect("valid")),
            );
        }
    });
    let render = |opacities: &[f32]| {
        surface.update(|tx| {
            for (member, &opacity) in members.iter().zip(opacities.iter()) {
                tx[member].opacity(opacity);
            }
        });
        engine.render(FrameTime::now()).expect("render");
        surface.readback().expect("readback")
    };
    let only_a = render(&[0.5, 0.0]);
    let only_b = render(&[0.0, 0.5]);
    let mut edge_pixels = 0usize;
    let mut pad_pixels = 0usize;
    #[expect(clippy::cast_precision_loss, reason = "pixel coords stay small")]
    for row in 4..38usize {
        for col in 4..44usize {
            let px = col as f32 + 0.5;
            let py = row as f32 + 0.5;
            let members = [rect_field(px, py, RECTS[0]), rect_field(px, py, RECTS[1])];
            let field = smin(
                members[0].0.min(members[1].0),
                members[0].0.max(members[1].0),
                20.0,
            );
            // The band's coverage is `field < outer`: at its outer AA
            // edge coverage is fractional.
            let cov = cover(field - OUTER);
            if !(0.02..0.98).contains(&cov) {
                continue;
            }
            edge_pixels += 1;
            let wa = ownership(&members, 0);
            let wb = ownership(&members, 1);
            let deposit_a = pixel(&only_a, col, row)[3] - 0.5;
            let deposit_b = pixel(&only_b, col, row)[3] - 0.5;
            // The ownership weights partition the actual deposit: each
            // member carries its share of the total, so a member whose
            // draw bound clipped the band would leave the sum short.
            let sum = deposit_a + deposit_b;
            assert!(
                wa.mul_add(-sum, deposit_a).abs() <= 3e-3,
                "pixel ({col}, {row}): member a's deposit {deposit_a} != {wa}·{sum}"
            );
            assert!(
                wb.mul_add(-sum, deposit_b).abs() <= 3e-3,
                "pixel ({col}, {row}): member b's deposit {deposit_b} != {wb}·{sum}"
            );
            // Well inside the edge's ramp the deposit is unmistakably
            // nonzero — a draw bound one pixel short would cut these.
            if cov > 0.3 {
                assert!(
                    sum > 1e-3,
                    "pixel ({col}, {row}): no deposit at the outer edge (cov = {cov})"
                );
            }
            // The owned pixels nearest the draw bound: an owner reaches
            // `r(n) + outer − 0.5` past its bounds before the field's
            // coverage wall — bound-distance in (9, 10.5] — where a
            // bound more than half a pixel short would clip it.
            for (i, rect) in RECTS.iter().enumerate() {
                if ownership(&members, i) < 0.05 {
                    continue;
                }
                let dist = rect_field(px, py, *rect).0.max(0.0);
                if (9.0..=10.5).contains(&dist) {
                    let deposit = pixel(if i == 0 { &only_a } else { &only_b }, col, row)[3] - 0.5;
                    pad_pixels += 1;
                    assert!(
                        deposit > 1e-3,
                        "pixel ({col}, {row}): member {i}'s deposit {deposit} was clipped by the draw bound"
                    );
                }
            }
        }
    }
    assert!(edge_pixels > 0, "the outer antialiased edge had no pixels");
    assert!(
        pad_pixels > 0,
        "no pixel lay in the draw bound's last half pixel"
    );
}

/// A destructive-blend union member under a masked ancestor: the member
/// clip (`RoundedRect`, unmergeable against the ancestor's path mask)
/// isolates, so the member's sample must composite under the ancestor
/// clip the scope applies — not under a clip value captured before.
/// `SrcOut` drops the backdrop under the member, leaving `0.5·src` —
/// an instance clipped against mask texel (0, 0) would deposit nothing.
/// Asserts the same pixels as the GPU twin
/// (`a_destructive_member_under_a_masked_ancestor_samples_its_union`).
#[test]
fn a_destructive_member_under_a_masked_ancestor_samples_its_union() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((48, 40), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::FULL.union(cherenkov::BackdropUnion::new(20.0).expect("union")),
    );
    let decoy = surface.layer();
    let ancestor = surface.layer();
    let a = surface.layer();
    let b = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 48.0, 40.0),
                WorkingColor::new([1.0, 0.0, 0.0, 0.5]),
            );
        }));
        tx[surface.root()].push(&decoy);
        // A decoy path clip lands in the mask atlas first: without it
        // the ancestor's triangle would sit at texel (0, 0), exactly
        // where an unpatched instance reads — the test would pass while
        // the defect it checks for is still live.
        tx[&decoy]
            .clip(cherenkov::ShapeData::Path {
                elements: vec![
                    cherenkov::kurbo::PathEl::MoveTo((0.0, 30.0).into()),
                    cherenkov::kurbo::PathEl::LineTo((6.0, 40.0).into()),
                    cherenkov::kurbo::PathEl::LineTo((0.0, 40.0).into()),
                    cherenkov::kurbo::PathEl::ClosePath,
                ]
                .into(),
                rule: cherenkov::FillRule::NonZero,
            })
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(0.0, 30.0, 6.0, 40.0),
                    WorkingColor::new([0.0, 1.0, 0.0, 1.0]),
                );
            }));
        tx[surface.root()].push(&ancestor);
        // A triangle clip masks: the ancestor's clip has no analytic
        // edge a member clip could merge with.
        tx[&ancestor].clip(cherenkov::ShapeData::Path {
            elements: vec![
                cherenkov::kurbo::PathEl::MoveTo((0.0, 0.0).into()),
                cherenkov::kurbo::PathEl::LineTo((48.0, 0.0).into()),
                cherenkov::kurbo::PathEl::LineTo((48.0, 40.0).into()),
                cherenkov::kurbo::PathEl::ClosePath,
            ]
            .into(),
            rule: cherenkov::FillRule::NonZero,
        });
        tx[&ancestor].push(&a);
        tx[&ancestor].push(&b);
        tx[&a]
            .clip(RoundedRect::new(30.0, 10.0, 40.0, 20.0, 3.0))
            .backdrop(group.sample())
            .blend(cherenkov::BlendMode::SrcOut)
            .opacity(0.5f32);
        tx[&b]
            .clip(RoundedRect::new(36.0, 18.0, 46.0, 28.0, 3.0))
            .backdrop(group.sample())
            .opacity(0.5f32);
    });
    engine.render(FrameTime::now()).expect("render");
    let readback = surface.readback().expect("readback");
    // Deep in member a's zone (w_a = 1, coverage 1) the member's sample
    // deposits the captured red at strength: over the `0.5`-alpha
    // backdrop the pixel reads `0.5 + 0.25·0.5 = 0.625`. An instance
    // clipped against mask texel (0, 0) would deposit nothing here.
    assert_pixel(pixel(&readback, 32, 14), [0.625, 0.0, 0.0, 0.625], 3e-3);
    // At the seam midpoint (35.5, 20.5) the weights split: w_a = 0.5.
    assert_pixel(pixel(&readback, 35, 20), [0.5625, 0.0, 0.0, 0.5625], 4e-3);
}

/// Union members whose layer-level isolation opens a scratch — `SrcOut`
/// blend, `Screen` blend, opacity over overlapping content, filter with
/// opacity — must agree under a path-clipped (masked) ancestor and under
/// none. A sample emitted inside the scratch that carried the ancestor
/// clip would read the mask atlas at texel (0, 0) with no patch; the
/// decoy's triangle lands there first, so the ancestor's own triangle
/// sits elsewhere and the wrong read loses the deposit. Asserts the
/// same pixels as the GPU twin
/// (`isolated_members_under_a_masked_ancestor_match_unclipped`). The
/// engine is per-render: a raster cache an earlier render leaves would
/// paper over an unpatched mask reference.
#[test]
fn isolated_members_under_a_masked_ancestor_match_unclipped() {
    let render = |masked: bool, mode: u32| {
        let engine = engine();
        let surface = engine
            .surface(Offscreen::new((48, 40), OffscreenFormat::LinearF32), || {})
            .expect("surface");
        let identity = engine.filter(filtrate::filters::ColorMatrix([
            1.0_f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
        ]));
        let group = surface.backdrop_group_unfiltered(
            cherenkov::BackdropSpec::FULL
                .union(cherenkov::BackdropUnion::new(20.0).expect("union")),
        );
        let decoy = surface.layer();
        let ancestor = surface.layer();
        let a = surface.layer();
        let b = surface.layer();
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|r| {
                r.fill(
                    Rect::new(0.0, 0.0, 48.0, 40.0),
                    WorkingColor::new([1.0, 0.0, 0.0, 0.5]),
                );
            }));
            tx[surface.root()].push(&decoy);
            // The decoy's mask keeps the ancestor's off texel (0, 0).
            tx[&decoy]
                .clip(cherenkov::ShapeData::Path {
                    elements: vec![
                        cherenkov::kurbo::PathEl::MoveTo((0.0, 30.0).into()),
                        cherenkov::kurbo::PathEl::LineTo((6.0, 40.0).into()),
                        cherenkov::kurbo::PathEl::LineTo((0.0, 40.0).into()),
                        cherenkov::kurbo::PathEl::ClosePath,
                    ]
                    .into(),
                    rule: cherenkov::FillRule::NonZero,
                })
                .content(surface.record(|r| {
                    r.fill(
                        Rect::new(0.0, 30.0, 6.0, 40.0),
                        WorkingColor::new([0.0, 1.0, 0.0, 1.0]),
                    );
                }));
            tx[surface.root()].push(&ancestor);
            if masked {
                // A triangle clip masks: the ancestor's clip has no
                // analytic edge a member clip could merge with.
                tx[&ancestor].clip(cherenkov::ShapeData::Path {
                    elements: vec![
                        cherenkov::kurbo::PathEl::MoveTo((0.0, 0.0).into()),
                        cherenkov::kurbo::PathEl::LineTo((48.0, 0.0).into()),
                        cherenkov::kurbo::PathEl::LineTo((48.0, 40.0).into()),
                        cherenkov::kurbo::PathEl::ClosePath,
                    ]
                    .into(),
                    rule: cherenkov::FillRule::NonZero,
                });
            }
            tx[&ancestor].push(&a);
            tx[&ancestor].push(&b);
            {
                let la = &mut tx[&a];
                la.clip(RoundedRect::new(30.0, 10.0, 40.0, 20.0, 3.0))
                    .backdrop(group.sample());
                match mode {
                    // `SrcOut` isolates destructively, `Screen`
                    // non-destructively, low opacity and a filter open a
                    // scratch for the body.
                    0 => {
                        la.blend(cherenkov::BlendMode::SrcOut).opacity(0.5f32);
                    }
                    1 => {
                        la.blend(cherenkov::BlendMode::Screen);
                    }
                    2 => {
                        la.opacity(0.5f32).content(surface.record(|r| {
                            r.fill(
                                Rect::new(30.0, 10.0, 40.0, 20.0),
                                WorkingColor::new([0.0, 0.0, 1.0, 0.25]),
                            );
                        }));
                    }
                    _ => {
                        la.opacity(0.5f32).filter(identity.id());
                    }
                }
            }
            tx[&b]
                .clip(RoundedRect::new(36.0, 18.0, 46.0, 28.0, 3.0))
                .backdrop(group.sample())
                .opacity(0.5f32);
        });
        engine.render(FrameTime::now()).expect("render");
        surface.readback().expect("readback")
    };
    for mode in 0..4 {
        let plain = render(false, mode);
        let masked = render(true, mode);
        // Deep inside the triangle the ancestor's coverage is 1, so the
        // masked render must read like the unclipped one.
        for (x, y) in [(32usize, 14usize), (35, 20), (38, 12), (33, 11)] {
            assert_pixel(pixel(&masked, x, y), pixel(&plain, x, y), 3e-3);
        }
    }
}

/// Members whose clip's analytic SDF degenerates cannot fold into the
/// union field and reject by name — a zero-radius circle and an ellipse
/// collapsed in either radius.
#[test]
fn a_degenerate_union_member_is_unsupported() {
    for clip in [
        cherenkov::ShapeData::Circle(Circle::new((16.0, 16.0), 0.0)),
        cherenkov::ShapeData::Ellipse(Ellipse::new((16.0, 16.0), (4.0, 0.0), 0.0)),
        cherenkov::ShapeData::Ellipse(Ellipse::new((16.0, 16.0), (0.0, 4.0), 0.0)),
    ] {
        let engine = engine();
        let surface = engine
            .surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {})
            .expect("surface");
        let group = surface.backdrop_group_unfiltered(
            cherenkov::BackdropSpec::FULL
                .union(cherenkov::BackdropUnion::new(20.0).expect("union")),
        );
        let ok = surface.layer();
        let degenerate = surface.layer();
        surface.update(|tx| {
            tx[surface.root()].push(&ok);
            tx[&ok]
                .clip(Rect::new(4.0, 8.0, 20.0, 24.0))
                .backdrop(group.sample());
            tx[surface.root()].push(&degenerate);
            tx[&degenerate].clip(clip.clone()).backdrop(group.sample());
        });
        let result = engine.render(FrameTime::now());
        assert!(
            matches!(
                result,
                Err(cherenkov::RenderError::Unsupported(name))
                    if name == "backdrop-union-degenerate-member"
            ),
            "degenerate clip {clip:?}: {result:?}"
        );
    }
}

/// A union group past `BackdropUnion::MAX_MEMBERS` rejects by name.
#[test]
fn union_members_past_the_cap_are_unsupported() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::FULL.union(cherenkov::BackdropUnion::new(20.0).expect("union")),
    );
    let members: Vec<_> = (0..=cherenkov::BackdropUnion::MAX_MEMBERS)
        .map(|_| surface.layer())
        .collect();
    surface.update(|tx| {
        for member in &members {
            tx[surface.root()].push(member);
            tx[member]
                .clip(Rect::new(4.0, 8.0, 20.0, 24.0))
                .backdrop(group.sample());
        }
    });
    let result = engine.render(FrameTime::now());
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(name)) if name == "backdrop-union-members"
        ),
        "{result:?}"
    );
}

/// A member under a projective descendant of the anchor is outside the
/// anchor's canvas — the strict rule admits no descendant exemption.
#[test]
fn a_member_under_a_projective_descendant_is_unsupported() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let anchor = surface.layer();
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::new(cherenkov::CaptureScale::FULL, cherenkov::CaptureLevels::ONE)
            .anchor(anchor.id()),
    );
    let projected = surface.layer();
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&anchor);
        // A projection makes `projected`'s subtree its own canvas — the
        // member inside it is not in the anchor's canvas.
        tx[surface.root()].push(&projected);
        tx[&projected]
            .clip(Rect::new(0.0, 0.0, 32.0, 32.0))
            .projection(cherenkov::Projective::perspective(100.0).expect("projection"));
        tx[&projected].push(&member);
        tx[&member]
            .clip(Rect::new(8.0, 8.0, 24.0, 24.0))
            .backdrop(group.sample());
    });
    let result = engine.render(FrameTime::now());
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(
                "backdrop-member-outside-anchor-canvas"
            ))
        ),
        "unexpected result {result:?}"
    );
}

/// An anchor's filtered child is its own compositing canvas: a member
/// inside it is the anchor's descendant but outside its canvas — the
/// strict rule admits no descendant exemption.
#[test]
fn a_member_inside_the_anchors_filtered_child_is_unsupported() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let anchor = surface.layer();
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::new(cherenkov::CaptureScale::FULL, cherenkov::CaptureLevels::ONE)
            .anchor(anchor.id()),
    );
    let filtered = surface.layer();
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&anchor);
        tx[&anchor].push(&filtered);
        let blur = engine.filter(filtrate::filters::GaussianBlur::new(1.0f32));
        tx[&filtered].filter(blur.id());
        tx[&filtered].push(&member);
        tx[&member]
            .clip(Rect::new(8.0, 8.0, 24.0, 24.0))
            .backdrop(group.sample());
    });
    let result = engine.render(FrameTime::now());
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(
                "backdrop-member-outside-anchor-canvas"
            ))
        ),
        "unexpected result {result:?}"
    );
}

/// An anchor's projective child is its own canvas: a member inside it is
/// rejected — the projective subtree admits no descendant exemption.
#[test]
fn a_member_inside_the_anchors_projective_child_is_unsupported() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let anchor = surface.layer();
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::new(cherenkov::CaptureScale::FULL, cherenkov::CaptureLevels::ONE)
            .anchor(anchor.id()),
    );
    let projected = surface.layer();
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&anchor);
        tx[&anchor].push(&projected);
        tx[&projected]
            .clip(Rect::new(0.0, 0.0, 32.0, 32.0))
            .projection(cherenkov::Projective::perspective(100.0).expect("projection"));
        tx[&projected].push(&member);
        tx[&member]
            .clip(Rect::new(8.0, 8.0, 24.0, 24.0))
            .backdrop(group.sample());
    });
    let result = engine.render(FrameTime::now());
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(
                "backdrop-member-outside-anchor-canvas"
            ))
        ),
        "unexpected result {result:?}"
    );
}

/// A filtered anchor's children paint in the filter's own canvas, not
/// the anchor's — an isolating anchor rejects its own member children.
#[test]
fn an_isolating_anchor_rejects_its_member_children() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let anchor = surface.layer();
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::new(cherenkov::CaptureScale::FULL, cherenkov::CaptureLevels::ONE)
            .anchor(anchor.id()),
    );
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&anchor);
        let blur = engine.filter(filtrate::filters::GaussianBlur::new(1.0f32));
        tx[&anchor].filter(blur.id());
        tx[&anchor].push(&member);
        tx[&member]
            .clip(Rect::new(8.0, 8.0, 24.0, 24.0))
            .backdrop(group.sample());
    });
    let result = engine.render(FrameTime::now());
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(
                "backdrop-member-outside-anchor-canvas"
            ))
        ),
        "unexpected result {result:?}"
    );
}

/// An anchor at the surface's root layer has nothing beneath it — the
/// capture would be the clear colour. The engine rejects it by name,
/// matching the scene format's `backdrop-anchor-at-root` error.
#[test]
fn an_anchor_at_the_root_is_unsupported() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let root = surface.root();
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::new(cherenkov::CaptureScale::FULL, cherenkov::CaptureLevels::ONE)
            .anchor(root.id()),
    );
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&member);
        tx[&member]
            .clip(Rect::new(8.0, 8.0, 24.0, 24.0))
            .backdrop(group.sample());
    });
    let result = engine.render(FrameTime::now());
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported("backdrop-anchor-at-root"))
        ),
        "unexpected result {result:?}"
    );
}
