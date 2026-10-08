//! Backdrop groups: bounded f16 captures sampled by member layers.

use cherenkov::kurbo::{Circle, Ellipse, Rect, RoundedRect};
use cherenkov::{__engine_fn as split_fn, __engine_test as split_test, __engine_wait as wait};
use cherenkov::{
    BackdropOuter, Bytes, Draw, Engine, FrameTime, Offscreen, OffscreenFormat, WorkingColor,
};
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
        "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
            let rim = clamp(1.0 + px.sdf / 4.0, 0.0, 1.0);
            return vec4<f32>(backdrop_sample(px.p).rgb * (1.0 + params[0].x * rim), backdrop_sample(px.p).a);
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
        "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
            return vec4<f32>(px.size.x / 256.0, px.size.y / 256.0, 0.0, 1.0);
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
        "fn backdrop_effect(px: BackdropPixel {",
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
            "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                return backdrop_sample(px.p);
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
            "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                return backdrop_sample(px.p);
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
        "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
            let rim = clamp(1.0 + px.sdf / 4.0, 0.0, 1.0);
            return vec4<f32>(backdrop_sample(px.p).rgb * (1.0 + params[0].x * rim), backdrop_sample(px.p).a);
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
        "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
            return backdrop_sample_level(px.p, params[0].x);
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

/// The `backdrop resolve` `BindGroups` events `sink` accumulated, in
/// order, as dropped counts.
fn resolve_bind_drops(sink: &cherenkov_gpu::diag::Sink) -> Vec<u64> {
    sink.take()
        .iter()
        .filter_map(|event| match event.kind {
            cherenkov_gpu::diag::EventKind::BindGroups {
                dropped,
                reason: "backdrop resolve",
            } => Some(dropped),
            _ => None,
        })
        .collect()
}

split_test! {
/// A group's direct-resolve cache holds the view it was built on: a
/// resize replacing the surface's parts must drop the cached bind, or
/// the retired texture stays alive past the accounting (#2091). The
/// drop reports through `BindGroups` with reason "backdrop resolve".
fn a_resized_surface_drops_the_direct_resolve_bind_on_its_part() -> Result<(), Box<dyn std::error::Error>> {
    let sink = cherenkov_gpu::diag::Sink::new();
    let engine = wait!(Engine::<Gpu>::new(GpuConfig {
        alloc_diag: Some(sink.clone()),
        ..Default::default()
    }))?;
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))?;
    // A reduced capture under nothing looked through resolves straight
    // from its part: its bind caches that view.
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::new(0.5)?);
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
            .clip(Rect::new(8.0, 8.0, 24.0, 24.0))
            .backdrop(group.sample());
    });
    wait!(engine.render(FrameTime::now()))?;
    assert!(engine.stats().frame.is_some(), "the frame drew");
    let _ = sink.take();
    // The resize replaces every part; then the group keeps no region,
    // so nothing rebuilds the bind — only the resize can drop it.
    surface.resize((32, 32))?;
    surface.update(|tx| {
        tx[surface.root()].remove(&member);
    });
    wait!(engine.render(FrameTime::now()))?;
    assert!(engine.stats().frame.is_some(), "the frame drew");
    assert_eq!(resolve_bind_drops(&sink), vec![1]);
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

split_test! {
/// Two groups anchored at the same layer sample one frozen copy taken at
/// the anchor's paint position: neither group sees what the other paints,
/// and each group runs its own chain (#2097).
fn anchored_groups_sample_the_anchors_frozen_copy() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
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
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // Inside A1 only: 50% green over the frozen red.
    assert_pixel(pixel(&readback, 6, 6), [0.5, 0.5, 0.0, 1.0], 1e-3);
    // Inside A1 and B1: B1's capture is the anchor's frozen blue — not
    // A1's composite — then 50% white over it. A first-member capture
    // would hold A1's [0.0, 0.5, 0.5] here and give [0.5, 0.75, 0.75].
    assert_pixel(pixel(&readback, 16, 16), [0.5, 0.5, 1.0, 1.0], 1e-3);
    // Inside B1 and A2: A2 samples the same frozen blue — not B1's
    // [0.5, 0.5, 1.0] composite — then 50% green over it.
    assert_pixel(pixel(&readback, 24, 24), [0.0, 0.5, 0.5, 1.0], 1e-3);
    Ok(())
}
}

