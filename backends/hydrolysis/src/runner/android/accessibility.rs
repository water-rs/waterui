//! The accessibility snapshot published to the Kotlin host.
//!
//! The renderer's merged `accesskit::TreeUpdate` is diffed against the last
//! one published — [`crate::runner::android_accessibility::diff_events`]
//! decides which Android events the change owes services — serialized, and
//! pushed to `HydrolysisAccessibilityProvider`, which serves
//! `AccessibilityNodeInfo` for explore-by-touch without an invisible view
//! tree. A publish whose semantics did not change produces no events and is
//! never serialized. Actions the provider dispatches come back through
//! `nativeAccessibilityAction` and run through
//! `handle_accessibility_action` — the same code path desktop uses.

use super::jni::JniError;
#[cfg(feature = "accessibility")]
use crate::renderer::accessibility::AccessibilityContentTypes;

/// The JSON document `nativeAccessibilityTree` hands the provider: the
/// merged `TreeUpdate` under `update`, plus every text field's declared
/// content type under `contentTypes` — keyed by the same merged node id,
/// with the camelCase names the provider maps onto `HintConstants`.
#[cfg(feature = "accessibility")]
#[derive(serde::Serialize)]
struct AccessibilityPayload<'a> {
    update: &'a accesskit::TreeUpdate,
    #[serde(rename = "contentTypes")]
    content_types: &'a AccessibilityContentTypes,
}

/// The host's copy of the accessibility tree — whole-tree publishes only;
/// `accesskit` updates already carry the minimal node deltas.
#[derive(Default)]
pub struct AccessibilitySnapshot {
    /// Serialized `accesskit::TreeUpdate` JSON for the provider.
    #[cfg(feature = "accessibility")]
    tree_json: Option<String>,
    #[cfg(feature = "accessibility")]
    dirty: bool,
    /// The update the last publish offered the host — kept in `accesskit`
    /// form so `nativeAccessibilityHitTest` answers over exactly the tree
    /// the provider serves.
    #[cfg(feature = "accessibility")]
    published: Option<accesskit::TreeUpdate>,
    /// The content types the last publish carried — compared alone, so a
    /// declaration change with an identical tree still republishes.
    #[cfg(feature = "accessibility")]
    published_content_types: AccessibilityContentTypes,
}

impl AccessibilitySnapshot {
    /// The latest published tree, consumed by `nativeAccessibilityTree`.
    #[cfg(feature = "accessibility")]
    pub fn take_json(&mut self) -> Option<&str> {
        if self.dirty {
            self.dirty = false;
            self.tree_json.as_deref()
        } else {
            None
        }
    }

    /// The last update offered to the host — what
    /// `nativeAccessibilityHitTest` maps pointer coordinates onto.
    #[cfg(feature = "accessibility")]
    pub const fn published(&self) -> Option<&accesskit::TreeUpdate> {
        self.published.as_ref()
    }
}

