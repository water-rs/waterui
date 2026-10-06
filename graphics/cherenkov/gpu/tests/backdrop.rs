//! Backdrop groups: bounded f16 captures sampled by member layers.

use cherenkov::kurbo::{Rect, RoundedRect};
use cherenkov::{__engine_fn as split_fn, __engine_test as split_test, __engine_wait as wait};
use cherenkov::{Bytes, Draw, Engine, FrameTime, Offscreen, OffscreenFormat, WorkingColor};
use cherenkov_gpu::{Gpu, GpuConfig};

fn pixel(readback: &cherenkov::Readback, x: usize, y: usize) -> [f32; 4] {
    let p = &readback.pixels[y * readback.width as usize + x];
    [p[0], p[1], p[2], p[3]]
}

/// The CPU side of `backdrop_sample_level` for the
/// `sample_level_mixes_the_pyramid_trilinearly` scene: the red coverage
/// the bilinear read at device point `p` sees in level `k`, whose texel
/// column `i` covers `[2^k·i, 2^k·(i+1))` device columns — red below 16.
fn red_share(k: u32, p: f32) -> f32 {
    let size = f32::from(32u16 >> k);
    let div = f32::from(1u16 << k);
    let column_red = |i: f32| i.mul_add(-div, 16.0f32).clamp(0.0, div) / div;
    let f = (p / div - 0.5).clamp(0.0, size - 1.0);
    let lo = f.floor();
    let hi = (lo + 1.0).min(size - 1.0);
    let t = f - lo;
    column_red(hi).mul_add(t, column_red(lo) * (1.0 - t))
}

fn assert_pixel(actual: [f32; 4], expected: [f32; 4], tolerance: f32) {
    for (a, e) in actual.iter().zip(expected) {
        assert!(
            (a - e).abs() <= tolerance,
            "pixel {actual:?}, expected {expected:?}"
        );
    }
}

split_test! {
fn unfiltered_member_samples_what_is_behind_it() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
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
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // src_over(50% white, solid red) = [1.0, 0.5, 0.5, 1.0].
    assert_pixel(pixel(&readback, 12, 12), [1.0, 0.5, 0.5, 1.0], 1e-3);
    // src_over(50% white, solid blue) = [0.5, 0.5, 1.0, 1.0].
    assert_pixel(pixel(&readback, 20, 12), [0.5, 0.5, 1.0, 1.0], 1e-3);
    // Outside the member's clip the surface is untouched.
    assert_pixel(pixel(&readback, 4, 4), [1.0, 0.0, 0.0, 1.0], 1e-3);
    let memory = wait!(engine.memory());
    assert_eq!(memory.backdrop_captures, Bytes(16 * 16 * 8));
    assert_eq!(memory.backdrop_capture_format, Some("rgba16float"));
    Ok(())
}
}

split_test! {
fn blurred_backdrop_keeps_extended_range() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    let group = surface.backdrop_group(
        filtrate::filters::GaussianBlur::new(4.0f32),
        cherenkov::CaptureScale::FULL,
    );
    let glass = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 32.0, 64.0),
                WorkingColor::new([16.0, 16.0, 16.0, 1.0]),
            );
            r.fill(
                Rect::new(32.0, 0.0, 64.0, 64.0),
                WorkingColor::new([0.25, 0.25, 0.25, 1.0]),
            );
        }));
        tx[surface.root()].push(&glass);
        tx[&glass]
            .clip(Rect::new(8.0, 8.0, 56.0, 56.0))
            .backdrop(group.sample());
    });
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // HDR whites survive the capture, blur and sampling unclamped: the
    // blur's support stays inside the left half at x=16.
    let bright = pixel(&readback, 16, 32);
    assert!(bright[..3].iter().all(|c| *c > 15.0), "pixel {bright:?}");
    assert_pixel(pixel(&readback, 48, 32), [0.25, 0.25, 0.25, 1.0], 0.05);
    // The step's midpoint blurs to roughly the mean of both halves.
    let edge = pixel(&readback, 32, 32);
    let expected = f32::midpoint(16.0, 0.25);
    assert!(
        (edge[0] - expected).abs() < 1.0 && (edge[1] - expected).abs() < 1.0,
        "edge pixel {edge:?}, expected about {expected}"
    );
    let memory = wait!(engine.memory());
    // footprint = ceil(4 * 3) = 12 → region (8-12..56+12) ∩ 0..64 = 64×64.
    // The filter's input/output intermediates count in `gpu`, not in
    // `backdrop_captures` — only the one capture texture does.
    assert_eq!(memory.backdrop_captures, Bytes(64 * 64 * 8));
    assert_eq!(memory.backdrop_capture_format, Some("rgba16float"));
    Ok(())
}
}

split_test! {
fn nested_groups_capture_in_paint_order() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
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
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // Inside m1 but outside m2: 50% blue over red = [0.5, 0.0, 0.5].
    assert_pixel(pixel(&readback, 8, 8), [0.5, 0.0, 0.5, 1.0], 1e-3);
    // Inside m2 the inner capture saw m1's composite ([0.5, 0.0, 0.5]),
    // then 50% green drew over it: [0.25, 0.5, 0.25].
    assert_pixel(pixel(&readback, 16, 16), [0.25, 0.5, 0.25, 1.0], 1e-3);
    Ok(())
}
}

split_test! {
fn member_inside_clip_only_isolation_sees_the_surface() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
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
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // The member samples the surface's red plus the clip scratch's blue
    // painted before it, each at the right device pixels.
    assert_pixel(pixel(&readback, 10, 16), [0.0, 0.0, 1.0, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 16, 16), [1.0, 0.0, 0.0, 1.0], 1e-3);
    Ok(())
}
}

split_test! {
fn member_without_clip_is_unsupported() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&member);
        tx[&member].backdrop(group.sample());
    });
    let result = wait!(engine.render(FrameTime::now()));
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(name)) if name == "backdrop-unclipped"
        ),
        "unexpected result {result:?}"
    );
    Ok(())
}
}

split_test! {
fn dropped_group_fails_the_frame() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
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
    let result = wait!(engine.render(FrameTime::now()));
    assert!(
        matches!(result, Err(cherenkov::RenderError::Render(_))),
        "unexpected result {result:?}"
    );
    Ok(())
}
}

split_test! {
fn two_members_share_one_capture() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
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
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // Both members sample the one capture: the green surface behind them.
    assert_pixel(pixel(&readback, 4, 4), [0.0, 1.0, 0.0, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 28, 4), [0.0, 1.0, 0.0, 1.0], 1e-3);
    let memory = wait!(engine.memory());
    // One capture for the group, bounded to the members' union (x 2..30,
    // y 2..6) — not one per member nor the whole surface.
    assert_eq!(memory.backdrop_captures, Bytes(28 * 4 * 8));
    assert_eq!(memory.backdrop_capture_format, Some("rgba16float"));
    Ok(())
}
}

