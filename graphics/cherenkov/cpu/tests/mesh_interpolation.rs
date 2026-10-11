//! Mesh colour weight selection on this backend.
#![cfg(not(target_arch = "wasm32"))]
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
#[path = "../../tests/common/mesh_interpolation.rs"]
mod common;
split_test! {
fn mesh_modes_patch_only_their_own_command() {
    wait!(common::interpolation::<cherenkov_cpu::Raster>(cherenkov_cpu::RasterConfig::default()));
}
}
