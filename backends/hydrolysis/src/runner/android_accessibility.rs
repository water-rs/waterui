//! The Android accessibility publish protocol in pure `accesskit` terms —
//! the sibling of [`crate::runner::editing`] for the `InputConnection` side,
//! compiled on Android for the JNI bridge and on host for its tests.
//!
//! Two decisions that need accesskit internals — and therefore cannot be
//! made by the Kotlin host — live here:
//!
//! - **Which events a publish produces.** Android expects
//!   `TYPE_WINDOW_CONTENT_CHANGED` scoped to each changed node with a
//!   `contentChangeTypes` mask naming the change, `TYPE_VIEW_FOCUSED` when
//!   focus moves, and `TYPE_VIEW_TEXT_CHANGED` when an editable's text
//!   changes. The renderer's merged `TreeUpdate` is produced on every
//!   flushed frame whether or not anything semantically changed (the
//!   retained scene reassembles a full tree each flush), so publishing it
//!   unconditionally floods the platform — the Pixel evidence in #246 was
//!   88 window-content events in a few seconds, all no-ops that kept the UI
//!   permanently non-idle for services. Consecutive published updates are
//!   diffed here with `Node`'s semantic `PartialEq`; an update that changes
//!   nothing produces no events and is never serialized for transport.
//! - **Which virtual node a screen point hits.** Explore-by-touch needs
//!   `dispatchHoverEvent` mapped onto the served tree. The hit test runs
//!   over the same published update the provider serves: reachable from the
//!   root without crossing a `Hidden` subtree, `TouchTransparent`
//!   decoration skipped, innermost (latest-painted) node first.
//!
//! Both results travel to Kotlin as plain data — an event list in JSON, a
//! node id for the hit test — so the platform policy is testable end-to-end
//! on the host and Kotlin stays a transport.

use std::collections::{HashMap, HashSet};

use accesskit::{Node, NodeId, Point, Role, TreeUpdate};

/// Android `AccessibilityEvent.getEventType()` values the diff emits. The
/// numbers are the platform contract; Kotlin replays them verbatim.
pub const TYPE_VIEW_FOCUSED: i32 = 0x8;
pub const TYPE_VIEW_TEXT_CHANGED: i32 = 0x10;
pub const TYPE_VIEW_SCROLLED: i32 = 0x1000;
pub const TYPE_WINDOW_CONTENT_CHANGED: i32 = 0x800;

/// `AccessibilityEvent.setContentChangeTypes()` bits the diff uses.
pub const CHANGE_SUBTREE: i32 = 0x1;
pub const CHANGE_TEXT: i32 = 0x2;
pub const CHANGE_CONTENT_DESCRIPTION: i32 = 0x4;
pub const CHANGE_STATE_DESCRIPTION: i32 = 0x40;
pub const CHANGE_ENABLED: i32 = 0x1000;

/// Past this many per-node content changes in one publish, a single subtree
/// invalidation on the root is the honest signal — it tells services to
/// refetch rather than replay a burst of stale-on-arrival detail.
const MAX_SCOPED_EVENTS: usize = 8;

/// One accessibility event the host must dispatch, in Android terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct A11yEvent {
    /// The virtual node the event is scoped to; `None` addresses the host
    /// view itself (a whole-tree change with no meaningful node scope).
    pub node: Option<NodeId>,
    /// `AccessibilityEvent.getEventType()` value.
    pub event_type: i32,
    /// `AccessibilityEvent.setContentChangeTypes()` mask for
    /// `TYPE_WINDOW_CONTENT_CHANGED` events; `0` for other event types.
    pub content_change_types: i32,
}