split_test! {
/// A member inside a tree layer that isolates only because a sibling
/// blends onto it: the layer's scratch is clip-only for capture purposes,
/// so the capture is the outer target with the layer's partial contents
/// composited over it — the member sees what painted earlier inside the
/// layer and nothing painted after it on the surface.
fn member_inside_blended_descendant_layer_sees_the_layer_contents()
-> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let outer = surface.layer();
    let cutout = surface.layer();
    let member = surface.layer();
    let late = surface.layer();
    surface.update(|tx| {
        // Opaque blue surface content.
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 32.0, 32.0),
                WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&outer);
        tx[surface.root()].push(&late);
        // `outer` is a Normal-blend layer; the DestOut child makes it
        // isolate (`blends_within`) without becoming a semantic isolation.
        tx[&outer].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 16.0, 32.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
        tx[&outer].push(&cutout);
        tx[&cutout]
            .blend(cherenkov::BlendMode::DestOut)
            .content(surface.record(|r| {
                r.fill(
                    Rect::new(24.0, 24.0, 32.0, 32.0),
                    WorkingColor::new([1.0, 1.0, 1.0, 1.0]),
                );
            }));
        tx[&outer].push(&member);
        tx[&member]
            .clip(Rect::new(4.0, 4.0, 12.0, 12.0))
            .backdrop(group.sample());
        // Painted after `outer` composites: must not be in the capture.
        tx[&late].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 4.0, 32.0),
                WorkingColor::new([0.0, 1.0, 0.0, 1.0]),
            );
        }));
    });
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // Inside the member's clip the sampled backdrop is the layer's red
    // over the blue base — the in-scratch content reached the capture.
    assert_pixel(pixel(&readback, 8, 8), [1.0, 0.0, 0.0, 1.0], 1e-3);
    // The later sibling overdraws outside the member's clip.
    assert_pixel(pixel(&readback, 2, 8), [0.0, 1.0, 0.0, 1.0], 1e-3);
    // The DestOut sibling cleared its corner inside `outer`'s scratch.
    assert_pixel(pixel(&readback, 28, 28), [0.0, 0.0, 1.0, 1.0], 1e-3);
    Ok(())
}
}

split_test! {
/// A member layer's own opacity fades the member as a whole, its
/// backdrop sample included: an unfiltered member's sample is its
/// canvas's bottom-most content and attenuates with it; a filtered
/// member's sample stays outside its isolation — the filter never
/// covers it — and still fades by the member's opacity, once (#1974).
fn member_sample_is_attenuated_by_layer_opacity() -> Result<(), Box<dyn std::error::Error>> {
    split_fn! {
// A red|blue step under a red↔blue-swapping group: the captured sample
// differs from the sharp backdrop everywhere — at the clip's
// antialiased rim too — so attenuating the sample and the member
// clip's coverage are both observable. `filter` gives the member a
// red↔blue swap or an identity filter.
fn render(engine: &Engine<Gpu>, opacity: f32, filter: Option<[f32; 12]>, blend: cherenkov::BlendMode) -> Result<cherenkov::Readback, Box<dyn std::error::Error>> {
        let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
        let group = surface.backdrop_group(
            filtrate::filters::ColorMatrix(SWAP),
            cherenkov::CaptureScale::FULL,
        );
        let member = surface.layer();
        let child = surface.layer();
        let member_filter = filter.map(|matrix| engine.filter(filtrate::filters::ColorMatrix(matrix)));
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
        wait!(engine.render(FrameTime::now()))?;
        Ok(wait!(surface.readback())?)
    }
    }

    const SWAP: [f32; 12] = [
        0.0, 0.0, 1.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        1.0, 0.0, 0.0, 0.0,
    ];
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
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let normal = cherenkov::BlendMode::Normal;
    let full = wait!(render(&engine, 1.0, None, normal))?;
    let half = wait!(render(&engine, 0.5, None, normal))?;
    // Sample-only over red, content-covered at the step, covered
    // content, and the clip's corner edge.
    for &(x, y) in &[(8, 16), (16, 16), (24, 16), (5, 5)] {
        assert_pixel(pixel(&half, x, y), faded(&full, x, y), 1e-3);
    }
    // Outside the clip the surface is untouched.
    assert_pixel(pixel(&half, 1, 16), [1.0, 0.0, 0.0, 1.0], 1e-3);
    // A filtered member at the same pixels: the sample is attenuated —
    // never swapped — and fades with the member's content, so the rule
    // holds at covered pixels too; the member scope compositing sample
    // and content together is what keeps it true where content covers
    // the step.
    let filtered_full = wait!(render(&engine, 1.0, Some(SWAP), normal))?;
    let filtered_half = wait!(render(&engine, 0.5, Some(SWAP), normal))?;
    for &(x, y) in &[(8, 16), (16, 16), (24, 16), (5, 5)] {
        assert_pixel(
            pixel(&filtered_half, x, y),
            faded(&filtered_full, x, y),
            1e-3,
        );
    }
    // An identity-filtered member equals the unfiltered member: the
    // member scope's nested scopes change nothing — on the clip's rim,
    // where the swapped sample differs from the sharp backdrop and any
    // extra clip-edge coverage would show, and in the interior, where
    // the assertion also pins that the filter never covers the sample.
    let identity_full = wait!(render(&engine, 1.0, Some(IDENTITY), normal))?;
    let identity_half = wait!(render(&engine, 0.5, Some(IDENTITY), normal))?;
    for &(x, y) in &[(6, 5), (5, 5), (8, 16), (16, 16), (24, 16)] {
        assert_pixel(pixel(&identity_full, x, y), pixel(&full, x, y), 1e-3);
        assert_pixel(pixel(&identity_half, x, y), pixel(&half, x, y), 1e-3);
    }
    // A member's non-Normal blend applies to its sample, for both member
    // kinds: `Multiply` blends the whole member — sample and content —
    // against the backdrop, so only channels shared with the backdrop
    // survive and covered content fades to black.
    for filter in [None, Some(SWAP)] {
        let multi = wait!(render(&engine, 1.0, filter, cherenkov::BlendMode::Multiply))?;
        let base = if filter.is_some() { &filtered_full } else { &full };
        // Sample-only pixel over red: Multiply blends S as S·D — the
        // swapped sample is blue over red, so only black survives.
        let s = pixel(base, 8, 16);
        assert_pixel(pixel(&multi, 8, 16), [s[0], 0.0, 0.0, 1.0], 1e-3);
        // Covered content over the step: green × blue = black.
        assert_pixel(pixel(&multi, 24, 16), [0.0, 0.0, 0.0, 1.0], 1e-3);
    }
    Ok(())
}
}

