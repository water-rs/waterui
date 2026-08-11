//! Handles a rendering backend uses to report what it did.
//!
//! Frames and accessibility trees are backend knowledge, so the backend pushes
//! them rather than the inspector pulling. Both recorders answer `is_active`
//! cheaply, so a backend can skip the measurement entirely when nobody is
//! watching — the point of a subscription-based protocol is that unobserved work
//! is never done.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use waterui_inspector_protocol::{
    Channel, FrameSample, InspectorEvent, NodeId, TreeNode, TreeUpdate,
};

use super::hub::EventHub;

/// Reports rendered frames to a connected inspector.
#[derive(Debug, Clone)]
pub struct FrameRecorder {
    hub: Arc<EventHub>,
}

impl FrameRecorder {
    pub(super) const fn new(hub: Arc<EventHub>) -> Self {
        Self { hub }
    }

    /// Whether frame timings are worth measuring right now.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.hub.wants(Channel::Frames)
    }

    /// Reports one frame.
    pub fn record(&self, sample: FrameSample) {
        self.hub.publish(InspectorEvent::Frame(sample));
    }
}

/// Reports accessibility-tree changes to a connected inspector.
///
/// Backends generally rebuild their accessibility tree wholesale, which is fine
/// for a screen reader consuming one tree but ruinous as a stream: a 500-node
/// tree at 120 Hz is a firehose. So a backend hands over the whole tree and this
/// recorder turns it into a delta, sending the full set only to a subscriber
/// that has not seen one yet. Doing the diff here rather than in each backend
/// means every backend gets it, and gets it the same way.
#[derive(Debug, Clone)]
pub struct TreeRecorder {
    hub: Arc<EventHub>,
    state: Arc<Mutex<TreeState>>,
}

#[derive(Debug, Default)]
struct TreeState {
    revision: u64,
    /// What the connected subscribers have already been told.
    ///
    /// Empty means the next update must be a full one.
    published: HashMap<NodeId, TreeNode>,
}

impl TreeRecorder {
    pub(super) fn new(hub: Arc<EventHub>) -> Self {
        Self {
            hub,
            state: Arc::new(Mutex::new(TreeState::default())),
        }
    }

    /// Whether tree updates are worth computing right now.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.hub.wants(Channel::Tree)
    }

    /// Reports the complete accessibility tree, publishing only what changed.
    ///
    /// Publishes nothing when the tree is identical to the last one, which is
    /// the common case: most frames change pixels, not structure.
    pub fn record_snapshot(&self, root: NodeId, focus: Option<NodeId>, nodes: Vec<TreeNode>) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };

        if !self.is_active() {
            // Subscribers may return later, and they will need a full tree when
            // they do.
            state.published.clear();
            return;
        }

        let full = state.published.is_empty();
        let mut changed = Vec::new();
        let mut current = HashMap::with_capacity(nodes.len());
        for node in nodes {
            if state.published.get(&node.id) != Some(&node) {
                changed.push(node.clone());
            }
            current.insert(node.id, node);
        }
        let removed: Vec<NodeId> = state
            .published
            .keys()
            .filter(|id| !current.contains_key(id))
            .copied()
            .collect();

        if !full && changed.is_empty() && removed.is_empty() {
            return;
        }

        state.published = current;
        let revision = state.revision;
        state.revision = state.revision.wrapping_add(1);
        drop(state);

        self.hub.publish(InspectorEvent::Tree(TreeUpdate {
            revision,
            root,
            focus,
            full,
            changed,
            removed,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::{EventHub, TreeRecorder};
    use std::sync::Arc;
    use waterui_inspector_protocol::{
        ChannelSet, InspectorEvent, NodeId, NodeState, TreeNode, TreeUpdate,
    };

    use super::super::hub::Dispatch;

    fn node(id: u64, label: &str) -> TreeNode {
        TreeNode {
            id: NodeId(id),
            role: String::from("button"),
            label: Some(label.to_string()),
            value: None,
            bounds: None,
            state: NodeState::default(),
            children: Vec::new(),
        }
    }

    fn drain(receiver: &async_channel::Receiver<Dispatch>) -> Vec<TreeUpdate> {
        let mut updates = Vec::new();
        while let Ok(Dispatch::Event(envelope)) = receiver.try_recv() {
            if let InspectorEvent::Tree(update) = envelope.event {
                updates.push(update);
            }
        }
        updates
    }

    /// A backend must not be asked to compute a tree nobody wants.
    #[test]
    fn an_unsubscribed_recorder_is_inactive() {
        let (hub, receiver) = EventHub::new();
        let recorder = TreeRecorder::new(Arc::clone(&hub));

        assert!(!recorder.is_active());
        recorder.record_snapshot(NodeId(0), None, vec![node(1, "a")]);
        assert!(receiver.is_empty());
    }

    /// The first update a subscriber sees must be the whole tree; after that
    /// only what actually changed, and nothing at all when nothing did.
    #[test]
    fn the_first_update_is_full_and_the_rest_are_deltas() {
        let (hub, receiver) = EventHub::new();
        hub.set_subscribed(ChannelSet::TREE);
        let recorder = TreeRecorder::new(Arc::clone(&hub));

        recorder.record_snapshot(NodeId(0), None, vec![node(1, "a"), node(2, "b")]);
        let first = drain(&receiver);
        assert_eq!(first.len(), 1);
        assert!(first[0].full);
        assert_eq!(first[0].changed.len(), 2);

        // An identical tree is the common case and must cost nothing.
        recorder.record_snapshot(NodeId(0), None, vec![node(1, "a"), node(2, "b")]);
        assert!(drain(&receiver).is_empty());

        // One relabelled node, one removed.
        recorder.record_snapshot(NodeId(0), None, vec![node(1, "renamed")]);
        let third = drain(&receiver);
        assert_eq!(third.len(), 1);
        assert!(!third[0].full);
        assert_eq!(third[0].changed, vec![node(1, "renamed")]);
        assert_eq!(third[0].removed, vec![NodeId(2)]);
    }

    /// Losing every subscriber invalidates the delta stream, so the next one is
    /// owed a full tree rather than deltas against a tree it never saw.
    #[test]
    fn dropping_all_subscribers_restarts_the_delta_stream() {
        let (hub, receiver) = EventHub::new();
        hub.set_subscribed(ChannelSet::TREE);
        let recorder = TreeRecorder::new(Arc::clone(&hub));

        recorder.record_snapshot(NodeId(0), None, vec![node(1, "a")]);
        drain(&receiver);

        hub.set_subscribed(ChannelSet::empty());
        recorder.record_snapshot(NodeId(0), None, vec![node(1, "a")]);

        hub.set_subscribed(ChannelSet::TREE);
        recorder.record_snapshot(NodeId(0), None, vec![node(1, "a")]);
        let resumed = drain(&receiver);
        assert_eq!(resumed.len(), 1);
        assert!(resumed[0].full, "a new subscriber is owed the whole tree");
    }
}
