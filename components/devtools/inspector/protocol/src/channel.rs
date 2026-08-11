//! Event channels and subscription sets.
//!
//! The runtime produces nothing for a channel nobody subscribed to. Every
//! channel is therefore both a routing key and a permission: a backend asks
//! [`ChannelSet::contains`] before it spends anything measuring.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// A set of event channels.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ChannelSet: u16 {
        /// Per-frame render timings from the rendering backend.
        const FRAMES = 1 << 0;
        /// Accessibility-tree snapshots and incremental updates.
        const TREE = 1 << 1;
        /// Aggregated main-thread task occupancy, plus individual stalls.
        const TASKS = 1 << 2;
        /// `tracing` records emitted by the target application.
        const LOGS = 1 << 3;
        /// Reactive-graph lifecycle and notification events.
        const SIGNALS = 1 << 4;
    }
}

impl ChannelSet {
    /// The channels an inspector opens by default.
    ///
    /// [`ChannelSet::SIGNALS`] is deliberately excluded: it reports every
    /// notification in the reactive graph and is opened deliberately, per
    /// investigation, rather than left running.
    #[must_use]
    pub const fn default_subscription() -> Self {
        Self::FRAMES
            .union(Self::TREE)
            .union(Self::TASKS)
            .union(Self::LOGS)
    }
}

/// One event channel.
///
/// Used where a single channel must be named — reporting a drop, for instance —
/// rather than a set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Channel {
    /// Per-frame render timings.
    Frames,
    /// Accessibility-tree updates.
    Tree,
    /// Main-thread task occupancy.
    Tasks,
    /// `tracing` records.
    Logs,
    /// Reactive-graph events.
    Signals,
}

impl Channel {
    /// Every channel, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::Frames,
        Self::Tree,
        Self::Tasks,
        Self::Logs,
        Self::Signals,
    ];

    /// The single-channel set for this channel.
    #[must_use]
    pub const fn as_set(self) -> ChannelSet {
        match self {
            Self::Frames => ChannelSet::FRAMES,
            Self::Tree => ChannelSet::TREE,
            Self::Tasks => ChannelSet::TASKS,
            Self::Logs => ChannelSet::LOGS,
            Self::Signals => ChannelSet::SIGNALS,
        }
    }

    /// Stable lowercase name, for display and for command-line selection.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Frames => "frames",
            Self::Tree => "tree",
            Self::Tasks => "tasks",
            Self::Logs => "logs",
            Self::Signals => "signals",
        }
    }
}

impl core::fmt::Display for Channel {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::{Channel, ChannelSet};

    /// `ALL` and the bitflag set must not drift apart — a channel added to one
    /// and forgotten in the other would be silently unroutable.
    #[test]
    fn every_channel_has_a_distinct_bit() {
        let union = Channel::ALL
            .iter()
            .fold(ChannelSet::empty(), |set, channel| set | channel.as_set());
        assert_eq!(union, ChannelSet::all());
        assert_eq!(Channel::ALL.len(), ChannelSet::all().bits().count_ones() as usize);
    }

    #[test]
    fn signals_are_not_subscribed_by_default() {
        assert!(!ChannelSet::default_subscription().contains(ChannelSet::SIGNALS));
    }
}