split_test! {
fn colour_effect_tints_the_sampled_capture() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 32.0, 32.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&member);
        // rows: r' = r, g' = 0.5 (bias x alpha), b' = 0, alpha passes.
        tx[&member]
            .clip(Rect::new(8.0, 8.0, 24.0, 24.0))
            .backdrop(group.sample_with(cherenkov::ColorMatrix([
                1.0, 0.0, 0.0, 0.0, //
                0.0, 0.0, 0.0, 0.5, //
                0.0, 0.0, 0.0, 0.0,
            ])));
    });
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    assert_pixel(pixel(&readback, 16, 16), [1.0, 0.5, 0.0, 1.0], 1e-3);
    // Outside the member clip the surface is untouched.
    assert_pixel(pixel(&readback, 4, 4), [1.0, 0.0, 0.0, 1.0], 1e-3);
    Ok(())
}
}

split_test! {
fn refraction_displaces_edge_samples_but_not_the_centre() -> Result<(), Box<dyn std::error::Error>>
{
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 32.0, 64.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
            r.fill(
                Rect::new(32.0, 0.0, 64.0, 64.0),
                WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&member);
        tx[&member]
            .clip(Rect::new(8.0, 8.0, 56.0, 56.0))
            .backdrop(group.sample_with(cherenkov::Refraction {
                depth: 8.0,
                strength: 40.0,
            }));
    });
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // At the member's left edge the sample point pulls inward along the
    // normal by up to `strength` pixels: still red (x + 20 < 32 stays in
    // the red half, so probe near the blue/red boundary instead).
    // Near the right edge the normal points +x and the sample lands up to
    // `strength` px to the left of the edge — into the red half.
    assert_pixel(pixel(&readback, 54, 32), [1.0, 0.0, 0.0, 1.0], 1e-3);
    // The centre is past `depth` from every edge: the unshifted sample.
    assert_pixel(pixel(&readback, 32, 32), [0.0, 0.0, 1.0, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 20, 32), [1.0, 0.0, 0.0, 1.0], 1e-3);
    Ok(())
}
}

split_test! {
fn shader_effect_lights_the_rim() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader = engine.backdrop_shader(cherenkov::BackdropShaderSource::wgsl(
        "fn backdrop_effect(p: vec2<f32>, sdf: f32, normal: vec2<f32>, size: vec2<f32>, params: array<vec4<f32>, 16>) -> vec4<f32> {
            let rim = clamp(1.0 + sdf / 4.0, 0.0, 1.0);
            return vec4<f32>(backdrop_sample(p).rgb * (1.0 + params[0].x * rim), backdrop_sample(p).a);
        }",
    ))?;
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 64.0, 64.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&member);
        tx[&member]
            .clip(Rect::new(8.0, 8.0, 56.0, 56.0))
            .backdrop(group.sample_with(shader.effect(vec![3.0])));
    });
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // A pixel inside the 4 px rim is multiplied by 1 + 3*rim > 1.
    let lit = pixel(&readback, 10, 32);
    assert!(lit[0] > 1.5, "rim pixel {lit:?}");
    // The centre keeps the plain sample.
    assert_pixel(pixel(&readback, 32, 32), [1.0, 0.0, 0.0, 1.0], 1e-3);
    Ok(())
}
}

split_test! {
fn shader_effect_size_is_the_unclipped_member_size() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader = engine.backdrop_shader(cherenkov::BackdropShaderSource::wgsl(
        "fn backdrop_effect(p: vec2<f32>, sdf: f32, normal: vec2<f32>, size: vec2<f32>, params: array<vec4<f32>, 16>) -> vec4<f32> {
            return vec4<f32>(size.x / 256.0, size.y / 256.0, 0.0, 1.0);
        }",
    ))?;
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 64.0, 64.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&member);
        // The member clip runs 32 px off the left of the surface: its
        // device bounds are 64x16, only half of it visible.
        tx[&member]
            .clip(Rect::new(-32.0, 0.0, 32.0, 16.0))
            .backdrop(group.sample_with(shader.effect(Vec::new())));
    });
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // `size` reports the member's 64x16 device bounds, not the 32x16
    // intersection with the capture region.
    assert_pixel(pixel(&readback, 8, 8), [0.25, 0.0625, 0.0, 1.0], 1e-3);
    Ok(())
}
}

split_test! {
fn invalid_backdrop_shader_source_is_a_shader_error() {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default())).expect("engine");
    let result = engine.backdrop_shader(cherenkov::BackdropShaderSource::wgsl(
        "fn backdrop_effect(p: vec2<f32> {",
    ));
    assert!(
        matches!(result, Err(cherenkov::ResourceError::Shader(_))),
        "unexpected result {result:?}"
    );
}
}

split_test! {
fn refraction_on_a_path_clip_is_unsupported() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&member);
        tx[&member]
            .clip(cherenkov::ShapeData::Path {
                elements: vec![
                    cherenkov::kurbo::PathEl::MoveTo((8.0, 8.0).into()),
                    cherenkov::kurbo::PathEl::LineTo((24.0, 8.0).into()),
                    cherenkov::kurbo::PathEl::LineTo((24.0, 24.0).into()),
                    cherenkov::kurbo::PathEl::ClosePath,
                ]
                .into(),
                rule: cherenkov::FillRule::NonZero,
            })
            .backdrop(group.sample_with(cherenkov::Refraction {
                depth: 4.0,
                strength: 4.0,
            }));
    });
    let result = wait!(engine.render(FrameTime::now()));
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(name)) if name == "backdrop-effect-sdf-path"
        ),
        "unexpected result {result:?}"
    );
    Ok(())
}
}

split_test! {
fn dropped_backdrop_shader_fails_the_frame() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let member = surface.layer();
    {
        let shader = engine.backdrop_shader(cherenkov::BackdropShaderSource::wgsl(
            "fn backdrop_effect(p: vec2<f32>, sdf: f32, normal: vec2<f32>, size: vec2<f32>, params: array<vec4<f32>, 16>) -> vec4<f32> {
                return backdrop_sample(p);
            }",
        ))?;
        surface.update(|tx| {
            tx[surface.root()].push(&member);
            tx[&member]
                .clip(Rect::new(8.0, 8.0, 24.0, 24.0))
                .backdrop(group.sample_with(shader.effect(vec![])));
        });
    }
    let result = wait!(engine.render(FrameTime::now()));
    assert!(
        matches!(result, Err(cherenkov::RenderError::Render(_))),
        "unexpected result {result:?}"
    );
    Ok(())
}
}

