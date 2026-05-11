//! Core Filter trait for GPU filter pipelines.
//!
//! Filters are pure data — they hold no GPU state. The [`Filter`] trait
//! provides the recipe (shader fragments, parameters, and stage layout)
//! that the runtime in `filtrate` compiles and executes.
//!
//! # Automatic Shader Fusion
//!
//! Filters with `COLOR_ONLY = true` (per-pixel operations that don't sample
//! neighbors) can be fused into a single GPU pass. The [`Chain`] type
//! preserves this information at compile time, enabling automatic
//! optimization at runtime via [`Filter::collect_stages`].
//!
//! # Example
//!
//! ```ignore
//! use filtrate_core::{Filter, FilterExt};
//!
//! // Build a filter chain. Type encodes fusion potential.
//! let chain = Grayscale(1.0)
//!     .then(Invert)         // Fuses with Grayscale
//!     .then(Blur(5.0))      // Separate pass (spatial filter)
//!     .then(Brightness(0.2));
//! ```

use crate::{FragmentList, ParamArray, SignalVisitor, StageCollector, visitor::OffsetVisitor};

/// A GPU filter that processes textures.
///
/// Filters are pure data — they provide shader fragments, parameters, and
/// a stage layout, but hold no GPU state. The pipeline handles compilation
/// and execution.
pub trait Filter: 'static {
    /// Whether this filter performs only per-pixel color operations.
    ///
    /// `true` filters can be fused with adjacent color-only filters into a
    /// single fragment-shader pass; `false` filters require their own pass.
    const COLOR_ONLY: bool;

    /// Parameter array layout for this filter (or chain).
    type Params: ParamArray;

    /// Shader fragment(s) for this filter (or chain).
    type Fragments: FragmentList;

    /// Snapshot the current parameter values.
    fn params(&self) -> Self::Params;

    /// Get the shader fragment(s).
    fn fragments(&self) -> Self::Fragments;

    /// Number of GPU passes this filter requires. Default `1`. Separable
    /// filters (e.g. two-pass blur) override to return more.
    fn pass_count(&self) -> u32 {
        1
    }

    /// Resolve output dimensions from input dimensions. Default returns
    /// the input unchanged. Filters that downsample / upsample override.
    fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32) {
        let _ = (input_width, input_height);
        (input_width, input_height)
    }

    /// Walk every atomic GPU stage this filter requires, in order. Each
    /// call to [`StageCollector`] records one shader source plus the
    /// number of `f32` parameters it consumes.
    ///
    /// Chains forward this method to their children.
    fn collect_stages<C: StageCollector>(&self, collector: &mut C);

    /// Walk every reactive parameter this filter owns, in flattened order.
    /// Default does nothing — useful for static filters with no signal
    /// inputs.
    fn visit_signals<V: SignalVisitor>(&self, _visitor: &mut V) {}
}

/// A chain of two filters.
///
/// `Chain` implements [`Filter`], preserving type information for automatic
/// fusion optimization. Consecutive color-only filters are fused into a
/// single GPU pass at runtime.
#[derive(Debug, Clone, Copy)]
pub struct Chain<A: Filter, B: Filter> {
    /// The first filter in the chain.
    pub first: A,
    /// The second filter in the chain.
    pub second: B,
}

impl<A: Filter, B: Filter> Filter for Chain<A, B> {
    /// Chain is color-only iff both children are color-only.
    const COLOR_ONLY: bool = A::COLOR_ONLY && B::COLOR_ONLY;

    type Params = (A::Params, B::Params);
    type Fragments = (A::Fragments, B::Fragments);

    #[inline]
    fn params(&self) -> Self::Params {
        (self.first.params(), self.second.params())
    }

    #[inline]
    fn fragments(&self) -> Self::Fragments {
        (self.first.fragments(), self.second.fragments())
    }

    #[inline]
    fn pass_count(&self) -> u32 {
        self.first.pass_count() + self.second.pass_count()
    }

