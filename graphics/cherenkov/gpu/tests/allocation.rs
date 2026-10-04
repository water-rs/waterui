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

split_test! {
/// Two dirty surfaces grow the shared instance buffer exactly once in a
/// single frame — frame-wide sizing, never one grow per surface.
fn multi_surface_frame_grows_instances_once() -> Result<(), Box<dyn std::error::Error>> {
    let sink = Sink::new();
    let engine = wait!(diag_engine(&sink))?;
    let a = wait!(engine.surface(Offscreen::new((128, 128), OffscreenFormat::LinearF16)))?;
    let b = wait!(engine.surface(Offscreen::new((128, 128), OffscreenFormat::LinearF16)))?;
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
    let surface = wait!(engine.surface(Offscreen::new((512, 512), OffscreenFormat::LinearF16)))?;
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
/// A backdrop capture regrowing across frames drops the unsubmitted
/// group-1 bind groups that held the predecessor's view — promptly, at
/// the binding-generation bump, not at the next encode's stamp clear.
/// The capture itself is grow-only: never recreated smaller.
fn stale_bind_groups_retire_at_capture_regen() -> Result<(), Box<dyn std::error::Error>> {
    let sink = Sink::new();
    let engine = wait!(diag_engine(&sink))?;
    let surface = wait!(engine.surface(Offscreen::new((128, 128), OffscreenFormat::LinearF16)))?;
    let group = surface.backdrop_group_unfiltered();
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
    let surface = wait!(engine.surface(Offscreen::new((256, 256), OffscreenFormat::LinearF16)))?;
    let group = surface.backdrop_group_unfiltered();
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