split_test! {
fn effect_members_do_not_duplicate_the_capture() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let tint = cherenkov::ColorMatrix([
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0,
    ]);
    let members: Vec<_> = (0..3).map(|_| surface.layer()).collect();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 32.0, 32.0),
                WorkingColor::new([0.0, 1.0, 0.0, 1.0]),
            );
        }));
        for member in &members {
            tx[surface.root()].push(member);
            tx[member]
                .clip(Rect::new(4.0, 4.0, 20.0, 20.0))
                .backdrop(group.sample_with(tint));
        }
    });
    wait!(engine.render(FrameTime::now()))?;
    let memory = wait!(engine.memory());
    // Three members over the same 16x16 union sample one shared capture.
    assert_eq!(memory.backdrop_captures, Bytes(16 * 16 * 8));
    assert_eq!(memory.backdrop_capture_format, Some("rgba16float"));
    Ok(())
}
}

split_test! {
fn a_shaders_reach_grows_the_capture_region() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader = engine.backdrop_shader(
        cherenkov::BackdropShaderSource::wgsl(
            "fn backdrop_effect(p: vec2<f32>, sdf: f32, normal: vec2<f32>, size: vec2<f32>, params: array<vec4<f32>, 16>) -> vec4<f32> {
                return backdrop_sample(p);
            }",
        )
        .reach(8.0),
    )?;
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 64.0, 64.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&member);
        tx[&member]
            .clip(Rect::new(16.0, 16.0, 48.0, 48.0))
            .backdrop(group.sample_with(shader.effect(vec![])));
    });
    wait!(engine.render(FrameTime::now()))?;
    let memory = wait!(engine.memory());
    // The 32x32 member union grows by the 8 px reach on every side.
    assert_eq!(memory.backdrop_captures, Bytes(48 * 48 * 8));
    assert_eq!(memory.backdrop_capture_format, Some("rgba16float"));
    Ok(())
}
}

split_test! {
/// Two members far enough apart take two capture regions; the pixels
/// inside each are identical to one single-member group each (#117).
fn far_members_take_two_regions() -> Result<(), Box<dyn std::error::Error>> {
    split_fn! {
#[cfg_attr(not(target_arch = "wasm32"), expect(clippy::type_complexity, reason = "test helper"))]
    fn render_bars(
        two_groups: bool,
    ) -> Result<
        (
            Engine<Gpu>,
            cherenkov::Surface<Gpu>,
            cherenkov::Readback,
            cherenkov::BackdropGroup,
            Option<cherenkov::BackdropGroup>,
        ),
        Box<dyn std::error::Error>,
    > {
        let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
        let surface = wait!(engine.surface(Offscreen::new((512, 512), OffscreenFormat::LinearF16), || {}))?;
        let group_a = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
        let group_b = two_groups
            .then(|| surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL));
        let top = surface.layer();
        let bottom = surface.layer();
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|r| {
                r.fill(
                    Rect::new(0.0, 0.0, 512.0, 512.0),
                    WorkingColor::new([0.0, 1.0, 0.0, 1.0]),
                );
            }));
            tx[surface.root()].push(&top).push(&bottom);
            tx[&top]
                .clip(Rect::new(0.0, 0.0, 512.0, 96.0))
                .backdrop(group_a.sample());
            tx[&bottom]
                .clip(Rect::new(0.0, 416.0, 512.0, 512.0))
                .backdrop(group_b.as_ref().unwrap_or(&group_a).sample());
        });
        wait!(engine.render(FrameTime::now()))?;
        let readback = wait!(surface.readback())?;
        Ok((engine, surface, readback, group_a, group_b))
    }
    }

    let (engine_a, _surface_a, readback_a, _group_a, _group_ba) = wait!(render_bars(false))?;
    // Two regions: 512x96 + 512x96 texels at 8 bytes.
    assert_eq!(wait!(engine_a.memory()).backdrop_captures, Bytes(2 * 512 * 96 * 8));
    let (engine_b, _surface_b, readback_b, _group_c, _group_d) = wait!(render_bars(true))?;
    assert_eq!(wait!(engine_b.memory()).backdrop_captures, Bytes(2 * 512 * 96 * 8));
    // Inside both bars the pixels are byte-identical between the
    // two-region one-group render and the two single-member groups.
    for y in (0..96).chain(416..512) {
        for x in 0..512 {
            assert_eq!(
                pixel(&readback_a, x, y),
                pixel(&readback_b, x, y),
                "pixel {x},{y} differs"
            );
        }
    }
    Ok(())
}
}

split_test! {
/// A member's effect reach inflates its aproned rect, so two members
/// whose raw bounds are apart merge into one region once `A_i` overlaps.
fn reach_merges_aproned_rects() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let a = surface.layer();
    let b = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&a).push(&b);
        // a: 4..12 x 4..12, A = raw 8x8.
        tx[&a]
            .clip(Rect::new(4.0, 4.0, 12.0, 12.0))
            .backdrop(group.sample());
        // b: 16..24 x 4..12 raw (no touch: 12 < 16); reach 8 inflates it
        // to 8..32 x -4..20, which overlaps a's A.
        tx[&b]
            .clip(Rect::new(16.0, 4.0, 24.0, 12.0))
            .backdrop(group.sample_with(cherenkov::Refraction {
                depth: 4.0,
                strength: 8.0,
            }));
    });
    wait!(engine.render(FrameTime::now()))?;
    // One merged region: A_a is [4,4,8,8]; A_b is b's bounds inflated by
    // the 8 px reach (8..32 x -4..20), clipped to [8,0,24,20]. They
    // overlap, so the union region is [4,0,28,20] = 560 texels — not the
    // 64+480 of two separate regions.
    assert_eq!(wait!(engine.memory()).backdrop_captures, Bytes(28 * 20 * 8));
    Ok(())
}
}

split_test! {
/// Removing a member drops its region and the capture bytes shrink.
fn removing_a_member_drops_its_region() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((512, 512), OffscreenFormat::LinearF16), || {}))?;
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let top = surface.layer();
    let bottom = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&top).push(&bottom);
        tx[&top]
            .clip(Rect::new(0.0, 0.0, 512.0, 96.0))
            .backdrop(group.sample());
        tx[&bottom]
            .clip(Rect::new(0.0, 416.0, 512.0, 512.0))
            .backdrop(group.sample());
    });
    wait!(engine.render(FrameTime::now()))?;
    assert_eq!(wait!(engine.memory()).backdrop_captures, Bytes(2 * 512 * 96 * 8));
    surface.update(|tx| {
        tx[surface.root()].remove(&bottom);
    });
    wait!(engine.render(FrameTime::now()))?;
    assert_eq!(wait!(engine.memory()).backdrop_captures, Bytes(512 * 96 * 8));
    Ok(())
}
}

