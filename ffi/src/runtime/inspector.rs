//! Bringing up the inspector from a native backend.
//!
//! A backend knows what a user did — a secondary click, a long press, a menu
//! item — and the runtime knows what to do about it. These two entry points are
//! the whole of that: everything else about inspection already crosses no
//! boundary, because the endpoint runs inside the application.
//!
//! Both are inert unless an inspector endpoint is running, which in practice
//! means a debug build.

use crate::WuiEnv;

/// Opens the inspector for this application.
///
/// Reveals nothing in particular: use this where the backend cannot say which
/// element the user meant, such as a menu item that is about the application
/// rather than about a point on screen.
///
/// # Safety
///
/// `env` must be a valid environment handle that stays alive for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_inspector_open(env: *const WuiEnv) {
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let env = unsafe { crate::borrow_ffi(env) };
    let Some(inspector) = env.get::<waterui::inspector::InspectorRuntime>() else {
        // Silence here is the worst answer: the gesture is only offered when
        // `waterui_inspector_is_available` said yes, so reaching this means the
        // two disagree about which environment they were asked about.
        tracing::warn!(
            target: "waterui::inspector",
            "Asked to open the inspector in an environment that has no endpoint"
        );
        return;
    };
    inspector.open();
}

/// Reveals one node in the inspector, opening one if none is attached.
///
/// `node` is an accessibility node id, which is what the inspector's tree is
/// keyed by. A backend that publishes no tree has no id to pass and should call
/// [`waterui_inspector_open`] instead.
///
/// # Safety
///
/// `env` must be a valid environment handle that stays alive for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_inspector_inspect_node(env: *const WuiEnv, node: u64) {
    // SAFETY: as above.
    let env = unsafe { crate::borrow_ffi(env) };
    let Some(inspector) = env.get::<waterui::inspector::InspectorRuntime>() else {
        // As above: an ignored request to inspect looks like a broken gesture.
        tracing::warn!(
            target: "waterui::inspector",
            node,
            "Asked to inspect a node in an environment that has no endpoint"
        );
        return;
    };
    inspector.inspect_node(waterui::inspector::protocol::NodeId(node));
}

/// Whether this build offers inspection at all.
///
/// A backend asks before putting "Inspect element" in front of a user, so that
/// a release build shows nothing rather than an entry that does nothing.
///
/// # Safety
///
/// `env` must be a valid environment handle that stays alive for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_inspector_is_available(env: *const WuiEnv) -> bool {
    // SAFETY: as above.
    let env = unsafe { crate::borrow_ffi(env) };
    env.get::<waterui::inspector::InspectorRuntime>().is_some()
}

/// One node of a backend's accessibility tree, as it crosses the boundary.
///
/// Strings are borrowed for the duration of the call: the runtime copies what
/// it keeps, so a backend can build these from whatever it already has without
/// handing over ownership.
#[repr(C)]
pub struct WuiInspectorNode {
    /// Identifier, stable for as long as the node exists.
    pub id: u64,
    /// Role, lowercase, as the platform names it.
    pub role: crate::WuiStr,
    /// Accessibility label; empty when the node has none.
    pub label: crate::WuiStr,
    /// Current value; empty when the node has none.
    pub value: crate::WuiStr,
    /// Whether `bounds` holds a rectangle.
    pub has_bounds: bool,
    /// Layout rectangle in window coordinates, when `has_bounds`.
    pub bounds: [f32; 4],
    /// Whether the node accepts interaction.
    pub enabled: bool,
    /// Whether the node is hidden from assistive technology.
    pub hidden: bool,
    /// Whether the node is selected.
    pub selected: bool,
    /// Whether `checked` means anything for this node.
    pub has_checked: bool,
    /// Toggle state, when `has_checked`.
    pub checked: bool,
    /// Children, in order.
    pub children: *const u64,
    /// Number of children.
    pub children_len: usize,
}

/// Whether anything is watching the accessibility tree.
///
/// A backend walks its view hierarchy only when the answer is yes: with no
/// inspector attached the tree costs nothing, which is the whole point of
/// asking before building it.
///
/// # Safety
///
/// `env` must be a valid environment handle that stays alive for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_inspector_wants_tree(env: *const WuiEnv) -> bool {
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let env = unsafe { crate::borrow_ffi(env) };
    env.get::<waterui::inspector::TreeRecorder>()
        .is_some_and(waterui::inspector::TreeRecorder::is_active)
}

/// Publishes a backend's accessibility tree to the inspector.
///
/// The whole tree each time: the runtime turns it into a delta, so a backend
/// does not have to track what changed.
///
/// # Safety
///
/// `env` must be a valid environment handle. `nodes` must point to `len`
/// initialised nodes, whose strings and children arrays stay valid for the
/// duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_inspector_publish_tree(
    env: *const WuiEnv,
    root: u64,
    has_focus: bool,
    focus: u64,
    nodes: *const WuiInspectorNode,
    len: usize,
) {
    use waterui::inspector::protocol::{Bounds, NodeId, NodeState, TreeNode};

    // SAFETY: as above.
    let env = unsafe { crate::borrow_ffi(env) };
    let Some(recorder) = env.get::<waterui::inspector::TreeRecorder>() else {
        return;
    };
    if !recorder.is_active() {
        return;
    }
    if nodes.is_null() {
        return;
    }

    // SAFETY: the caller contract makes this `len` initialised nodes.
    let raw = unsafe { core::slice::from_raw_parts(nodes, len) };
    let projected = raw
        .iter()
        .map(|node| {
            // SAFETY: the caller keeps every string alive for this call.
            let text = |value: &crate::WuiStr| unsafe { value.as_str() }.to_string();
            let optional = |value: &crate::WuiStr| {
                let text = text(value);
                (!text.is_empty()).then_some(text)
            };
            let children = if node.children.is_null() || node.children_len == 0 {
                Vec::new()
            } else {
                // SAFETY: as above, for the children array.
                unsafe { core::slice::from_raw_parts(node.children, node.children_len) }
                    .iter()
                    .map(|id| NodeId(*id))
                    .collect()
            };
            TreeNode {
                id: NodeId(node.id),
                role: text(&node.role),
                label: optional(&node.label),
                value: optional(&node.value),
                bounds: node.has_bounds.then(|| Bounds {
                    x: node.bounds[0],
                    y: node.bounds[1],
                    width: node.bounds[2],
                    height: node.bounds[3],
                }),
                state: NodeState {
                    enabled: node.enabled,
                    selected: node.selected,
                    checked: node.has_checked.then_some(node.checked),
                    expanded: None,
                    hidden: node.hidden,
                },
                children,
            }
        })
        .collect();

    recorder.record_snapshot(NodeId(root), has_focus.then_some(NodeId(focus)), projected);
}
