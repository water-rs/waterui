//! Layer removal ownership (water-rs/waterui#1804): a `Layer` handle owns
//! exactly its own layer. `LayerOp::Remove` removes only that layer — its
//! children stay in the tree, unattached and undrawn, until their own
//! handles drop or they are re-attached.

#![cfg(not(target_arch = "wasm32"))]

use cherenkov::kurbo::Rect;
use cherenkov::{Draw, Engine, FrameTime, Offscreen, OffscreenFormat, WorkingColor};
use cherenkov_cpu::{Raster, RasterConfig};

fn engine() -> Engine<Raster> {
    Engine::<Raster>::new(RasterConfig::default()).expect("engine")
}

const RED: WorkingColor = WorkingColor::new([1., 0., 0., 1.]);

/// The issue's repro: the parent drops first, then the child. Each drop
/// queues a `Remove` for its own layer only, so the child's `Remove` still
/// finds its layer in the tree.
#[test]
fn a_parent_dropped_before_its_child_removes_only_the_parent() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {})
        .expect("surface");
    let parent = surface.layer();
    let child = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&parent);
        tx[&parent].push(&child);
    });
    engine.render(FrameTime::now()).expect("first render");
    drop(parent);
    drop(child);
    engine.render(FrameTime::now()).expect("second render");
}

/// The reverse order — the child drops first, then the parent — leaves
/// the grandchild's layer in the tree for its own handle's `Remove`.
#[test]
fn a_child_dropped_before_its_parent_keeps_the_grandchild() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {})
        .expect("surface");
    let parent = surface.layer();
    let child = surface.layer();
    let grandchild = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&parent);
        tx[&parent].push(&child);
        tx[&child].push(&grandchild);
    });
    engine.render(FrameTime::now()).expect("first render");
    drop(child);
    drop(parent);
    engine.render(FrameTime::now()).expect("second render");
    drop(grandchild);
    engine.render(FrameTime::now()).expect("third render");
}

/// A child re-attached under another layer survives its old parent's
/// drop and keeps rendering under the new parent.
#[test]
fn a_child_reattached_before_its_parent_drops_still_renders() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let parent = surface.layer();
    let child = surface.layer();
    let other = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&parent);
        tx[surface.root()].push(&other);
        tx[&parent].push(&child);
        tx[&child].content(surface.record(|c| c.fill(Rect::new(0., 0., 32., 32.), RED)));
    });
    engine.render(FrameTime::now()).expect("first render");

    // The parent's `Remove` commits ahead of the re-attach: queued drops
    // come before the transaction's own ops.
    drop(parent);
    surface.update(|tx| {
        tx[&other].push(&child);
    });
    engine.render(FrameTime::now()).expect("second render");

    let rb = surface.readback().expect("readback");
    assert_eq!(
        rb.pixels[16 * 32 + 16],
        [1.0, 0.0, 0.0, 1.0],
        "the child still renders under its new parent"
    );
}