split_test! {
/// A backdrop shader whose last handle drops while a member still samples
/// it stays registered: the next redraw still runs its effect. Clearing the
/// member's effect carries out the release, so a kept effect value naming
/// the shader afterwards fails the frame (#199).
fn a_released_backdrop_shader_stays_while_a_member_samples_it()
-> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader = engine.backdrop_shader(cherenkov::BackdropShaderSource::wgsl(
        "fn backdrop_effect(p: vec2<f32>, sdf: f32, normal: vec2<f32>, size: vec2<f32>, params: array<vec4<f32>, 16>) -> vec4<f32> {
            let rim = clamp(1.0 + sdf / 4.0, 0.0, 1.0);
            return vec4<f32>(backdrop_sample(p).rgb * (1.0 + params[0].x * rim), backdrop_sample(p).a);
        }",
    ))?;
    let effect = shader.effect(vec![3.0]);
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let member = surface.layer();
    let fill = |color: [f32; 4]| {
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|r| {
                r.fill(Rect::new(0.0, 0.0, 64.0, 64.0), WorkingColor::new(color));
            }));
        });
    };
    fill([1.0, 0.0, 0.0, 1.0]);
    surface.update(|tx| {
        tx[surface.root()].push(&member);
        tx[&member]
            .clip(Rect::new(8.0, 8.0, 56.0, 56.0))
            .backdrop(group.sample_with(effect.clone()));
    });
    wait!(engine.render(FrameTime::now()))?;
    drop(shader);
    // A new backdrop colour redraws the member through the released shader.
    fill([0.0, 1.0, 0.0, 1.0]);
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    let lit = pixel(&readback, 10, 32);
    assert!(lit[1] > 1.5, "rim pixel {lit:?}");
    assert_pixel(pixel(&readback, 32, 32), [0.0, 1.0, 0.0, 1.0], 1e-3);

    surface.update(|tx| {
        tx[&member].backdrop(group.sample());
    });
    wait!(engine.render(FrameTime::now()))?;
    surface.update(|tx| {
        tx[&member].backdrop(group.sample_with(effect));
    });
    let result = wait!(engine.render(FrameTime::now()));
    assert!(
        matches!(&result, Err(cherenkov::RenderError::Render(message)) if message.contains("is not registered")),
        "a member sampling the freed shader: {result:?}"
    );
    Ok(())
}
}

split_test! {
fn reduced_capture_resolves_and_samples_bilinearly() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
    let quarter = cherenkov::CaptureScale::new(0.25)?;
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
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // Texel 3 covers device [12, 16) — red — and texel 4 [16, 20) — blue.
    // Pixel 15's centre lands at 15.5 / 4 = 3.875, 0.375 past texel 3's
    // centre; pixel 17's at 4.375, 0.875 past it.
    assert_pixel(pixel(&readback, 15, 16), [0.625, 0.0, 0.375, 1.0], 2e-3);
    assert_pixel(pixel(&readback, 17, 16), [0.125, 0.0, 0.875, 1.0], 2e-3);
    // The member's edge pixel 8 lands at 8.5 / 4 = 2.125, 0.375 short of
    // texel 2's centre: its taps are texel 1 — green [4, 8), outside the
    // member — and texel 2 — red [8, 12).
    assert_pixel(pixel(&readback, 8, 16), [0.625, 0.375, 0.0, 1.0], 2e-3);
    // Far from the steps every tap is one colour.
    assert_pixel(pixel(&readback, 11, 16), [1.0, 0.0, 0.0, 1.0], 1e-3);
    // The opposite edge pixel 23 lands at 23.5 / 4 = 5.875, 0.375 past
    // texel 5's centre: its taps are texel 5 — blue [20, 24) — and
    // texel 6 — green [24, 28), outside the member.
    assert_pixel(pixel(&readback, 23, 16), [0.0, 0.375, 0.625, 1.0], 2e-3);
    // The member [8, 24)² is texels [2, 6)², and the bilinear taps of its
    // edge pixels reach one texel further: [1, 7)², 36 texels, not 256
    // pixels.
    let memory = wait!(engine.memory());
    assert_eq!(memory.backdrop_captures, Bytes(6 * 6 * 8));
    Ok(())
}
}

split_test! {
fn reduced_capture_composes_clip_only_levels_before_resolving()
-> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
    let half = cherenkov::CaptureScale::new(0.5)?;
    let group = surface.backdrop_group_unfiltered(half);
    let p = surface.layer();
    let member = surface.layer();
    surface.update(|tx| {
        // As in `member_inside_clip_only_isolation_sees_the_surface`: P's
        // body lands in a clip-only scratch, composed over the surface
        // copy at device resolution before the resolve.
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
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // Texels 4 and 5 cover the blue [8, 12); texel 6 [12, 14) is red.
    // Pixel 10 samples between texels 4 and 5; pixel 12's centre lands at
    // 6.25, 0.75 past texel 5's centre.
    assert_pixel(pixel(&readback, 10, 16), [0.0, 0.0, 1.0, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 12, 16), [0.75, 0.0, 0.25, 1.0], 2e-3);
    assert_pixel(pixel(&readback, 18, 16), [1.0, 0.0, 0.0, 1.0], 1e-3);
    Ok(())
}
}

split_test! {
fn reduced_blur_counts_its_footprint_in_capture_texels() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    let half = cherenkov::CaptureScale::new(0.5)?;
    let group = surface.backdrop_group(filtrate::filters::GaussianBlur::new(2.0f32), half);
    let glass = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 32.0, 64.0),
                WorkingColor::new([16.0, 16.0, 16.0, 1.0]),
            );
            r.fill(
                Rect::new(32.0, 0.0, 64.0, 64.0),
                WorkingColor::new([0.25, 0.25, 0.25, 1.0]),
            );
        }));
        tx[surface.root()].push(&glass);
        tx[&glass]
            .clip(Rect::new(8.0, 8.0, 56.0, 56.0))
            .backdrop(group.sample());
    });
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // σ = 2 texels is 4 device pixels: HDR whites survive the resolve,
    // blur and sampling unclamped, and the step blurs to its mean.
    let bright = pixel(&readback, 12, 32);
    assert!(bright[..3].iter().all(|c| *c > 15.0), "pixel {bright:?}");
    assert_pixel(pixel(&readback, 52, 32), [0.25, 0.25, 0.25, 1.0], 0.05);
    let edge = pixel(&readback, 32, 32);
    let expected = f32::midpoint(16.0, 0.25);
    assert!(
        (edge[0] - expected).abs() < 1.5,
        "edge pixel {edge:?}, expected about {expected}"
    );
    // footprint 6 texels around texels [4, 28) → [0, 32)²: the capture
    // holds a quarter of the 1:1 region's 64 × 64 pixels.
    let memory = wait!(engine.memory());
    assert_eq!(memory.backdrop_captures, Bytes(32 * 32 * 8));
    Ok(())
}
}

