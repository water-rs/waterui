//! Shader paints on the cpu backend.
#![cfg(feature = "shaders")]
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
use cherenkov_cpu::{Raster, RasterConfig};
#[path = "../../tests/support/shader_paint.rs"]
mod common;

type Outcome = Result<(), Box<dyn std::error::Error>>;

split_test! {
fn shader_registration_rejects_invalid_source() -> Outcome {
    wait!(common::rejects_invalid_source::<Raster>(RasterConfig::default()))
}
}

split_test! {
fn shader_paint_uses_shape_coverage_and_presentation_time() -> Outcome {
    wait!(common::coverage_and_presentation_time::<Raster>(RasterConfig::default()))
}
}

split_test! {
fn producer_color_helpers_match_working_space_and_alpha() -> Outcome {
    wait!(common::producer_color_helpers::<Raster>(RasterConfig::default()))
}
}

split_test! {
fn live_shader_operands_keep_other_cached_texture_uses() -> Outcome {
    wait!(common::live_operands_keep_other_uses::<Raster>(RasterConfig::default()))
}
}

split_test! {
fn clipped_paths_keep_full_geometry_shader_coordinates() -> Outcome {
    wait!(common::clipped_paths_keep_full_geometry::<Raster>(RasterConfig::default()))
}
}

split_test! {
fn removed_shader_reports_an_error_instead_of_panicking() -> Outcome {
    wait!(common::removed_shader_is_an_error::<Raster>(RasterConfig::default()))
}
}

split_test! {
fn shader_strokes_and_degenerate_geometry_keep_ordinary_coverage() -> Outcome {
    wait!(common::strokes_and_degenerate_geometry_keep_coverage::<Raster>(RasterConfig::default()))
}
}

split_test! {
fn shader_stroke_coordinates_include_the_outline() -> Outcome {
    wait!(common::stroke_coordinates_include_the_outline::<Raster>(RasterConfig::default()))
}
}

split_test! {
fn shader_glyph_runs_span_their_ink() -> Outcome {
    wait!(common::glyph_runs_span_their_ink::<Raster>(RasterConfig::default()))
}
}

split_test! {
fn shader_paint_transform_changes_sampling_without_moving_geometry() -> Outcome {
    wait!(common::paint_transform_moves_sampling_not_geometry::<Raster>(RasterConfig::default()))
}
}
