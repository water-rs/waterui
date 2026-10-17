//! Internal surfaces for benches and tests, exposed by the `testing`
//! feature — the same pattern `cherenkov`'s `testing` uses: nothing here
//! is public API, and without the feature the module does not exist.

/// The span kernels and the minimum entry points a benchmark needs to
/// drive them: the real `paint_data` lowering and the real `CpuImage`
/// decode, so a bench measures data the renderer actually produces.
///
/// The kernel entry points take a [`Level`] and dispatch once per call —
/// the renderer threads the concrete token through the run instead.
pub mod kernels {
    use std::mem::MaybeUninit;
    use std::sync::Arc;

    use cherenkov::kurbo::Affine;
    use cherenkov::{ImageUpload, Paint, RenderError, ResourceError};
    use rustc_hash::FxHashMap;

    use crate::Level;
    use crate::render::paint_data;

    pub use crate::render::{Accum, ClipMask, ClipRef, ClipRun, CpuImage, Edge, PaintData};
    pub use cherenkov::BlendSpace;

    /// Lowers `paint` into [`PaintData`] exactly as the renderer does:
    /// `inv` maps device space into content space and `images` are the
    /// registered images keyed by `ImageId::raw`.
    ///
    /// # Errors
    /// The same as `paint_data`'s.
    pub fn paint(
        paint: &Paint,
        inv: Affine,
        images: impl IntoIterator<Item = (u64, Arc<CpuImage>)>,
    ) -> Result<PaintData, RenderError> {
        let images: FxHashMap<u64, Arc<CpuImage>> = images.into_iter().collect();
        paint_data(paint, inv, &images)
    }

    /// Decodes `upload` into a [`CpuImage`] exactly as image registration
    /// does.
    ///
    /// # Errors
    /// The same as [`CpuImage::decode`]'s.
    pub fn image(upload: &ImageUpload) -> Result<CpuImage, ResourceError> {
        CpuImage::decode(upload)
    }

    /// [`PaintData::eval_span`] under `level`'s dispatch.
    pub fn eval_span<'a>(
        level: Level,
        paint: &PaintData,
        x0: usize,
        py: usize,
        out: &'a mut [MaybeUninit<[f32; 4]>],
    ) -> &'a mut [[f32; 4]] {
        fearless_simd::dispatch!(level, simd => paint.eval_span(simd, x0, py, out))
    }

    /// `crate::render::composite_span` under `level`'s dispatch.
    pub fn composite_span(
        level: Level,
        dst: &mut [[f32; 4]],
        src: &[[f32; 4]],
        cov: &[f32],
        clip: ClipRun<'_>,
        space: BlendSpace,
    ) {
        fearless_simd::dispatch!(level, simd => {
            crate::render::composite_span(simd, dst, src, cov, clip, space);
        });
    }
}