/// Publishes a semantically changed accessibility tree to the host, plus
/// the per-node events the change owes services. Called at the end of the
/// frame transaction so one frame produces at most one publish; a frame
/// whose tree is unchanged publishes nothing — an animating indeterminate
/// indicator must not cost services a content-changed event per frame
/// (#246).
#[cfg(feature = "accessibility")]
pub fn publish_if_pending(session: &mut super::host::AndroidSession) {
    // Popups have no second band on Android — the merged iterator is empty.
    let Some(merged) = session
        .runtime
        .renderer
        .take_merged_accessibility_tree_update(std::iter::empty::<
            &mut crate::renderer::SemanticCore,
        >())
    else {
        return;
    };
    // Publish whenever the update or the content types differ from what was
    // last served — including changes no event is emitted for (bounds drift,
    // scroll metrics). The empty event list below then just marks the
    // provider dirty so the next query pulls a fresh tree. An identical
    // publish skips the serialize and the JNI crossing entirely.
    let changed = session.a11y.published.as_ref() != Some(&merged.tree_update)
        || session.a11y.published_content_types != merged.content_types;
    let events = crate::runner::android_accessibility::diff_events(
        session.a11y.published.as_ref(),
        &merged.tree_update,
    );
    session.a11y.published = Some(merged.tree_update);
    session.a11y.published_content_types = merged.content_types;
    if !changed {
        return;
    }
    let Some(published) = session.a11y.published.as_ref() else {
        return;
    };
    let payload = AccessibilityPayload {
        update: published,
        content_types: &session.a11y.published_content_types,
    };
    match serde_json::to_string(&payload) {
        Ok(json) => {
            session.a11y.tree_json = Some(json);
            session.a11y.dirty = true;
            session.runtime.platform.bridge.accessibility_tree_changed(
                &crate::runner::android_accessibility::events_json(&events),
            );
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
pub const fn publish_if_pending(_session: &mut super::host::AndroidSession) {}

/// The `accesskit::Action` bitmask index the provider echoes back. The Kotlin
/// side decodes a node's serialized `actions`/`childActions` bitmask and
/// advertises the platform actions each bit implies; a performed action comes
/// back as the same index, so the JNI edge carries no per-platform constants
/// at all. The table is `accesskit`'s declaration order — the index IS the
/// `ActionIndex`.
#[cfg(feature = "accessibility")]
fn map_action(action: i32) -> Result<accesskit::Action, JniError> {
    use accesskit::Action;
    const ACTIONS: &[Action] = &[
        Action::Click,
        Action::Focus,
        Action::Blur,
        Action::Collapse,
        Action::Expand,
        Action::CustomAction,
        Action::Decrement,
        Action::Increment,
        Action::HideTooltip,
        Action::ShowTooltip,
        Action::ReplaceSelectedText,
        Action::ScrollDown,
        Action::ScrollLeft,
        Action::ScrollRight,
        Action::ScrollUp,
        Action::ScrollIntoView,
        Action::ScrollToPoint,
        Action::SetScrollOffset,
        Action::SetTextSelection,
        Action::SetSequentialFocusNavigationStartingPoint,
        Action::SetValue,
        Action::ShowContextMenu,
    ];
    usize::try_from(action)
        .ok()
        .and_then(|index| ACTIONS.get(index))
        .copied()
        .ok_or_else(|| {
            JniError(format!(
                "hydrolysis android: unsupported accessibility action {action}"
            ))
        })
}

/// Routes an action the provider dispatched for `virtual_view_id` (the
/// accesskit `NodeId` value) back into the renderer — inside the frame
/// boundary like any other input.
///
/// The provider sends the data kind the target's role expects: `arg1`/`arg2`
/// carry the `SetTextSelection` UTF-16 bounds, `text` the `SetValue`/
/// `ReplaceSelectedText` string (Android's `ACTION_SET_TEXT` replaces the
/// full contents), and `numeric` the `SetValue`/`CustomAction` payload on a
/// range or custom action. A request mixing channels is a provider bug, so
/// it errors rather than guessing.
///
/// Actions on a node a text input claims do not take the generic
/// `ActionRequest` path: `TalkBack` editing runs over the session's
/// [`EditingSession`](crate::runner::editing::EditingSession), the same
/// writer the `InputConnection` mirror uses, so the IME-visible state never
/// diverges from the semantic tree's value.
#[cfg(feature = "accessibility")]
pub fn perform_action(
    session: &mut super::host::AndroidSession,
    virtual_view_id: i64,
    action: i32,
    arg1: i32,
    arg2: i32,
    text: Option<String>,
    numeric: Option<f64>,
) -> Result<bool, JniError> {
    use accesskit::{Action, ActionData, ActionRequest, NodeId, TreeId};

    let action = map_action(action)?;
    let node = NodeId(crate::num_cast::i64_as_u64(virtual_view_id.max(0)));
    if let Some(handled) =
        text_input_action(session, node, action, arg1, arg2, text.as_ref(), numeric)?
    {
        return Ok(handled);
    }

    // The action decides which payload channel is meaningful, so a provider
    // that sends both (or the wrong one) errors instead of being guessed at.
    let data = match (action, text, numeric) {
        (Action::CustomAction, None, Some(index)) => {
            Some(ActionData::CustomAction(crate::num_cast::f64_as_i32(index)))
        }
        (Action::SetValue, Some(text), None) => Some(ActionData::Value(text.into_boxed_str())),
        (Action::SetValue, None, Some(numeric)) => Some(ActionData::NumericValue(numeric)),
        (_, None, None) => None,
        _ => {
            return Err(JniError(format!(
                "hydrolysis android: accessibility action {action:?} carries mismatched data"
            )));
        }
    };
    if action != Action::SetTextSelection && (arg1 >= 0 || arg2 >= 0) {
        return Err(JniError(format!(
            "hydrolysis android: accessibility action {action:?} carries unexpected selection bounds"
        )));
    }
    let request = ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: node,
        data,
    };
    let handled = session
        .runtime
        .renderer
        .handle_accessibility_action(request, &session.env);
    if handled {
        // A focus the action moved lands in the mirror now — the connection
        // rebinds while the screen reader's announcement is still live,
        // not at the next vsync.
        session.editing_sync();
    }
    Ok(handled)
}

/// The editing-session path for nodes a text input claims. Returns `None`
/// when the action is not an editing action or the target has no text
/// input, leaving it to the generic `ActionRequest` dispatch.
#[cfg(feature = "accessibility")]
fn text_input_action(
    session: &mut super::host::AndroidSession,
    node: accesskit::NodeId,
    action: accesskit::Action,
    arg1: i32,
    arg2: i32,
    text: Option<&String>,
    numeric: Option<f64>,
) -> Result<Option<bool>, JniError> {
    use accesskit::Action;

    let is_editing_action = matches!(
        action,
        Action::Click
            | Action::Focus
            | Action::SetValue
            | Action::ReplaceSelectedText
            | Action::SetTextSelection
    );
    if !is_editing_action
        || !session
            .runtime
            .renderer
            .accessibility_node_is_text_input(node)
    {
        return Ok(None);
    }

    // An action on an unfocused editable activates it first — the same
    // transition a tap performs: focus moves, the mirror adopts the new
    // editor with a fresh generation, and the frame transaction publishes
    // the text-input target the IME shows against.
    if session
        .runtime
        .renderer
        .focused_text_input_accessibility_node()
        != Some(node)
    {
        session
            .runtime
            .renderer
            .focus_text_input_for_accessibility_node(node);
        session.editing_sync();
    }
    let editor_id = session.ime.session.editor_id();
    let editing = &mut session.ime.session;
    let handled = match action {
        Action::Click | Action::Focus => true,
        // ACTION_SET_TEXT replaces the field's whole contents: select
        // everything (the op clamps to the text length) and commit.
        Action::SetValue => {
            let Some(text) = text else {
                return Err(JniError(
                    "hydrolysis android: editable SetValue requires a text payload".into(),
                ));
            };
            if numeric.is_some() {
                return Err(JniError(
                    "hydrolysis android: editable SetValue carries a numeric payload".into(),
                ));
            }
            editing.set_selection(editor_id, 0, i32::MAX) && editing.commit_text(editor_id, text, 1)
        }
        Action::ReplaceSelectedText => {
            let Some(text) = text else {
                return Err(JniError(
                    "hydrolysis android: editable ReplaceSelectedText requires a text payload"
                        .into(),
                ));
            };
            editing.commit_text(editor_id, text, 1)
        }
        Action::SetTextSelection => {
            if arg1 < 0 || arg2 < 0 {
                return Err(JniError(
                    "hydrolysis android: SetTextSelection requires start and end bounds".into(),
                ));
            }
            editing.set_selection(editor_id, arg1, arg2)
        }
        _ => unreachable!("filtered above"),
    };
    if handled {
        session.editing_flush_and_sync();
    }
    Ok(Some(handled))
}

/// Without the feature there is no tree to act on.
#[cfg(not(feature = "accessibility"))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "shares the JNI signature with the accessibility-enabled sibling, which does fail"
)]
pub fn perform_action(
    _session: &mut super::host::AndroidSession,
    _virtual_view_id: i64,
    _action: i32,
    _arg1: i32,
    _arg2: i32,
    _text: Option<String>,
    _numeric: Option<f64>,
) -> Result<bool, JniError> {
    Ok(false)
}
