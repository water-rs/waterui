//! Modal dialog layer: `.dialog(...)` (water-rs/waterui#1210).
//!
//! A dialog is window-modal while its `is_presented` binding reads `true`.
//! The `.dialog` node's flush registers it here every frame; after the tree
//! flushes, [`HydrolysisRenderer::render_dialogs`] draws the frontmost open
//! registration full-window — the `Dialog`'s own body: the `Scrim` backdrop
//! plus the composed card — into the modal layer above everything else the
//! window drew.
//!
//! One dialog presents per window at a time: the first open registration in
//! flush order takes the slot and every later one waits in presentation
//! order, so a second dialog presented while one is up draws when the first
//! closes. A node that stops registering closed with the subtree that carried
//! it — the pass writes its binding back to `false`.
//!
//! Input and focus containment come from the environment the registration
//! carries: `apply_dialog` extends the node's environment with
//! [`ModalInteraction`], so every interactive target the card registers is
//! modal-flagged, the modal shield gates pointer and keyboard input to them,
//! and Escape dispatches the modal's escape action — the dialog's cancel
//! path, shared with the Android system back mapping in
//! `runner/android/host.rs`.

use std::cell::RefCell;
use std::rc::Rc;

use nami::Binding;
use waterui::dialog::Dialog;
use waterui_backend_core::widget::ModalInteraction;
use waterui_core::Environment;
use waterui_core::handler::shared_action;

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;

/// What a `.dialog(...)` node's flush registered: whether the dialog is open
/// this frame, the handles the post-flush render pass needs, and the
/// environment the card presents in — extended with the modal scope. The
/// content slot lives on the effect (`Rc`-shared) so the card's built node
/// survives both closes and the node's own reconcile.
pub struct RegisteredDialog {
    /// Whether the dialog is presented this frame.
    pub presented: bool,
    /// The presentation binding — the pass writes `false` to it when the node
    /// that owned this registration left the tree.
    pub is_presented: Binding<bool>,
    /// The environment the card is built and flushed in: the node's
    /// environment plus the `ModalInteraction` scope that makes the layer
    /// modal — its targets modal-flagged, Escape routed to the cancel path.
    pub env: Environment,
    /// The retained layer content — the dialog's scrim-plus-card body —
    /// shared with the effect so it survives across frames.
    pub content: Rc<RefCell<Option<RetainedSubview>>>,
    /// Identity shared between the effect and its registration, so the render
    /// pass can tell a re-registered node from a dropped one.
    pub marker: Rc<()>,
}

/// What the render pass leaves behind for the next frame: the marker and
/// binding of the dialog it drew. An entry the next frame's flush does not
/// re-register is a node that left the tree — its dialog closes through the
/// binding.
pub struct PresentedDialog {
    /// The registration identity the drawn dialog came from.
    pub marker: Rc<()>,
    /// The binding to close when the owning node stops registering.
    pub is_presented: Binding<bool>,
}

/// The environment a dialog's layer content presents in: the declaring
/// node's environment plus the `ModalInteraction` scope whose escape action
/// is the dialog's cancel path — Escape, the Android back gesture and the
/// scrim tap all reach `Dialog::run_cancel`, which is a no-op when the dialog
/// declared no `Cancel` action.
pub fn dialog_modal_environment(env: &Environment, dialog: &Dialog) -> Environment {
    let escape = {
        let dialog = dialog.clone();
        shared_action(move |env: Environment| dialog.run_cancel(&env))
    };
    env.extending(ModalInteraction::new(true, escape))
}

impl HydrolysisRenderer {
    /// Draws the frontmost open `.dialog(...)` registration into the modal
    /// layer. Registered this frame by every `.dialog` node's flush; drawn
    /// after the tree flush returns, so nothing the window flushed can stack
    /// above the scrim. The layer content is full-window — the `Dialog`'s
    /// body is the scrim backdrop plus the centered card — so it measures and
    /// places at the window bounds directly.
    ///
    /// # Panics
    ///
    /// Panics when a material-group scope is still open on entry — the tree
    /// flush must have left the scope stack empty before this pass runs.
    pub fn render_dialogs(
        &mut self,
        transform: kurbo::Affine,
        safe_area: &crate::renderer::SafeAreaLayout,
    ) {
        assert!(
            self.material_group_scopes.is_empty(),
            "hydrolysis renderer: the dialog pass must start with an empty \
             material-scope stack — the tree flush leaves no scope open"
        );
        let registered = core::mem::take(&mut self.popup_menu.dialogs);
        let last_presented = core::mem::take(&mut self.popup_menu.presented_dialogs);
        let mut presented = Vec::with_capacity(last_presented.len());

        // A node that drew last frame but registered nothing this frame left
        // the tree: close its dialog through the binding — the contract
        // closes the presentation with the node that carried it.
        for dialog in &last_presented {
            if !registered
                .iter()
                .any(|entry| Rc::ptr_eq(&entry.marker, &dialog.marker))
            {
                dialog.is_presented.set(false);
            }
        }

        let window = self.window_bounds;
        let mut slot_taken = false;
        for entry in registered {
            // One dialog presents per window: the first open registration in
            // flush order takes the layer; later open ones wait their turn,
            // and a closed or not-yet-presented registration draws nothing.
            if slot_taken || !entry.presented {
                continue;
            }
            let Some(mut content) = entry.content.borrow_mut().take() else {
                continue;
            };
            let proposal = bounded_proposal(window);
            let (ideal, _stretch) = content.patch_and_measure(self, &entry.env, proposal);
            let _ = ideal;
            self.register_hit_test_occluder(window);
            content.place_detached(
                self,
                RenderContext {
                    local: transform,
                    bounds: window,
                },
                &entry.env,
                proposal,
                window,
                Some(safe_area.with_frame(window)),
            );
            *entry.content.borrow_mut() = Some(content);

            presented.push(PresentedDialog {
                marker: entry.marker.clone(),
                is_presented: entry.is_presented.clone(),
            });
            slot_taken = true;
        }
        self.popup_menu.presented_dialogs = presented;
    }
}
