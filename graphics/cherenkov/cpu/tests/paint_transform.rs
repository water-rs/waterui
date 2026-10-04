//! Independent paint coordinates on the cpu backend.
#![cfg(not(target_arch = "wasm32"))]
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
#[path = "../../tests/support/paint_transform.rs"]
mod common;

split_test! {
fn live_paint_transforms_preserve_geometry_and_retained_output() {
    wait!(common::retained::<cherenkov_cpu::Raster>(cherenkov_cpu::RasterConfig::default()));
}
}
split_test! {
fn nested_paint_transforms_compose_and_sample_analytically() {
    wait!(common::composition::<cherenkov_cpu::Raster>(cherenkov_cpu::RasterConfig::default()));
}
}
split_test! {
fn singular_and_non_finite_paint_transforms_fail() {
    wait!(common::invalid::<cherenkov_cpu::Raster>(cherenkov_cpu::RasterConfig::default()));
}
}

split_test! {
fn explicit_identity_mapping_preserves_pixels_exactly() {
    wait!(common::identity::<cherenkov_cpu::Raster>(cherenkov_cpu::RasterConfig::default()));
}
}
