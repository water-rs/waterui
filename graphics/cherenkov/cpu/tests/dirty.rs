//! Retained/full equivalence on the CPU raster backend.

#![cfg(not(target_arch = "wasm32"))]
use cherenkov::Backend;

#[test]
fn randomized_incremental_matches_full_lowering() {
    let (mut renderer, _) =
        cherenkov_cpu::Raster::init(cherenkov_cpu::RasterConfig::default()).expect("CPU renderer");
    let font = cherenkov::FontData {
        data: std::fs::read("../scenes/fonts/NotoSans.ttf")
            .expect("scenes/fonts/NotoSans.ttf")
            .into(),
        index: 0,
    };
    cherenkov::testing::incremental::equivalence(&mut renderer, font);
}
