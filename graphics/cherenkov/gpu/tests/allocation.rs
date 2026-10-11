//! Allocation-lifetime gate tests (#169 A5): multi-surface frame growth,
//! atlas growth/retry, stale bind-group retirement and a repeated
//! small→big→small run with submissions in flight.

use cherenkov::Instant;
use cherenkov::{__engine_fn as split_fn, __engine_test as split_test, __engine_wait as wait};
use std::time::Duration;

use cherenkov::kurbo::{BezPath, Point, Rect};
use cherenkov::{
    Bytes, Draw, Engine, FrameTime, Offscreen, OffscreenFormat, Pressure, WorkingColor,
};
use cherenkov_gpu::diag::{AllocEvent, EventKind, Sink};
use cherenkov_gpu::{Gpu, GpuConfig};

split_fn! {
fn diag_engine(sink: &Sink) -> Result<Engine<Gpu>, Box<dyn std::error::Error>> {
    Ok(wait!(Engine::<Gpu>::new(GpuConfig {
        alloc_diag: Some(sink.clone()),
        ..Default::default()
    }))?)
}
}

fn grows(events: &[AllocEvent], label: &'static str) -> Vec<u64> {
    events
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::Grow { label: l, new, .. } if l == label => Some(new),
            _ => None,
        })
        .collect()
}

fn bind_drops(events: &[AllocEvent]) -> Vec<(u64, &'static str)> {
    events
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::BindGroups { dropped, reason } => Some((dropped, reason)),
            _ => None,
        })
        .collect()
}

fn uploads(events: &[AllocEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event.kind, EventKind::Upload { .. }))
        .count()
}

/// `(x, y, w, h)` of every `write_texture` into the glyph atlas.
fn atlas_writes(events: &[AllocEvent]) -> Vec<(u32, u32, u32, u32)> {
    events
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::Upload {
                label: "glyph atlas",
                rect: Some(rect),
                ..
            } => Some(rect),
            _ => None,
        })
        .collect()
}

/// A closed polygonal path, so the fills rasterize into atlas cells
/// instead of taking the analytic rect fast path.
fn polygon(points: &[(f64, f64)]) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(points[0]);
    for &p in &points[1..] {
        path.line_to(p);
    }
    path.close_path();
    path
}

split_test! {
/// Two dirty surfaces grow the shared instance buffer exactly once in a
/// single frame — frame-wide sizing, never one grow per surface.
fn multi_surface_frame_grows_instances_once() -> Result<(), Box<dyn std::error::Error>> {
    let sink = Sink::new();
    let engine = wait!(diag_engine(&sink))?;
    let a = wait!(engine.surface(Offscreen::new((128, 128), OffscreenFormat::LinearF16), || {}))?;
    let b = wait!(engine.surface(Offscreen::new((128, 128), OffscreenFormat::LinearF16), || {}))?;
    // 40 fills a side: 80 instances ≫ the 16-instance initial buffer.
    for surface in [&a, &b] {
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|r| {
                for i in 0..40 {
                    r.fill(
                        Rect::new(
                            f64::from(i) * 2.0,
                            0.0,
                            f64::from(i).mul_add(2.0, 60.0),
                            128.0,
                        ),
                        WorkingColor::new([0.2, 0.4, 0.6, 1.0]),
                    );
                }
            }));
        });
    }
    let _ = sink.take();
    let start = Instant::now();
    wait!(engine.render(FrameTime::at(start)))?;
    let grown = grows(&sink.take(), "instances");
    assert_eq!(
        grown.len(),
        1,
        "two dirty surfaces grew `instances` {}×, not once: {grown:?}",
        grown.len()
    );
    // The same frame re-rendered keeps the grown buffer: no regrow.
    let _ = sink.take();
    wait!(engine.render(FrameTime::at(start + Duration::from_millis(16))))?;
    assert_eq!(
        grows(&sink.take(), "instances").len(),
        0,
        "an unchanged second frame regrew `instances`"
    );
    Ok(())
}
}

