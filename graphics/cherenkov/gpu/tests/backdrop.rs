//! Backdrop groups: bounded f16 captures sampled by member layers.

use cherenkov::kurbo::{Rect, RoundedRect};
use cherenkov::{__engine_fn as split_fn, __engine_test as split_test, __engine_wait as wait};
use cherenkov::{Bytes, Draw, Engine, FrameTime, Offscreen, OffscreenFormat, WorkingColor};
use cherenkov_gpu::{Gpu, GpuConfig};

fn pixel(readback: &cherenkov::Readback, x: usize, y: usize) -> [f32; 4] {
    let p = &readback.pixels[y * readback.width as usize + x];
    [p[0], p[1], p[2], p[3]]
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
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
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
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    let group = surface.backdrop_group(filtrate::filters::GaussianBlur(4.0f32));
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
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
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
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
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
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
    let group = surface.backdrop_group_unfiltered();
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
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
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
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
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
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
    let group = surface.backdrop_group_unfiltered();
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
/// A member layer that itself isolates (opacity < 1 on a Normal blend):
/// the member's sample draws to the current target under the member
/// clip, before and outside the layer's own isolation, so it is not
/// attenuated by the layer opacity; the layer's content is (#134).
fn member_sample_is_not_attenuated_by_layer_opacity() -> Result<(), Box<dyn std::error::Error>> {
    split_fn! {
// One engine for both opacities: each render builds its own surface, so
// the two scenes stay independent while engine construction is shared.
fn render(engine: &Engine<Gpu>, opacity: f32) -> Result<cherenkov::Readback, Box<dyn std::error::Error>> {
        let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
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
        wait!(engine.render(FrameTime::now()))?;
        Ok(wait!(surface.readback())?)
    }
    }

    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let half = wait!(render(&engine, 0.5))?;
    // Sample-only area (left half of the clip): the red backdrop at full
    // strength, unaffected by the layer's 0.5 opacity.
    assert_pixel(pixel(&half, 10, 16), [1.0, 0.0, 0.0, 1.0], 1e-3);
    // Inside the child: 50% blue over the sampled red = [0.5, 0.0, 0.5].
    assert_pixel(pixel(&half, 20, 16), [0.5, 0.0, 0.5, 1.0], 1e-3);
    // Outside the clip: the surface is untouched.
    assert_pixel(pixel(&half, 1, 16), [1.0, 0.0, 0.0, 1.0], 1e-3);
    // The clip edge is not squared: a corner pixel's coverage matches the
    // same layer at full opacity.
    let full = wait!(render(&engine, 1.0))?;
    assert_pixel(pixel(&half, 5, 5), pixel(&full, 5, 5), 1e-3);
    assert_pixel(pixel(&half, 7, 7), pixel(&full, 7, 7), 1e-3);
    Ok(())
}
}

split_test! {
fn colour_effect_tints_the_sampled_capture() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
    let group = surface.backdrop_group_unfiltered();
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
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    let group = surface.backdrop_group_unfiltered();
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
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    let group = surface.backdrop_group_unfiltered();
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
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    let group = surface.backdrop_group_unfiltered();
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
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
    let group = surface.backdrop_group_unfiltered();
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
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
    let group = surface.backdrop_group_unfiltered();
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
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
    let group = surface.backdrop_group_unfiltered();
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
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    let group = surface.backdrop_group_unfiltered();
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
        let surface = wait!(engine.surface(Offscreen::new((512, 512), OffscreenFormat::LinearF16)))?;
        let group_a = surface.backdrop_group_unfiltered();
        let group_b = two_groups.then(|| surface.backdrop_group_unfiltered());
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
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    let group = surface.backdrop_group_unfiltered();
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
    let surface = wait!(engine.surface(Offscreen::new((512, 512), OffscreenFormat::LinearF16)))?;
    let group = surface.backdrop_group_unfiltered();
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
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    let group = surface.backdrop_group_unfiltered();
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
