//! Component transforms reuse recorded content on the cpu backend.
#![cfg(not(target_arch = "wasm32"))]
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
#[path = "../../tests/common/component_animation.rs"]
mod common;

split_test! {
fn live_components_reuse_recorded_content() {
    wait!(common::component_animation::<cherenkov_cpu::Raster>(cherenkov_cpu::RasterConfig::default()));
}
}
