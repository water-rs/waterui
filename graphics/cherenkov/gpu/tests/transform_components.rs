//! Component transforms reuse recorded content on the gpu backend.
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
#[path = "../../tests/common/component_animation.rs"]
mod common;

split_test! {
fn live_components_reuse_recorded_content() {
    wait!(common::component_animation::<cherenkov_gpu::Gpu>(cherenkov_gpu::GpuConfig::default()));
}
}
