//! Optional CPU implementations of colour filters.

use crate::{Chain, ColorFilter, Filter, Footprint, WorkingSpace};

/// A window into a full CPU image.
///
/// `pixels` contains complete rows from `top` to `top + pixels.len() /
/// size.0`, while `size` is the full image size. Pixel `(x, y)` is sampled
/// at UV `((x + 0.5) / size.0, (top + y + 0.5) / size.1)`. Samples outside
/// the window clamp to its first or last row; columns clamp to the image
/// edges. A spatial filter's output is exact for rows at least
/// `Footprint::resolve` pixels from non-image-edge window boundaries.
#[derive(Debug)]
pub struct CpuImage<'a> {
    /// Premultiplied RGBA pixels in row-major order.
    pub pixels: &'a mut [[f32; 4]],
    /// The first row's y-coordinate in the full image.
    pub top: usize,
    /// The full image width and height.
    pub size: (usize, usize),
}

/// Why a CPU filter could not process its image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuFilterError {
    /// The filter requires an auxiliary GPU image without CPU texels.
    GpuImage {
        /// The index of the required image.
        index: usize,
    },
}

impl core::fmt::Display for CpuFilterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::GpuImage { index } => {
                write!(f, "auxiliary image {index} has no CPU texels")
            }
        }
    }
}

impl core::error::Error for CpuFilterError {}

/// Applies a filter to a CPU image window.
///
/// Spatial filters receive an apron around the output window. They must
/// clamp samples to the window boundaries; rows at least the resolved
/// [`Footprint`](crate::Footprint) from non-image-edge boundaries must match
/// a full-image application exactly.
pub trait CpuFilter: Filter {
    /// The rows and columns beyond a pixel its output reads: [`Footprint::ZERO`]
    /// for colour filters, [`SpatialFilter::footprint_of`](crate::SpatialFilter::footprint_of)
    /// for spatial ones.
    fn cpu_footprint(params: &Self::Params) -> Footprint;

    /// Apply the filter in place.
    ///
    /// # Errors
    /// Returns [`CpuFilterError::GpuImage`] when an auxiliary image has no
    /// CPU-side texels.
    fn apply_cpu_image(
        &self,
        params: &Self::Params,
        space: &WorkingSpace,
        image: &mut CpuImage<'_>,
    ) -> Result<(), CpuFilterError>;
}

/// A CPU implementation of a colour filter, which CPU backends run instead of
/// its shader.
///
/// A kernel must compute what the filter's stages compute: executors and the
/// correctness oracle cross-check the two.
pub trait CpuKernel: ColorFilter {
    /// Applies the filter to `pixels` in place. Pixels are premultiplied
    /// RGBA in the filter's operating space.
    fn apply_cpu(params: &Self::Params, space: &WorkingSpace, pixels: &mut [[f32; 4]]);

    /// Applies the filter with its current parameters.
    fn apply_cpu_now(&self, space: &WorkingSpace, pixels: &mut [[f32; 4]]) {
        Self::apply_cpu(&self.params(), space, pixels);
    }
}

impl<A: CpuKernel, B: CpuKernel> CpuKernel for Chain<A, B> {
    fn apply_cpu(params: &Self::Params, space: &WorkingSpace, pixels: &mut [[f32; 4]]) {
        A::apply_cpu(&params.0, space, pixels);
        B::apply_cpu(&params.1, space, pixels);
    }
}

impl<A: CpuFilter, B: CpuFilter> CpuFilter for Chain<A, B> {
    fn cpu_footprint(params: &Self::Params) -> Footprint {
        A::cpu_footprint(&params.0) + B::cpu_footprint(&params.1)
    }

    fn apply_cpu_image(
        &self,
        params: &Self::Params,
        space: &WorkingSpace,
        image: &mut CpuImage<'_>,
    ) -> Result<(), CpuFilterError> {
        self.first.apply_cpu_image(&params.0, space, image)?;
        self.second.apply_cpu_image(&params.1, space, image)
    }
}
