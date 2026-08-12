//! Event fan-out.
//!
//! Producers never take a lock and never touch the client list. They publish
//! into one bounded channel; a single dispatcher thread owns every connected
//! client and does the fan-out. That keeps the hot path — a reactive-graph
//! notification, which can fire thousands of times per second — down to an
//! atomic load, a sequence bump, and a `try_send`.
//!
//! When the channel is full the event is dropped and counted. A target that
//! outruns its socket must say so; a counter that silently means "events that
//! happened to fit" is wrong exactly when the system is interesting.

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use waterui_inspector_protocol::{Channel, ChannelSet, InspectorEvent, InspectorEventEnvelope};

/// Capacity of the publish channel, in events.
///
/// Deep enough to absorb a burst between dispatcher wake-ups, shallow enough
/// that a wedged client is noticed rather than silently buffering forever.
const PUBLISH_CAPACITY: usize = 8192;

/// What the dispatcher thread consumes.
pub(super) enum Dispatch {
    /// An event to fan out to subscribed clients.
    Event(InspectorEventEnvelope),
    /// An authenticated client, handed over by the acceptor.
    Connected {
        /// Identifier the dispatcher will track it by.
        client: ClientId,
        /// The write half the dispatcher takes exclusive ownership of.
        socket: std::net::TcpStream,
        /// Channels negotiated during the handshake.
        channels: ChannelSet,
    },
    /// A client changed which channels it wants.
    Subscribed {
        /// Client whose subscription changed.
        client: ClientId,
        /// The channels it now wants.
        channels: ChannelSet,
    },
    /// A client asked whether the target is alive.
    Ping {
        /// Client to answer.
        client: ClientId,
    },
    /// A client's socket went away.
    Disconnected {
        /// Client to retire.
        client: ClientId,
    },
}

/// Identifies one connected inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct ClientId(pub(super) u64);

/// Shared publishing endpoint held by every producer.
#[derive(Debug)]
pub struct EventHub {
    seq: AtomicU64,
    next_client: AtomicU64,
    /// Union of every connected client's subscription.
    ///
    /// Producers read this to decide whether to do any work at all, so it must
    /// be a plain atomic load rather than anything that could block.
    subscribed: AtomicU16,
    dropped: [AtomicU64; Channel::ALL.len()],
    sender: async_channel::Sender<Dispatch>,
}

impl EventHub {
    /// Creates a hub and the receiver its dispatcher thread will drain.
    pub(super) fn new() -> (Arc<Self>, async_channel::Receiver<Dispatch>) {
        let (sender, receiver) = async_channel::bounded(PUBLISH_CAPACITY);
        let hub = Arc::new(Self {
            seq: AtomicU64::new(0),
            next_client: AtomicU64::new(0),
            subscribed: AtomicU16::new(ChannelSet::empty().bits()),
            dropped: [const { AtomicU64::new(0) }; Channel::ALL.len()],
            sender,
        });
        (hub, receiver)
    }

    /// Whether any connected inspector wants events on `channel`.
    ///
    /// Producers must consult this *before* measuring anything. Producing an
    /// event nobody subscribed to is the defect this whole design exists to
    /// avoid.
    #[must_use]
    pub fn wants(&self, channel: Channel) -> bool {
        let subscribed = ChannelSet::from_bits_truncate(self.subscribed.load(Ordering::Relaxed));
        subscribed.contains(channel.as_set())
    }

    /// Publishes an event, dropping and counting it if the channel is full.
    pub fn publish(&self, event: InspectorEvent) {
        let channel = event.channel();
        if !self.wants(channel) {
            return;
        }

        let envelope = InspectorEventEnvelope {
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            timestamp_unix_ms: unix_millis(),
            event,
        };

        if self.sender.try_send(Dispatch::Event(envelope)).is_err() {
            self.record_drop(channel);
        }
    }

    /// Counts one event lost on `channel`.
    pub(super) fn record_drop(&self, channel: Channel) {
        self.dropped[channel as usize].fetch_add(1, Ordering::Relaxed);
    }

    /// Takes the number of events dropped on `channel` since the last call.
    pub(super) fn take_dropped(&self, channel: Channel) -> u64 {
        self.dropped[channel as usize].swap(0, Ordering::Relaxed)
    }

    /// Publishes a control message from a client's reader thread.
    ///
    /// Control messages are rare, so losing one would be a correctness bug
    /// rather than backpressure; this blocks instead of dropping.
    pub(super) fn send_control(&self, dispatch: Dispatch) {
        // A closed channel means the dispatcher is gone and the process is
        // shutting down; there is nothing useful to do about it here.
        let _ = self.sender.send_blocking(dispatch);
    }

    /// Allocates the next client identifier.
    pub(super) fn next_client_id(&self) -> ClientId {
        ClientId(self.next_client.fetch_add(1, Ordering::Relaxed))
    }

    /// Replaces the union of client subscriptions.
    pub(super) fn set_subscribed(&self, channels: ChannelSet) {
        self.subscribed.store(channels.bits(), Ordering::Relaxed);
    }
}

/// Milliseconds since the Unix epoch.
///
/// # Panics
///
/// Panics when the system clock is before the Unix epoch, which would make
/// every timestamp in the session meaningless.
fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before the UNIX epoch")
        .as_millis()
        .try_into()
        .expect("UNIX millisecond timestamp overflowed u64")
}

#[cfg(test)]
mod tests {
    use super::{Dispatch, EventHub};
    use waterui_inspector_protocol::{Channel, ChannelSet, FrameKind, FrameSample, InspectorEvent};

    fn frame() -> InspectorEvent {
        InspectorEvent::Frame(FrameSample {
            kind: FrameKind::Refresh,
            layout_us: 1,
            encode_us: 1,
            present_us: 1,
            total_us: 3,
            budget_us: 8333,
            refresh_hz: 120.0,
            layers: 1,
            clip_depth: 1,
        })
    }

    /// With nobody subscribed, publishing must not even enqueue. This is the
    /// property that keeps an idle inspector free.
    #[test]
    fn unsubscribed_channels_produce_nothing() {
        let (hub, receiver) = EventHub::new();
        assert!(!hub.wants(Channel::Frames));
        hub.publish(frame());
        assert!(receiver.is_empty());
    }

    #[test]
    fn subscribed_channels_are_enqueued_in_sequence() {
        let (hub, receiver) = EventHub::new();
        hub.set_subscribed(ChannelSet::FRAMES);

        hub.publish(frame());
        hub.publish(frame());

        let seqs: Vec<u64> = (0..2)
            .map(
                |_| match receiver.try_recv().expect("event should be queued") {
                    Dispatch::Event(envelope) => envelope.seq,
                    _ => panic!("expected an event"),
                },
            )
            .collect();
        assert_eq!(seqs, vec![0, 1]);
    }

    /// Overflow must be counted, not silently swallowed.
    #[test]
    fn overflow_is_counted_per_channel() {
        let (hub, receiver) = EventHub::new();
        hub.set_subscribed(ChannelSet::all());

        for _ in 0..super::PUBLISH_CAPACITY + 10 {
            hub.publish(frame());
        }

        assert_eq!(hub.take_dropped(Channel::Frames), 10);
        // Draining the counter must not double-report.
        assert_eq!(hub.take_dropped(Channel::Frames), 0);
        assert_eq!(hub.take_dropped(Channel::Signals), 0);
        assert_eq!(receiver.len(), super::PUBLISH_CAPACITY);
    }
}
