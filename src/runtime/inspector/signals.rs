//! Reactive-graph events, bridged from `nami` to the `signals` channel.
//!
//! This is the only channel that can see *why* the UI updated: which piece of
//! state changed, how many watchers it had, and where it was created. It is also
//! the loudest, which is why it is not subscribed by default and why every
//! callback here starts by asking whether anyone is listening.
//!
//! Node identity is `nami`'s own [`SignalIdentity`], a pointer-derived value.
//! Pointers are reused after a node is dropped, so a consumer must treat
//! `Created` as the start of an identity's life and `Dropped` as its end rather
//! than assuming ids are unique forever.

use std::sync::Arc;

use nami::SignalIdentity;
use nami::observe::{SignalNode, SignalObserver};
use waterui_inspector_protocol::{Channel, InspectorEvent, SignalEvent, SignalId, SignalNodeInfo};

use super::hub::EventHub;

/// Forwards `nami` graph events to the inspector.
pub(super) struct SignalBridge {
    hub: Arc<EventHub>,
}

impl SignalBridge {
    /// Creates a bridge publishing to `hub`.
    pub(super) const fn new(hub: Arc<EventHub>) -> Self {
        Self { hub }
    }

    /// Publishes `event` when anyone is subscribed to the signals channel.
    fn publish(&self, event: SignalEvent) {
        if !self.hub.wants(Channel::Signals) {
            return;
        }
        self.hub.publish(InspectorEvent::Signal(event));
    }
}

impl SignalObserver for SignalBridge {
    fn on_create(&self, node: SignalNode) {
        if !self.hub.wants(Channel::Signals) {
            return;
        }
        let location = node.location();
        // Only creation carries strings. Notifications are the hot path and
        // reference the node by id alone.
        self.publish(SignalEvent::Created(SignalNodeInfo {
            node: signal_id(node.identity()),
            type_name: node.type_name().to_string(),
            file: location.file().to_string(),
            line: location.line(),
            column: location.column(),
        }));
    }

    fn on_subscribe(&self, node: SignalNode, subscribers: usize) {
        self.publish(SignalEvent::Subscribed {
            node: signal_id(node.identity()),
            subscribers: saturating_u32(subscribers),
        });
    }

    fn on_unsubscribe(&self, node: SignalNode, subscribers: usize) {
        self.publish(SignalEvent::Unsubscribed {
            node: signal_id(node.identity()),
            subscribers: saturating_u32(subscribers),
        });
    }

    fn on_notify(&self, node: SignalNode, subscribers: usize) {
        self.publish(SignalEvent::Notified {
            node: signal_id(node.identity()),
            subscribers: saturating_u32(subscribers),
        });
    }

    fn on_drop(&self, node: SignalNode) {
        self.publish(SignalEvent::Dropped {
            node: signal_id(node.identity()),
        });
    }
}

fn signal_id(identity: SignalIdentity) -> SignalId {
    SignalId(identity.raw() as u64)
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