split_test! {
/// The anchored groups of `anchored_groups_sample_the_anchors_frozen_copy`
/// cost one shared copy beneath the anchor plus each group's own capture.
fn anchored_groups_capture_beneath_the_anchor() -> Result<(), Box<dyn std::error::Error>> {
    let sink = cherenkov_gpu::diag::Sink::new();
    let engine = wait!(Engine::<Gpu>::new(GpuConfig {
        alloc_diag: Some(sink.clone()),
        ..Default::default()
    }))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
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
        tx[surface.root()].push(&anchor);
        tx[surface.root()].push(&a1).push(&b1).push(&a2);
        tx[&a1]
            .clip(Rect::new(4.0, 4.0, 20.0, 28.0))
            .backdrop(group_a.sample());
        tx[&b1]
            .clip(Rect::new(12.0, 4.0, 28.0, 28.0))
            .backdrop(group_b.sample());
        tx[&a2]
            .clip(Rect::new(20.0, 20.0, 28.0, 28.0))
            .backdrop(group_a.sample());
    });
    let _ = sink.take();
    wait!(engine.render(FrameTime::now()))?;
    // Both groups capture straight from the semantic target at the
    // anchor's position — there is no shared copy texture: the memory
    // accounting below is exactly the groups' own captures.
    let _ = sink.take();
    let memory = wait!(engine.memory());
    // Each group keeps only its own capture: A's 24×24 member union
    // and B's 16×24 member rect.
    assert_eq!(memory.backdrop_captures, Bytes((24 * 24 + 16 * 24) * 8));
    Ok(())
}
}

split_test! {
/// A member that paints before its anchor fails the frame with the named
/// error — the anchored rule never falls back to the first-member capture.
fn a_member_before_the_anchor_is_unsupported() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
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
    let result = wait!(engine.render(FrameTime::now()));
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(name)) if name == "backdrop-member-before-anchor"
        ),
        "unexpected result {result:?}"
    );
    Ok(())
}
}

split_test! {
/// A member inside a compositing canvas of its own — a blended sibling's
/// scratch — fails the frame with the named error: it is outside the
/// anchor's canvas even though it paints after the anchor in the tree.
fn a_member_outside_the_anchors_canvas_is_unsupported() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
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
    let result = wait!(engine.render(FrameTime::now()));
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(name)) if name == "backdrop-member-outside-anchor-canvas"
        ),
        "unexpected result {result:?}"
    );
    Ok(())
}
}

split_test! {
/// The same layout without an anchor keeps the first-member rule: group
/// B's capture is taken at its own member's position and holds what A's
/// member already painted.
fn unanchored_groups_keep_the_first_member_rule() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
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
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // Inside A1 and B1: B1 captured at its own paint position — A1's
    // [0.0, 0.5, 0.5] composite — then 50% white over it.
    assert_pixel(pixel(&readback, 16, 16), [0.5, 0.75, 0.75, 1.0], 1e-3);
    // Inside B1 and A2: A2 still samples group A's frozen copy from its
    // own first member — the blue — unchanged by anchoring absence.
    assert_pixel(pixel(&readback, 24, 24), [0.0, 0.5, 0.5, 1.0], 1e-3);
    Ok(())
}
}

// `smin`, `rect_sdf`, `cover`, `circle_field` and `ownership` live in
// `cherenkov::testing`, shared with the CPU suite. `union_bridge` stays
// per-suite: its engine call shape differs (`wait!`-driven `&Engine<Gpu>`
// here vs a fresh sync `Engine<Raster>` in the CPU file).
use cherenkov::testing::{circle_field, cover, ownership, rect_field, rect_sdf, smin};

