//! Core Filter trait for GPU filter pipelines.
//!
//! Filters are pure data - they hold no GPU state. The [`Filter`] trait
//! provides the recipe (shader fragments and parameters) that the pipeline
//! compiles and executes.
//!
//! # Automatic Shader Fusion
//!
//! Filters with `COLOR_ONLY = true` (per-pixel operations that don't sample
//! neighbors) can be fused into a single GPU pass. The [`Chain`] type
//! preserves this information at compile time, enabling automatic optimization.
//!
//! # Example
//!
//! ```ignore
//! use filtrate_core::{Filter, FilterExt};
//!
//! // Create a filter chain
//! let chain = Grayscale(1.0)
//!     .then(Invert)           // Fuses with Grayscale
//!     .then(Blur(5.0))        // Separate pass (spatial filter)
//!     .then(Brightness(0.2)); // Fuses with nothing (after spatial)
//!
//! // Type encodes fusion potential:
//! // Chain<Chain<Chain<Grayscale, Invert>, Blur>, Brightness>
//! // COLOR_ONLY: false (because Blur is not color-only)
//! ```

use crate::{FragmentList, ParamArray};

/// A GPU filter that processes textures.
///
/// Filters are pure data - they provide shader fragments and parameters,
/// but hold no GPU state. The pipeline handles compilation and execution.
///
/// # Associated Types
///
/// - `Params`: The parameter values (e.g., `[f32; 1]` for brightness amount)
/// - `Fragments`: The shader code fragments (e.g., `&'static str`)
///
/// # Constants
///
/// - `COLOR_ONLY`: Whether this filter only performs per-pixel color operations
///   (doesn't sample neighboring pixels). Color-only filters can be fused
///   into a single pass for better performance.
pub trait Filter: 'static {
    /// Whether this filter performs only per-pixel color operations.
    ///
    /// - `true`: Filter reads only the current pixel, can be fused with adjacent
    ///   color-only filters into a single pass.
    /// - `false`: Filter samples neighboring pixels (e.g., blur, sharpen),
    ///   requires its own pass with intermediate texture.
    const COLOR_ONLY: bool;

    /// The parameter array type for this filter.
    type Params: ParamArray;

    /// The shader fragment list type for this filter.
    type Fragments: FragmentList;

    /// Get the current parameter values.
    ///
    /// For animated parameters, this samples the current value.
    /// Called each frame during render.
    fn params(&self) -> Self::Params;

    /// Get the shader fragments for this filter.
    ///
    /// Returns static shader code that will be composed with other fragments.
    fn fragments(&self) -> Self::Fragments;
}

/// A chain of two filters.
///
/// `Chain` implements `Filter`, preserving type information for automatic
/// fusion optimization. Consecutive color-only filters are fused into
/// a single GPU pass.
///
/// # Type-Level Fusion
///
/// ```ignore
/// // These three are all color-only, so Chain::COLOR_ONLY = true
/// type Fused = Chain<Chain<Grayscale, Brightness>, Contrast>;
///
/// // Blur is not color-only, so Chain::COLOR_ONLY = false
/// type Mixed = Chain<Fused, Blur>;
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Chain<A: Filter, B: Filter> {
    /// The first filter in the chain.
    pub first: A,
    /// The second filter in the chain.
    pub second: B,
}

impl<A: Filter, B: Filter> Filter for Chain<A, B> {
    /// Chain is color-only only if both filters are color-only.
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
}

/// Extension trait for chaining filters.
///
/// Provides the `.then()` method for composing filters into chains.
pub trait FilterExt: Filter + Sized {
    /// Chain this filter with another.
    ///
    /// The resulting `Chain` type preserves fusion information at compile time.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let chain = Grayscale(1.0)
    ///     .then(Brightness(0.1))
    ///     .then(Contrast(1.2));
    /// ```
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

    // Test filter for color-only operations
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
    }

    // Test filter for spatial operations
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
    }

    #[test]
    fn test_color_only_chain() {
        type ChainType = Chain<ColorFilter, ColorFilter>;
        assert!(ChainType::COLOR_ONLY);
    }

    #[test]
    fn test_mixed_chain() {
        type ChainType = Chain<ColorFilter, SpatialFilter>;
        assert!(!ChainType::COLOR_ONLY);
    }

    #[test]
    fn test_chain_params() {
        let chain = ColorFilter.then(SpatialFilter);
        let (a, b) = chain.params();
        assert_eq!(a, [1.0]);
        assert_eq!(b, [2.0, 3.0]);
    }

    #[test]
    fn test_chain_fragments() {
        let chain = ColorFilter.then(SpatialFilter);
        let (a, b) = chain.fragments();
        assert_eq!(a, "// color");
        assert_eq!(b, "// spatial");
    }

    #[test]
    fn test_deep_chain() {
        let chain = ColorFilter.then(ColorFilter).then(SpatialFilter);
        assert!(!<Chain<Chain<ColorFilter, ColorFilter>, SpatialFilter>>::COLOR_ONLY);
    }
}
