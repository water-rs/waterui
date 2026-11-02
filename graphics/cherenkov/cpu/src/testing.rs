//! Internal surfaces for benches and tests, exposed by the `testing`
//! feature — the same pattern `cherenkov`'s `testing` uses: nothing here
//! is public API, and without the feature the module does not exist.

pub use crate::render::GeometryChange;

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

    pub use crate::render::coverage::testing::RowReader;
    pub use crate::render::{ClipCoverage, ClipMask, ClipRef, ClipRun, CpuImage, Edge, PaintData};
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
        #[expect(
            clippy::inline_always,
            reason = "evaluated under the dispatched CPU level"
        )]
        #[inline(always)]
        fn eval<'a, S: fearless_simd::Simd>(
            simd: S,
            (paint, x0, py, out): (&PaintData, usize, usize, &'a mut [MaybeUninit<[f32; 4]>]),
        ) -> &'a mut [[f32; 4]] {
            paint.eval_span(simd, x0, py, out)
        }

        // Move the arguments into vectorize together, rather than capturing
        // references to the scalar coordinates and loading them again inside it.
        let args = (paint, x0, py, out);
        cherenkov_simd::dispatch!(level, simd => eval(simd, args))
    }

    /// `crate::render::composite_span` under `level`'s dispatch.
    ///
    /// Inlined into its caller, so a kernel benchmark counts one `Level`
    /// match and the dispatched kernel, not a call into this function that
    /// LLVM keeps or inlines depending on the size of the baseline
    /// level's inline arm.
    #[expect(
        clippy::inline_always,
        reason = "the per-call count must not depend on an inlining heuristic"
    )]
    #[inline(always)]
    pub fn composite_span(
        level: Level,
        dst: &mut [[f32; 4]],
        src: &[[f32; 4]],
        cov: &[f32],
        clip: ClipRun<'_>,
        space: BlendSpace,
    ) {
        cherenkov_simd::dispatch!(level, simd => {
            crate::render::composite_span(simd, dst, src, cov, clip, space);
        });
    }
    /// The opaque-solid specialization under `level`'s dispatch.
    pub fn composite_solid(
        level: Level,
        dst: &mut [[f32; 4]],
        cov: &[f32],
        clip: ClipRun<'_>,
        color: [f32; 4],
        space: BlendSpace,
    ) {
        #[expect(
            clippy::inline_always,
            reason = "evaluated under the dispatched CPU level"
        )]
        #[inline(always)]
        fn composite<S: fearless_simd::Simd>(
            simd: S,
            (dst, cov): (&mut [[f32; 4]], &[f32]),
            clip: ClipRun<'_>,
            color: [f32; 4],
            space: BlendSpace,
        ) {
            crate::render::composite_solid(simd, dst, cov, clip, color, space);
        }

        let args = (dst, cov);
        cherenkov_simd::dispatch!(level, simd => composite(simd, args, clip, color, space));
    }

    /// Uniform coverage through the production solid kernel.
    pub fn composite_uniform(
        level: Level,
        dst: &mut [[f32; 4]],
        coverage: f32,
        clip: ClipRun<'_>,
        color: [f32; 4],
        space: BlendSpace,
    ) {
        cherenkov_simd::dispatch!(level, simd => {
            crate::render::composite_solid(
                simd, dst, crate::render::coverage::Uniform(coverage), clip, color, space,
            );
        });
    }

    /// Bidirectional filter conversion under `level`'s dispatch.
    pub fn convert_span(level: Level, pixels: &mut [[f32; 4]], from: BlendSpace, to: BlendSpace) {
        cherenkov_simd::dispatch!(level, simd => {
            crate::render::convert_span(simd, pixels, from, to);
        });
    }
}
