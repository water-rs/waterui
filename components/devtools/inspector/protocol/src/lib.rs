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

use std::fmt;

use serde::{Deserialize, Serialize};

mod channel;
pub mod discovery;
pub mod event;
pub mod tcp;
pub mod transport;

pub use channel::{Channel, ChannelSet};
pub use event::{
    Bounds, FrameKind, FrameSample, InspectorEvent, LogLevel, LogRecord, NodeId, NodeState,
    SignalEvent, SignalId, SignalNodeInfo, StallSample, TaskAggregate, TaskWindow, TreeNode,
    TreeUpdate,
};

/// Version of the protocol crate this build carries.
pub const INSPECTOR_PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Git commit of the checkout this build came from.
///
/// `None` in a release build: a published protocol identifies itself by its
/// version alone, and the build script refuses to stamp a checkout without a
/// full commit or a release with an override.
pub const INSPECTOR_PROTOCOL_COMMIT: Option<&str> =
    option_env!("WATERUI_INSPECTOR_PROTOCOL_COMMIT");

/// Returns protocol metadata for handshakes.
#[must_use]
pub fn protocol_info() -> InspectorProtocolInfo {
    InspectorProtocolInfo {
        version: INSPECTOR_PROTOCOL_VERSION.to_string(),
        build_commit: INSPECTOR_PROTOCOL_COMMIT.map(str::to_string),
    }
}

/// Protocol metadata exchanged during the handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectorProtocolInfo {
    /// Protocol crate version of the peer.
    pub version: String,
    /// Checkout commit of the peer's protocol crate; `None` in a release build.
    pub build_commit: Option<String>,
}

impl InspectorProtocolInfo {
    /// Whether this peer speaks exactly the same protocol build as the local
    /// crate — version and checkout commit, with no wildcards.
    #[must_use]
    pub fn is_compatible(&self) -> bool {
        *self == protocol_info()
    }
}

impl fmt::Display for InspectorProtocolInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.version)?;
        if let Some(commit) = &self.build_commit {
            write!(f, " ({commit})")?;
        }
        Ok(())
    }
}

/// A build's full handshake identity — framework version and protocol.
///
/// The framework half is what makes two released apps on the same protocol
/// version distinguishable: a mismatch there is a protocol disagreement the
/// wire format cannot explain away, so peers compare the whole identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectorBuildIdentity {
    /// Version of the compiled `waterui` facade the peer links.
    pub framework_version: String,
    /// Protocol identity of this build.
    pub protocol: InspectorProtocolInfo,
}

impl fmt::Display for InspectorBuildIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "waterui {} / {}", self.framework_version, self.protocol)
    }
}

/// Returns this build's full identity for handshakes — the caller's compiled
/// `waterui` facade version plus this crate's protocol identity.
#[must_use]
pub fn build_identity(framework_version: impl Into<String>) -> InspectorBuildIdentity {
    InspectorBuildIdentity {
        framework_version: framework_version.into(),
        protocol: protocol_info(),
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
        /// Full build identity of the inspector, checked against the target's.
        identity: InspectorBuildIdentity,
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
    /// Ask the endpoint for its build identity.
    ///
    /// The endpoint-first query a launcher makes before it knows which
    /// client to build or fetch: the endpoint answers
    /// [`InspectorServerMessage::Identity`] and closes — no session, no
    /// channel subscription, no hub registration.
    Identify {
        /// One-time session token.
        token: String,
    },
}

/// Messages sent by the target application to the inspector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InspectorServerMessage {
    /// Handshake accepted.
    Welcome {
        /// Full build identity of the target.
        identity: InspectorBuildIdentity,
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
    /// Show this node: the user asked to inspect an element in the target.
    ///
    /// Sent unprompted, when someone picks "inspect" on something they are
    /// pointing at. An inspector that has not opened the tree channel still
    /// receives it, and is expected to open the tree and reveal the node.
    Select {
        /// The node to reveal, in the same id space as the tree channel.
        node: NodeId,
    },
    /// Liveness response.
    Pong,
    /// A request could not be served.
    Error {
        /// Human-readable reason.
        message: String,
    },
    /// The endpoint's build identity, answering [`InspectorClientMessage::Identify`].
    Identity {
        /// Full build identity of the target.
        identity: InspectorBuildIdentity,
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
            identity: super::build_identity("0.0.1"),
            channels: ChannelSet::default_subscription(),
        };
        transport::write_frame_blocking(&mut bytes, &sent).unwrap();

        let received: InspectorClientMessage =
            transport::read_frame_blocking(&mut bytes.as_slice()).unwrap();
        match received {
            InspectorClientMessage::Hello {
                token,
                channels,
                identity,
            } => {
                assert_eq!(identity.framework_version, "0.0.1");
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
            version: super::INSPECTOR_PROTOCOL_VERSION.to_string(),
            build_commit: Some(String::from("0000000000000000000000000000000000000000")),
        };
        assert!(!foreign.is_compatible());
        assert!(super::protocol_info().is_compatible());
    }

    #[test]
    fn a_peer_at_a_different_version_is_incompatible() {
        let local = super::protocol_info();
        let older = InspectorProtocolInfo {
            version: "0.0.0".to_string(),
            build_commit: local.build_commit,
        };
        assert!(!older.is_compatible());
    }

    #[test]
    fn build_identities_round_trip_through_the_transport() {
        let mut bytes = Vec::new();
        let sent = InspectorServerMessage::Welcome {
            identity: super::build_identity("0.0.2"),
            target: super::TargetInfo {
                pid: 42,
                name: String::from("round-trip"),
                backend: String::from("test"),
            },
            available: ChannelSet::default_subscription(),
        };
        transport::write_frame_blocking(&mut bytes, &sent).unwrap();
        let received: InspectorServerMessage =
            transport::read_frame_blocking(&mut bytes.as_slice()).unwrap();
        match received {
            InspectorServerMessage::Welcome {
                identity, target, ..
            } => {
                assert_eq!(identity, super::build_identity("0.0.2"));
                assert_eq!(target.name, "round-trip");
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn identify_and_identity_round_trip_through_the_transport() {
        let mut bytes = Vec::new();
        transport::write_frame_blocking(
            &mut bytes,
            &InspectorClientMessage::Identify {
                token: String::from("session-token"),
            },
        )
        .unwrap();
        let received: InspectorClientMessage =
            transport::read_frame_blocking(&mut bytes.as_slice()).unwrap();
        match received {
            InspectorClientMessage::Identify { token } => {
                assert_eq!(token, "session-token");
            }
            other => panic!("unexpected message: {other:?}"),
        }

        let mut bytes = Vec::new();
        let identity = super::build_identity("0.0.2");
        transport::write_frame_blocking(
            &mut bytes,
            &InspectorServerMessage::Identity {
                identity: identity.clone(),
            },
        )
        .unwrap();
        let received: InspectorServerMessage =
            transport::read_frame_blocking(&mut bytes.as_slice()).unwrap();
        match received {
            InspectorServerMessage::Identity { identity: reported } => {
                assert_eq!(reported, identity);
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn a_release_peer_and_a_checkout_peer_never_agree() {
        let local = super::protocol_info();
        // The opposite build flavour at the same version still differs: a
        // release has no commit, a checkout does, and neither is a wildcard.
        let flipped = InspectorProtocolInfo {
            version: local.version.clone(),
            build_commit: if local.build_commit.is_some() {
                None
            } else {
                Some(String::from("0000000000000000000000000000000000000000"))
            },
        };
        assert!(!flipped.is_compatible());
    }
}
