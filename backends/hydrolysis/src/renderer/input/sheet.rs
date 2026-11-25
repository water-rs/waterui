//! Modal sheet layer: `.sheet(...)` (water-rs/waterui#659).
//!
//! A sheet is window-modal while its `is_presented` binding reads `true`.
//! Every `.sheet` node reports its presentation to the window's [`SheetStack`]
//! each pass — the rendered flush and the semantic walk alike — and the stack
//! answers which sheet is on screen: one sheet per window, in presentation
//! order. A sheet presented while another is up waits behind it and comes up
//! when the one in front closes. A node that stops reporting left the tree, and
//! its sheet closes with it through the binding.
//!
//! After the tree flushes (and before `.dialog` alerts), [`HydrolysisRenderer::render_sheets`]
//! draws the front registration full-window — the `Sheet`'s modal layer: the
//! `Scrim` backdrop plus the detent-sized modal surface — above all regular
//! window content.
//!
//! Input and focus containment come from [`sheet_modal_environment`], which
//! extends the declaring node's environment with [`ModalInteraction`],
//! [`SheetLayerScope`], and `State(SheetDismiss)`.

use std::cell::RefCell;
use std::rc::Rc;

use kurbo::Affine;
use nami::Signal as _;
use waterui::Plugin;
use waterui::sheet::Sheet;
use waterui_core::Environment;
use waterui_core::extract::State as EnvState;
use waterui_core::handler::shared_action;

use super::InteractionKey;
use crate::renderer::{
    HydrolysisRenderer, RenderContext, RetainedSubview, SafeAreaLayout, bounded_proposal,
};
use crate::text::TextEngine;
use crate::theme::ModalInteraction;

/// Marks the environment a presented sheet's modal layer renders in.
///
/// Backend-internal: the accessibility container that carries the sheet
/// surface's `Dialog` role reads it (when outside `SheetContentScope`) to
/// publish the node as a modal `Dialog`.
#[derive(Debug, Clone, Copy)]
pub struct SheetLayerScope;

impl Plugin for SheetLayerScope {}

/// One open sheet in the window's presentation order.
struct StackedSheet {
    marker: Rc<()>,
    sheet: Sheet,
    reported: bool,
}

/// The window's open sheets in presentation order. The first entry is the sheet
/// on screen; later ones wait for earlier entries to close.
#[derive(Default)]
pub struct SheetStack {
    entries: Vec<StackedSheet>,
    shown: Option<Rc<()>>,
    return_focus: Option<InteractionKey>,
}

/// What a closed pass asks of keyboard focus.
pub enum SheetFocus {
    /// The sheet on screen is unchanged: focus stays where it is.
    Keep,
    /// A sheet came on screen: focus moves into it.
    Enter,
    /// The last sheet closed: focus returns to where it was before the first
    /// one came up, or stays put when nothing held it then.
    Return(Option<InteractionKey>),
}

impl SheetStack {
    /// Opens a whole-tree pass: drops sheets closed since the last pass so the
    /// next sheet in line moves to the front before nodes report.
    pub(crate) fn begin_pass(&mut self) {
        self.entries
            .retain(|entry| entry.sheet.is_presented().snapshot());
        for entry in &mut self.entries {
            entry.reported = false;
        }
    }

    /// Reports a `.sheet` node for this pass and returns whether it is the
    /// front sheet on screen.
    pub(crate) fn report(
        &mut self,
        marker: &Rc<()>,
        sheet: &Sheet,
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
                self.entries.push(StackedSheet {
                    marker: Rc::clone(marker),
                    sheet: sheet.clone(),
                    reported: true,
                });
                self.entries.len() == 1
            }
        }
    }

    /// Closes a whole-tree pass: closes any sheet whose node left the tree and
    /// returns the keyboard-focus transition required.
    pub(crate) fn finish_pass(&mut self) -> SheetFocus {
        self.entries.retain(|entry| {
            if !entry.reported {
                entry.sheet.is_presented().set(false);
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
            (false, _) => SheetFocus::Keep,
            (true, Some(_)) => SheetFocus::Enter,
            (true, None) => SheetFocus::Return(self.return_focus.take()),
        };
        self.shown = front;
        focus
    }
}

/// What a `.sheet(...)` node's flush registered for the post-flush pass.
pub struct RegisteredSheet<E: TextEngine> {
    pub(crate) env: Environment,
    pub(crate) sheet: Sheet,
    pub(crate) content: Rc<RefCell<Option<RetainedSubview<E>>>>,
}

/// Builds the environment a sheet's modal layer presents in: the declaring
/// node's environment plus `ModalInteraction` (routing Escape and platform back
/// to [`Sheet::run_user_dismiss`]), [`SheetLayerScope`], and
/// `State(SheetDismiss)`.
pub fn sheet_modal_environment(env: &Environment, sheet: &Sheet) -> Environment {
    let escape = {
        let sheet = sheet.clone();
        shared_action(move |env: Environment| sheet.run_user_dismiss(&env))
    };
    env.extending(ModalInteraction::new(true, escape))
        .extending(SheetLayerScope)
        .extending(EnvState(sheet.dismiss_handle()))
}

impl<E: TextEngine> HydrolysisRenderer<E> {
    /// Draws the front `.sheet(...)` registration into the modal sheet layer.
    ///
    /// Runs after the main tree flush and before `.dialog` alerts so a
    /// confirmation alert opened from inside a sheet stacks cleanly above it.
    pub(crate) fn render_sheets(&mut self, transform: Affine, safe_area: &SafeAreaLayout) {
        assert!(
            self.material_group_scopes.is_empty(),
            "hydrolysis renderer: the sheet pass must start with an empty \
             material-scope stack — the tree flush leaves no scope open"
        );
        let Some(front) = self.popup_menu.front_sheet.take() else {
            return;
        };
        let window = self.window_bounds;
        let proposal = bounded_proposal(window);
        let mut slot = front.content.borrow_mut();
        let content = slot.get_or_insert_with(|| RetainedSubview::new(front.sheet.modal_layer()));
        let _ = content.patch_and_measure(self, &front.env, proposal);
        self.register_hit_test_occluder(window);
        content.place_detached(
            self,
            RenderContext::new(transform, window),
            &front.env,
            proposal,
            window,
            Some(safe_area.with_frame(window)),
        );
    }
}