    #[inline]
    fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32) {
        let (mid_w, mid_h) = self.first.output_size(input_width, input_height);
        self.second.output_size(mid_w, mid_h)
    }

    fn collect_stages<C: StageCollector>(&self, collector: &mut C) {
        self.first.collect_stages(collector);
        self.second.collect_stages(collector);
    }

    fn visit_signals<V: SignalVisitor>(&self, visitor: &mut V) {
        self.first.visit_signals(visitor);
        let mut offset = OffsetVisitor::new(visitor, <A::Params as ParamArray>::LEN);
        self.second.visit_signals(&mut offset);
    }
}

/// Extension trait for chaining filters.
pub trait FilterExt: Filter + Sized {
    /// Chain this filter with another.
    ///
    /// The resulting [`Chain`] preserves fusion information at compile
    /// time and is itself a [`Filter`].
    fn then<F: Filter>(self, filter: F) -> Chain<Self, F> {
        Chain {
            first: self,
            second: filter,
        }
    }
}

impl<T: Filter> FilterExt for T {}

#[cfg(test)]
mod tests {
    use super::*;

    struct ColorFilter;
    impl Filter for ColorFilter {
        const COLOR_ONLY: bool = true;
        type Params = [f32; 1];
        type Fragments = &'static str;

        fn params(&self) -> [f32; 1] {
            [1.0]
        }
        fn fragments(&self) -> &'static str {
            "// color"
        }
        fn collect_stages<C: StageCollector>(&self, c: &mut C) {
            c.color_fragment(self.fragments(), 1);
        }
    }

    struct SpatialFilter;
    impl Filter for SpatialFilter {
        const COLOR_ONLY: bool = false;
        type Params = [f32; 2];
        type Fragments = &'static str;

        fn params(&self) -> [f32; 2] {
            [2.0, 3.0]
        }
        fn fragments(&self) -> &'static str {
            "// spatial"
        }
        fn collect_stages<C: StageCollector>(&self, c: &mut C) {
            c.spatial_shader(self.fragments(), 2);
        }
    }

    #[test]
    fn color_only_chain_is_fully_color_only() {
        type ChainType = Chain<ColorFilter, ColorFilter>;
        assert!(ChainType::COLOR_ONLY);
    }

    #[test]
    fn mixed_chain_is_not_color_only() {
        type ChainType = Chain<ColorFilter, SpatialFilter>;
        assert!(!ChainType::COLOR_ONLY);
    }

    #[test]
    fn chain_params_are_concatenated_tuple() {
        let chain = ColorFilter.then(SpatialFilter);
        let (a, b) = chain.params();
        assert_eq!(a, [1.0]);
        assert_eq!(b, [2.0, 3.0]);
    }

    #[test]
    fn chain_fragments_are_concatenated_tuple() {
        let chain = ColorFilter.then(SpatialFilter);
        let (a, b) = chain.fragments();
        assert_eq!(a, "// color");
        assert_eq!(b, "// spatial");
    }

    #[test]
    fn deep_chain_inherits_spatial_color_only_correctly() {
        assert!(!<Chain<Chain<ColorFilter, ColorFilter>, SpatialFilter>>::COLOR_ONLY);
    }

    #[test]
    fn chain_pass_count_sums_children() {
        let chain = ColorFilter.then(SpatialFilter);
        assert_eq!(chain.pass_count(), 2);
    }

    #[test]
    fn collect_stages_walks_chain_in_order() {
        struct RecordingCollector(Vec<&'static str>);
        impl StageCollector for RecordingCollector {
            fn color_fragment(&mut self, src: &'static str, _: usize) {
                self.0.push(src);
            }
            fn spatial_shader(&mut self, src: &'static str, _: usize) {
                self.0.push(src);
            }
        }

        let chain = ColorFilter.then(SpatialFilter);
        let mut c = RecordingCollector(Vec::new());
        chain.collect_stages(&mut c);
        assert_eq!(c.0, vec!["// color", "// spatial"]);
    }
}