split_test! {
/// Path cells exceeding the initial 1 MiB atlas take the transactional
/// grow path: one `Grow` on the glyph atlas, then the committed batch
/// uploads — the render does not abandon mid-frame.
fn atlas_growth_retries_and_commits() -> Result<(), Box<dyn std::error::Error>> {
    let sink = Sink::new();
    let engine = wait!(diag_engine(&sink))?;
    let surface = wait!(engine.surface(Offscreen::new((512, 512), OffscreenFormat::LinearF16), || {}))?;
    // 144 distinct stars with 34–48 px radii: each ~70–96² × 4 B of
    // fresh path cells ≈ 2–3.5 MiB, over the 1 MiB starting atlas.
    // Distinct geometry per star keeps the cells from deduplicating.
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            for i in 0..144u32 {
                let mut path = BezPath::new();
                let cx = f64::from(i % 12).mul_add(42.0, 26.0);
                let cy = f64::from(i / 12).mul_add(42.0, 26.0);
                for point in 0..5u32 {
                    let angle = f64::from(point)
                        .mul_add(144.0 + f64::from(i), -90.0)
                        .to_radians();
                    let radius = f64::from(i % 8).mul_add(2.0, 34.0);
                    let p = Point::new(
                        angle.cos().mul_add(radius, cx),
                        angle.sin().mul_add(radius, cy),
                    );
                    if point == 0 {
                        path.move_to(p);
                    } else {
                        path.line_to(p);
                    }
                }
                path.close_path();
                r.fill(path, WorkingColor::new([0.8, 0.3, 0.1, 1.0]));
            }
        }));
    });
    let _ = sink.take();
    wait!(engine.render(FrameTime::now()))?;
    let grown = grows(&sink.take(), "glyph atlas");
    assert!(
        grown.iter().any(|new| *new > 1024 * 1024),
        "a fresh path set bigger than the atlas must grow it: {grown:?}"
    );
    // The committed batch is stable: an identical second frame produces
    // no atlas uploads at all.
    wait!(engine.render(FrameTime::now()))?;
    let writes = uploads(&sink.take());
    assert_eq!(writes, 0, "a cached frame re-uploaded atlas cells");
    Ok(())
}
}

split_test! {
/// A path cell wider than the live atlas edge grows the atlas even when
/// eviction has left a dead band tall enough for it: the reclaimed band
/// spans only the page's width, so placing the cell there would upload
/// past the texture's x extent and kill the render thread (#2396).
fn a_wide_cell_never_takes_a_dead_band() -> Result<(), Box<dyn std::error::Error>> {
    let sink = Sink::new();
    let engine = wait!(diag_engine(&sink))?;
    // The width the issue's window failed at: 1100 px.
    let surface =
        wait!(engine.surface(Offscreen::new((1100, 240), OffscreenFormat::LinearF16), || {}))?;
    surface.clear_color(WorkingColor::new([0.0, 0.0, 0.0, 1.0]));
    let ink = WorkingColor::new([0.8, 0.3, 0.1, 1.0]);
    // Frame 1 stacks three shelves on the 1024 atlas: a class-8 strip
    // shelf at the origin (the box's fractional top and bottom rows), a
    // class-24 shelf (the tall triangle's single cell) and a class-16
    // shelf (the short triangle). Only the last touches the layout's
    // frontier, so evicting either of the others leaves a dead band,
    // not virgin rows.
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                polygon(&[(10.0, 10.5), (60.0, 10.5), (60.0, 30.5), (10.0, 30.5)]),
                ink,
            );
            r.fill(polygon(&[(90.0, 90.0), (110.0, 90.0), (100.0, 108.0)]), ink);
            r.fill(polygon(&[(130.0, 90.0), (150.0, 90.0), (140.0, 100.0)]), ink);
        }));
    });
    wait!(engine.render(FrameTime::now()))?;
    // Frame 2 needs one strip cell 1098 texels wide — a band across the
    // 1100 px surface whose inflated coverage fits one strip. No live
    // shelf or virgin row takes it on the 1024 edge, and the cold shelves
    // eviction reclaims leave a dead band tall enough for its class.
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                polygon(&[(1.0, 202.5), (1099.0, 202.5), (1099.0, 205.0), (1.0, 205.0)]),
                ink,
            );
        }));
    });
    let _ = sink.take();
    wait!(engine.render(FrameTime::now()))?;
    let grown = grows(&sink.take(), "glyph atlas");
    assert!(
        grown.iter().any(|new| *new >= 2048 * 2048),
        "a cell wider than the 1024 edge must grow the atlas edge: {grown:?}"
    );
    let pixels = wait!(surface.readback())?;
    let px = |x: u32, y: u32| pixels.pixels[(y * pixels.width + x) as usize];
    // The half-covered top row reads half ink across the cell's whole
    // width, past the old 1024 edge included.
    for x in [2, 512, 1050, 1097] {
        let [r, _, _, a] = px(x, 202);
        assert!((r - 0.4).abs() < 0.02 && a > 0.99, "row 202 at {x}: {r} {a}");
        let [r, _, _, _] = px(x, 203);
        assert!((r - 0.8).abs() < 0.02, "row 203 at {x}: {r}");
    }
    Ok(())
}
}

