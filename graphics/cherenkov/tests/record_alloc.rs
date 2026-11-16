//! Regression test for allocation-free steady-state recording and rendering.

#![cfg(all(feature = "testing", not(target_arch = "wasm32")))]

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc;

use cherenkov::kurbo::{PathEl, Point, Rect};
use cherenkov::testing::allocations::{AllocationCounts, ThreadAllocator, count};
use cherenkov::testing::{Null, NullConfig};
use cherenkov::{
    Draw, Engine, FillRule, Fixed, FrameTime, Glyph, GlyphRun, GlyphStyle, Offscreen,
    OffscreenFormat, ShapeData, WorkingColor,
};

#[global_allocator]
static ALLOCATOR: ThreadAllocator = ThreadAllocator;

/// No allocation, reallocation or free.
const NONE: AllocationCounts = AllocationCounts {
    allocations: 0,
    reallocations: 0,
    frees: 0,
};

#[test]
fn recording_and_rendering_reuse_ui_thread_allocations() {
    let (events, _receiver) = mpsc::channel();
    let engine = Engine::<Null>::new(NullConfig {
        events,
        reject: HashSet::new(),
        image_limits: cherenkov::ImageLimits::UNLIMITED,
    })
    .expect("init");
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {})
        .expect("surface");
    let layer = surface.layer();
    let path = ShapeData::Path {
        elements: Arc::from([
            PathEl::MoveTo(Point::new(1.0, 1.0)),
            PathEl::LineTo(Point::new(20.0, 1.0)),
            PathEl::LineTo(Point::new(10.0, 20.0)),
            PathEl::ClosePath,
        ]),
        rule: FillRule::NonZero,
    };
    let run = GlyphRun {
        font: cherenkov::FontId::new(1),
        size: 16.0,
        coords: Arc::from([]),
        glyphs: Arc::from([Glyph {
            id: 1,
            x: 1.0,
            y: 16.0,
            transform: None,
        }]),
        style: GlyphStyle::Fill,
    };

    for frame in 0..12 {
        let time = FrameTime::now();
        let ((), counts) = count(|| {
            surface.update(|tx| {
                tx[&layer].record(|recorder| {
                    recorder.fill(Fixed(path.clone()), Fixed(WorkingColor::WHITE));
                    recorder.fill(
                        Fixed(Rect::new(2.0, 2.0, 24.0, 24.0)),
                        Fixed(WorkingColor::BLACK),
                    );
                    recorder.glyphs(Fixed(run.clone()), Fixed(WorkingColor::WHITE));
                });
            });
        });
        if frame >= 3 {
            assert_eq!(counts, NONE, "frame {}", frame + 1);
        }
        engine.render(time).expect("render");
    }
}

/// A steady-state edit transaction — `run_transaction` with one opacity
/// edit — holds a fixed allocation budget: after warmup every transaction
/// allocates exactly the baseline, never more. The baseline is pinned at
/// what dev measures for the same loop (2 allocations, 2 frees: the
/// transaction's `Ops` batch and its edit record); an increase is a
/// regression in the edit path, not noise.
#[test]
fn edit_transactions_reuse_their_allocations() {
    let (events, _receiver) = mpsc::channel();
    let engine = Engine::<Null>::new(NullConfig {
        events,
        reject: HashSet::new(),
        image_limits: cherenkov::ImageLimits::UNLIMITED,
    })
    .expect("init");
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {})
        .expect("surface");
    let layer = surface.layer();
    surface.update(|tx| {
        tx[&layer].record(|recorder| {
            recorder.fill(
                Fixed(Rect::new(0.0, 0.0, 4.0, 4.0)),
                Fixed(WorkingColor::WHITE),
            );
        });
    });

    for transaction in 0..12 {
        let ((), counts) = count(|| {
            surface.update(|tx| {
                tx[&layer].opacity(0.5_f32);
            });
        });
        if transaction >= 3 {
            assert_eq!(
                counts,
                AllocationCounts {
                    allocations: 2,
                    reallocations: 0,
                    frees: 2,
                },
                "transaction {}",
                transaction + 1
            );
        }
        // Rendering drains the transaction's staged ops so `pending` does
        // not carry capacity growth into the next iteration's count.
        engine.render(FrameTime::now()).expect("render");
    }
}

/// Dropping a layer whose only edit was a constant holds dev's budget:
/// nothing. A constant's watch drops its watcher, so the edit keeps no
/// binding entry, and the removal reaches the layer's own bindings
/// through the surface's per-layer index — it neither collects dead
/// entries nor scans the surface's other bindings.
#[test]
fn removing_a_constant_edited_layer_allocates_nothing() {
    let (events, _receiver) = mpsc::channel();
    let engine = Engine::<Null>::new(NullConfig {
        events,
        reject: HashSet::new(),
        image_limits: cherenkov::ImageLimits::UNLIMITED,
    })
    .expect("init");
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {})
        .expect("surface");

    for removal in 0..12 {
        let layer = surface.layer();
        surface.update(|tx| {
            tx[&layer].opacity(0.5_f32);
        });
        engine.render(FrameTime::now()).expect("render");
        let ((), counts) = count(|| drop(layer));
        if removal >= 3 {
            assert_eq!(counts, NONE, "removal {}", removal + 1);
        }
        engine.render(FrameTime::now()).expect("render");
    }
}
