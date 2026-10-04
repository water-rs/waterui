//! General silhouette convolution.
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
#[path = "../../tests/common/silhouette_shadows.rs"]
mod common;
split_test! {
fn live_shadows_keep_offscreen_contributors_and_outer_clips() {
    wait!(common::retained_and_padded::<cherenkov_gpu::Gpu>(cherenkov_gpu::GpuConfig::default()));
}
}

split_test! {
fn invalid_silhouette_inputs_fail_explicitly() {
    wait!(common::invalid::<cherenkov_gpu::Gpu>(cherenkov_gpu::GpuConfig::default));
}
}
