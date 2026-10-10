//! Modal dialog layer: `.dialog(...)` (water-rs/waterui#1210).
//!
//! A dialog is window-modal while its `is_presented` binding reads `true`.
//! Every `.dialog` node reports its presentation to the window's
//! [`DialogStack`] each pass — the rendered flush and the semantic walk
//! alike — and the stack answers which one is on screen: one dialog per
//! window, in presentation order. A dialog presented while another is up
//! waits behind it and comes up when the one in front closes, whatever the
//! two nodes' order in the tree. A node that stops reporting left the tree,
//! and its dialog closes with it through the binding.
//!
//! After the tree flushes, [`HydrolysisRenderer::render_dialogs`] draws the
//! front registration full-window — the `Dialog`'s modal layer: the `Scrim`
//! backdrop plus the composed card — into the modal layer above everything
//! else the window drew.
//!
//! Input and focus containment come from the environment the registration
//! carries: [`dialog_modal_environment`] extends the node's environment with
//! [`ModalInteraction`], so every interactive target the card registers is
//! modal-flagged, the modal shield gates pointer and keyboard input to them,
//! and Escape dispatches the modal's escape action — the dialog's cancel
//! path, shared with the system back mapping in `handle_back_navigation`.
//! Return runs the primary action of the dialog on screen whatever control
//! holds focus, as Space activates the focused one (`handle_keyboard_key_down`
//! reads [`DialogStack::front`]).
//! The same environment carries [`DialogLayerScope`], which turns the card's
//! `Dialog` accessibility node into a modal `AlertDialog`; the published tree
//! then holds that node alone under the window, so the content beneath is
//! inert for assistive technology.

use std::cell::RefCell;
use std::rc::Rc;

use waterui::Plugin;
use waterui::dialog::Dialog;
use waterui_backend_core::widget::ModalInteraction;
use waterui_core::Environment;
use waterui_core::handler::shared_action;

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;

/// Marks the environment a presented dialog's modal layer renders in.
///
/// Backend-internal: the accessibility container that carries the card's
/// `Dialog` role reads it to publish the node as a modal `AlertDialog`.
#[derive(Debug, Clone, Copy)]
pub struct DialogLayerScope;

impl Plugin for DialogLayerScope {}

/// One open dialog in the window's presentation order.
struct StackedDialog {
    /// The reporting node's identity.
    marker: Rc<()>,
    /// The dialog — its presentation binding is written `false` when the
    /// node leaves the tree while the dialog is open, and its primary action
    /// answers Return while it is on screen.
    dialog: Dialog,
    /// Whether the node reported during the current pass.
    reported: bool,
}

/// The window's open dialogs in presentation order. The first entry is the
/// dialog on screen; every later one waits for the entries before it to
/// close.
#[derive(Default)]
pub struct DialogStack {
    entries: Vec<StackedDialog>,
    /// The dialog on screen when the last pass closed.
    shown: Option<Rc<()>>,
    /// The keyboard focus the window held when the first dialog came up —
    /// where focus returns once the last one closes.
    return_focus: Option<InteractionKey>,
}

/// What a closed pass asks of keyboard focus.
pub enum DialogFocus {
    /// The dialog on screen is unchanged: focus stays where it is.
    Keep,
    /// A dialog came on screen: focus moves into it.
    Enter,
    /// The last dialog closed: focus returns to where it was before the
    /// first one came up, or stays put when nothing held it then.
    Return(Option<InteractionKey>),
}

impl DialogStack {
    /// Opens a whole-tree pass: drops the dialogs the application or an
    /// answered action closed since the last pass, so the next one in line
    /// is in front before any node reports.
    pub(crate) fn begin_pass(&mut self) {
        self.entries
            .retain(|entry| entry.dialog.is_presented().snapshot());
        for entry in &mut self.entries {
            entry.reported = false;
        }
    }

    /// A `.dialog` node's report for this pass: whether its dialog is
    /// presented, and the window's keyboard focus at the time. Returns
    /// whether this node's dialog is the one on screen.
    pub(crate) fn report(
        &mut self,
        marker: &Rc<()>,
        dialog: &Dialog,
        presented: bool,
        focus: Option<InteractionKey>,
    ) -> bool {
        let position = self
            .entries
            .iter()
            .position(|entry| Rc::ptr_eq(&entry.marker, marker));
        match (presented, position) {
            (false, Some(index)) => {
                self.entries.remove(index);
                false
            }
            (false, None) => false,
            (true, Some(index)) => {
                self.entries[index].reported = true;
                index == 0
            }
            (true, None) => {
                if self.shown.is_none() && self.entries.is_empty() {
                    self.return_focus = focus;
                }
                self.entries.push(StackedDialog {
                    marker: Rc::clone(marker),
                    dialog: dialog.clone(),
                    reported: true,
                });
                self.entries.len() == 1
            }
        }
    }

