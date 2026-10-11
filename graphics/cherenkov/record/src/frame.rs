//! Frame-rate vocabulary shared by the layer tree's sampler and its
//! consumers.

use std::ops::RangeInclusive;

/// A display's refresh-rate range in frames per second.
///
/// The range is as a scheduler reports it. Animation classification
/// ([`AnimationTrack::is_fast`](crate::animation::AnimationTrack::is_fast))
/// always uses the top of the range.
pub type RefreshRange = RangeInclusive<u32>;
