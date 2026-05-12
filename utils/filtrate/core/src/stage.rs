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
pub trait StageCollector {
    /// Record a color-only fragment. The fragment will be inlined into a
    /// fused fragment shader together with adjacent color-only fragments.
    fn color_fragment(&mut self, source: &'static str, param_count: usize);

    /// Record a standalone compute shader that samples neighboring pixels.
    /// Spatial stages are not fused; each gets its own pass.
    fn spatial_shader(&mut self, source: &'static str, param_count: usize);

    /// Record a standalone compute shader that also needs the original input
    /// texture bound at binding 3, in addition to the previous stage output.
    fn spatial_shader_with_original(&mut self, source: &'static str, param_count: usize) {
        self.spatial_shader(source, param_count);
    }
}
