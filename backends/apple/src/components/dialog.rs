//! The `dialog` metadata: a platform alert — an `NSAlert` sheet on macOS, a
//! `UIAlertController` on iOS — presented while `is_presented` reads `true`
//! (water-rs/waterui#1210).
//!
//! The leaf is the transparent container the overlay contract asks for: the
//! wrapped content mounts and lays out untouched, and the dialog itself is
//! the platform object beside it. `is_presented` drives present/dismiss,
//! `Dialog::run_action` runs an action's handler and writes the binding
//! back to `false`, and the reactive title/message keep streaming into the
//! live alert.

use alloc::rc::Rc;
#[cfg(target_os = "macos")]
use alloc::vec::Vec;
use core::cell::RefCell;

use cocoa_ui::geometry::Rect as KitRect;
use cocoa_ui::view;
use cocoa_ui::{PlatformView, Retained};
#[cfg(target_os = "macos")]
use waterui::dialog::DialogAction;
use waterui::dialog::{Dialog, DialogRole};
use waterui::reactive::{Binding, Computed, Signal};
use waterui::text::StyledStr;
use waterui_backend_core::Environment;
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HostView, SheetAlert, window_of};
#[cfg(target_os = "macos")]
use cocoa_ui::objc2_foundation::NSString;
#[cfg(target_os = "ios")]
use cocoa_ui::objc2_ui_kit::UIAlertActionStyle;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{AlertController, HostView};

/// The leaf's live state: the mounted content child, the dialog being
/// presented, its resolved texts, and the platform alert while it is up.
struct DialogState {
    /// Proof of the main thread for kit calls inside callbacks.
    mtm: cocoa_ui::MainThreadMarker,
    /// The wrapper's own view — the window the alert presents from.
    host: Retained<HostView>,
    /// The mounted content — the dialog presents beside it.
    child: Mounted,
    /// The environment handlers resolve in — the environment the wrapper
    /// was rendered under.
    env: Environment,
    /// The dialog: binding, actions, and the cancel path.
    dialog: Dialog,
    /// The presentation binding — the watcher that drives the alert.
    is_presented: Binding<bool>,
    /// The reactive title, kept live while the alert is up.
    title: Computed<StyledStr>,
    /// The reactive message, when the dialog declared one.
    message: Option<Computed<StyledStr>>,
    /// The platform alert while presented; `None` once it ends.
    #[cfg(target_os = "macos")]
    alert: RefCell<Option<SheetAlert>>,
    /// The platform alert while presented; `None` once it ends.
    #[cfg(target_os = "ios")]
    alert: RefCell<Option<AlertController>>,
}

impl core::fmt::Debug for DialogState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DialogState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: transparent — every answer the child's.
struct DialogSubView {
    /// The leaf's state.
    state: Rc<DialogState>,
}

impl core::fmt::Debug for DialogSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DialogSubView").finish_non_exhaustive()
    }
}

impl SubView for DialogSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.child.layout().priority()
    }

    fn is_empty(&self) -> bool {
        self.state.child.layout().is_empty()
    }
}

/// The actions `run_action` dispatches on: the platform order — `AppKit`'s
/// most-to-least-prominent add order (primary first) — so a button index
/// always reaches the declared action.
#[cfg(target_os = "macos")]
struct ActionRun {
    /// The resolved actions in the order they were handed to the platform.
    ordered: Vec<DialogAction>,
}

#[cfg(target_os = "macos")]
fn present(state: &Rc<DialogState>) {
    let Some(window) = window_of(&state.host) else {
        // Not in a window yet — nothing to sheet against; the binding stays
        // true and the next present call retries, matching the
        // anchored-overlay's early-window contract.
        return;
    };
    let title = state.title.snapshot().to_semantic().to_string();
    let message = state
        .message
        .as_ref()
        .map(|message| message.snapshot().to_semantic().to_string());
    let alert = SheetAlert::new(state.mtm, &title, message.as_deref());

    // AppKit ordering: the primary — the first Default — is added first so
    // it is the rightmost button bound to Return; every other action
    // follows in resolved order (Cancel, then Destructive, then the
    // remaining Defaults), landing leftward.
    let resolved = state.dialog.resolved_actions();
    let primary = resolved
        .iter()
        .position(|action| action.role() == DialogRole::Default);
    let mut ordered = Vec::with_capacity(resolved.len());
    if let Some(primary) = primary {
        ordered.push(resolved[primary].clone());
    }
    ordered.extend(
        resolved
            .iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != primary)
            .map(|(_, action)| action.clone()),
    );

    let actions = Rc::new(ActionRun { ordered });
    for action in &actions.ordered {
        let button = alert.add_button(
            action
                .title()
                .resolve(&state.env)
                .content
                .snapshot()
                .to_semantic()
                .as_ref(),
        );
        match action.role() {
            DialogRole::Cancel => {
                button.setKeyEquivalent(&NSString::from_str("\u{1b}"));
            }
            DialogRole::Destructive => button.setHasDestructiveAction(true),
            DialogRole::Default => {}
        }
    }
    alert.begin_sheet(&window, {
        let state = Rc::downgrade(state);
        let actions = Rc::clone(&actions);
        move |index| {
            let Some(state) = state.upgrade() else {
                return;
            };
            let Some(action) = actions.ordered.get(index) else {
                return;
            };
            state.alert.borrow_mut().take();
            state.dialog.run_action(action, &state.env);
        }
    });
    *state.alert.borrow_mut() = Some(alert);
}

