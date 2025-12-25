//! Filter trait for GPU processing.

use crate::RenderContext;

/// A filter that encodes processing commands.
///
/// Filters transform textures by encoding compute or render passes.
/// They read from an input texture and write to an output texture.
///
/// # Implementation
///
/// When `encode` is called, the filter should:
/// 1. Create any needed pipelines (if not cached)
/// 2. Encode a compute/render pass to `ctx.encoder`
///
/// The key insight is that `encode` does NOT submit to the queue.
/// This allows multiple filters to fuse into one submission.
pub trait Filter: Send + Sync {
    /// Encodes the filter operation.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The render context with shared encoder
    /// * `input` - The input texture view to read from
    /// * `output` - The output texture view to write to
    fn encode(
        &self,
        ctx: &mut RenderContext<'_>,
        input: &wgpu::TextureView,
        output: &wgpu::TextureView,
    );
}
