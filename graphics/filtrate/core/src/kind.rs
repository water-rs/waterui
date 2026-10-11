//! Filter kinds as types.
//!
//! [`Filter::Kind`](crate::Filter::Kind) is [`Color`] or [`Spatial`]. The
//! kind of a [`Chain`](crate::Chain) is computed by [`Kind::Then`]: colour
//! followed by colour is colour, and anything involving a spatial filter is
//! spatial. The traits [`ColorFilter`](crate::ColorFilter) and
//! [`SpatialFilter`](crate::SpatialFilter) require the matching kind, so a
//! chain's kind is decided by the type system, never by a runtime flag.

use crate::{Filter, SpatialFilter};

mod sealed {
    pub trait Sealed {}
}

/// The kind of a filter: [`Color`] or [`Spatial`]. Sealed.
pub trait Kind: sealed::Sealed + 'static {
    /// The kind of a chain whose first filter has this kind and whose second
    /// filter has kind `K`.
    type Then<K: Kind>: Kind;

    /// The kind as a value, for executors that dispatch on it.
    const VALUE: FilterKind;
}

/// A filter that maps each pixel's colour to a colour.
#[derive(Debug)]
pub enum Color {}

/// A filter that samples its input around each pixel.
#[derive(Debug)]
pub enum Spatial {}

impl sealed::Sealed for Color {}
impl sealed::Sealed for Spatial {}

impl Kind for Color {
    type Then<K: Kind> = K;
    const VALUE: FilterKind = FilterKind::Color;
}

impl Kind for Spatial {
    type Then<K: Kind> = Self;
    const VALUE: FilterKind = FilterKind::Spatial;
}

/// A filter kind as a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterKind {
    /// [`Color`].
    Color,
    /// [`Spatial`].
    Spatial,
}

/// How a [`Chain`](crate::Chain) of a filter of kind `Self.0` and a filter of
/// kind `Self.1` combines its halves' footprints.
///
/// Implemented for the three kind pairs whose chain is spatial; a colour half
/// contributes nothing, and two spatial halves add, because the second samples
/// what the first already spread.
pub trait ChainFootprint<A: Filter, B: Filter> {
    /// The chain's footprint for the halves' parameters.
    fn footprint(first: &A::Params, second: &B::Params) -> crate::Footprint;
}

impl<A, B> ChainFootprint<A, B> for (Color, Spatial)
where
    A: Filter<Kind = Color>,
    B: SpatialFilter,
{
    fn footprint(_first: &A::Params, second: &B::Params) -> crate::Footprint {
        B::footprint_of(second)
    }
}

impl<A, B> ChainFootprint<A, B> for (Spatial, Color)
where
    A: SpatialFilter,
    B: Filter<Kind = Color>,
{
    fn footprint(first: &A::Params, _second: &B::Params) -> crate::Footprint {
        A::footprint_of(first)
    }
}

impl<A, B> ChainFootprint<A, B> for (Spatial, Spatial)
where
    A: SpatialFilter,
    B: SpatialFilter,
{
    fn footprint(first: &A::Params, second: &B::Params) -> crate::Footprint {
        A::footprint_of(first) + B::footprint_of(second)
    }
}