/// The events one publish must produce to move an observing service from
/// `prev` to `next`. A publish whose semantic content did not change
/// produces none. The first publish (or one whose root changed) is a single
/// whole-tree event on the host.
///
/// A node that differs only in properties Android has no named change type
/// for (bounds, text metrics, flags, …) still emits
/// `TYPE_WINDOW_CONTENT_CHANGED`, carrying an empty mask — the platform's
/// `CONTENT_CHANGE_TYPE_UNDEFINED` convention for a change with no specific
/// type — so a real change is announced rather than silently dropped.
/// Scroll offsets are the one unclassified case the platform gives a
/// dedicated event: a scroll-only diff emits `TYPE_VIEW_SCROLLED` alone,
/// like a real scrolling view would, instead of also reporting UNDEFINED.
pub fn diff_events(prev: Option<&TreeUpdate>, next: &TreeUpdate) -> Vec<A11yEvent> {
    let same_root = matches!(
        (prev.and_then(|p| p.tree.as_ref()), next.tree.as_ref()),
        (Some(before), Some(after)) if before.root == after.root
    );
    if prev.is_none() || !same_root {
        return vec![A11yEvent {
            node: None,
            event_type: TYPE_WINDOW_CONTENT_CHANGED,
            content_change_types: CHANGE_SUBTREE,
        }];
    }
    let prev = prev.unwrap();

    let old: HashMap<NodeId, &Node> = prev.nodes.iter().map(|(id, node)| (*id, node)).collect();
    let mut events = Vec::new();
    for (id, node) in &next.nodes {
        // Added nodes need no event of their own: the surviving parent's
        // changed `children` list already carries CHANGE_SUBTREE.
        let Some(before) = old.get(id) else { continue };
        let scrolled = scroll_changed(before, node);
        if scrolled {
            events.push(A11yEvent {
                node: Some(*id),
                event_type: TYPE_VIEW_SCROLLED,
                content_change_types: 0,
            });
        }
        let Some(mask) = node_change_mask(before, node) else {
            continue;
        };
        if mask == 0 && scrolled {
            // The scroll metrics were the unclassified diff, and the
            // dedicated SCROLLED event already announces it.
            continue;
        }
        events.push(A11yEvent {
            node: Some(*id),
            event_type: TYPE_WINDOW_CONTENT_CHANGED,
            content_change_types: mask,
        });
        if mask & CHANGE_TEXT != 0 && is_editable(node.role()) {
            events.push(A11yEvent {
                node: Some(*id),
                event_type: TYPE_VIEW_TEXT_CHANGED,
                content_change_types: 0,
            });
        }
    }

    let detail = events
        .iter()
        .filter(|e| e.event_type == TYPE_WINDOW_CONTENT_CHANGED)
        .count();
    if detail > MAX_SCOPED_EVENTS {
        events.retain(|e| e.event_type != TYPE_WINDOW_CONTENT_CHANGED);
        if let Some(root) = next.tree.as_ref().map(|tree| tree.root) {
            events.insert(
                0,
                A11yEvent {
                    node: Some(root),
                    event_type: TYPE_WINDOW_CONTENT_CHANGED,
                    content_change_types: CHANGE_SUBTREE,
                },
            );
        }
    }

    if prev.focus != next.focus && next.nodes.iter().any(|(id, _)| *id == next.focus) {
        events.push(A11yEvent {
            node: Some(next.focus),
            event_type: TYPE_VIEW_FOCUSED,
            content_change_types: 0,
        });
    }
    events
}

/// The `contentChangeTypes` mask describing a node change, or `None` when
/// the nodes are semantically identical — `Node`'s `PartialEq` covers role,
/// action masks, flags and every property.
fn node_change_mask(before: &Node, after: &Node) -> Option<i32> {
    if before == after {
        return None;
    }
    let mut mask = 0;
    // A node's served membership flips with `hidden`, so a visibility
    // change is a structural change to the tree services see.
    if before.children() != after.children() || before.is_hidden() != after.is_hidden() {
        mask |= CHANGE_SUBTREE;
    }
    if before.label() != after.label() || before.description() != after.description() {
        mask |= CHANGE_CONTENT_DESCRIPTION;
    }
    if before.value() != after.value() || before.text_selection() != after.text_selection() {
        mask |= CHANGE_TEXT;
    }
    if before.toggled() != after.toggled()
        || before.is_selected() != after.is_selected()
        || before.is_expanded() != after.is_expanded()
        || before.level() != after.level()
        || before.position_in_set() != after.position_in_set()
        || before.size_of_set() != after.size_of_set()
        || before.numeric_value() != after.numeric_value()
        || before.min_numeric_value() != after.min_numeric_value()
        || before.max_numeric_value() != after.max_numeric_value()
        || before.placeholder() != after.placeholder()
    {
        mask |= CHANGE_STATE_DESCRIPTION;
    }
    if before.is_disabled() != after.is_disabled() {
        mask |= CHANGE_ENABLED;
    }
    Some(mask)
}

