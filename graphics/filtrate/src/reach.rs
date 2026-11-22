//! A filter's footprint whatever its kind.
//!
//! [`SpatialFilter::footprint_of`] exists only for spatial filters, so code
//! generic over every [`Filter`] cannot ask for it. [`Reach`] answers for
//! both kinds, dispatching on [`Filter::Kind`] the way
//! [`ChainFootprint`](filtrate_core::kind::ChainFootprint) does for chains:
//! a colour filter reads only its own texel, a spatial one reports its
//! footprint.

use filtrate_core::{
    Filter, Footprint, SpatialFilter,
    kind::{Color, Kind, Spatial},
};

/// How filters of a kind report their footprint.
///
/// Implemented for the two kinds: [`Color`] for every colour filter,
/// [`Spatial`] for every [`SpatialFilter`].
pub trait KindReach<F: Filter>: Kind {
    /// The footprint of a filter of this kind for `params`.
    fn reach_of(params: &F::Params) -> Footprint;
}

impl<F: Filter<Kind = Self>> KindReach<F> for Color {
    fn reach_of(_params: &F::Params) -> Footprint {
        Footprint::ZERO
    }
}

impl<F: SpatialFilter> KindReach<F> for Spatial {
    fn reach_of(params: &F::Params) -> Footprint {
        F::footprint_of(params)
    }
}

/// A filter's [`Footprint`] for its parameters: [`Footprint::ZERO`] for a
/// colour filter, [`SpatialFilter::footprint_of`] for a spatial one.
///
/// Every filter that implements the [`ColorFilter`](filtrate_core::ColorFilter)
/// or [`SpatialFilter`] trait its kind requires implements it.
pub trait Reach: Filter {
    /// The largest distance between an output pixel and any input texel it
    /// reads, for `params` (see [`SpatialFilter::footprint_of`]).
    fn reach_of(params: &Self::Params) -> Footprint;
}

impl<F: Filter> Reach for F
where
    F::Kind: KindReach<F>,
{
    fn reach_of(params: &Self::Params) -> Footprint {
        <F::Kind as KindReach<F>>::reach_of(params)
    }
}

#[cfg(test)]
mod tests {
    use super::Reach;
    use crate::filters::{Blur, Brightness};
    use filtrate_core::{Filter, FilterExt, Footprint, SpatialFilter};

    #[test]
    fn colour_filters_reach_nothing_and_spatial_filters_report_their_footprint() {
        let brightness = Brightness(0.2f32);
        assert_eq!(
            <Brightness<f32> as Reach>::reach_of(&brightness.params()),
            Footprint::ZERO
        );
        let chain = Brightness(0.2f32).then(Blur(3.0f32));
        assert_eq!(reach(&chain), chain.footprint());
        assert!(reach(&chain).pixels > 0.0);
    }

    fn reach<F: Reach>(filter: &F) -> Footprint {
        F::reach_of(&filter.params())
    }

    #[test]
    fn generic_code_reaches_either_kind() {
        assert_eq!(reach(&Brightness(0.5f32)), Footprint::ZERO);
        assert_eq!(reach(&Blur(2.0f32)), Blur(2.0f32).footprint());
    }
}
