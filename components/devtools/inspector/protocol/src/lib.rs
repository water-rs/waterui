//! Wire protocol between a running `WaterUI` application and the Inspector app.
//!
//! # Shape
//!
//! The target application is the server: it owns the data and listens on a
//! loopback socket. The inspector connects, authenticates with a one-time token,
//! and *subscribes* to the [channels](channel::Channel) it wants. Nothing is
//! produced for an unsubscribed channel, so an inspector that only wants frame
//! timings does not make the target pay for reactive-graph tracing.
//!
//! # Honesty about loss
//!
//! A target under load can outrun its socket. When that happens the runtime
//! drops events and says so with [`InspectorServerMessage::Dropped`]. A counter
//! that silently means "events I happened to receive" is worse than no counter,
//! because it is wrong exactly when it matters.

use serde::{Deserialize, Serialize};

pub mod channel;
pub mod event;
pub mod tcp;
pub mod transport;

pub use channel::{Channel, ChannelSet};
pub use event::{
    Bounds, FrameKind, FrameSample, InspectorEvent, LogLevel, LogRecord, NodeId, NodeState,
    SignalEvent, SignalId, SignalNodeInfo, StallSample, TaskAggregate, TaskWindow, TreeNode,
    TreeUpdate,
};

/// Build commit of this protocol, used to reject mismatched peers.
pub const INSPECTOR_PROTOCOL_COMMIT: &str = env!("WATERUI_INSPECTOR_PROTOCOL_COMMIT");

/// Returns protocol metadata for handshakes.
#[must_use]
pub fn protocol_info() -> InspectorProtocolInfo {
    InspectorProtocolInfo {
        build_commit: INSPECTOR_PROTOCOL_COMMIT.to_string(),
    }
}

/// Protocol metadata exchanged during the handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectorProtocolInfo {
    /// Build commit of the peer's protocol crate.
    pub build_commit: String,
}

impl InspectorProtocolInfo {
    /// Whether this peer speaks the same protocol build as the local crate.
    #[must_use]
    pub fn is_compatible(&self) -> bool {
        self.build_commit == INSPECTOR_PROTOCOL_COMMIT
    }
}

/// Description of the application being inspected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetInfo {
    /// Process id of the target application.
    pub pid: u32,
    /// Human-readable application name.
    pub name: String,
    /// Rendering backend driving the application (`hydrolysis`, `apple`, ...).
    pub backend: String,
}

/// Messages sent by the inspector to the target application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InspectorClientMessage {
    /// Authenticate and declare which channels to open.
    Hello {
        /// One-time session token.
        token: String,
        /// Protocol build of the inspector, checked against the target's.
        protocol: InspectorProtocolInfo,
        /// Channels to subscribe to immediately.
        channels: ChannelSet,
    },
    /// Replace the active subscription.
    Subscribe {
        /// The channels that should be active from now on.
        channels: ChannelSet,
    },
    /// Liveness probe.
    Ping,
}

/// Messages sent by the target application to the inspector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InspectorServerMessage {
    /// Handshake accepted.
    Welcome {
        /// Protocol build of the target.
        protocol: InspectorProtocolInfo,
        /// What is being inspected.
        target: TargetInfo,
        /// Channels this target can actually produce.
        ///
        /// A channel may be absent because the backend does not implement it or
        /// because the target was not built with the required feature. Asking
        /// for it is not an error; it simply yields nothing.
        available: ChannelSet,
    },
    /// One sequenced event.
    Event {
        /// The event and its sequence metadata.
        envelope: InspectorEventEnvelope,
    },
    /// Events were produced but could not be delivered.
    Dropped {
        /// Which channel lost events.
        channel: Channel,
        /// How many events were lost since the last report.
        events: u64,
    },
    /// Liveness response.
    Pong,
    /// A request could not be served.
    Error {
        /// Human-readable reason.
        message: String,
    },
}

/// An event together with its sequence metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectorEventEnvelope {
    /// Monotonic sequence number, unique across all channels.
    pub seq: u64,
    /// UTC timestamp in milliseconds since the Unix epoch.
    pub timestamp_unix_ms: u64,
    /// The event itself.
    pub event: InspectorEvent,
}

#[cfg(test)]
mod tests {
    use super::{
        Channel, ChannelSet, InspectorClientMessage, InspectorProtocolInfo, InspectorServerMessage,
        transport,
    };

    #[test]
    fn frames_round_trip_through_the_transport() {
        let mut bytes = Vec::new();
        let sent = InspectorClientMessage::Hello {
            token: String::from("token"),
            protocol: super::protocol_info(),
            channels: ChannelSet::default_subscription(),
        };
        transport::write_frame_blocking(&mut bytes, &sent).unwrap();

        let received: InspectorClientMessage =
            transport::read_frame_blocking(&mut bytes.as_slice()).unwrap();
        match received {
            InspectorClientMessage::Hello { token, channels, .. } => {
                assert_eq!(token, "token");
                assert_eq!(channels, ChannelSet::default_subscription());
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn dropped_events_name_their_channel() {
        let mut bytes = Vec::new();
        transport::write_frame_blocking(
            &mut bytes,
            &InspectorServerMessage::Dropped {
                channel: Channel::Signals,
                events: 4_096,
            },
        )
        .unwrap();

        let received: InspectorServerMessage =
            transport::read_frame_blocking(&mut bytes.as_slice()).unwrap();
        match received {
            InspectorServerMessage::Dropped { channel, events } => {
                assert_eq!(channel, Channel::Signals);
                assert_eq!(events, 4_096);
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn a_foreign_protocol_build_is_incompatible() {
        let foreign = InspectorProtocolInfo {
            build_commit: String::from("not-this-build"),
        };
        assert!(!foreign.is_compatible());
        assert!(super::protocol_info().is_compatible());
    }
}