/// Whether any scroll metric changed on the node. The platform answer to a
/// moving scroll offset is `TYPE_VIEW_SCROLLED`, not a content change.
fn scroll_changed(before: &Node, after: &Node) -> bool {
    before.scroll_x() != after.scroll_x()
        || before.scroll_y() != after.scroll_y()
        || before.scroll_x_min() != after.scroll_x_min()
        || before.scroll_x_max() != after.scroll_x_max()
        || before.scroll_y_min() != after.scroll_y_min()
        || before.scroll_y_max() != after.scroll_y_max()
}

/// The roles the Kotlin provider serves as editable `EditText`s — the set
/// its `isEditable` reports; `TYPE_VIEW_TEXT_CHANGED` only makes sense for
/// them.
const fn is_editable(role: Role) -> bool {
    matches!(
        role,
        Role::TextInput | Role::MultilineTextInput | Role::SearchInput | Role::PasswordInput
    )
}

/// The innermost served node containing `(x, y)` — the node a
/// `dispatchHoverEvent` maps to. "Served" mirrors the provider's filter:
/// reachable from the root without crossing a `Hidden` subtree, and not
/// `TouchTransparent` itself. Nodes without bounds can never be hit.
pub fn hit_test(update: &TreeUpdate, x: f64, y: f64) -> Option<NodeId> {
    let root = update.tree.as_ref()?.root;
    let by_id: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, node)| (*id, node)).collect();

    let mut reachable: HashSet<NodeId> = HashSet::with_capacity(update.nodes.len());
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let Some(node) = by_id.get(&id) else { continue };
        if node.is_hidden() || !reachable.insert(id) {
            continue;
        }
        stack.extend(node.children().iter().copied());
    }

    let point = Point::new(x, y);
    // Reverse declaration order is the painted order's topmost first — the
    // same scan `node_at_point_where` uses for renderer-side hit tests.
    update
        .nodes
        .iter()
        .rev()
        .find(|(id, node)| {
            reachable.contains(id)
                && !node.is_touch_transparent()
                && node.bounds().is_some_and(|bounds| bounds.contains(point))
        })
        .map(|(id, _)| *id)
}