split_test! {
/// A frame's pending path cells hold no atlas origin yet: resolving
/// their still-`(0, 0)` UVs pins whatever shelf covers the atlas
/// origin, so the cold band at `y = 0` survives eviction and the
/// commit grows the atlas instead of reclaiming it (#2440).
fn a_pending_cell_does_not_pin_the_origin_shelf() -> Result<(), Box<dyn std::error::Error>> {
    let sink = Sink::new();
    let engine = wait!(diag_engine(&sink))?;
    let surface =
        wait!(engine.surface(Offscreen::new((1030, 480), OffscreenFormat::LinearF16), || {}))?;
    surface.clear_color(WorkingColor::new([0.0, 0.0, 0.0, 1.0]));
    let ink = WorkingColor::new([0.8, 0.3, 0.1, 1.0]);
    // Frame 1: a box with fractional top and bottom rows puts a class-8
    // strip shelf at the atlas origin. Rendered alone so it lands
    // before the page filler's allocations.
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                polygon(&[(10.0, 10.5), (60.0, 10.5), (60.0, 30.5), (10.0, 30.5)]),
                ink,
            );
        }));
    });
    wait!(engine.render(FrameTime::now()))?;
    // Frame 2's fillers live on their own layer, so the root replace
    // below leaves every one of their shelves sampled: three clip
    // masks stack class-264/272/480 bands — exactly the rest of the
    // page — and their masked fills pin them every frame.
    let fillers = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&fillers);
        tx[&fillers].content(surface.record(|r| {
            for clip in [
                polygon(&[(420.0, 4.0), (550.0, 4.0), (485.0, 264.0)]),
                polygon(&[(556.0, 4.0), (686.0, 4.0), (621.0, 270.0)]),
                polygon(&[(692.0, 4.0), (822.0, 4.0), (757.0, 476.0)]),
            ] {
                r.clip(clip, |r| {
                    r.fill(Rect::new(0.0, 0.0, 1030.0, 480.0), ink);
                });
            }
        }));
    });
    wait!(engine.render(FrameTime::now()))?;
    // Frame 3: replace the content with one path whose single strip
    // cell — 1020 texels of class 8 — fits neither the origin shelf's
    // spent run nor the exhausted virgin rows. Reclaiming the cold
    // shelf at `y = 0` takes the cell; pinning that shelf through the
    // cell's unwritten UV would grow the atlas instead (#2440).
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                polygon(&[(1.0, 270.5), (1019.0, 270.5), (1019.0, 272.5), (1.0, 272.5)]),
                ink,
            );
        }));
    });
    let _ = sink.take();
    wait!(engine.render(FrameTime::now()))?;
    let events = sink.take();
    let grown = grows(&events, "glyph atlas");
    assert!(
        grown.is_empty(),
        "reclaiming the cold origin band must not grow the atlas: {grown:?}"
    );
    assert!(
        atlas_writes(&events).iter().any(|&(_, y, _, _)| y == 0),
        "the strip cell should land in the evicted band at y = 0: {:?}",
        atlas_writes(&events)
    );
    let pixels = wait!(surface.readback())?;
    let px = |x: u32, y: u32| pixels.pixels[(y * pixels.width + x) as usize];
    for x in [2, 512, 800, 1018] {
        let [r, _, _, a] = px(x, 270);
        assert!((r - 0.4).abs() < 0.02 && a > 0.99, "row 270 at {x}: {r} {a}");
        let [r, _, _, _] = px(x, 271);
        assert!((r - 0.8).abs() < 0.02, "row 271 at {x}: {r}");
    }
    Ok(())
}
}

