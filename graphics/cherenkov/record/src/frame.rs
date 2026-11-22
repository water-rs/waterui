//! Frame-rate vocabulary shared by the layer tree's sampler and its
//! consumers.

use std::ops::RangeInclusive;

/// A display's refresh-rate range in frames per second.
///
/// The range is as a scheduler reports it. Animation classification into
/// the rate classes ([`RATE_FAST`](crate::tree::RATE_FAST),
/// [`RATE_SLOW`](crate::tree::RATE_SLOW)) always uses the top of the range.
pub type RefreshRange = RangeInclusive<u32>;