split_fn! {
/// Two bridge members 6 px apart on the shared field (`k = 20` joins a
/// gap below `k/2`), rendered with the given member opacities and paint
/// order.
fn union_bridge(
    engine: &Engine<Gpu>,
    a_opacity: f32,
    b_opacity: f32,
    swap: bool,
) -> Result<cherenkov::Readback, Box<dyn std::error::Error>> {
    let surface =
        wait!(engine.surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {}))?;
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
    wait!(engine.render(FrameTime::now()))?;
    Ok(wait!(surface.readback())?)
}
}

split_test! {
fn union_members_composite_identically_in_any_order() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let ab = wait!(union_bridge(&engine, 0.5, 0.5, false))?;
    let ba = wait!(union_bridge(&engine, 0.5, 0.5, true))?;
    assert_eq!(
        ab.pixels, ba.pixels,
        "the union composite depends on member order"
    );
    Ok(())
}
}

split_test! {
fn union_members_partition_the_bridge_alpha() -> Result<(), Box<dyn std::error::Error>> {
    // Each member at opacity 0.5 over a 0.5-alpha capture: member i's
    // deposit above the root is `w_i · c · 0.5 · 0.5`. The ownership
    // weights sum to 1, so the deposits sum to `0.25 · c` — exactly
    // what one member covering the field deposits.
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let only_a = wait!(union_bridge(&engine, 0.5, 0.0, false))?;
    let only_b = wait!(union_bridge(&engine, 0.0, 0.5, false))?;
    let both = wait!(union_bridge(&engine, 0.5, 0.5, false))?;
    let a_clip = Rect::new(4.0, 8.0, 20.0, 24.0);
    let b_clip = Rect::new(26.0, 8.0, 42.0, 24.0);
    let bg = 0.5;
    for row in 10..22usize {
        for col in 18..30usize {
            let (px, py) = (f32::from(u16::try_from(col).unwrap()) + 0.5, f32::from(u16::try_from(row).unwrap()) + 0.5);
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
    Ok(())
}
}

split_fn! {
/// Circle members of a union group at the given opacities — `circles` is
/// `(cx, cy, r)` in paint order. The `union_bridge` counterpart for the
/// ownership-boundary tests, where non-parallel member normals give the
/// seam a real slope.
fn union_circles(
    engine: &Engine<Gpu>,
    circles: &[(f64, f64, f64)],
    opacities: &[f32],
) -> Result<cherenkov::Readback, Box<dyn std::error::Error>> {
    let surface =
        wait!(engine.surface(Offscreen::new((48, 40), OffscreenFormat::LinearF32), || {}))?;
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
    wait!(engine.render(FrameTime::now()))?;
    Ok(wait!(surface.readback())?)
}
}

split_test! {
/// The two ownership weights on a seam ramp across one pixel and sum to
/// 1. Two facing circles (`k = 20` joins them) share the ownership
/// boundary along their bisector x = 22.3, so the pixel column at x = 22
/// carries the fractional weight and its neighbours saturate.
fn the_seam_weights_ramp_across_one_pixel() -> Result<(), Box<dyn std::error::Error>> {
    const CIRCLES: &[(f64, f64, f64)] = &[(12.3, 16.0, 16.0), (32.3, 16.0, 16.0)];
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let only_a = wait!(union_circles(&engine, CIRCLES, &[0.5, 0.0]))?;
    let only_b = wait!(union_circles(&engine, CIRCLES, &[0.0, 0.5]))?;
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
            let field = smin(members[0].0.min(members[1].0), members[0].0.max(members[1].0), 20.0);
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
            // Never drawn twice or left open: the deposits partition
            // a single member's coverage.
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
        let field = smin(members[0].0.min(members[1].0), members[0].0.max(members[1].0), 20.0);
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
    Ok(())
}
}

split_test! {
/// Near a triple point the three ownership weights partition unity:
/// three mutually overlapping circles meet around their circumcentre and
/// every sampled pixel's `a_i / Σ a_j` weights sum to 1, so no member is
/// starved and none is drawn twice.
fn the_three_weights_sum_to_one_near_a_triple_point() -> Result<(), Box<dyn std::error::Error>> {
    // Centres on a ~equilateral triangle, r = 12: the circumcentre at
    // about (24, 19.7) sits inside all three members, the pixel
    // neighbourhood where all three weights are of order 1/3.
    const CIRCLES: &[(f64, f64, f64)] = &[
        (24.0, 13.04, 12.0),
        (29.0, 22.5, 12.0),
        (19.0, 22.5, 12.0),
    ];
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let mut alones = Vec::new();
    for i in 0..3 {
        let mut opacities = [0.0f32; 3];
        opacities[i] = 0.5;
        alones.push(wait!(union_circles(&engine, CIRCLES, &opacities))?);
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
    Ok(())
}
}

split_test! {
fn outer_extent_draws_exactly_the_band_without_a_union() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {}))?;
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
        tx[&member]
            .clip(Rect::new(16.0, 8.0, 32.0, 24.0))
            .backdrop(group.sample_with(cherenkov::ColorMatrix([
                0.0, 0.0, 0.0, 1.0, //
                0.0, 0.0, 0.0, 1.0, //
                0.0, 0.0, 0.0, 1.0,
            ])).outer(BackdropOuter::new(4.5).expect("valid")));
    });
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // Inside the clip and inside the band the white is full.
    assert_pixel(pixel(&readback, 24, 16), [1.0, 1.0, 1.0, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 34, 16), [1.0, 1.0, 1.0, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 35, 16), [1.0, 1.0, 1.0, 1.0], 1e-3);
    // At the band's outer edge (`d = outer` at pixel centre) the
    // coverage is half — half white over the red.
    assert_pixel(pixel(&readback, 36, 16), [1.0, 0.5, 0.5, 1.0], 2e-3);
    // Beyond `outer` the root's red survives untouched.
    assert_pixel(pixel(&readback, 37, 16), [1.0, 0.0, 0.0, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 8, 16), [1.0, 0.0, 0.0, 1.0], 1e-3);
    Ok(())
}
}