    /// The dialog on screen, while it is still presented — `None` once an
    /// answer closed it, before the next pass brings the next one in line
    /// forward.
    pub(crate) fn front(&self) -> Option<&Dialog> {
        let shown = self.shown.as_ref()?;
        self.entries
            .iter()
            .find(|entry| Rc::ptr_eq(&entry.marker, shown))
            .map(|entry| &entry.dialog)
            .filter(|dialog| dialog.is_presented().snapshot())
    }

    /// Closes a whole-tree pass: an open dialog whose node did not report
    /// left the tree, and the presentation closes with the subtree that
    /// carried it. Returns what the change of the dialog on screen asks of
    /// keyboard focus.
    pub(crate) fn finish_pass(&mut self) -> DialogFocus {
        self.entries.retain(|entry| {
            if !entry.reported {
                entry.dialog.is_presented().set(false);
            }
            entry.reported
        });
        let front = self.entries.first().map(|entry| Rc::clone(&entry.marker));
        let changed = match (&front, &self.shown) {
            (Some(front), Some(shown)) => !Rc::ptr_eq(front, shown),
            (None, None) => false,
            _ => true,
        };
        let focus = match (changed, &front) {
            (false, _) => DialogFocus::Keep,
            (true, Some(_)) => DialogFocus::Enter,
            (true, None) => DialogFocus::Return(self.return_focus.take()),
        };
        self.shown = front;
        focus
    }
}

/// What a `.dialog(...)` node's flush registered for the post-flush pass:
/// the environment the card presents in — extended with the modal scope —
/// and the retained layer content. Only the dialog the [`DialogStack`] puts
/// in front registers.
pub struct RegisteredDialog {
    /// The environment the card is built and flushed in: the node's
    /// environment plus the `ModalInteraction` scope that makes the layer
    /// modal — its targets modal-flagged, Escape routed to the cancel path.
    pub(crate) env: Environment,
    /// The retained layer content — the dialog's scrim-plus-card body —
    /// shared with the effect so it survives across frames.
    pub(crate) content: Rc<RefCell<RetainedSubview>>,
}

/// The environment a dialog's layer content presents in: the declaring
/// node's environment plus the `ModalInteraction` scope whose escape action
/// is the dialog's cancel path — Escape, the system back gesture and the
/// scrim tap all reach `Dialog::run_cancel`, which is a no-op when the
/// dialog declared no `Cancel` action — and the [`DialogLayerScope`] marker.
pub fn dialog_modal_environment(env: &Environment, dialog: &Dialog) -> Environment {
    let escape = {
        let dialog = dialog.clone();
        shared_action(move |env: Environment| dialog.run_cancel(&env))
    };
    env.extending(ModalInteraction::new(true, escape))
        .extending(DialogLayerScope)
}

impl HydrolysisRenderer {
    /// Draws the front `.dialog(...)` registration into the modal layer.
    /// Registered by that node's flush; drawn after the tree flush returns,
    /// so nothing the window flushed can stack above the scrim. The layer
    /// content is full-window — the scrim backdrop plus the centered card —
    /// so it measures and places at the window bounds directly.
    ///
    /// # Panics
    ///
    /// Panics when a material-group scope is still open on entry — the tree
    /// flush must have left the scope stack empty before this pass runs.
    pub(crate) fn render_dialogs(
        &mut self,
        transform: kurbo::Affine,
        safe_area: &crate::renderer::SafeAreaLayout,
    ) {
        assert!(
            self.material_group_scopes.is_empty(),
            "hydrolysis renderer: the dialog pass must start with an empty \
             material-scope stack — the tree flush leaves no scope open"
        );
        let Some(front) = self.popup_menu.front_dialog.take() else {
            return;
        };
        let window = self.window_bounds;
        let proposal = bounded_proposal(window);
        let mut content = front.content.borrow_mut();
        let _ = content.patch_and_measure(self, &front.env, proposal);
        self.register_hit_test_occluder(window);
        content.place_detached(
            self,
            RenderContext {
                local: transform,
                bounds: window,
            },
            &front.env,
            proposal,
            window,
            Some(safe_area.with_frame(window)),
        );
    }
}
