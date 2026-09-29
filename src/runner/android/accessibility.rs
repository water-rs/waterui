//! The accessibility snapshot published to the Kotlin host.
//!
//! The renderer's merged `accesskit::TreeUpdate` is serialized once per
//! change and pushed to `HydrolysisAccessibilityProvider`, which serves
//! `AccessibilityNodeInfo` for explore-by-touch without an invisible view
//! tree. Actions the provider dispatches come back through
//! `nativeAccessibilityAction` and run through
//! `handle_accessibility_action` — the same code path desktop uses.

use super::jni::JniError;

/// The host's copy of the accessibility tree — whole-tree publishes only;
/// `accesskit` updates already carry the minimal node deltas.
#[derive(Default)]
pub(crate) struct AccessibilitySnapshot {
    /// Serialized `accesskit::TreeUpdate` JSON for the provider.
    #[cfg(feature = "accessibility")]
    tree_json: Option<String>,
    #[cfg(feature = "accessibility")]
    dirty: bool,
}

impl AccessibilitySnapshot {
    /// The latest published tree, consumed by `nativeAccessibilityTree`.
    #[cfg(feature = "accessibility")]
    pub(crate) fn take_json(&mut self) -> Option<&str> {
        if self.dirty {
            self.dirty = false;
            self.tree_json.as_deref()
        } else {
            None
        }
    }
}

/// Publishes a changed accessibility tree to the host. Called at the end of
/// the frame transaction so one frame produces at most one snapshot.
#[cfg(feature = "accessibility")]
pub(crate) fn publish_if_pending(session: &mut super::host::AndroidSession) {
    // Popups have no second band on Android — the merged iterator is empty.
    let Some(update) = session
        .runtime
        .renderer
        .take_merged_accessibility_tree_update(std::iter::empty::<
            &mut crate::renderer::SemanticCore,
        >())
    else {
        return;
    };
    match serde_json::to_string(&update) {
        Ok(json) => {
            session.a11y.tree_json = Some(json);
            session.a11y.dirty = true;
            session.runtime.platform.bridge.accessibility_tree_changed();
        }
        Err(error) => {
            tracing::error!(
                target: "waterui::hydrolysis::android",
                %error,
                "accessibility tree serialization failed"
            );
        }
    }
}

/// Without the `accessibility` feature the module compiles to nothing —
/// the crate's existing contract is that this feature is a build-time gate.
#[cfg(not(feature = "accessibility"))]
pub(crate) fn publish_if_pending(_session: &mut super::host::AndroidSession) {}

/// The `android.view.accessibility` action constants the provider can
/// dispatch, mapped onto `accesskit::Action`. Anything else is an explicit
/// unsupported action — named in the error, never folded into a default.
#[cfg(feature = "accessibility")]
fn map_action(action: i32) -> Result<accesskit::Action, JniError> {
    // android.view.accessibility.AccessibilityNodeInfo constants
    const ACTION_FOCUS: i32 = 0x00000001;
    const ACTION_CLEAR_FOCUS: i32 = 0x00000002;
    const ACTION_CLICK: i32 = 0x00000010;
    const ACTION_ACCESSIBILITY_FOCUS: i32 = 0x00000040;
    const ACTION_CLEAR_ACCESSIBILITY_FOCUS: i32 = 0x00000080;
    const ACTION_SCROLL_FORWARD: i32 = 0x00001000;
    const ACTION_SCROLL_BACKWARD: i32 = 0x00002000;
    const ACTION_SET_TEXT: i32 = 0x00200000;
    const ACTION_EXPAND: i32 = 0x00040000;
    const ACTION_COLLAPSE: i32 = 0x00080000;
    const ACTION_SCROLL_UP: i32 = 16908344;
    const ACTION_SCROLL_DOWN: i32 = 16908345;
    const ACTION_SCROLL_LEFT: i32 = 16908346;
    const ACTION_SCROLL_RIGHT: i32 = 16908347;
    const ACTION_SHOW_TOOLTIP: i32 = 16908373;
    const ACTION_HIDE_TOOLTIP: i32 = 16908374;

    use accesskit::Action;
    match action {
        ACTION_FOCUS | ACTION_ACCESSIBILITY_FOCUS => Ok(Action::Focus),
        ACTION_CLEAR_FOCUS | ACTION_CLEAR_ACCESSIBILITY_FOCUS => Ok(Action::Blur),
        ACTION_CLICK => Ok(Action::Click),
        ACTION_EXPAND => Ok(Action::Expand),
        ACTION_COLLAPSE => Ok(Action::Collapse),
        ACTION_SET_TEXT => Ok(Action::ReplaceSelectedText),
        ACTION_SCROLL_FORWARD | ACTION_SCROLL_DOWN => Ok(Action::ScrollDown),
        ACTION_SCROLL_BACKWARD | ACTION_SCROLL_UP => Ok(Action::ScrollUp),
        ACTION_SCROLL_LEFT => Ok(Action::ScrollLeft),
        ACTION_SCROLL_RIGHT => Ok(Action::ScrollRight),
        ACTION_SHOW_TOOLTIP => Ok(Action::ShowTooltip),
        ACTION_HIDE_TOOLTIP => Ok(Action::HideTooltip),
        other => Err(JniError(format!(
            "hydrolysis android: unsupported accessibility action {other}"
        ))),
    }
}

/// Routes an action the provider dispatched for `virtual_view_id` (the
/// accesskit `NodeId` value) back into the renderer — inside the frame
/// boundary like any other input.
#[cfg(feature = "accessibility")]
pub(crate) fn perform_action(
    session: &mut super::host::AndroidSession,
    virtual_view_id: i64,
    action: i32,
    value: Option<String>,
) -> Result<bool, JniError> {
    use accesskit::{ActionData, ActionRequest, NodeId, TreeId};

    let request = ActionRequest {
        action: map_action(action)?,
        target_tree: TreeId::ROOT,
        target_node: NodeId(virtual_view_id.max(0) as u64),
        data: value.map(|value| ActionData::Value(value.into_boxed_str())),
    };
    Ok(session
        .runtime
        .renderer
        .handle_accessibility_action(request, &session.env))
}

/// Without the feature there is no tree to act on.
#[cfg(not(feature = "accessibility"))]
pub(crate) fn perform_action(
    _session: &mut super::host::AndroidSession,
    _virtual_view_id: i64,
    _action: i32,
    _value: Option<String>,
) -> Result<bool, JniError> {
    Ok(false)
}