split_test! {
fn union_member_with_a_path_clip_is_unsupported() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {}))?;
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::FULL
            .union(cherenkov::BackdropUnion::new(20.0).expect("union")),
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
/// Behavioural: a member's shape change re-renders the neighbour's
/// bridge pixels across frames — a changed surface lowers again in
/// full (the engine has no damage rects for this to leak through).
fn a_member_shape_change_redraws_the_neighbours_bridge() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {}))?;
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::FULL
            .union(cherenkov::BackdropUnion::new(20.0).expect("union")),
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
    wait!(engine.render(FrameTime::now()))?;
    let first = wait!(surface.readback())?;
    scene(20.0);
    wait!(engine.render(FrameTime::now()))?;
    let second = wait!(surface.readback())?;
    assert_ne!(
        first.pixels, second.pixels,
        "the neighbour's bridge pixels stayed stale across frames"
    );
    Ok(())
}
}

split_test! {
/// A bridge pixel's `px.sdf`, `px.own_sdf` and `px.normal` against the
/// analytic fold: at (22.5, 16.5) member `a` owns the pixel outright,
/// `m = min(d_a, d_b) − h²·k/4`, and the normal is the folded gradient.
fn bridge_pixel_reads_the_union_field() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let shader = engine.backdrop_shader(cherenkov::BackdropShaderSource::wgsl(
        "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
            return vec4<f32>(
                (px.sdf + 10.0) / 20.0,
                (px.own_sdf + 10.0) / 20.0,
                px.normal.x * 0.5 + 0.5,
                0.5
            );
        }",
    ))?;
    let surface = wait!(engine.surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {}))?;
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::FULL
            .union(cherenkov::BackdropUnion::new(20.0).expect("union")),
    );
    let a = surface.layer();
    let b = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 48.0, 32.0),
                WorkingColor::new([0.0, 0.0, 0.0, 0.0]),
            );
        }));
        tx[surface.root()].push(&a);
        tx[&a]
            .clip(Rect::new(4.0, 8.0, 20.0, 24.0))
            .backdrop(group.sample_with(shader.effect(Vec::new())));
        tx[surface.root()].push(&b);
        tx[&b]
            .clip(Rect::new(26.0, 8.0, 42.0, 24.0))
            .backdrop(group.sample());
    });
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // At (22.5, 16.5): d_a = 2.5, d_b = 3.5 — a owns the pixel (w_a = 1)
    // at coverage 1, so the premultiplied encoding lands verbatim.
    // m = 2.5 − (19/20)²·5 = −2.0125; the folded gradient is
    // (1 − h/2)·∇a + (h/2)·∇b = (0.525·(+1) + 0.475·(−1), 0) → +x.
    let px = pixel(&readback, 22, 16);
    let (sdf, own, nx) = (
        px[0].mul_add(20.0, -10.0),
        px[1].mul_add(20.0, -10.0),
        px[2].mul_add(2.0, -1.0),
    );
    assert!((sdf - -2.0125).abs() <= 0.05, "px.sdf {sdf}, expected −2.0125");
    assert!((own - 2.5).abs() <= 0.05, "px.own_sdf {own}, expected 2.5");
    assert!((nx - 1.0).abs() <= 0.05, "px.normal.x {nx}, expected +1");
    Ok(())
}
}

