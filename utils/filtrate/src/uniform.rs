//! Uniform buffer types for filter shaders.

use bytemuck::{Pod, Zeroable};

use crate::Filter;

/// Uniform buffer for filter parameters.
///
/// This struct is passed to the GPU shader and contains all parameters
/// needed for any filter type. The `filter_type` field determines which
/// parameters are used.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct FilterUniform {
    /// Filter type ID (see Filter::type_id)
    pub filter_type: u32,
    /// Primary parameter (meaning depends on filter type)
    pub param1: f32,
    /// Secondary parameter (meaning depends on filter type)
    pub param2: f32,
    /// Padding for alignment
    pub _padding: u32,
    /// Texture dimensions (width, height)
    pub dimensions: [f32; 2],
    /// Additional padding for 16-byte alignment
    pub _padding2: [f32; 2],
}

impl FilterUniform {
    /// Create a uniform buffer from a filter and texture dimensions.
    #[must_use]
    pub fn from_filter(filter: &Filter, width: u32, height: u32) -> Self {
        let (filter_type, param1, param2) = match filter {
            Filter::Blur { radius } => (0, *radius, 0.0),
            Filter::Brightness { amount } => (1, *amount, 0.0),
            Filter::Saturation { amount } => (2, *amount, 0.0),
            Filter::Contrast { amount } => (3, *amount, 0.0),
            Filter::Grayscale { intensity } => (4, *intensity, 0.0),
            Filter::HueRotation { angle } => (5, *angle, 0.0),
            Filter::Invert => (6, 0.0, 0.0),
            Filter::Sepia { intensity } => (7, *intensity, 0.0),
            Filter::Vignette { radius, softness } => (8, *radius, *softness),
            Filter::Sharpen { amount } => (9, *amount, 0.0),
        };

        Self {
            filter_type,
            param1,
            param2,
            _padding: 0,
            dimensions: [width as f32, height as f32],
            _padding2: [0.0, 0.0],
        }
    }
}
