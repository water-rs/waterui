//! Host-side check that each scenario's layer tree commits cleanly:
//! each tree is built on an offscreen surface and rendered, so a broken
//! op stream (a `Push`/`Content`/`Transform` for a layer whose `Create`
//! is missing) surfaces here as `render` returning `Err`.

use cherenkov::{Engine, FrameTime, Offscreen, OffscreenFormat};
use cherenkov_gpu::{Gpu, GpuConfig};

use apple_planes::scenario::Scenario;

fn engine() -> Engine<Gpu> {
    Engine::<Gpu>::new(GpuConfig::default()).expect("host GPU engine")
}

fn surface(engine: &Engine<Gpu>) -> cherenkov::Surface<Gpu> {
    engine
        .surface(Offscreen::new((960, 2142), OffscreenFormat::LinearF16))
        .expect("offscreen surface")
}

#[test]
fn every_scenario_tree_commits_and_renders() {
    let engine = engine();
    for name in ["overlay", "in-engine"] {
        let scenario = Scenario::parse(name).unwrap_or_else(|e| panic!("{e}"));
        let surface = surface(&engine);
        // Keep the handles alive for the renders, as `Run` does.
        let _built = scenario.build(&surface);
        engine
            .render(FrameTime::now())
            .unwrap_or_else(|e| panic!("scenario {name}: first frame failed: {e}"));
        engine
            .render(FrameTime::now())
            .unwrap_or_else(|e| panic!("scenario {name}: second frame failed: {e}"));
    }
}
