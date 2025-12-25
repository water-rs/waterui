//! Source trait for lazy texture generation.

use crate::RenderContext;

/// A lazy texture source that encodes generation commands.
///
/// Sources represent content that can be rendered to a texture,
/// such as barcodes, procedural textures, or loaded images.
///
/// # Implementation
///
/// When `resolve` is called, the source should:
/// 1. Create any needed GPU resources (if not cached)
/// 2. Encode compute/render passes to `ctx.encoder`
/// 3. Return a view to the generated texture
///
/// The key insight is that `resolve` does NOT submit to the queue.
/// This allows multiple sources and filters to fuse into one submission.
pub trait Source: Send + Sync {
    /// Resolves the source, encoding generation commands and returning a texture view.
    ///
    /// This method should be idempotent when the source data hasn't changed.
    /// Implementations should cache their output texture and only regenerate
    /// when the underlying data is dirty.
    fn resolve(&mut self, ctx: &mut RenderContext<'_>) -> wgpu::TextureView;

    /// Returns the dimensions of the output texture.
    fn dimensions(&self) -> (u32, u32);
}