split_fn! {
/// `union_circles` with a per-member blend mode — an isolating member
/// must still reach the union zone outside its own clip.
fn union_circles_blended(
    engine: &Engine<Gpu>,
    circles: &[(f64, f64, f64)],
    opacities: &[f32],
    blends: &[cherenkov::BlendMode],
) -> Result<cherenkov::Readback, Box<dyn std::error::Error>> {
    let surface =
        wait!(engine.surface(Offscreen::new((48, 40), OffscreenFormat::LinearF32), || {}))?;
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
    wait!(engine.render(FrameTime::now()))?;
    Ok(wait!(surface.readback())?)
}
}

split_test! {
/// An isolating union member — here blending `Screen` — must still
/// composite into the union zone outside its own clip: the bridge and
/// the band belong to the field, not to the member shape.
fn a_screen_member_bridges_outside_its_clip() -> Result<(), Box<dyn std::error::Error>> {
    const CIRCLES: &[(f64, f64, f64)] = &[(12.3, 16.0, 16.0), (32.3, 16.0, 16.0)];
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let only_a = wait!(union_circles_blended(
        &engine,
        CIRCLES,
        &[0.5, 0.0],
        &[cherenkov::BlendMode::Screen, cherenkov::BlendMode::Normal],
    ))?;
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
            // Screen brightens toward the backdrop: premultiplied, the
            // member's captured red adds `s + d - s·d` — the deposit's
            // share reaches the red channel too, not just alpha.
            let rgb = pixel(&only_a, col, row);
            let want_r = 0.5 + want;
            assert!(
                (rgb[0] - want_r).abs() <= 4e-3,
                "pixel ({col}, {row}): Screen red {} != {want_r}",
                rgb[0]
            );
        }
    }
    assert!(
        outside_clip,
        "no pixel outside member a's clip carried a's deposit"
    );
    Ok(())
}
}

