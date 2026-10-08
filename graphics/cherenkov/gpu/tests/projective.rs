//! Projective layers on the gpu backend.
#[path = "../../tests/common/projective.rs"]
mod common;

#[test]
fn identity_matches_affine() {
    common::identity_matches_affine::<cherenkov_gpu::Gpu>(cherenkov_gpu::GpuConfig::default);
}

#[test]
fn hidden_and_edge_on_contribute_nothing() {
    common::hidden_and_edge_on_contribute_nothing::<cherenkov_gpu::Gpu>(
        cherenkov_gpu::GpuConfig::default,
    );
}

#[test]
fn turns_keep_winding_and_show_both_sides() {
    common::turns_keep_winding_and_show_both_sides::<cherenkov_gpu::Gpu>(
        cherenkov_gpu::GpuConfig::default,
    );
}

#[test]
fn cached_and_fresh_realizations_are_identical() {
    common::cached_and_fresh_realizations_are_identical::<cherenkov_gpu::Gpu>(
        cherenkov_gpu::GpuConfig::default,
    );
}

#[test]
fn destructive_blend_keeps_its_operator_domain() {
    common::destructive_blend_keeps_its_operator_domain::<cherenkov_gpu::Gpu>(
        cherenkov_gpu::GpuConfig::default,
    );
}

#[test]
fn invalid_poses_are_errors() {
    common::invalid_poses_are_errors::<cherenkov_gpu::Gpu>(cherenkov_gpu::GpuConfig::default);
}

#[test]
fn tilt_animates_and_clears() {
    common::tilt_animates_and_clears::<cherenkov_gpu::Gpu>(cherenkov_gpu::GpuConfig::default);
}

#[test]
fn horizon_crossing_excludes_the_back_half_space() {
    common::horizon_crossing_excludes_the_back_half_space::<cherenkov_gpu::Gpu>(
        cherenkov_gpu::GpuConfig::default,
    );
}

#[test]
fn limits_are_explicit_errors() {
    common::limits_are_explicit_errors::<cherenkov_gpu::Gpu>(cherenkov_gpu::GpuConfig::default);
}

#[test]
fn backdrop_spaces_are_checked() {
    common::backdrop_spaces_are_checked::<cherenkov_gpu::Gpu>(cherenkov_gpu::GpuConfig::default);
}

#[test]
fn image_replacement_reaches_local_images() {
    common::image_replacement_reaches_local_images::<cherenkov_gpu::Gpu>(
        cherenkov_gpu::GpuConfig::default,
    );
}

#[test]
fn released_resources_leave_no_local_image() {
    common::released_resources_leave_no_local_image::<cherenkov_gpu::Gpu>(
        cherenkov_gpu::GpuConfig::default,
    );
}
