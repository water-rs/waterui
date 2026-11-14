//! Host-side check that every scenario's layer tree commits cleanly:
//! each tree is built on an offscreen surface and rendered, so a broken
//! op stream (a `Push`/`Content`/`Transform` for a layer whose `Create`
//! is missing — the ordering bug that killed the Pixel render thread)
//! surfaces here as `render` returning `Err`.

use cherenkov::kurbo::Rect;
use cherenkov::{Color, Content, Draw, Engine, FrameTime, Offscreen, OffscreenFormat, Srgb};
use cherenkov_gpu::{Gpu, GpuConfig};

use android_planes::scenario::Scenario;

fn controls(surface: &cherenkov::Surface<Gpu>) -> Content {
    surface.record(|c| {
        c.fill(
            Rect::new(24.0, 24.0, 660.0, 140.0),
            Color::<Srgb>::new([0.07, 0.09, 0.14, 0.72]),
        );
    })
}

fn engine() -> Engine<Gpu> {
    Engine::<Gpu>::new(GpuConfig::default()).expect("host GPU engine")
}

fn surface(engine: &Engine<Gpu>) -> cherenkov::Surface<Gpu> {
    engine
        .surface(
            Offscreen::new((960, 2142), OffscreenFormat::LinearF16),
            || {},
        )
        .expect("offscreen surface")
}

#[test]
fn every_scenario_tree_commits_and_renders() {
    let engine = engine();
    for name in [
        "overlay",
        "hdr",
        "clipped",
        "quarter",
        "rotated",
        "rounded",
        "no-overlay",
        "two",
        "in-engine",
    ] {
        let scenario = Scenario::parse(name);
        let surface = surface(&engine);
        // Keep both handles alive for the renders, as `Run` does.
        let (_videos, _rest, _controls) = scenario.build(&surface, controls(&surface));
        engine
            .render(FrameTime::now())
            .unwrap_or_else(|e| panic!("scenario {name}: first frame failed: {e}"));
        engine
            .render(FrameTime::now())
            .unwrap_or_else(|e| panic!("scenario {name}: second frame failed: {e}"));
    }
}

/// A layer allocated, pushed and dropped inside one transaction leaves a
/// `Push` naming the removed child: the commit drops it with the layer's
/// other stream edits, so only `Create` and `Remove` reach the tree and
/// rendering survives (water-rs/waterui#3453). The fail-fast on a raw
/// `Remove`-before-`Push` stream stays with the tree —
/// `cherenkov_record::tree` pins it.
#[test]
fn a_layer_dropped_inside_its_own_transaction_commits_cleanly() {
    let engine = engine();
    let surface = surface(&engine);
    surface.update(|tx| {
        let transient = surface.layer();
        tx[surface.root()].push(&transient);
    });
    engine
        .render(FrameTime::now())
        .expect("a transient layer commits only its Create and Remove");
}