split_test! {
/// In a backdrop group with no union field, a member carrying an outer
/// band and a plain member mix: lowering must not panic, the band
/// member draws its band, and the plain member keeps its clip coverage.
fn a_mixed_group_without_a_union_draws_the_band() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {}))?;
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
        tx[&banded]
            .clip(Rect::new(4.0, 8.0, 20.0, 24.0))
            .backdrop(
                group
                    .sample_with(cherenkov::ColorMatrix([
                        0.0, 0.0, 0.0, 1.0, //
                        0.0, 0.0, 0.0, 1.0, //
                        0.0, 0.0, 0.0, 1.0,
                    ]))
                    .outer(cherenkov::BackdropOuter::new(4.5).expect("valid")),
            );
        tx[&plain]
            .clip(Rect::new(26.0, 8.0, 42.0, 24.0))
            .backdrop(group.sample_with(cherenkov::ColorMatrix([
                0.0, 0.0, 0.0, 1.0, //
                0.0, 0.0, 0.0, 1.0, //
                0.0, 0.0, 0.0, 1.0,
            ])));
    });
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // The banded member fills its band (clip x1 = 20, band to 24.5).
    assert_pixel(pixel(&readback, 22, 16), [1.0, 1.0, 1.0, 1.0], 1e-3);
    // The plain member draws only its clip — no band at (44, 16).
    assert_pixel(pixel(&readback, 40, 16), [1.0, 1.0, 1.0, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 44, 16), [1.0, 0.0, 0.0, 1.0], 1e-3);
    assert_pixel(pixel(&readback, 46, 16), [1.0, 0.0, 0.0, 1.0], 1e-3);
    Ok(())
}
}

split_test! {
/// A destructive-blend union member under a masked ancestor: the member
/// clip (`RoundedRect`, unmergeable against the ancestor's path mask)
/// isolates, so the member's sample must composite under the ancestor
/// clip the scope applies — not under a clip value captured before.
/// `SrcOut` drops the backdrop under the member, leaving `0.5·src` —
/// an instance clipped against mask texel (0, 0) would deposit nothing.
fn a_destructive_member_under_a_masked_ancestor_samples_its_union() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((48, 40), OffscreenFormat::LinearF32), || {}))?;
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
        // A decoy path clip lands in the mask atlas first: without it
        // the ancestor's triangle would sit at texel (0, 0), exactly
        // where an unpatched instance reads — the test would pass while
        // the defect it checks for is still live.
        tx[surface.root()].push(&decoy);
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
    wait!(engine.render(FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    // Deep in member a's zone (w_a = 1, coverage 1) the member's sample
    // deposits the captured red at strength: over the `0.5`-alpha
    // backdrop the pixel reads `0.5 + 0.25·0.5 = 0.625`. An instance
    // clipped against mask texel (0, 0) would deposit nothing here.
    assert_pixel(pixel(&readback, 32, 14), [0.625, 0.0, 0.0, 0.625], 3e-3);
    // At the seam midpoint (35.5, 20.5) the weights split: w_a = 0.5.
    assert_pixel(pixel(&readback, 35, 20), [0.5625, 0.0, 0.0, 0.5625], 4e-3);
    Ok(())
}
}

split_fn! {
/// Renders the masked-ancestor isolation probe: a union member
/// compositing through a layer-level isolation under an optional
/// path-clipped ancestor. `mode`: 0 `SrcOut` blend, 1 `Screen` blend,
/// 2 opacity over overlapping content, 3 filter with opacity. The
/// engine is per-render: the raster cache an earlier render leaves
/// would paper over an unpatched mask reference.
fn masked_ancestor_isolation(
    masked: bool,
    mode: u32,
) -> Result<cherenkov::Readback, Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(
        Offscreen::new((48, 40), OffscreenFormat::LinearF32),
        || {},
    ))?;
    let identity = engine.filter(filtrate::filters::ColorMatrix([
        1.0_f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ]));
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
        let la = &mut tx[&a];
        la.clip(RoundedRect::new(30.0, 10.0, 40.0, 20.0, 3.0))
            .backdrop(group.sample());
        match mode {
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
        tx[&b]
            .clip(RoundedRect::new(36.0, 18.0, 46.0, 28.0, 3.0))
            .backdrop(group.sample())
            .opacity(0.5f32);
    });
    wait!(engine.render(FrameTime::now()))?;
    Ok(wait!(surface.readback())?)
}
}

split_fn! {
fn member_render(
    surface: &cherenkov::Surface<Gpu>,
    members: &[cherenkov::Layer],
    opacities: &[f32],
    engine: &Engine<Gpu>,
) -> Result<cherenkov::Readback, Box<dyn std::error::Error>> {
    surface.update(|tx| {
        for (member, &opacity) in members.iter().zip(opacities.iter()) {
            tx[member].opacity(opacity);
        }
    });
    wait!(engine.render(FrameTime::now()))?;
    Ok(wait!(surface.readback())?)
}
}