split_test! {
/// A backdrop capture regrowing across frames drops the unsubmitted
/// group-1 bind groups that held the predecessor's view — promptly, at
/// the binding-generation bump, not at the next encode's stamp clear.
/// The capture itself is grow-only: never recreated smaller.
fn stale_bind_groups_retire_at_capture_regen() -> Result<(), Box<dyn std::error::Error>> {
    let sink = Sink::new();
    let engine = wait!(diag_engine(&sink))?;
    let surface = wait!(engine.surface(Offscreen::new((128, 128), OffscreenFormat::LinearF16), || {}))?;
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let glass = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 128.0, 128.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
        tx[surface.root()].push(&glass);
        tx[&glass]
            .clip(Rect::new(0.0, 0.0, 16.0, 16.0))
            .backdrop(group.sample());
    });
    let start = Instant::now();
    wait!(engine.render(FrameTime::at(start)))?;
    surface.update(|tx| {
        tx[&glass].clip(Rect::new(0.0, 0.0, 64.0, 64.0));
    });
    wait!(engine.render(FrameTime::at(start + Duration::from_millis(16))))?;
    let drops = bind_drops(&sink.take());
    assert!(
        drops
            .iter()
            .any(|(dropped, reason)| *dropped > 0 && *reason == "capture regen"),
        "no bind groups retired at capture regen: {drops:?}"
    );
    assert_eq!(
        wait!(engine.memory()).backdrop_captures,
        Bytes(64 * 64 * 8),
        "the capture retained its largest frame size (grow-only)"
    );
    // Now shrink the clip: the capture must NOT follow the smaller
    // region — grow-only means no regen back down.
    surface.update(|tx| {
        tx[&glass].clip(Rect::new(0.0, 0.0, 16.0, 16.0));
    });
    wait!(engine.render(FrameTime::at(start + Duration::from_millis(32))))?;
    assert_eq!(
        wait!(engine.memory()).backdrop_captures,
        Bytes(64 * 64 * 8),
        "a smaller clip shrank the capture inside the frame path"
    );
    Ok(())
}
}

split_test! {
/// A small→big→small sequence renders correctly while earlier
/// submissions are still in flight, and an explicit `trim` afterwards
/// releases the big frame's scratch and capture textures.
fn small_big_small_with_work_in_flight() -> Result<(), Box<dyn std::error::Error>> {
    let sink = Sink::new();
    let engine = wait!(diag_engine(&sink))?;
    let surface = wait!(engine.surface(Offscreen::new((256, 256), OffscreenFormat::LinearF16), || {}))?;
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let glass = surface.layer();
    let start = Instant::now();
    let mut was_big = false;
    let mut render = |big: bool, frame: u64| {
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|r| {
                r.fill(
                    Rect::new(0.0, 0.0, 256.0, 256.0),
                    WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
                );
            }));
            if big && !was_big {
                tx[surface.root()].push(&glass);
                tx[&glass]
                    .clip(Rect::new(0.0, 0.0, 256.0, 256.0))
                    .backdrop(group.sample());
            } else if !big && was_big {
                tx[surface.root()].remove(&glass);
            }
        });
        was_big = big;
        engine.render(FrameTime::at(start + Duration::from_millis(16 * frame)))
    };
    // No readback or poll between renders — submissions stay in flight.
    wait!(render(false, 0))?;
    wait!(render(true, 1))?;
    wait!(render(false, 2))?;
    wait!(render(true, 3))?;
    wait!(render(false, 4))?;
    let before_trim = wait!(engine.memory());
    engine.trim(Pressure::Moderate);
    let after_trim = wait!(engine.memory());
    assert!(
        after_trim.gpu < before_trim.gpu,
        "trim did not release the retired scratch/capture textures: \
         {before_trim:?} -> {after_trim:?}"
    );
    // The surface still renders after its scratch was trimmed.
    wait!(render(true, 5))?;
    assert_eq!(wait!(engine.memory()).backdrop_captures, Bytes(256 * 256 * 8));
    Ok(())
}
}