/// The wire form handed to `onNativeAccessibilityTreeChanged`:
/// `{"events":[{"id":5,"type":2048,"mask":2}]}`, with `id` `-1` for
/// host-level events.
pub fn events_json(events: &[A11yEvent]) -> String {
    let events: Vec<serde_json::Value> = events
        .iter()
        .map(|event| {
            serde_json::json!({
                "id": event.node.map_or(-1, |id| {
                    crate::num_cast::u64_as_i64(id.0)
                }),
                "type": event.event_type,
                "mask": event.content_change_types,
            })
        })
        .collect();
    serde_json::json!({ "events": events }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use accesskit::{Rect, Role, TextSelection, TreeId, TreeInfo};

    fn node(id: u64, role: Role, children: &[u64]) -> (NodeId, Node) {
        let mut node = Node::new(role);
        node.set_children(
            children
                .iter()
                .copied()
                .map(NodeId::from)
                .collect::<Vec<_>>(),
        );
        (NodeId::from(id), node)
    }

    fn update(nodes: Vec<(NodeId, Node)>, root: u64, focus: u64) -> TreeUpdate {
        TreeUpdate {
            nodes,
            tree: Some(TreeInfo::new(NodeId::from(root))),
            tree_id: TreeId::ROOT,
            focus: NodeId::from(focus),
        }
    }

    #[test]
    fn identical_publishes_emit_nothing() {
        let first = update(
            vec![node(0, Role::Window, &[1]), node(1, Role::Button, &[])],
            0,
            0,
        );
        let mut second = first.clone();
        // Reordering changes serialization but not semantics.
        second.nodes.reverse();
        assert_eq!(diff_events(Some(&first), &second), []);
    }

    #[test]
    fn first_publish_is_one_host_level_subtree_event() {
        let first = update(vec![node(0, Role::Window, &[])], 0, 0);
        assert_eq!(
            diff_events(None, &first),
            vec![A11yEvent {
                node: None,
                event_type: TYPE_WINDOW_CONTENT_CHANGED,
                content_change_types: CHANGE_SUBTREE,
            }]
        );
    }

    #[test]
    fn changed_label_scopes_to_the_node() {
        let old = update(
            vec![node(0, Role::Window, &[1]), node(1, Role::Button, &[])],
            0,
            0,
        );
        let mut new_button = node(1, Role::Button, &[]).1;
        new_button.set_label("Increment");
        let new = update(
            vec![node(0, Role::Window, &[1]), (NodeId::from(1), new_button)],
            0,
            0,
        );
        assert_eq!(
            diff_events(Some(&old), &new),
            vec![A11yEvent {
                node: Some(NodeId::from(1)),
                event_type: TYPE_WINDOW_CONTENT_CHANGED,
                content_change_types: CHANGE_CONTENT_DESCRIPTION,
            }]
        );
    }

    #[test]
    fn added_child_reports_on_the_parent() {
        let old = update(
            vec![node(0, Role::Window, &[1]), node(1, Role::Button, &[])],
            0,
            0,
        );
        let new = update(
            vec![
                node(0, Role::Window, &[1, 2]),
                node(1, Role::Button, &[]),
                node(2, Role::Button, &[]),
            ],
            0,
            0,
        );
        assert_eq!(
            diff_events(Some(&old), &new),
            vec![A11yEvent {
                node: Some(NodeId::from(0)),
                event_type: TYPE_WINDOW_CONTENT_CHANGED,
                content_change_types: CHANGE_SUBTREE,
            }]
        );
    }

    #[test]
    fn editable_value_change_also_reports_text_changed() {
        let old = update(
            vec![node(0, Role::Window, &[1]), node(1, Role::TextInput, &[])],
            0,
            1,
        );
        let mut field = node(1, Role::TextInput, &[]).1;
        field.set_value("me@example.com");
        let new = update(
            vec![node(0, Role::Window, &[1]), (NodeId::from(1), field)],
            0,
            1,
        );
        assert_eq!(
            diff_events(Some(&old), &new),
            vec![
                A11yEvent {
                    node: Some(NodeId::from(1)),
                    event_type: TYPE_WINDOW_CONTENT_CHANGED,
                    content_change_types: CHANGE_TEXT,
                },
                A11yEvent {
                    node: Some(NodeId::from(1)),
                    event_type: TYPE_VIEW_TEXT_CHANGED,
                    content_change_types: 0,
                },
            ]
        );
    }

    #[test]
    fn non_editable_value_change_is_not_text_changed() {
        let old = update(
            vec![node(0, Role::Window, &[1]), node(1, Role::Label, &[])],
            0,
            0,
        );
        let mut label = node(1, Role::Label, &[]).1;
        label.set_value("3");
        let new = update(
            vec![node(0, Role::Window, &[1]), (NodeId::from(1), label)],
            0,
            0,
        );
        assert_eq!(
            diff_events(Some(&old), &new),
            vec![A11yEvent {
                node: Some(NodeId::from(1)),
                event_type: TYPE_WINDOW_CONTENT_CHANGED,
                content_change_types: CHANGE_TEXT,
            }]
        );
    }

    #[test]
    fn focus_move_reports_view_focused() {
        let old = update(
            vec![
                node(0, Role::Window, &[1, 2]),
                node(1, Role::Button, &[]),
                node(2, Role::Button, &[]),
            ],
            0,
            1,
        );
        let new = update(
            vec![
                node(0, Role::Window, &[1, 2]),
                node(1, Role::Button, &[]),
                node(2, Role::Button, &[]),
            ],
            0,
            2,
        );
        assert_eq!(
            diff_events(Some(&old), &new),
            vec![A11yEvent {
                node: Some(NodeId::from(2)),
                event_type: TYPE_VIEW_FOCUSED,
                content_change_types: 0,
            }]
        );
    }

    #[test]
    fn toggled_change_reports_state_description() {
        let old = update(
            vec![node(0, Role::Window, &[1]), node(1, Role::CheckBox, &[])],
            0,
            0,
        );
        let mut toggled = node(1, Role::CheckBox, &[]).1;
        toggled.set_toggled(accesskit::Toggled::True);
        let new = update(
            vec![node(0, Role::Window, &[1]), (NodeId::from(1), toggled)],
            0,
            0,
        );
        let events = diff_events(Some(&old), &new);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].content_change_types, CHANGE_STATE_DESCRIPTION);
    }

    #[test]
    fn unclassified_change_announces_undefined() {
        let old = update(
            vec![node(0, Role::Window, &[1]), node(1, Role::Button, &[])],
            0,
            0,
        );
        // An identical rebuild emits nothing at all.
        assert_eq!(diff_events(Some(&old), &old.clone()), []);

        // A real change in a property with no named change type — bounds
        // here — still announces, carrying the platform's
        // CONTENT_CHANGE_TYPE_UNDEFINED (an empty mask) instead of being
        // suppressed. The republished tree carries the new bounds for the
        // next query regardless.
        let mut moved = node(1, Role::Button, &[]).1;
        moved.set_bounds(Rect::new(0.0, 0.0, 20.0, 20.0));
        let new = update(
            vec![node(0, Role::Window, &[1]), (NodeId::from(1), moved)],
            0,
            0,
        );
        assert_eq!(
            diff_events(Some(&old), &new),
            vec![A11yEvent {
                node: Some(NodeId::from(1)),
                event_type: TYPE_WINDOW_CONTENT_CHANGED,
                content_change_types: 0,
            }]
        );
    }

    #[test]
    fn scroll_offset_reports_view_scrolled() {
        let old = update(
            vec![node(0, Role::Window, &[1]), node(1, Role::ScrollView, &[])],
            0,
            0,
        );
        let mut scrolled = node(1, Role::ScrollView, &[]).1;
        scrolled.set_scroll_y(120.0);
        scrolled.set_scroll_y_max(800.0);
        let new = update(
            vec![node(0, Role::Window, &[1]), (NodeId::from(1), scrolled)],
            0,
            0,
        );
        assert_eq!(
            diff_events(Some(&old), &new),
            vec![A11yEvent {
                node: Some(NodeId::from(1)),
                event_type: TYPE_VIEW_SCROLLED,
                content_change_types: 0,
            }]
        );
    }

    #[test]
    fn hidden_flip_is_a_subtree_change() {
        let old = update(
            vec![node(0, Role::Window, &[1]), node(1, Role::Button, &[])],
            0,
            0,
        );
        let mut hidden = node(1, Role::Button, &[]).1;
        hidden.set_hidden();
        let new = update(
            vec![node(0, Role::Window, &[1]), (NodeId::from(1), hidden)],
            0,
            0,
        );
        assert_eq!(
            diff_events(Some(&old), &new),
            vec![A11yEvent {
                node: Some(NodeId::from(1)),
                event_type: TYPE_WINDOW_CONTENT_CHANGED,
                content_change_types: CHANGE_SUBTREE,
            }]
        );
    }

    #[test]
    fn wholesale_change_collapses_to_root_subtree() {
        let old = update(
            vec![node(0, Role::Window, &[1, 2, 3, 4, 5, 6, 7, 8, 9])]
                .into_iter()
                .chain((1..=9u64).map(|id| node(id, Role::Label, &[])))
                .collect(),
            0,
            0,
        );
        let new_nodes: Vec<(NodeId, Node)> =
            vec![node(0, Role::Window, &[1, 2, 3, 4, 5, 6, 7, 8, 9])]
                .into_iter()
                .chain((1..=9u64).map(|id| {
                    let mut label = node(id, Role::Label, &[]).1;
                    label.set_label(format!("row {id}"));
                    (NodeId::from(id), label)
                }))
                .collect();
        let new = update(new_nodes, 0, 0);
        assert_eq!(
            diff_events(Some(&old), &new),
            vec![A11yEvent {
                node: Some(NodeId::from(0)),
                event_type: TYPE_WINDOW_CONTENT_CHANGED,
                content_change_types: CHANGE_SUBTREE,
            }]
        );
    }

    fn point_tree() -> TreeUpdate {
        // Root 100x100; a button covering [10,10]-[50,50]; a hidden subtree
        // over the same region; a touch-transparent decorator on top.
        let mut window = node(0, Role::Window, &[1, 2, 4]).1;
        window.set_bounds(Rect::new(0.0, 0.0, 100.0, 100.0));
        let mut button = node(1, Role::Button, &[]).1;
        button.set_bounds(Rect::new(10.0, 10.0, 50.0, 50.0));
        let mut hidden = node(2, Role::GenericContainer, &[3]).1;
        hidden.set_hidden();
        hidden.set_bounds(Rect::new(10.0, 10.0, 50.0, 50.0));
        let mut hidden_child = node(3, Role::Button, &[]).1;
        hidden_child.set_bounds(Rect::new(10.0, 10.0, 50.0, 50.0));
        let mut overlay = node(4, Role::GenericContainer, &[]).1;
        overlay.set_touch_transparent();
        overlay.set_bounds(Rect::new(0.0, 0.0, 100.0, 100.0));
        update(
            vec![
                (NodeId::from(0), window),
                (NodeId::from(1), button),
                (NodeId::from(2), hidden),
                (NodeId::from(3), hidden_child),
                (NodeId::from(4), overlay),
            ],
            0,
            0,
        )
    }

    #[test]
    fn hit_test_finds_innermost_served_node() {
        let tree = point_tree();
        assert_eq!(hit_test(&tree, 20.0, 20.0), Some(NodeId::from(1)));
    }

    #[test]
    fn hit_test_falls_through_to_root() {
        let tree = point_tree();
        assert_eq!(hit_test(&tree, 80.0, 80.0), Some(NodeId::from(0)));
        assert_eq!(hit_test(&tree, 200.0, 200.0), None);
    }

    #[test]
    fn events_json_shape() {
        let json = events_json(&[
            A11yEvent {
                node: None,
                event_type: TYPE_WINDOW_CONTENT_CHANGED,
                content_change_types: CHANGE_SUBTREE,
            },
            A11yEvent {
                node: Some(NodeId::from(7)),
                event_type: TYPE_VIEW_FOCUSED,
                content_change_types: 0,
            },
        ]);
        assert_eq!(
            json,
            r#"{"events":[{"id":-1,"mask":1,"type":2048},{"id":7,"mask":0,"type":8}]}"#
        );
    }

    #[test]
    fn selection_change_is_a_text_change() {
        let old = update(
            vec![node(0, Role::Window, &[1]), node(1, Role::TextInput, &[])],
            0,
            1,
        );
        let mut field = node(1, Role::TextInput, &[]).1;
        field.set_text_selection(TextSelection {
            anchor: accesskit::TextPosition {
                node: NodeId::from(1),
                character_index: 0,
            },
            focus: accesskit::TextPosition {
                node: NodeId::from(1),
                character_index: 4,
            },
        });
        let new = update(
            vec![node(0, Role::Window, &[1]), (NodeId::from(1), field)],
            0,
            1,
        );
        let events = diff_events(Some(&old), &new);
        assert!(
            events
                .iter()
                .any(|e| e.event_type == TYPE_VIEW_TEXT_CHANGED)
        );
    }
}