split_test! {
/// A union member compositing through a layer-level isolation — a
/// destructive blend, a non-destructive blend, opacity over overlapping
/// content, or a filter — keeps sampling its union under a masked
/// ancestor: the scratch's composite applies the ancestors itself, so
/// the member's sample inside the scratch must carry no ancestor clip —
/// an analytic one would multiply coverage twice, a masked one has no
/// `mask_pending` patch and would read the wrong atlas texel. A
/// path-clipped decoy lands in the atlas first, so the ancestor's
/// triangle does not sit at texel (0, 0).
fn isolated_members_under_a_masked_ancestor_match_unclipped() -> Result<(), Box<dyn std::error::Error>> {
    for mode in 0..4 {
        let plain = wait!(masked_ancestor_isolation(false, mode))?;
        let masked = wait!(masked_ancestor_isolation(true, mode))?;
        // Deep inside the triangle the ancestor's coverage is 1, so the
        // masked render must read like the unclipped one — a stale
        // ancestor clip on the sample shows up as missing or doubled
        // coverage here.
        for (x, y) in [(32usize, 14usize), (35, 20), (38, 12), (33, 11)] {
            let p = pixel(&plain, x, y);
            let m = pixel(&masked, x, y);
            let diff = p
                .iter()
                .zip(m)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(
                diff <= 3e-3,
                "mode {mode} pixel ({x},{y}): unclipped {p:?} vs masked {m:?}"
            );
        }
    }
    Ok(())
}
}

split_test! {
/// Union members carrying an outer band keep ownership weights summing
/// to one at the field's outer antialiased edge. The members are rects:
/// a second member offset so their top corners nearly tie keeps band
/// coverage and a shared ownership weight on pixels out to the field's
/// coverage wall — bound-distance past `r(n) + outer − 0.5` — so a
/// draw bound more than half a pixel short would clip the outermost
/// sampled pixels a member still owns. (No owned pixel can reach the
/// pad's last pixel: the field's coverage dies at `r(n) + outer + 0.5`,
/// before the `+1.5` bound's margin ends.)
fn the_seam_weights_sum_to_one_at_the_outer_edge() -> Result<(), Box<dyn std::error::Error>> {
    const RECTS: [Rect; 2] = [Rect::new(4.0, 14.0, 10.0, 28.0), Rect::new(16.0, 14.4, 24.0, 28.4)];
    const OUTER: f32 = 4.0;
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((48, 40), OffscreenFormat::LinearF32), || {}))?;
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
            tx[member]
                .clip(*rect)
                .backdrop(
                    group
                        .sample()
                        .outer(cherenkov::BackdropOuter::new(OUTER).expect("valid")),
                );
        }
    });

    let only_a = wait!(member_render(&surface, &members, &[0.5, 0.0], &engine))?;
    let only_b = wait!(member_render(&surface, &members, &[0.0, 0.5], &engine))?;
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
            // The pad's edge: an owned pixel can reach `r(n) + outer +
            // 0.5` past its bounds — bound-distance in (9, 10.5] —
            // where a bound any shorter than the full `+1.5` pad clips
            // the deposit away. Only the corner near-tie puts owned
            // pixels this far out.
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
    Ok(())
}
}

split_test! {
/// Members whose clip's analytic SDF degenerates cannot fold into the
/// union field and reject by name — a zero-radius circle and an ellipse
/// collapsed in either radius.
fn a_degenerate_union_member_is_unsupported() -> Result<(), Box<dyn std::error::Error>> {
    for clip in [
        cherenkov::ShapeData::Circle(Circle::new((16.0, 16.0), 0.0)),
        cherenkov::ShapeData::Ellipse(Ellipse::new((16.0, 16.0), (4.0, 0.0), 0.0)),
        cherenkov::ShapeData::Ellipse(Ellipse::new((16.0, 16.0), (0.0, 4.0), 0.0)),
    ] {
        let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
        let surface =
            wait!(engine.surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {}))?;
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
        let result = wait!(engine.render(FrameTime::now()));
        assert!(
            matches!(
                result,
                Err(cherenkov::RenderError::Unsupported(name))
                    if name == "backdrop-union-degenerate-member"
            ),
            "degenerate clip {clip:?}: {result:?}"
        );
    }
    Ok(())
}
}