#[cfg(target_os = "ios")]
fn present(state: &Rc<DialogState>) {
    let title = state.title.snapshot().to_semantic().to_string();
    let message = state
        .message
        .as_ref()
        .map(|message| message.snapshot().to_semantic().to_string());
    let controller = AlertController::new(state.mtm, &title, message.as_deref());

    // UIKit takes the resolved order verbatim — the platform lays Cancel
    // out separately and emboldens the preferred action, pinned to the
    // first Default.
    let resolved = state.dialog.resolved_actions();
    let mut preferred = None;
    for action in &resolved {
        let style = match action.role() {
            DialogRole::Default => UIAlertActionStyle::Default,
            DialogRole::Cancel => UIAlertActionStyle::Cancel,
            DialogRole::Destructive => UIAlertActionStyle::Destructive,
        };
        let title = action
            .title()
            .resolve(&state.env)
            .content
            .snapshot()
            .to_semantic()
            .to_string();
        let run = {
            let state = Rc::downgrade(state);
            let action = action.clone();
            move || {
                let Some(state) = state.upgrade() else {
                    return;
                };
                state.alert.borrow_mut().take();
                state.dialog.run_action(&action, &state.env);
            }
        };
        let platform_action = controller.add_action(&title, style, run);
        if preferred.is_none() && action.role() == DialogRole::Default {
            preferred = Some(platform_action);
        }
    }
    if let Some(preferred) = preferred {
        controller.set_preferred(&preferred);
    }
    if !controller.present(&state.host) {
        return;
    }
    *state.alert.borrow_mut() = Some(controller);
}

/// Ends a live presentation — the binding wrote `false` without a button.
fn dismiss(state: &Rc<DialogState>) {
    if let Some(alert) = state.alert.borrow_mut().take() {
        alert.dismiss();
    }
}

/// Pushes the current resolved texts into a live alert — `text!` content
/// staying reactive while the dialog is up.
fn refresh_texts(state: &Rc<DialogState>) {
    let alert_slot = state.alert.borrow();
    let Some(alert) = alert_slot.as_ref() else {
        return;
    };
    let title = state.title.snapshot().to_semantic().to_string();
    alert.set_title(&title);
    let message = state
        .message
        .as_ref()
        .map(|message| message.snapshot().to_semantic().to_string());
    alert.set_message(message.as_deref());
}

/// Installs the `dialog` handler on the dispatcher: `Metadata<Dialog>` maps
/// to a transparent container that presents the platform alert while the
/// binding reads `true`.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<Dialog>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, KitRect::ZERO);
        let host_view: &PlatformView = &host;

        let mounted = ctx.render(metadata.content).mount(host_view);
        view::set_translates_autoresizing(mounted.view(), true);
        crate::primary_content::forward(&host, mounted.view());

        let dialog = metadata.value;
        let state = Rc::new(DialogState {
            mtm,
            host: host.clone(),
            child: mounted,
            env: ctx.env().clone(),
            is_presented: dialog.is_presented().clone(),
            title: dialog.title().resolve(ctx.env()).content,
            message: dialog
                .message_text()
                .map(|message| message.resolve(ctx.env()).content),
            dialog,
            alert: RefCell::new(None),
        });

        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host| {
                view::set_frame(state.child.view(), view::bounds(host));
            }
        });
        let sink_guard = proposal::register_sink(host_view, {
            let state = Rc::clone(&state);
            move |selected| {
                proposal::deliver(state.child.view(), selected);
            }
        });

        let mut leaf = NativeLeaf::new(
            host_view,
            DialogSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);
        leaf.keep(Rc::clone(&state));

        leaf.watch(&state.is_presented, {
            let state = Rc::downgrade(&state);
            move |ctx| {
                let Some(state) = state.upgrade() else {
                    return;
                };
                if *ctx.value() {
                    present(&state);
                } else {
                    dismiss(&state);
                }
            }
        });
        leaf.watch(&state.title, {
            let state = Rc::downgrade(&state);
            move |_ctx| {
                if let Some(state) = state.upgrade() {
                    refresh_texts(&state);
                }
            }
        });
        if let Some(message) = &state.message {
            leaf.watch(message, {
                let state = Rc::downgrade(&state);
                move |_ctx| {
                    if let Some(state) = state.upgrade() {
                        refresh_texts(&state);
                    }
                }
            });
        }

        if state.is_presented.snapshot() {
            present(&state);
        }

        leaf
    });
}
