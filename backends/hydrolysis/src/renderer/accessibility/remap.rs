//! Remapping of captured accessibility subtree node ids into the live
//! tree's id space when a retained subtree is replayed.

use super::*;

#[cfg(feature = "accessibility")]
pub(crate) fn kurbo_rect_to_accesskit_rect(rect: vello::kurbo::Rect) -> AccessibilityRect {
    AccessibilityRect {
        x0: rect.x0,
        y0: rect.y0,
        x1: rect.x1,
        y1: rect.y1,
    }
}

#[cfg(feature = "accessibility")]
pub(crate) fn accesskit_rect_to_kurbo_rect(rect: AccessibilityRect) -> vello::kurbo::Rect {
    vello::kurbo::Rect::new(rect.x0, rect.y0, rect.x1, rect.y1)
}

#[cfg(feature = "accessibility")]
#[derive(Clone, Copy)]
pub(crate) struct AccessibilityNodeIdRemap {
    first_mapped: u64,
}

#[cfg(feature = "accessibility")]
impl AccessibilityNodeIdRemap {
    pub(crate) const fn new(first_mapped: u64) -> Self {
        Self { first_mapped }
    }

    pub(crate) fn map(self, node_id: AccessibilityNodeId) -> AccessibilityNodeId {
        let offset = node_id
            .0
            .checked_sub(ACCESSIBILITY_FIRST_NODE_ID)
            .expect("hydrolysis dynamic accessibility node id underflow");
        AccessibilityNodeId(
            self.first_mapped
                .checked_add(offset)
                .expect("hydrolysis dynamic accessibility node id overflow"),
        )
    }
}

#[cfg(feature = "accessibility")]
pub(crate) fn remap_accessibility_node_id(
    node_id: AccessibilityNodeId,
    id_map: AccessibilityNodeIdRemap,
) -> AccessibilityNodeId {
    id_map.map(node_id)
}

#[cfg(feature = "accessibility")]
pub(crate) fn remap_accessibility_node_id_vec(
    node_ids: &[AccessibilityNodeId],
    id_map: AccessibilityNodeIdRemap,
) -> Vec<AccessibilityNodeId> {
    node_ids
        .iter()
        .copied()
        .map(|node_id| remap_accessibility_node_id(node_id, id_map))
        .collect()
}

#[cfg(feature = "accessibility")]
pub(crate) fn remap_accessibility_node_references(
    node: &mut AccessibilityNode,
    id_map: AccessibilityNodeIdRemap,
) {
    let children = node.children();
    if !children.is_empty() {
        let node_ids = remap_accessibility_node_id_vec(children, id_map);
        node.set_children(node_ids);
    }
    let controls = node.controls();
    if !controls.is_empty() {
        let node_ids = remap_accessibility_node_id_vec(controls, id_map);
        node.set_controls(node_ids);
    }
    let details = node.details();
    if !details.is_empty() {
        let node_ids = remap_accessibility_node_id_vec(details, id_map);
        node.set_details(node_ids);
    }
    let described_by = node.described_by();
    if !described_by.is_empty() {
        let node_ids = remap_accessibility_node_id_vec(described_by, id_map);
        node.set_described_by(node_ids);
    }
    let flow_to = node.flow_to();
    if !flow_to.is_empty() {
        let node_ids = remap_accessibility_node_id_vec(flow_to, id_map);
        node.set_flow_to(node_ids);
    }
    let labelled_by = node.labelled_by();
    if !labelled_by.is_empty() {
        let node_ids = remap_accessibility_node_id_vec(labelled_by, id_map);
        node.set_labelled_by(node_ids);
    }
    let owns = node.owns();
    if !owns.is_empty() {
        let node_ids = remap_accessibility_node_id_vec(owns, id_map);
        node.set_owns(node_ids);
    }
    let radio_group = node.radio_group();
    if !radio_group.is_empty() {
        let node_ids = remap_accessibility_node_id_vec(radio_group, id_map);
        node.set_radio_group(node_ids);
    }

    if let Some(node_id) = node.active_descendant() {
        node.set_active_descendant(remap_accessibility_node_id(node_id, id_map));
    }
    if let Some(node_id) = node.error_message() {
        node.set_error_message(remap_accessibility_node_id(node_id, id_map));
    }
    if let Some(node_id) = node.in_page_link_target() {
        node.set_in_page_link_target(remap_accessibility_node_id(node_id, id_map));
    }
    if let Some(node_id) = node.member_of() {
        node.set_member_of(remap_accessibility_node_id(node_id, id_map));
    }
    if let Some(node_id) = node.next_on_line() {
        node.set_next_on_line(remap_accessibility_node_id(node_id, id_map));
    }
    if let Some(node_id) = node.previous_on_line() {
        node.set_previous_on_line(remap_accessibility_node_id(node_id, id_map));
    }
    if let Some(node_id) = node.popup_for() {
        node.set_popup_for(remap_accessibility_node_id(node_id, id_map));
    }
}

