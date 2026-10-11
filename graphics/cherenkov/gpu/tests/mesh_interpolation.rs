//! Mesh colour weight selection on this backend.
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
#[path = "../../tests/common/mesh_interpolation.rs"]
mod common;
split_test! {
fn mesh_modes_patch_only_their_own_command() {
    wait!(common::interpolation::<cherenkov_gpu::Gpu>(cherenkov_gpu::GpuConfig::default()));
}
}