split_test! {
fn reduced_refraction_samples_the_displaced_point_on_the_capture_grid()
-> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    let half = cherenkov::CaptureScale::new(0.5)?;
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
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // Pixel 54's centre is 1.5 inside the right edge: t = 1 − 1.5/8 =
    // 0.8125, so q = 54.5 − 4·t² = 51.859375 device pixels, 25.9296875 on
    // the grid — 0.4296875 past texel 25's centre, between texel 25 (red,
    // device [50, 52)) and texel 26 (blue, [52, 54)). The undisplaced
    // point would read only blue texels.
    assert_pixel(
        pixel(&readback, 54, 32),
        [0.570_312_5, 0.0, 0.429_687_5, 1.0],
        2e-3,
    );
    // The centre is past `depth` from every edge: q = p, at 16.25 on the
    // grid, between two red texels.
    assert_pixel(pixel(&readback, 32, 32), [1.0, 0.0, 0.0, 1.0], 1e-3);
    Ok(())
}
}

split_test! {
fn reduced_rim_lights_the_bilinear_sample_on_the_capture_grid()
-> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    let half = cherenkov::CaptureScale::new(0.5)?;
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
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // Pixel 54's centre lands at 27.25 on the grid, 0.75 past texel 26's
    // centre: a quarter of texel 26 (red, device [52, 54)) and three
    // quarters of texel 27 (blue, [54, 56)). It is 1.5 inside the right
    // edge, so the rim adds 1 · 2 · (1 − 1.5/4)² = 0.78125 of green.
    assert_pixel(
        pixel(&readback, 54, 32),
        [0.25, 0.781_25, 0.75, 1.0],
        2e-3,
    );
    // Past the rim's width the sample is unlit.
    assert_pixel(pixel(&readback, 32, 32), [1.0, 0.0, 0.0, 1.0], 1e-3);
    Ok(())
}
}

/// The `backdrop staging` grow events `sink` accumulated, in order, as
/// `(old, new)` byte pairs.
fn staging_grows(sink: &cherenkov_gpu::diag::Sink) -> Vec<(u64, u64)> {
    sink.take()
        .iter()
        .filter_map(|event| match event.kind {
            cherenkov_gpu::diag::EventKind::Grow {
                label: "backdrop staging",
                old,
                new,
                ..
            } => Some((old, new)),
            _ => None,
        })
        .collect()
}

split_test! {
/// Reduced captures under translucent ancestors on one surface share the
/// surface's one staging texture per capture format, grown to the
/// largest staged device rect and reused across frames:
/// `backdrop_captures` counts only the captures (#1994). A capture under
/// a fading ancestor that painted nothing below the member adds no
/// composite to its pass — it resolves straight from its source,
/// unstaged (#2009).
fn staged_resolves_share_the_surface_staging_texture() -> Result<(), Box<dyn std::error::Error>> {
    let sink = cherenkov_gpu::diag::Sink::new();
    let engine = wait!(Engine::<Gpu>::new(GpuConfig {
        alloc_diag: Some(sink.clone()),
        ..Default::default()
    }))?;
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    let half = cherenkov::CaptureScale::new(0.5)?;
    // Each painted group's filter swaps the capture's red and blue
    // channels, so a missing composite or a zeroed staging changes the
    // member's output.
    let swap_rb = || {
        filtrate::filters::ColorMatrix([
            0.0, 0.0, 1.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            1.0, 0.0, 0.0, 0.0,
        ])
    };
    let big = surface.backdrop_group(swap_rb(), half);
    let small = surface.backdrop_group(swap_rb(), half);
    let empty = surface.backdrop_group_unfiltered(half);
    let big_parent = surface.layer();
    let big_member = surface.layer();
    let small_parent = surface.layer();
    let small_member = surface.layer();
    let empty_parent = surface.layer();
    let empty_member = surface.layer();
    // The empty ancestor alone: its member still captures reduced, but
    // nothing below it was painted, so no staging texture grows at all.
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 64.0, 64.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&empty_parent);
        tx[&empty_parent].opacity(0.5f32);
        tx[&empty_parent].push(&empty_member);
        tx[&empty_member]
            .clip(Rect::new(0.0, 0.0, 64.0, 64.0))
            .backdrop(empty.sample());
    });
    wait!(engine.render(FrameTime::now()))?;
    assert!(engine.stats().frame.is_some(), "the frame drew");
    assert_eq!(staging_grows(&sink), Vec::<(u64, u64)>::new());
    // Add the small painted group: its capture covers texels [3, 13)² —
    // a 22² device rect with the shader margin — and its staged resolve
    // grows the staging once, proving the capture is staged. The member
    // samples its parent's blue; the group's filter turns it red — a
    // missing composite or a zeroed staging would read (0.5, 0, 0.5).
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 64.0, 64.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&small_parent);
        tx[&small_parent].opacity(0.5f32).content(surface.record(|r| {
            r.fill(
                Rect::new(10.0, 10.0, 20.0, 20.0),
                WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            );
        }));
        tx[&small_parent].push(&small_member);
        tx[&small_member]
            .clip(Rect::new(8.0, 8.0, 24.0, 24.0))
            .backdrop(small.sample());
    });
    wait!(engine.render(FrameTime::now()))?;
    assert!(engine.stats().frame.is_some(), "the frame drew");
    assert_eq!(staging_grows(&sink), vec![(0, 22 * 22 * 8)]);
    let readback = wait!(surface.readback())?;
    assert_pixel(pixel(&readback, 15, 15), [1.0, 0.0, 0.0, 1.0], 1e-3);
    // Add the large painted group: the staging grows to its 62² device
    // rect — the `old` half of the pair can only be the small capture's
    // texture if the big capture replaced it — which the small resolve
    // then reuses without a second grow. `MemoryUsage::gpu` counts the
    // staging: its growth lands there on top of the new captures (the
    // depth-0 scratch has covered the surface since phase 1 — a capture
    // inside an isolation forces it — so nothing else grows).
    let before = wait!(engine.memory());
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 64.0, 64.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&big_parent);
        tx[&big_parent].opacity(0.5f32).content(surface.record(|r| {
            r.fill(
                Rect::new(30.0, 30.0, 50.0, 50.0),
                WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            );
        }));
        tx[&big_parent].push(&big_member);
        tx[&big_member]
            .clip(Rect::new(4.0, 4.0, 60.0, 60.0))
            .backdrop(big.sample());
    });
    wait!(engine.render(FrameTime::now()))?;
    assert!(engine.stats().frame.is_some(), "the frame drew");
    assert_eq!(staging_grows(&sink), vec![(22 * 22 * 8, 62 * 62 * 8)]);
    // Each painted member shows what its capture saw, permuted by the
    // group's red/blue swap. The big member sits above the small one: at
    // (15,15) its capture holds the small composite's red and the filter
    // turns it blue — a missing filter would read (1,0,0); at (40,40) it
    // holds the parent's own blue fill, permuted to red — a missing
    // composite or a zeroed staging would read (0.5,0,0.5).
    let readback = wait!(surface.readback())?;
    assert_pixel(pixel(&readback, 15, 15), [0.5, 0.0, 0.5, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 40, 40), [1.0, 0.0, 0.0, 1.0], 1e-3);
    // Only the captures count: 30² + 10² + 32² f16 texels — the empty
    // ancestor's member still captures (clamped to the 32-texel extent).
    let memory = wait!(engine.memory());
    assert_eq!(
        memory.backdrop_captures,
        Bytes(30 * 30 * 8 + 10 * 10 * 8 + 32 * 32 * 8)
    );
    assert!(
        memory.gpu.0 - before.gpu.0 - (memory.backdrop_captures.0 - before.backdrop_captures.0)
            >= (62 * 62 - 22 * 22) * 8,
        "MemoryUsage::gpu counts the staging's regrowth"
    );
    // The shared texture is kept across frames: a re-committed frame
    // grows nothing.
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 64.0, 64.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
    });
    wait!(engine.render(FrameTime::now()))?;
    assert!(engine.stats().frame.is_some(), "the frame drew");
    assert_eq!(staging_grows(&sink), Vec::<(u64, u64)>::new());
    // A painted sibling at the empty ancestor's depth before it: a
    // painted-set scan that ignored `passes_start` would credit the
    // empty ancestor with the sibling's draws and stage the member's
    // 64² rect — a grow.
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 64.0, 64.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&empty_parent);
    });
    wait!(engine.render(FrameTime::now()))?;
    assert!(engine.stats().frame.is_some(), "the frame drew");
    assert_eq!(staging_grows(&sink), Vec::<(u64, u64)>::new());
    Ok(())
}
}

