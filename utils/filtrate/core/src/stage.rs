//! Stage collection abstraction for filter graph compilation.
//!
//! `Filter::collect_stages` walks a (possibly chained) filter and records
//! every atomic GPU pass it requires. The runtime planner then fuses
//! consecutive color-only stages into a single fragment shader and emits
//! separate compute passes for spatial filters.
//!
//! `StageCollector` is intentionally narrow: it only accepts shader source
//! strings and parameter counts. The runtime is responsible for everything
//! else (bind groups, scratch textures, ping-pong, …).

/// Sink for atomic stages emitted by [`crate::Filter::collect_stages`].
///
/// Every method is required: a collector that silently coalesced
/// [`Self::spatial_shader_with_original`] into [`Self::spatial_shader`]
/// would bind a different layout than the shader declares.
pub trait StageCollector {
    /// Record a color-only fragment. The fragment will be inlined into a
    /// fused fragment shader together with adjacent color-only fragments.
    fn color_fragment(&mut self, source: &'static str, param_count: usize);

    /// Record a standalone compute shader that samples neighboring pixels.
    /// Spatial stages are not fused; each gets its own pass.
    fn spatial_shader(&mut self, source: &'static str, param_count: usize);

    /// Record a standalone compute shader that additionally binds the
    /// texture that fed this filter's FIRST stage at binding 3 (the
    /// "original" input, before any of this filter's own passes ran).
    ///
    /// A filter must emit such a stage only as a follow-up to a preceding
    /// stage of the same filter (e.g. bloom's vertical pass following its
    /// horizontal pass): the runtime resolves the original as the source
    /// texture of the immediately preceding pass, falling back to the
    /// pipeline input when this is the first pass overall.
    fn spatial_shader_with_original(&mut self, source: &'static str, param_count: usize);
}