split_test! {
/// A union group past `BackdropUnion::MAX_MEMBERS` rejects by name.
fn union_members_past_the_cap_are_unsupported() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface =
        wait!(engine.surface(Offscreen::new((48, 32), OffscreenFormat::LinearF32), || {}))?;
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
    let result = wait!(engine.render(FrameTime::now()));
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(name)) if name == "backdrop-union-members"
        ),
        "{result:?}"
    );
    Ok(())
}
}

split_test! {
/// A member under a projective descendant of the anchor is outside the
/// anchor's canvas — the strict rule admits no descendant exemption.
fn a_member_under_a_projective_descendant_is_unsupported() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
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
    let result = wait!(engine.render(FrameTime::now()));
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(name)) if name == "backdrop-member-outside-anchor-canvas"
        ),
        "unexpected result {result:?}"
    );
    Ok(())
}
}

split_test! {
/// An anchor's filtered child is its own compositing canvas: a member
/// inside it is the anchor's descendant but outside its canvas — the
/// strict rule admits no descendant exemption.
fn a_member_inside_the_anchors_filtered_child_is_unsupported() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
    let anchor = surface.layer();
    let group = surface.backdrop_group_unfiltered(
        cherenkov::BackdropSpec::new(cherenkov::CaptureScale::FULL, cherenkov::CaptureLevels::ONE)
            .anchor(anchor.id()),
    );
    let filtered = surface.layer();
    let member = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&anchor);
        // `filtered` is the anchor's own child; the filter still makes
        // the member's canvas the filter's, not the anchor's.
        tx[&anchor].push(&filtered);
        let blur = engine.filter(filtrate::filters::GaussianBlur::new(1.0f32));
        tx[&filtered].filter(blur.id());
        tx[&filtered].push(&member);
        tx[&member]
            .clip(Rect::new(8.0, 8.0, 24.0, 24.0))
            .backdrop(group.sample());
    });
    let result = wait!(engine.render(FrameTime::now()));
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(name)) if name == "backdrop-member-outside-anchor-canvas"
        ),
        "unexpected result {result:?}"
    );
    Ok(())
}
}

split_test! {
/// An anchor's projective child is its own canvas: a member inside it is
/// rejected — the projective subtree admits no descendant exemption.
fn a_member_inside_the_anchors_projective_child_is_unsupported() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
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
    let result = wait!(engine.render(FrameTime::now()));
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(name)) if name == "backdrop-member-outside-anchor-canvas"
        ),
        "unexpected result {result:?}"
    );
    Ok(())
}
}

split_test! {
/// A filtered anchor's children paint in the filter's own canvas, not
/// the anchor's — an isolating anchor rejects its own member children.
fn an_isolating_anchor_rejects_its_member_children() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
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
    let result = wait!(engine.render(FrameTime::now()));
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(name)) if name == "backdrop-member-outside-anchor-canvas"
        ),
        "unexpected result {result:?}"
    );
    Ok(())
}
}

split_test! {
/// An anchor at the surface's root layer has nothing beneath it — the
/// capture would be the clear colour. The engine rejects it by name,
/// matching the scene format's `backdrop-anchor-at-root` error.
fn an_anchor_at_the_root_is_unsupported() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
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
    let result = wait!(engine.render(FrameTime::now()));
    assert!(
        matches!(
            result,
            Err(cherenkov::RenderError::Unsupported(name)) if name == "backdrop-anchor-at-root"
        ),
        "unexpected result {result:?}"
    );
    Ok(())
}
}