split_test! {
/// A WGSL member effect reading the capture pyramid at a fractional
/// level: `backdrop_sample_level(p, 1.5)` is the trilinear mix of the
/// level-1 and level-2 box reductions — checked per pixel against the
/// same computation run here on the capture (#1786).
fn sample_level_mixes_the_pyramid_trilinearly() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader = engine.backdrop_shader(cherenkov::BackdropShaderSource::wgsl(
        "fn backdrop_effect(p: vec2<f32>, sdf: f32, normal: vec2<f32>, size: vec2<f32>, params: array<vec4<f32>, 16>) -> vec4<f32> {
            return backdrop_sample_level(p, params[0].x);
        }",
    ))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
    let spec = cherenkov::BackdropSpec::new(
        cherenkov::CaptureScale::FULL,
        cherenkov::CaptureLevels::new(3)?,
    );
    let group = surface.backdrop_group_unfiltered(spec);
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
            .backdrop(group.sample_with(shader.effect(vec![1.5])));
    });
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;

    let expected = |x: u16| {
        let p = f32::from(x) + 0.5;
        let red = f32::midpoint(red_share(1, p), red_share(2, p));
        [red, 0.0, 1.0 - red, 1.0]
    };
    // Deep in either half both levels agree.
    assert_pixel(pixel(&readback, 8, 16), expected(8), 0.01);
    assert_pixel(pixel(&readback, 24, 16), expected(24), 0.01);
    // Around the step the two levels blend over different spans: x = 14
    // is pure red at level 1 but 7/8 red at level 2, x = 17 pure blue at
    // level 1 but 1/8 red at level 2 — neither integer level produces
    // the level-1.5 read.
    assert_pixel(pixel(&readback, 14, 16), expected(14), 0.01);
    assert_pixel(pixel(&readback, 15, 16), expected(15), 0.01);
    assert_pixel(pixel(&readback, 16, 16), expected(16), 0.01);
    assert_pixel(pixel(&readback, 17, 16), expected(17), 0.01);
    let memory = wait!(engine.memory());
    // The whole-surface member's 32×32 capture plus its two levels.
    assert_eq!(
        memory.backdrop_captures,
        Bytes((32 * 32 + 16 * 16 + 8 * 8) * 8)
    );
    assert_eq!(memory.backdrop_capture_format, Some("rgba16float"));
    Ok(())
}
}

split_test! {
/// Under `ScratchFormat::Rgba8Unorm` the two staging slots serve
/// different copy sources in one frame: a staged capture copying a part
/// uses the surface-format staging, one copying a semantic isolation's
/// scratch uses the scratch-format staging (#1994).
fn staged_resolves_use_one_staging_per_format() -> Result<(), Box<dyn std::error::Error>> {
    let sink = cherenkov_gpu::diag::Sink::new();
    let engine = wait!(Engine::<Gpu>::new(GpuConfig {
        scratch_format: cherenkov_gpu::ScratchFormat::Rgba8Unorm,
        alloc_diag: Some(sink.clone()),
        ..Default::default()
    }))?;
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    let identity = engine.filter(filtrate::filters::ColorMatrix([
        1.0_f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ]));
    let half = cherenkov::CaptureScale::new(0.5)?;
    // The group filters permute the captures' channels, so a missing
    // composite or a zeroed staging changes each member's output.
    let part_group = surface.backdrop_group(
        filtrate::filters::ColorMatrix([
            0.0, 0.0, 1.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            1.0, 0.0, 0.0, 0.0,
        ]),
        half,
    );
    let scratch_group = surface.backdrop_group(
        filtrate::filters::ColorMatrix([
            1.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 1.0, 0.0, 0.0,
        ]),
        half,
    );
    let part_parent = surface.layer();
    let part_member = surface.layer();
    let semantic = surface.layer();
    let fading = surface.layer();
    let scratch_member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 64.0, 64.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
        // A member under a fading ancestor: its capture copies the
        // surface target — the part format.
        tx[surface.root()].push(&part_parent);
        tx[&part_parent].opacity(0.5f32).content(surface.record(|r| {
            r.fill(
                Rect::new(10.0, 10.0, 20.0, 20.0),
                WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            );
        }));
        tx[&part_parent].push(&part_member);
        tx[&part_member]
            .clip(Rect::new(8.0, 8.0, 24.0, 24.0))
            .backdrop(part_group.sample());
        // A member under a semantic (filtered) isolation, through a
        // painted fading level: its capture copies the isolation's
        // scratch — the scratch format.
        tx[surface.root()].push(&semantic);
        tx[&semantic]
            .clip(Rect::new(32.0, 32.0, 64.0, 64.0))
            .filter(identity.id());
        tx[&semantic].push(&fading);
        tx[&fading].opacity(0.5f32).content(surface.record(|r| {
            r.fill(
                Rect::new(40.0, 40.0, 56.0, 56.0),
                WorkingColor::new([0.0, 1.0, 0.0, 1.0]),
            );
        }));
        tx[&fading].push(&scratch_member);
        tx[&scratch_member]
            .clip(Rect::new(36.0, 36.0, 60.0, 60.0))
            .backdrop(scratch_group.sample());
    });
    wait!(engine.render(FrameTime::now()))?;
    assert!(engine.stats().frame.is_some(), "the frame drew");
    // One grow per slot: the part-copying capture's f16 staging at
    // its 22² device rect, then the scratch-copying capture's rgba8
    // staging at its 30² device rect.
    assert_eq!(staging_grows(&sink), vec![(0, 22 * 22 * 8), (0, 30 * 30 * 4)]);
    let readback = wait!(surface.readback())?;
    // The part member's permuted sample: the parent's blue over red
    // reads red — a missing composite or a zeroed staging gives
    // (0.5, 0, 0.5).
    assert_pixel(pixel(&readback, 15, 15), [1.0, 0.0, 0.0, 1.0], 1e-3);
    // The scratch member's permuted sample: its fading parent's green
    // becomes the member's blue paint, faded to (0.5, 0, 0.5) — a
    // missing composite gives (0.5, 0.5, 0).
    assert_pixel(pixel(&readback, 48, 48), [0.5, 0.0, 0.5, 1.0], 2e-2);
    Ok(())
}
}

split_test! {
/// The pyramid's odd-grid partial box as a pixel read: a 33×33
/// full-scale capture's level 1 is 17 texels wide, its last a partial
/// box over column 32 alone. Column 32 is blue and the rest red, so
/// texel 16 is pure blue — the mean of the texels present — and pixel
/// 31 (level-1 coordinate 31.5 / 2 − 0.5 = 15.25) mixes red texel 15
/// and blue texel 16 at 0.25. A level below 0 reads level 0, one above
/// `n − 1` level `n − 1` (#1786).
fn level_ramp_reads_odd_grid_partial_boxes_and_clamps_levels()
-> Result<(), Box<dyn std::error::Error>> {
    split_fn! {
fn render(engine: &Engine<Gpu>, level: f32) -> Result<cherenkov::Readback, Box<dyn std::error::Error>> {
        let surface = wait!(engine.surface(Offscreen::new((33, 33), OffscreenFormat::LinearF16), || {}))?;
        let spec = cherenkov::BackdropSpec::new(
            cherenkov::CaptureScale::FULL,
            cherenkov::CaptureLevels::new(2)?,
        );
        let group = surface.backdrop_group_unfiltered(spec);
        let ramp = cherenkov::LevelRamp::new(1.0, level, level)?;
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
        wait!(engine.render(FrameTime::now()))?;
        Ok(wait!(surface.readback())?)
    }
    }

    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    for level in [1.0, 7.0] {
        let readback = wait!(render(&engine, level))?;
        assert_pixel(pixel(&readback, 31, 16), [0.75, 0.0, 0.25, 1.0], 2e-3);
        // Pixel 32 lands at 15.75: three quarters of the blue texel.
        assert_pixel(pixel(&readback, 32, 16), [0.25, 0.0, 0.75, 1.0], 2e-3);
        // The bottom row's partial boxes and the corner hold the texels
        // present too.
        assert_pixel(pixel(&readback, 31, 32), [0.75, 0.0, 0.25, 1.0], 2e-3);
        assert_pixel(pixel(&readback, 32, 32), [0.25, 0.0, 0.75, 1.0], 2e-3);
        assert_pixel(pixel(&readback, 8, 32), [1.0, 0.0, 0.0, 1.0], 2e-3);
    }
    let below = wait!(render(&engine, -3.0))?;
    assert_pixel(pixel(&below, 31, 16), [1.0, 0.0, 0.0, 1.0], 2e-3);
    assert_pixel(pixel(&below, 32, 16), [0.0, 0.0, 1.0, 1.0], 2e-3);
    Ok(())
}
}

split_test! {
/// Levels on a reduced capture: at scale 0.5 a 2-level pyramid's level
/// 1 texel covers 4 device columns, so a `LevelRamp` at level 1 reads
/// the same coverage as level 2 of a full-scale capture (#1786).
fn level_ramp_reads_levels_of_a_reduced_capture() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
    let spec = cherenkov::BackdropSpec::new(
        cherenkov::CaptureScale::new(0.5)?,
        cherenkov::CaptureLevels::new(2)?,
    );
    let group = surface.backdrop_group_unfiltered(spec);
    let ramp = cherenkov::LevelRamp::new(1.0, 1.0, 1.0)?;
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
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    for x in [8u16, 13, 14, 15, 16, 17, 18, 24] {
        let red = red_share(2, f32::from(x) + 0.5);
        assert_pixel(pixel(&readback, usize::from(x), 16), [red, 0.0, 1.0 - red, 1.0], 2e-3);
    }
    Ok(())
}
}

split_test! {
/// A member's deep-level read does not depend on the other members: a
/// σ = 2 blur at scale 0.25 into 5 levels, the member at x 600..840
/// reading level 4 over 16-px stripes, renders the same alone — its
/// region then starts at capture texel 112 (device x 448) — and with an
/// adjacent plain member at x 0..596 that pulls the region to texel 0
/// (#1786).
fn deep_level_reads_are_independent_of_the_other_members()
-> Result<(), Box<dyn std::error::Error>> {
    split_fn! {
fn render(engine: &Engine<Gpu>, with_plain: bool) -> Result<cherenkov::Readback, Box<dyn std::error::Error>> {
        let surface = wait!(engine.surface(Offscreen::new((1024, 64), OffscreenFormat::LinearF16), || {}))?;
        let spec = cherenkov::BackdropSpec::new(
            cherenkov::CaptureScale::new(0.25)?,
            cherenkov::CaptureLevels::new(5)?,
        );
        let group = surface.backdrop_group(filtrate::filters::GaussianBlur::new(2.0f32), spec);
        let ramp = cherenkov::LevelRamp::new(1.0, 4.0, 4.0)?;
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
        wait!(engine.render(FrameTime::now()))?;
        Ok(wait!(surface.readback())?)
    }
    }

    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let alone = wait!(render(&engine, false))?;
    let shared = wait!(render(&engine, true))?;
    for y in 8..56 {
        for x in 600..840 {
            assert_eq!(
                pixel(&alone, x, y),
                pixel(&shared, x, y),
                "member pixel ({x}, {y}) depends on the other member"
            );
        }
    }
    Ok(())
}
}
